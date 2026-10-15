#!/usr/bin/env python3
"""Step 10 real PG/API/browser acceptance, only owned synthetic resources; no .env."""
import hashlib, json, os, pathlib, subprocess, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.request import Request, urlopen
from urllib.error import HTTPError
from frozen_input_harness import Harness, Upstream, ROOT, ident, encoded
from mcp_read_fixture import archives, literal, install_identity
OUT = ROOT / 'docs/audits/2026-10-14-storage-panel'
FRONT = ROOT.parent / 'NexoFolio-Front'
NODE = pathlib.Path('/Users/sheldon/.nvm/versions/node/v22.22.0/bin/node')
class Fixture(Harness):
    enabled = False
    def env(self, db, api_port):
        return {**super().env(db,api_port), 'NEXOFOLIO_RETENTION_ENABLED':'1' if self.enabled else '0'}
OUT.mkdir(parents=True,exist_ok=True)
pathlib.Path('/tmp/nexofolio-step04').mkdir(exist_ok=True)
install_identity()
h=Fixture(OUT, ROOT/'target/debug/nexofolio-admin')
db='nexofolio_step04_storage'; result={}; server=None
try:
    h.create(db); h.start(db,ROOT/'target/debug/nexofolio-api','storage-disabled')
    meta=h.seed(db,1); p=meta['project']; base=f'/v1/projects/{p}/storage'
    meta.update(target=h.sql(db,"SELECT id FROM observe.endpoints WHERE path_template='/synthetic/orders'"),environment=h.sql(db,'SELECT id FROM access.environments LIMIT 1'))
    meta['other_environment']=meta['environment']; ids=archives(h,db,meta)
    for version in ['2','3','4']:
        context=dict(observations=dict(archive_id=ids[version],total=1,hash='synthetic-field-hash',states={}))
        h.sql(db,f"UPDATE knowledge.field_notes SET context={literal(context)}::jsonb WHERE endpoint_id='{meta['target']}' AND path='request_body:manual{version}'")
    h.sql(db,"UPDATE knowledge.candidates SET created_at='2025-01-01',updated_at='2025-01-01'")
    snapshot=dict(endpoints=[meta['target']],folders=[],placements=[],notes=[],fields=[],links=[])
    def candidate(name,state,at):
        c=ident(name)
        h.sql(db,f"INSERT INTO knowledge.candidates(id,project_id,state,created_at,updated_at,base_hash,base,view,batches,progress,applied_rounds) VALUES('{c}','{p}','{state}','{at}','{at}','synthetic',{literal(snapshot)}::jsonb,{literal(snapshot)}::jsonb,'[]','{{}}','[]')")
        return c
    for n in range(5): candidate('storage-recent-'+str(n),'discarded','2026-10-14')
    stopped=candidate('storage-recoverable','interrupted','2025-01-01')
    h.sql(db,f"INSERT INTO knowledge.curation_tasks(id,project_id,candidate_id,actor,state,created_at,updated_at) VALUES('{stopped}','{p}','{stopped}','{{\"kind\":\"unknown\"}}','stopped','2025-01-01','2025-01-01')")
    def invariants():
        return h.sql(db,"SELECT jsonb_build_object('samples',(SELECT jsonb_agg(to_jsonb(x)) FROM observe.fingerprints x),'manual',(SELECT jsonb_agg(to_jsonb(x) ORDER BY endpoint_id,path) FROM knowledge.field_notes x),'lifecycle',(SELECT jsonb_agg(to_jsonb(x)) FROM knowledge.document_lifecycle x),'recoverable',(SELECT to_jsonb(x) FROM knowledge.candidates x WHERE id='"+stopped+"'))")
    before=invariants()
    assert h.request(base+'/maintenance',dict(action='execute'))[0]==400
    assert h.request(base+'/maintenance',dict(action='enable'))[0]==400
    assert h.request(base+'/maintenance',dict(action='preview'))[0]==200
    assert h.sql(db,'SELECT count(*) FROM knowledge.retention_projects')=='0'
    result['server_disabled']=True
    h.stop(); h.enabled=True; h.start(db,ROOT/'target/debug/nexofolio-api','storage')
    admin=h.token
    result['permissions']={}
    for label,mutation,restore in [
        ('anonymous',None,None),
        ('ordinary',"UPDATE access.users SET account='ordinary' WHERE lower(account)='a80646'","UPDATE access.users SET account='A80646' WHERE account='ordinary'"),
        ('disabled',"UPDATE access.users SET enabled=false","UPDATE access.users SET enabled=true"),
        ('revoked',"UPDATE access.internal_sessions SET revoked_at=now()","UPDATE access.internal_sessions SET revoked_at=NULL")]:
        if mutation: h.sql(db,mutation)
        else: h.token=None
        codes=[h.request(base)[0],h.request(base+'/refresh',{})[0],h.request(base+'/maintenance',dict(action='execute'))[0]]
        assert all(x in [401,403,404] for x in codes),(label,codes)
        result['permissions'][label]=codes
        if restore:h.sql(db,restore)
        h.token=admin
    h.sql(db,"UPDATE access.users SET account='a80646'")
    assert h.request(base)[0]==200
    h.sql(db,"UPDATE access.users SET account='A80646'")
    assert h.request(base+'/maintenance',dict(action='execute'))[0]==400
    first=h.request(base)[1]
    assert all(m['value'] is None for m in first['snapshot']['modules'])
    assert h.request(base+'/refresh',{})[0]==200
    def sampled(predicate=lambda x:all(m['value'] for m in x['snapshot']['modules'])):
        until=time.monotonic()+20
        while time.monotonic()<until:
            code,x=h.request(base);assert code==200
            if predicate(x):return x
            time.sleep(.15)
        raise AssertionError(('sample deadline',x))
    sample=sampled(); result['sample']=sample
    groups=[g for m in sample['snapshot']['modules'] for g in m['value']['groups']]
    raw=int(h.sql(db,f"SELECT sum(pg_column_size(f.sample)+pg_column_size(f.structure)) FROM observe.fingerprints f JOIN observe.endpoints e ON e.id=f.endpoint_id WHERE e.project_id='{p}'"))
    assert groups[0]['column_bytes']==raw and groups[0]['logical_bytes'] is None
    assert sample['snapshot']['retention']['value']['stopped_recoverable']==[stopped]
    for module in sample['snapshot']['modules']:
        for table in module['value']['tables']:
            physical=h.sql(db,f"SELECT pg_table_size('{table['name']}'),pg_indexes_size('{table['name']}')").split('|')
            assert [table['table_bytes'],table['index_bytes']]==list(map(int,physical))
    h.sql(db,"ALTER SYSTEM SET log_statement='all'; SELECT pg_reload_conf()")
    time.sleep(.2)
    log_path=h.tmp/'pg.log'; offset=log_path.stat().st_size
    timings=[]; cached=[]
    for _ in range(8):
        started=time.monotonic(); cached.append(h.request(base)[1]); timings.append((time.monotonic()-started)*1000)
    time.sleep(.2)
    statements=log_path.read_bytes()[offset:].decode()
    assert 'pg_column_size' not in statements and 'retention_projects' not in statements and 'UPDATE candidates' not in statements
    result['cached_get_sql']={'storage_statements':0,'latency_ms':timings}
    h.sql(db,"ALTER SYSTEM RESET log_statement; SELECT pg_reload_conf()")
    assert all(x==sample for x in cached), 'GET must return unchanged cached samples'
    assert h.sql(db,'SELECT count(*) FROM knowledge.retention_projects')=='0'
    result['cached_gets']=len(cached)
    assert h.request(base+'/maintenance',dict(action='preview'))[1]['report']['compacted']==0
    result['activation']=h.request(base+'/maintenance',dict(action='enable'))[1]
    assert result['activation']['report']['committed'] and result['activation']['report']['compacted']==0
    invalidated=h.request(base)[1]['snapshot']['retention']
    assert invalidated['error'] or invalidated['value']['enabled_at'] is not None
    result['maintenance_invalidates_cache']=True
    assert h.request(base+'/maintenance',dict(action='enable'))[0]==400
    protected=h.request(base+'/maintenance',dict(action='execute'))[1]
    assert protected['report']['skipped']=={'activation_grace_7_days':1}
    h.sql(db,f"UPDATE knowledge.retention_projects SET enabled_at='2025-01-01T00:00:00Z' WHERE project_id='{p}'")
    preview=h.request(base+'/maintenance',dict(action='preview'))[1]; result['preview']=preview
    assert preview['report']['compacted']==3 and not preview['report']['committed']
    assert h.sql(db,'SELECT count(*) FROM knowledge.candidates WHERE history_pruned_at IS NOT NULL')=='0'
    (OUT/'api-checkpoint.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
    # Browser gets the same project at the exact preview-before-execution point.
    config=h.tmp/'vite.config.mjs'; dist=h.tmp/'dist'
    config.write_text("import vue from '"+str(FRONT/'node_modules/@vitejs/plugin-vue/dist/index.mjs')+"'; export default { root:"+json.dumps(str(FRONT))+",envDir:"+json.dumps(str(h.tmp))+",plugins:[vue()],resolve:{alias:{'@':"+json.dumps(str(FRONT/'src'))+"}},build:{outDir:"+json.dumps(str(dist))+",emptyOutDir:true},define:{'import.meta.env.VITE_WORKSPACE_ENABLED':'true'}}")
    with (OUT/'frontend-build.log').open('w') as log:
        subprocess.run([str(NODE),str(FRONT/'node_modules/vite/bin/vite.js'),'build','--config',str(config)],cwd=FRONT,env={'PATH':str(NODE.parent)+':'+os.environ['PATH']},stdout=log,stderr=log,check=True)
    class Front(BaseHTTPRequestHandler):
        def log_message(self,*args):pass
        def do_GET(self):self.handle_request()
        def do_POST(self):self.handle_request()
        def handle_request(self):
            if self.path.startswith('/api/'):
                body=self.rfile.read(int(self.headers.get('Content-Length','0'))) if self.command=='POST' else None
                try:
                    r=urlopen(Request(h.url+self.path[4:],data=body,headers={k:v for k,v in self.headers.items() if k.lower() in ['authorization','content-type']},method=self.command),timeout=20)
                except HTTPError as e:r=e
                self.send_response(r.status); self.send_header('Content-Type',r.headers.get('Content-Type','application/json'));self.end_headers()
                try:self.wfile.write(r.read())
                except (BrokenPipeError,ConnectionResetError):pass
            else:
                import mimetypes
                file=dist/self.path.split('?')[0].lstrip('/')
                if not file.is_file():file=dist/'index.html'
                self.send_response(200);self.send_header('Content-Type',mimetypes.guess_type(str(file))[0] or 'application/octet-stream');self.end_headers();self.wfile.write(file.read_bytes())
    server=ThreadingHTTPServer(('127.0.0.1',0),Front);threading.Thread(target=server.serve_forever,daemon=True).start()
    # Ensure stale pre-activation cache has been replaced before browser actions.
    time.sleep(15);h.request(base+'/refresh',{})
    sampled(lambda x:x['snapshot']['retention']['value']['enabled_at']=='2025-01-01T00:00:00Z')
    other=h.sql(db,f"SELECT id FROM access.projects WHERE id<>'{p}' LIMIT 1")
    (h.tmp/'browser.json').write_text(json.dumps(dict(front=f'http://127.0.0.1:{server.server_port}',project=p,other=other,primary_name=h.sql(db,f"SELECT name FROM access.projects WHERE id='{p}'"),secondary_name=h.sql(db,f"SELECT name FROM access.projects WHERE id='{other}'"),token=h.token,out=str(OUT))))
    subprocess.run([str(NODE),str(ROOT/'scripts/test-storage-panel-browser.cjs'),str(h.tmp)],check=True)
    assert h.sql(db,'SELECT count(*) FROM knowledge.candidates WHERE history_pruned_at IS NOT NULL')=='3'
    assert invariants()==before,'protected sources, human knowledge and recoverable work unchanged'
    assert h.sql(db,'SELECT count(*) FROM knowledge.field_value_archives')=='2'
    from urllib.parse import urlencode
    for version in ['2','3','4']:
        code, values=h.request(f"/v1/projects/{p}/endpoints/{meta['target']}/field-values?"+urlencode(dict(archive=ids[version],field='request_body:remark')))
        assert code==200 and values['items'], (version,code,values)
    result['execution_verified']=True
    # Growing projects must still yield complete aggregates; no permanent row cutoff.
    h.sql(db,f"INSERT INTO knowledge.field_notes(endpoint_id,project_id,path,text,author) SELECT '{meta['target']}','{p}','synthetic-bound-'||n,'bounded','{{\"by\":\"person\"}}' FROM generate_series(1,1001) n")
    time.sleep(15);h.request(base+'/refresh',{})
    expanded=sampled(lambda x:any(g['rows'] is not None and g['rows'] > 1000 for m in x['snapshot']['modules'] if m['value'] for g in m['value']['groups']))
    complete=[g for m in expanded['snapshot']['modules'] if m['value'] for g in m['value']['groups'] if g['rows'] > 1000]
    assert complete and all(g['column_bytes'] is not None for g in complete)
    result['complete_over_1000']=complete
    assert not Upstream.calls,Upstream.calls
    result.update(model_calls=0,api_sha256=h.active_binary_sha,admin_sha256=hashlib.sha256(h.admin.read_bytes()).hexdigest(),protected_sha256=hashlib.sha256(before.encode()).hexdigest(),ports=dict(pg=h.pg_port,api=h.url,front=server.server_port))
    (OUT/'probe-results.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps(dict(api='passed',browser='passed',compacted=3,model_calls=0)))
finally:
    if server:server.shutdown()
    h.cleanup()
