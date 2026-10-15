#!/usr/bin/env python3
"""Bounded acceptance on owned PG/API/Chrome; run only after dependency-gate.json passes."""
import concurrent.futures, hashlib, importlib.util, io, json, math, os, pathlib, shutil, subprocess, sys, threading, time, uuid
from urllib.request import Request, urlopen
from urllib.error import HTTPError
from frozen_input_harness import Harness, Upstream, ROOT, PG, encoded, ident, port
from mcp_read_fixture import literal
from frozen_input_fixture import letters
from step11_model_gate import Gate, benchmark_bodies, RUN, OUT

DB = 'nexofolio_step04_step11_60min'
NODE = '/Users/sheldon/.nvm/versions/node/v22.22.0/bin/node'
FRONT = ROOT.parent / 'NexoFolio-Front'
EVENT_LOCK = threading.Lock()
START = None
END = float('inf')
STAGE = 'preparation'
ATTEMPT = 1
STOP = threading.Event()
FAULT = False
PAID = False
GATE = None
CAPTURE = []
original_post = Upstream.do_POST


def save(name, value):
    (OUT/name).write_text(json.dumps(value, ensure_ascii=False, indent=2)+'\n')


def event(name, **values):
    row = dict(event=name, stage=STAGE, attempt=ATTEMPT, wall=time.time(), mono=time.monotonic(), elapsed=None if START is None else time.monotonic()-START, **values)
    with EVENT_LOCK:
        with (OUT/'events.jsonl').open('a') as f: f.write(encoded(row)+'\n'); f.flush()


def identity_get(self):
    if self.path.startswith('/api.php/v1/user'):
        account = self.headers.get('Token','A80646'); n = int(account.split('-')[-1]) if account.startswith('reader-') else 1
        self.reply({'profile':dict(id=n,account=account,realname='第十一步合成用户',deleted='0',view={'projects':'11,12,13,14,15'})})
    elif self.path.startswith('/api.php/v1/projects'):
        self.reply(dict(page=1,limit=100,total=5,projects=[dict(id=n,name='Step11 synthetic '+str(n),status='doing') for n in range(11,16)]))
    else: self.reply({},404)


def upstream_post(self):
    wire=self.rfile.read(int(self.headers.get('Content-Length','0'))); body=json.loads(wire)
    if self.path.endswith('/tokens'): self.reply({'token':body['account']}); return
    if self.path != '/v1/chat/completions': self.reply({},404); return
    prompt=body['messages'][0]['content']; kind='fields' if '字段和可核查的证据：' in prompt else 'organize' if '接口列表:\n' in prompt else 'describe'
    CAPTURE.append(dict(stage=kind,phase=STAGE,mono=time.monotonic(),mode='paid' if PAID else 'local_503' if FAULT and kind=='fields' and 'retry_marker' in prompt else 'local_success',sha256=hashlib.sha256(wire).hexdigest(),bytes=len(wire)))
    if PAID:
        try: status,value=GATE.forward(body,'full')
        except Exception as error: status,value=503,{'error':{'code':type(error).__name__}}
        self.reply(value,status); return
    if FAULT and kind=='fields' and 'retry_marker' in prompt:
        event('injected_503',model_stage=kind); self.reply({'error':'synthetic upstream unavailable'},503); return
    self.rfile=io.BytesIO(wire); Upstream.mode='success'; original_post(self)


class Bench(Harness):
    def env(self, db, api_port):
        cfg=super().env(db,api_port)
        cfg.update(NEXOFOLIO_CATALOG_MODEL=GATE.cfg['NEXOFOLIO_CATALOG_MODEL'], NEXOFOLIO_MODEL_MAX_CONCURRENCY='8',
                   NEXOFOLIO_MODEL_THINKING='high',NEXOFOLIO_MODEL_JSON_OUTPUT='true',NEXOFOLIO_DB_MAX_CONNECTIONS='5',
                   NEXOFOLIO_COLLECT_IN_FLIGHT='4')
        return cfg

    def create(self, db, template=None):
        assert template is None
        subprocess.run([str(PG/'createdb'),'-h','127.0.0.1','-p',str(self.pg_port),'-E','UTF8','-T','template0',db],check=True)
        with (OUT/'migration.log').open('w') as log: subprocess.run([str(self.admin),'migrate'],env=self.env(db,0),stdout=log,stderr=log,check=True)

    def request(self, path, body=None):
        status,value=super().request(path,body)
        if path=='/v1/projects' and status==200: value['items'].sort(key=lambda x:x['name'])
        return status,value


