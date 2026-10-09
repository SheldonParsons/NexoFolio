#!/usr/bin/env python3
"""Explicit bounded paid comparison. Secrets stay in memory; output is aggregate only."""
import argparse,json,os,re,subprocess,urllib.parse
from pathlib import Path
root=Path(__file__).resolve().parent.parent
parser=argparse.ArgumentParser()
parser.add_argument('--effort',choices=['high','max'],required=True)
parser.add_argument('--cases',required=True,help='Comma-separated field-00..11 or stage-0..2')
parser.add_argument('--output-name',required=True)
args=parser.parse_args()
ids=args.cases.split(',')
assert 0<len(ids)<=15 and len(ids)==len(set(ids))
assert all(re.fullmatch(r'(field-(0[0-9]|1[01])|stage-[012])',i) for i in ids)
assert re.fullmatch(r'[a-z0-9-]+',args.output_name)
output=root/'experiments/results'/args.output_name
assert not output.exists(),'Output exists; no duplicate paid run'
def runtime():
 pid=subprocess.check_output(['lsof','-nP','-iTCP:18080','-sTCP:LISTEN','-t'],text=True).strip()
 command=subprocess.check_output(['ps','eww','-p',pid,'-o','command='],text=True)
 matches=list(re.finditer(r'(?:^|\s)([A-Za-z_][A-Za-z0-9_]*)=',command))
 settings={m.group(1):command[m.end():matches[i+1].start() if i+1<len(matches) else len(command)].strip() for i,m in enumerate(matches) if m.group(1)=='DATABASE_URL' or m.group(1).startswith('NEXOFOLIO_')}
 assert all(settings.get(k) for k in ['DATABASE_URL','NEXOFOLIO_CATALOG_MODEL_BASE_URL','NEXOFOLIO_CATALOG_MODEL_API_KEY'])
 safe={k:settings.get(k) for k in ['NEXOFOLIO_CATALOG_MODEL','NEXOFOLIO_MODEL_THINKING','NEXOFOLIO_MODEL_JSON_OUTPUT']}
 return pid,settings,safe
def state():
 q="SET default_transaction_read_only=on; SELECT json_build_object('active',(SELECT count(*) FROM knowledge.candidates WHERE state='running'),'candidate_hash',md5(row_to_json(c)::text),'candidate_state',c.state) FROM knowledge.candidates c WHERE c.id='a38d759a-05c7-4d3d-8485-ffb3b7b53a20';"
 r=subprocess.run(['docker','exec','-i','nexofolio-local-db-1','sh','-c','psql -X -qAt -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d nexofolio'],input=q,text=True,capture_output=True)
 if r.returncode:raise SystemExit('Read-only state query failed')
 return json.loads(r.stdout.strip())
def private_write(path,data):
 fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
 with os.fdopen(fd,'w') as f:json.dump(data,f,indent=2)
pid,settings,safe=runtime();before=state()
assert before['active']==0 and before['candidate_state']=='ready','An active round or changed candidate exists; comparison not started'
u=urllib.parse.urlsplit(settings['DATABASE_URL']);qs=urllib.parse.parse_qsl(u.query);qs.append(('options','-c default_transaction_read_only=on'))
env=os.environ.copy();env.update(settings)
env.update(READONLY_DATABASE_URL=urllib.parse.urlunsplit(u._replace(query=urllib.parse.urlencode(qs))),READONLY_PROJECT_ID='57a3a86b-2fe4-4d4e-95b9-64a1a086b7a7',READONLY_CANDIDATE_ID='a38d759a-05c7-4d3d-8485-ffb3b7b53a20',BENCHMARK_EFFORT=args.effort,BENCHMARK_CASE_IDS=args.cases,BENCHMARK_ALLOW_PAID=str(len(ids)),BENCHMARK_OUTPUT=str(output),BENCHMARK_SOURCE=str(root/'experiments/results'))
log=root/'experiments/results'/(args.output_name+'.log')
fd=os.open(log,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
print('Starting bounded comparison:',args.effort,'calls limit',len(ids),'concurrency 3; no retry; no service/configuration writes',flush=True)
with os.fdopen(fd,'w') as f:
 r=subprocess.run(['cargo','test','-q','-p','nexofolio-backend','--lib','model_effort_benchmark_tests::exact_prompt_flash_effort_comparison','--','--ignored','--nocapture'],cwd=root,env=env,stdout=f,stderr=subprocess.STDOUT)
after=state();current_pid,_,current_safe=runtime()
if output.exists():private_write(output/'state-verification.json',{'candidate_unchanged':before['candidate_hash']==after['candidate_hash'],'active_before':before['active'],'active_after':after['active'],'service_pid_unchanged':pid==current_pid,'runtime_settings_unchanged':safe==current_safe,'settings':current_safe})
print('Comparison exit',r.returncode,'candidate unchanged',before['candidate_hash']==after['candidate_hash'],'runtime settings unchanged',safe==current_safe)
raise SystemExit(r.returncode)
