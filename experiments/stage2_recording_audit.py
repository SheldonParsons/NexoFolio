"""Stages 2–4 fixed-recording audit. Creates/removes its own PostgreSQL container only.
Run from any directory: python3 experiments/stage2_recording_audit.py
Requires Docker, Cargo and the local ignored fixed recording. Only --model-run provider explicitly enables model calls; platform URLs are never called.
Real replay and deliberately synthetic boundary probes are reported separately.
"""
import argparse,subprocess,os,time,json,uuid,tarfile,hashlib,collections,tempfile
from pathlib import Path
parser=argparse.ArgumentParser();parser.add_argument('--stage',choices=['2','3','4'],default='2');parser.add_argument('--model-run',choices=['none','fixture','provider'],default='none');parser.add_argument('--output');parser.add_argument('--dataset');parser.add_argument('--preserve-evidence',action='store_true');parser.add_argument('--fixture-read-all',action='store_true');parser.add_argument('--restore-dump');parser.add_argument('--resume-run');parser.add_argument('--review-low',action='store_true');parser.add_argument('--fixture-fail-first',action='store_true');parser.add_argument('--wall-seconds',type=int,default=480);args=parser.parse_args();stage=args.stage
assert stage=='4' or args.model_run=='none'
assert 1 <= args.wall_seconds <= 7200
assert not args.resume_run or (stage=='4' and args.restore_dump)
root=Path(__file__).resolve().parents[1];out=root/f'experiments/results/stage{stage}-audit';base=root/'backups/recording-followup-20260916-passive'
if args.output: out=Path(args.output).resolve()
if args.dataset: base=Path(args.dataset).resolve()
assert not args.preserve_evidence or stage=='4'
assert not args.fixture_read_all or args.model_run=='fixture'
out.mkdir(parents=True,exist_ok=True)
blob_temp=tempfile.TemporaryDirectory(prefix='stage2-blobs-',dir=out)
os.chdir(root);os.umask(0o077)
name='nexo-stage2-'+uuid.uuid4().hex[:8];password=uuid.uuid4().hex;results=[]

def run(args,**kw):return subprocess.run(args,check=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,**kw).stdout

def command(args,label,env=None):
 t=time.time()
 with (out/(label+'.log')).open('wb') as f:r=subprocess.run(args,env=env,stdout=f,stderr=subprocess.STDOUT)
 results.append({'check':label,'exit_code':r.returncode,'seconds':round(time.time()-t,2)})
 (out/'checks.json').write_text(json.dumps(results,indent=2)+'\n');print(label,r.returncode,flush=True)
 assert r.returncode==0,label

def sql(q,read=True):
 text=('BEGIN READ ONLY;'+q+';COMMIT;') if read else q
 data=run(['docker','exec','-i',name,'psql','-X','-U','audit','-d','stage2_audit','-Atq','-v','ON_ERROR_STOP=1'],input=text.encode()).decode().strip()
 return json.loads(data) if data else None