def http(h,path,body=None,token=None,label=None):
    if START is not None and time.monotonic()>=END: raise RuntimeError('formal_deadline')
    headers={'Content-Type':'application/json','Authorization':'Bearer '+(token or h.token),'Accept':'application/json, text/event-stream','MCP-Protocol-Version':'2025-11-25'}
    started=time.monotonic()
    try:
        try: response=urlopen(Request(h.url+path,data=encoded(body).encode() if body is not None else None,headers=headers),timeout=30)
        except HTTPError as e: response=e
        with response:
            first=time.monotonic();raw=response.read();status=response.status;retry=response.headers.get('Retry-After')
        value=json.loads(raw) if raw else None
    except Exception as error:
        event('request_error',route=label or path,error=type(error).__name__);raise
    event('http',route=label or path,status=status,bytes=len(raw),ttfb_ms=(first-started)*1000,total_ms=(time.monotonic()-started)*1000,retry_after=retry)
    return status,value,float(retry or 1)


def batch(project,name,path='/capacity/a-fixture',fields=None):
    record=dict(id=ident('step11-record-'+name),kind='http_exchange',version=1,observed_at='2026-10-14T00:00:00Z',
                payload=dict(request=dict(method='POST',url='https://synthetic.example.invalid'+path,
                   body=dict(state='full',media_type='application/json',encoding='utf8',content=encoded(fields or {'value':0,'remark':'Synthetic test only'})))))
    return dict(batch_id=ident('step11-batch-'+name),platform='nexofolio-step11-synthetic',target=dict(project_id=project,environment=dict(name='合成环境')),records=[record])


def database(h):
    return json.loads(h.sql(DB,"SELECT jsonb_build_object('ledger',(SELECT count(*) FROM intake.batches),'accepted',(SELECT coalesce(sum((receipt->>'accepted')::int),0) FROM intake.batches),'calls',(SELECT coalesce(sum(calls),0) FROM observe.fingerprints),'tasks',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',id,'project_id',project_id,'candidate_id',candidate_id,'state',state,'generation',generation,'next_retry_at',next_retry_at,'updated_at',updated_at)), '[]') FROM knowledge.curation_tasks),'candidates',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',id,'project_id',project_id,'state',state,'batches',jsonb_array_length(batches),'batch_hash',md5(batches::text))), '[]') FROM knowledge.candidates),'active_connections',(SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND state='active'))"))


def checkpoint(h,project):
    return json.loads(h.sql(DB,f"SELECT coalesce(jsonb_agg(jsonb_build_object('id',id,'state',state,'batches',batches)),'[]') FROM knowledge.candidates WHERE project_id='{project}'"))


def seed(h):
    spec=importlib.util.spec_from_file_location('review_fixture',ROOT/'scripts/test-review-apply.py');review=importlib.util.module_from_spec(spec);spec.loader.exec_module(review);review.DB=DB
    large=review.seed(h,101)
    projects={p['name'].split()[-1]:p['project_id'] for p in h.request('/v1/projects')[1]['items']}
    pressure=projects['12']
    for offset in range(0,101,50):
        b=batch(pressure,'seed-pressure-batch-'+str(offset))
        b['records']=[batch(pressure,'seed-pressure-'+str(n),'/capacity/'+letters(n)+'-fixture',{'remark':'Synthetic test only','value':n})['records'][0] for n in range(offset,min(101,offset+50))]
        status,receipt=h.request('/v1/collect/batches',b);assert status==200 and receipt['accepted']==len(b['records'])
    h.sql(DB,f"INSERT INTO knowledge.field_notes(endpoint_id,project_id,path,text,author,context) SELECT id,project_id,'request_body:remark','Synthetic test remark',jsonb_build_object('by','person','user',(SELECT id FROM access.users LIMIT 1)), '{{\"location\":{{\"in\":\"request_body\"}},\"field_path\":\"remark\"}}' FROM observe.endpoints WHERE project_id='{pressure}'")
    for n,field in [('13','small_fixture'),('14','retry_marker'),('15','healthy_marker')]:
        status,v=h.request('/v1/collect/batches',batch(projects[n],'tiny-'+n,'/synthetic/'+field,{field:101}));assert status==200
    environment=h.sql(DB,f"SELECT id FROM access.environments WHERE project_id='{pressure}' LIMIT 1")
    status,mcp=h.request('/v1/auth/mcp-tokens',{'name':'step11 synthetic MCP'});assert status==201
    meta=dict(projects=projects,pressure=pressure,environment=environment,large=large)
    meta['large'].pop('base',None)
    save('fixture.json',meta)
    h.mcp=mcp['token']; return meta


def task(h,project):
    status,value,_=http(h,f'/v1/projects/{project}/curate',{},label='curation_submit');assert status==202
    event('task_accepted',project=project,receipt=value)
    return value


def wait_until(seconds):
    while not STOP.is_set() and time.monotonic()<min(START+seconds,END): STOP.wait(min(1,max(0,START+seconds-time.monotonic())))


def monitor(h):
    while not STOP.is_set() and time.monotonic()<END:
        try:
            if hasattr(h,'browser') and h.browser.poll() not in [None,0]:
                GATE.enabled=False;STOP.set();event('blocking_browser_exit');break
            pids=[h.pg_pid]+([h.api.pid] if h.api else [])
            raw=subprocess.check_output(['ps','-axo','pid=,ppid=,rss=,%cpu=,time=,comm='],text=True)
            rows=[]
            for line in raw.splitlines():
                cols=line.strip().split(None,5)
                if int(cols[0]) in pids or int(cols[1])==h.pg_pid:rows.append(cols)
                if pathlib.Path(cols[-1]).name in ['cargo','rustc','clang','cc1','swift-frontend']:
                    event('external_compiler_observed',pid=cols[0],program=pathlib.Path(cols[-1]).name)
            snap=database(h)
            status,ops,_=http(h,'/v1/operations/summary',label='observer_summary')
            event('resources',processes=rows,database=snap,operations=ops,status=status)
        except Exception as e: event('monitor_gap',error=type(e).__name__)
        STOP.wait(5)


def mcp_read(h,meta):
    query=dict(project_id=meta['pressure'],environment_id=meta['environment'],query='remark',search_in='fields',limit=20)
    body=dict(jsonrpc='2.0',id=1,method='tools/call',params=dict(name='search_interfaces',arguments=query))
    status,value,_=http(h,'/mcp',body,h.mcp,'original_mcp_search')
    assert status==200 and 'error' not in value and not value['result'].get('isError'),('mcp',status)
    value=value['result']; doc=value.get('structuredContent') or json.loads(value['content'][0]['text'])
    assert doc['total']==101 and len(doc['items'])==20,('mcp_hits',doc.get('total'))
    jump=doc['items'][0]['matched_field']['open'];body['params']=dict(name=jump['tool'],arguments=jump['arguments'])
    status,value,_=http(h,'/mcp',body,h.mcp,'original_mcp_field');assert status==200 and not value.get('error') and not value['result'].get('isError')


def load(h,meta,round_no,users,deadline,receipts,pending):
    large=meta['large'];base=f"/v1/projects/{large['project']}/curate/candidates/{large['candidate']}"
    def worker(n):
        seq=0;wire=None;next_at=0
        try:
            while not STOP.is_set() and time.monotonic()<deadline:
                for path,name in [('/v1/auth/me','original_me'),(f"/v1/projects/{meta['pressure']}/endpoints",'original_endpoints'),(f"/v1/projects/{meta['pressure']}/catalog",'original_catalog'),(f"/v1/projects/{meta['pressure']}/document",'document_index'),(f"/v1/projects/{meta['projects']['15']}/curate",'original_curation_progress')]:
                    if time.monotonic()>=deadline:break
                    assert http(h,path,label=name)[0]==200,name
                if time.monotonic()>=deadline:break
                if time.monotonic()>=next_at:
                    if wire is None:wire=batch(meta['pressure'],f'load-{round_no}-{users}-{n}-{seq}');pending[wire['batch_id']]=wire
                    status,v,retry=http(h,'/v1/collect/batches',wire,label='original_collect')
                    if status==429:next_at=time.monotonic()+max(1,retry)+n%5*.1
                    else:
                        assert status==200 and v['batch_id']==wire['batch_id'] and v['accepted']==1 and not v['rejected'],'receipt'
                        receipts[wire['batch_id']]=v;pending.pop(wire['batch_id'],None);wire=None;seq+=1
                mcp_read(h,meta)
                status,head,_=http(h,base+'/review',label='candidate_head');assert status==200 and head['view']['fields']==[]
                status,v,_=http(h,base+'/review?part=changes&section=fields&version='+str(head['version']),label='candidate_diff');assert status==200 and len(v['items'])==60
                if n==0:assert http(h,'/v1/operations/tasks',label='management_tasks')[0]==200
                STOP.wait(min(1,max(0,deadline-time.monotonic())))
        except Exception as e:
            event('blocking_load_error',worker=n,error=type(e).__name__,detail=str(e)[:200]);STOP.set()
    with concurrent.futures.ThreadPoolExecutor(max_workers=users) as pool:list(pool.map(worker,range(users)))