try:
 command(['cargo','build','-p','nexofolio-backend','--example','recording_regression','--locked'],'build')
 for f,h in json.loads((base/'manifest.json').read_text())['files'].items():assert hashlib.sha256((base/f).read_bytes()).hexdigest()==h
 run(['docker','run','-d','--name',name,'-e','POSTGRES_USER=audit','-e','POSTGRES_DB=stage2_audit','-e','POSTGRES_PASSWORD='+password,'-p','127.0.0.1::5432','postgres:17-bookworm'])
 for _ in range(100):
  if subprocess.run(['docker','exec',name,'pg_isready','-h','127.0.0.1','-U','audit','-d','stage2_audit'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0:break
  time.sleep(.2)
 port=run(['docker','port',name,'5432/tcp']).decode().strip().rsplit(':',1)[1];url=f'postgres://audit:{password}@127.0.0.1:{port}/stage2_audit'
 (out/'runtime.private.json').write_text(json.dumps({'container':name}))
 with (Path(args.restore_dump) if args.restore_dump else base/'database.dump').open('rb') as f:run(['docker','exec','-i',name,'pg_restore','-U','audit','-d','stage2_audit','--no-owner','--no-privileges','--exit-on-error'],stdin=f)
 blobs=Path(blob_temp.name)
 with tarfile.open(base/'blobs.tar.gz') as tar:tar.extractall(blobs,filter='data')
 if stage=='4':
  # Preflight by default; real provider execution requires an explicit option.
  worker=json.loads(run(['docker','inspect','nexofolio-local-worker-1']))[0]
  config=dict(item.split('=',1) for item in worker['Config']['Env'])
  inp={'mode':'stage4','isolated_database':True,'database_url':url,'blob_root':str(blobs),'output_dir':str(out),
       'context_tokens':int(config['NEXOFOLIO_MAINTENANCE_CONTEXT_TOKENS']),
       'max_calls':int(config['NEXOFOLIO_MAINTENANCE_MAX_CALLS']),'model':config['NEXOFOLIO_CATALOG_MODEL']}
  inp['preserve_evidence']=args.preserve_evidence
  inp['fixture_read_all']=args.fixture_read_all
  inp['execute_fixture']=args.model_run=='fixture'
  inp['execute_model']=args.model_run=='provider'
  if args.resume_run:inp['resume_run']=args.resume_run
  inp['wall_timeout_seconds']=args.wall_seconds
  inp['review_low']=args.review_low
  inp['fixture_fail_first']=args.fixture_fail_first
  inp['model_timeout_ms']=int(config['NEXOFOLIO_MAINTENANCE_MODEL_TIMEOUT_MS'])
  if args.model_run=='provider':
   keyfile=next(m['Source'] for m in worker['Mounts'] if m['Destination']=='/run/secrets/catalog-model-key')
   if not Path(keyfile).exists() and keyfile.startswith('/host_mnt/'):keyfile=keyfile.removeprefix('/host_mnt')
   inp['model_key_file']=keyfile;inp['model_base_url']=config['NEXOFOLIO_CATALOG_MODEL_BASE_URL']
  proc=subprocess.run([str(root/'target/debug/examples/recording_regression')],input=json.dumps(inp).encode(),stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=args.wall_seconds+240)
  (out/'preflight.stderr.private.log').write_bytes(proc.stderr)
  assert proc.returncode==0,'stage4 preflight failed'
  (out/'preflight-result.json').write_bytes(proc.stdout)
  print(proc.stdout.decode(),flush=True)
  if args.model_run!='none':
   with (out/'isolated-result.dump').open('wb') as f:subprocess.run(['docker','exec',name,'pg_dump','-U','audit','-d','stage2_audit','-Fc','--no-owner','--no-privileges'],stdout=f,check=True)
  raise SystemExit(0)
 http=sql("""SELECT json_agg(json_build_object('raw_hash',e.raw_hash,'project_id',e.project_id,'field',json_build_object('interface_id',o.interface_id,'environment_id',e.environment_id,'revision_id',o.compared_revision_id,'location','','path',''),'path_identity',i.path_identity)) FROM capture_events e JOIN interface_observations o ON o.ingestion_id=e.ingestion_id JOIN ingestion_inbox i ON i.id=o.ingestion_id WHERE e.kind='http_exchange'""")
 files={f.name:f for f in blobs.rglob('*') if f.is_file()}
 for case in http:
  f=files.get(case['raw_hash']) or files.get(case['raw_hash']+'.json')
  assert f,'blob lookup'
  raw=f.read_bytes();assert hashlib.sha256(raw).hexdigest()==case['raw_hash'];case['payload']=json.loads(raw)
 pre=sql("SELECT json_build_object('events',(SELECT count(*) FROM capture_events),'interfaces',(SELECT count(*) FROM interface_documents),'ui',(SELECT count(*) FROM capture_events WHERE kind='ui_snapshot'),'relations',(SELECT count(*) FROM evidence_facts WHERE kind='parameter_link_candidate'))")
 (out/'input-summary.json').write_text(json.dumps(pre,indent=2)+'\n')
 ui=sql("SELECT json_agg(json_build_object('id',id,'kind',kind,'raw_hash',raw_hash)) FROM capture_events WHERE kind IN ('interaction','ui_snapshot')")
 ui_stats=collections.Counter()
 for e in ui:
  data=json.loads(files[e['raw_hash']].read_bytes())
  for target in data.get('elements',[data.get('target',{})]):
   ui_stats[e['kind']+':'+str(target.get('value',{}).get('state','missing'))]+=1
 (out/'ui-coverage.json').write_text(json.dumps(ui_stats,indent=2)+'\n')
 for order in (['arrival','reverse-arrival'] if stage=='2' else ['snapshot']):
  if stage=='2':sql("TRUNCATE evidence_samples,evidence_relation_pairs,evidence_relation_values,evidence_ui_pairs,evidence_sample_groups,evidence_ui_bindings,evidence_value_index,evidence_facts; UPDATE capture_events SET evidence_status='pending',attempts=0,retry_at=clock_timestamp(),lease_until=NULL; UPDATE capture_backlog b SET pending=(SELECT count(*) FROM capture_events e WHERE e.project_id=b.project_id);",False)
  if order=='reverse-arrival':sql("UPDATE capture_events SET received_at=to_timestamp(2000000000-extract(epoch from received_at));",False)
  inp={'mode':'stage'+stage,'isolated_database':True,'database_url':url,'blob_root':str(blobs),'http':http}
  t=time.time()
  proc=subprocess.run([str(root/'target/debug/examples/recording_regression')],input=json.dumps(inp).encode(),stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=args.wall_seconds+240)
  (out/(order+'.stderr.private.log')).write_bytes(proc.stderr)
  assert proc.returncode==0,'replay failed: '+order
  result=json.loads(proc.stdout);result['ordering']=order;result['seconds']=round(time.time()-t,2)
  (out/(order+'.json')).write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
  print(order,'audit complete',flush=True)
 if stage=='3':raise SystemExit(0)
 env=os.environ.copy();env['TEST_DATABASE_URL']=url;env['NEXOFOLIO_STAGE2_REPORT']=str(out/'synthetic-gaps.json')
 for key in ['NEXOFOLIO_KEEP_CAPTURE_FIXTURE','NEXOFOLIO_PLUGIN_SAMPLE_BATCH']:env.pop(key,None)
 command(['cargo','test','-p','nexofolio-evidence','--locked'],'evidence-unit',env)
 command(['cargo','test','-p','nexofolio-backend','--test','capture_evidence','--locked','--','--ignored','--nocapture'],'capture-integration',env)
finally:
 subprocess.run(['docker','rm','-f',name],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
 blob_temp.cleanup()
 print('Isolated database removed; main service unchanged.',flush=True)