def build_front(h):
    # Exactly one final build with an empty envDir. No user Vite server is used.
    dist=h.tmp/'front-dist';config=h.tmp/'vite.mjs'
    config.write_text("import vue from '"+str(FRONT/'node_modules/@vitejs/plugin-vue/dist/index.mjs')+"';export default {root:"+json.dumps(str(FRONT))+",envDir:"+json.dumps(str(h.tmp))+",plugins:[vue()],resolve:{alias:{'@':"+json.dumps(str(FRONT/'src'))+"}},build:{outDir:"+json.dumps(str(dist))+",emptyOutDir:true},define:{'import.meta.env.VITE_WORKSPACE_ENABLED':'true'}}")
    with (OUT/'frontend-build.log').open('w') as log:subprocess.run([NODE,str(FRONT/'node_modules/vite/bin/vite.js'),'build','--config',str(config)],cwd=FRONT,stdout=log,stderr=log,env={'PATH':str(pathlib.Path(NODE).parent)+':'+os.environ['PATH']},check=True)
    save('frontend-assets.json',{str(p.relative_to(dist)):hashlib.sha256(p.read_bytes()).hexdigest() for p in dist.rglob('*') if p.is_file()})
    import mimetypes
    from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
    class Front(BaseHTTPRequestHandler):
        def log_message(self,*a):pass
        def do_GET(self):self.serve()
        def do_POST(self):self.serve()
        def serve(self):
            if self.path.startswith('/api/'):
                wire=self.rfile.read(int(self.headers.get('Content-Length','0'))) if self.command=='POST' else None
                try:r=urlopen(Request(h.url+self.path[4:],data=wire,headers={k:v for k,v in self.headers.items() if k.lower() in ['authorization','content-type']},method=self.command),timeout=120)
                except HTTPError as e:r=e
                except OSError:self.send_error(503);return
                self.send_response(r.status);self.send_header('Content-Type',r.headers.get('Content-Type','application/json'));self.end_headers()
                try:self.wfile.write(r.read())
                except (BrokenPipeError,ConnectionResetError):pass
            else:
                p=dist/self.path.split('?')[0].lstrip('/');p=p if p.is_file() else dist/'index.html'
                self.send_response(200);self.send_header('Content-Type',mimetypes.guess_type(str(p))[0] or 'application/octet-stream');self.end_headers();self.wfile.write(p.read_bytes())
    server=ThreadingHTTPServer(('127.0.0.1',0),Front);server.daemon_threads=True;threading.Thread(target=server.serve_forever,daemon=True).start()
    return server


def main():
    global START,END,STAGE,FAULT,PAID,GATE,ATTEMPT
    assert '--run' in sys.argv,'Explicit --run required; no implicit paid request or formal window'
    gate=json.loads((OUT/'dependency-gate.json').read_text());assert gate['passed'] and gate['dependency_complete']
    continuing='--continue-within-window' in sys.argv
    if continuing:
        old=json.loads((OUT/'window.json').read_text());START=old['start_monotonic'];END=old['deadline_monotonic'];ATTEMPT=2
        assert END-time.monotonic()>2550,'Insufficient original window remaining; do not extend'
        with (OUT/'continuation.json').open('x') as marker:marker.write(encoded(dict(start_wall=time.time(),start_monotonic=time.monotonic(),original_deadline=END,reason='driver integer version concatenation',paid_allowed=False)))
    else:assert not (OUT/'window.json').exists(),'One formal window only; never silently restart the allowance/window'
    for p,sha in gate['binary_hashes'].items():assert hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()==sha
    for p,sha in gate['production_hashes'].items():assert hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()==sha,p
    GATE=Gate();OUT.mkdir(exist_ok=True);pathlib.Path('/tmp/nexofolio-step04').mkdir(exist_ok=True)
    Upstream.do_GET,Upstream.do_POST=identity_get,upstream_post
    h=Bench(OUT,ROOT/'target/debug/nexofolio-admin');server=None;browser=None;monitor_thread=None
    result={'run':RUN,'formal_complete':False,'continuous_window':not continuing,'attempt':ATTEMPT};receipts={};pending={}
    try:
        frozen_api=h.tmp/'frozen-api';shutil.copy2(ROOT/'target/debug/nexofolio-api',frozen_api)
        h.create(DB);h.start(DB,frozen_api,'initial');meta=seed(h);server=build_front(h)
        config=dict(front=f'http://127.0.0.1:{server.server_port}',project=meta['large']['project'],candidate=meta['large']['candidate'],token=h.token,out=str(OUT),tmp=str(h.tmp))
        (h.tmp/'browser.json').write_text(encoded(config));os.chmod(h.tmp/'browser.json',0o600)
        # Preflight verifies route contracts with no external model or performance load.
        for path in [f"/v1/projects/{meta['pressure']}/document",'/v1/operations/summary','/v1/operations/tasks']:assert h.request(path)[0]==200,path
        mcp_read(h,meta)
        review_base=f"/v1/projects/{meta['large']['project']}/curate/candidates/{meta['large']['candidate']}"
        status,head=h.request(review_base+'/review');assert status==200 and head['view']['fields']==[]
        status,diff=h.request(review_base+'/review?part=changes&section=fields&version='+str(head['version']));assert status==200 and len(diff['items'])==60
        task(h,meta['projects']['15']);preflight_deadline=time.monotonic()+45
        while time.monotonic()<preflight_deadline:
            checks=[t for t in database(h)['tasks'] if t['project_id']==meta['projects']['15']]
            if checks and checks[0]['state']=='done':break
            time.sleep(1)
        else:raise RuntimeError('controlled pipeline preflight did not complete')
        initial=database(h);save('database-initial.json',initial)
        save('runtime.json',dict(pg_port=h.pg_port,pg_pid=h.pg_pid,api_pid=h.api.pid,api_url=h.url,tmp=str(h.tmp),db=DB))
        compilers=subprocess.check_output(['ps','-axo','comm='],text=True)
        assert not any(pathlib.Path(x.strip()).name in ['cargo','rustc','cc1','clang','swift-frontend'] for x in compilers.splitlines()),'compiler load present'
        for p,sha in gate['binary_hashes'].items():assert hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()==sha
        for p,sha in gate['production_hashes'].items():assert hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()==sha,p
        if not continuing:
            START=time.monotonic();END=START+3599;STAGE='real-model';GATE.deadline=START+600;GATE.enabled=True
            save('window.json',dict(run=RUN,start_wall=time.time(),start_monotonic=START,deadline_monotonic=END,max_seconds=3600))
        else:
            GATE.enabled=False;GATE.deadline=0;STAGE='resume-preparation'
            result['real_model_evidence']='attempt1/result.json; persistent llm-ledger.jsonl unchanged'
        event('formal_resume' if continuing else 'formal_start');monitor_thread=threading.Thread(target=monitor,args=(h,));monitor_thread.start()
        browser=subprocess.Popen([NODE,str(ROOT/'scripts/step11-browser.cjs'),str(h.tmp/'browser.json')],stdout=(OUT/'browser.log').open('w'),stderr=subprocess.STDOUT)
        h.browser=browser
        for _ in range(40):
            if (h.tmp/'browser-ready').exists():break
            assert browser.poll() is None,'initial real browser failed'
            time.sleep(1)
        else:raise RuntimeError('initial real browser not ready')
        if not continuing:
            duplicate=batch(meta['projects']['13'],'tiny-13','/synthetic/small_fixture',{'small_fixture':101})
            first=http(h,'/v1/collect/batches',duplicate,label='duplicate_collect')[1]
            second=http(h,'/v1/collect/batches',duplicate,label='duplicate_collect')[1]
            assert first==second and first['accepted']==1 and database(h)['calls']==initial['calls'],'duplicate accounting'
            PAID=True;task(h,meta['projects']['13'])
            # Bound full workflow waiting; no retries initiated by this driver.
            while not STOP.is_set() and time.monotonic()<START+250:
                rows=[t for t in database(h)['tasks'] if t['project_id']==meta['projects']['13']]
                if rows and rows[0]['state'] in ['done','needs_attention','stopped']:break
                time.sleep(2)
            GATE.enabled=False
            result['real_full_flow']=database(h)
            full=[t for t in result['real_full_flow']['tasks'] if t['project_id']==meta['projects']['13']]
            if full and full[0]['state']=='done':
                status,review,_=http(h,f"/v1/projects/{meta['projects']['13']}/curate/candidates/{full[0]['candidate_id']}/review",label='real_model_review')
                assert status==200;save('real-model-review.json',review)
            if full and full[0]['state'] not in ['done','stopped']:
                http(h,f"/v1/projects/{meta['projects']['13']}/curate/tasks/{full[0]['id']}/stop",{},label='bounded_real_task_stop')
            PAID=False
            if STOP.is_set():raise RuntimeError('blocking basic check failure')
            # Same 8 inputs/options, 4 then 8; order/cache confounding is reported.
            GATE.enabled=True
            for concurrency in [4,8]:
                if time.monotonic()>START+450: result['model_comparison_incomplete']=True;break
                try:
                    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
                        values=list(pool.map(lambda b:GATE.forward(b,'concurrency-'+str(concurrency)),benchmark_bodies(GATE.cfg['NEXOFOLIO_CATALOG_MODEL'])))
                    result['model_status_'+str(concurrency)]=[x[0] for x in values]
                except RuntimeError:
                    result['model_comparison_incomplete']=True;break
            GATE.enabled=False;event('paid_gate_closed');wait_until(600)
        shift=max(0,time.monotonic()-(START+600)) if continuing else 0
        assert START+3000+shift<END-120,'Insufficient final verification time'
        event('remaining_schedule',shift_seconds=shift,deadline=END,paid_enabled=GATE.enabled)
        for r in range(3):
            if not STOP.is_set():task(h,meta['projects']['15'])
            for users in [5,10,20]:
                if STOP.is_set():break
                STAGE=f'load-r{r+1}-u{users}';event('segment_start',users=users,round=r+1)
                # Absolute endpoints avoid stretching the 27-minute schedule.
                index=r*3+[5,10,20].index(users);deadline=START+600+shift+(index+1)*180
                load(h,meta,r+1,users,deadline,receipts,pending);event('segment_end',users=users,round=r+1)
            if STOP.is_set():break
        if STOP.is_set():raise RuntimeError('blocking load failure; formal load stopped')
        STAGE='upstream-unavailable';FAULT=True;event('fault_start');task(h,meta['projects']['14']);wait_until(2240+shift);task(h,meta['projects']['15']);wait_until(2700+shift)
        if STOP.is_set():raise RuntimeError('blocking fault-phase check failure')
        STAGE='restart-recovery';before=database(h);save('before-restart.json',before)
        saved_before=checkpoint(h,meta['projects']['14']);save('checkpoint-before.json',saved_before)
        FAULT=False;event('fault_end')
        (h.tmp/'restarting').write_text('owned API only')
        h.stop();event('owned_api_stopped');h.start(DB,frozen_api,'restarted');event('owned_api_restarted',pid=h.api.pid)
        (h.tmp/'restarting').unlink()
        # Session/MCP credentials are persisted and API binary remains identical.
        wait_until(3000+shift);STAGE='drain-apply';save('recovery-50min.json',database(h))
        if STOP.is_set():raise RuntimeError('blocking recovery-phase check failure')
        # Drain original unacknowledged batches while honoring each new Retry-After.
        for bid,wire in list(pending.items()):
            if time.monotonic()>END-120:break
            status,v,retry=http(h,'/v1/collect/batches',wire,label='drain_collect')
            if status==429:time.sleep(max(1,retry));continue
            assert status==200 and v['accepted']==1;receipts[bid]=v;pending.pop(bid,None)
        save('receipts.json',receipts);save('pending.json',pending)
        large=meta['large'];base=f"/v1/projects/{large['project']}/curate/candidates/{large['candidate']}"
        (h.tmp/'apply-now').write_text('ready')
        deadline=END-60
        while time.monotonic()<deadline and not (OUT/'browser-apply.json').exists():time.sleep(1)
        assert (OUT/'browser-apply.json').exists(),'browser apply did not finish within planned allowance'
        apply=json.loads((OUT/'browser-apply.json').read_text());assert apply['passed'],apply
        official=int(h.sql(DB,f"SELECT count(*) FROM knowledge.field_notes WHERE project_id='{large['project']}'"))
        assert official==large['commands'],('formal count',official)
        # Compare full persisted bodies to candidate commands, not only row counts.
        mismatches=h.sql(DB,f"SELECT count(*) FROM knowledge.candidates c CROSS JOIN LATERAL jsonb_array_elements(c.view->'fields') f LEFT JOIN knowledge.field_notes n ON n.project_id=c.project_id AND n.endpoint_id=(f->>'endpoint')::uuid AND n.path=f->>'path' WHERE c.id='{large['candidate']}' AND (n.text IS DISTINCT FROM f->>'text' OR n.context IS DISTINCT FROM f->'context')")
        assert mismatches=='0',('formal bodies',mismatches)
        event('formal_apply_verified',fields=official,body_mismatches=int(mismatches))
        wait_until(3570);result['final_database']=database(h);save('database-final.json',result['final_database'])
        assert result['final_database']['ledger']-initial['ledger']==len(receipts),'ledger mismatch'
        assert result['final_database']['calls']-initial['calls']==len(receipts),'calls mismatch'
        assert result['final_database']['accepted']==result['final_database']['calls'],'accepted/calls mismatch'
        assert int(h.sql(DB,'SELECT count(*) FROM observe.seen_batches'))==result['final_database']['ledger'],'observe dedupe mismatch'
        result['pending_batches']=len(pending);result['receipts']=len(receipts)
        retry=[t for t in result['final_database']['tasks'] if t['project_id']==meta['projects']['14']]
        prior=[t for t in before['tasks'] if t['project_id']==meta['projects']['14']]
        result['recovery_same_task_candidate']=bool(retry and prior and retry[0]['id']==prior[0]['id'] and retry[0]['candidate_id']==prior[0]['candidate_id'] and retry[0]['state']=='done')
        assert result['recovery_same_task_candidate'],'same task recovery incomplete'
        saved_after=checkpoint(h,meta['projects']['14']);save('checkpoint-after.json',saved_after)
        from collections import Counter
        commands=lambda rows:Counter(encoded(c) for candidate in rows for b in candidate['batches'] for c in b['commands'])
        previous,current=commands(saved_before),commands(saved_after)
        assert previous and all(current[k]==v for k,v in previous.items()),'saved commands lost or duplicated'
        result['saved_commands_preserved']=sum(previous.values())
        result['source_stable']=all(hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()==sha for p,sha in gate['production_hashes'].items())
        assert result['source_stable'],'source changed during formal window'
        wait_until(3600);result['formal_complete']=True
    except Exception as error:
        result['error']=dict(type=type(error).__name__,detail=str(error)[:500]);event('run_stopped',**result['error'])
    finally:
        GATE.enabled=False;PAID=False;FAULT=False;STOP.set()
        (h.tmp/'stop-browser').write_text('stop new browser load')
        if START is not None:
            result.update(end_wall=time.time(),end_monotonic=time.monotonic(),elapsed=time.monotonic()-START);event('formal_end')
        if monitor_thread:monitor_thread.join(timeout=35)
        save('receipts.json',receipts);save('pending.json',pending)
        try:
            save('database-closeout.json',database(h))
            save('ledger-closeout.json',json.loads(h.sql(DB,"SELECT coalesce(jsonb_agg(jsonb_build_object('batch_id',batch_id,'receipt',receipt)),'[]') FROM intake.batches")))
            if 'meta' in locals():save('small-candidates-closeout.json',{n:checkpoint(h,meta['projects'][n]) for n in ['13','14','15']})
        except Exception as error:result['closeout_error']=type(error).__name__
        if browser and browser.poll() is None:
            (h.tmp/'stop-browser').write_text('stop')
            try:browser.wait(timeout=15)
            except subprocess.TimeoutExpired:browser.terminate();browser.wait(timeout=10)
        if server:server.shutdown()
        save('controlled-calls.json',CAPTURE)
        if (h.tmp/'pg.log').exists():shutil.copy2(h.tmp/'pg.log',OUT/'postgres.log')
        ports=[h.pg_port]+([int(h.url.rsplit(':',1)[1])] if hasattr(h,'url') else [])
        h.cleanup();result['cleanup']=dict(tmp_removed=not h.tmp.exists(),owned_ports=ports)
        save('result.json',result);print(encoded(result),flush=True)
        if not result['formal_complete']:sys.exit(1)


if __name__=='__main__':main()
