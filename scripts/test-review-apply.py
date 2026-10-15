#!/usr/bin/env python3
"""Bounded real PG/API review/apply comparison; all writes target owned synthetic PG."""
import copy, hashlib, json, pathlib, sys, time, re, collections, subprocess, os, uuid, socket
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from frozen_input_harness import Harness, Upstream, ROOT, ident, encoded
from mcp_read_fixture import install_identity, literal
OUT = ROOT / 'docs/audits/2026-10-14-candidate-review-apply'
DB = 'nexofolio_step04_review_apply'

def measure(h, path, body=None):
    headers={'Authorization':'Bearer '+h.token, 'Content-Type':'application/json'}
    start=time.perf_counter()
    with urlopen(Request(h.url+path, data=encoded(body).encode() if body is not None else None,headers=headers),timeout=120) as r:
        first=time.perf_counter(); raw=r.read(); end=time.perf_counter()
    return json.loads(raw), dict(bytes=len(raw),ttfb_ms=(first-start)*1000,total_ms=(end-start)*1000)

def seed(h, interfaces=101, per_endpoint=20):
    meta=h.seed(DB,interfaces); project=meta['project']
    h.sql(DB,'DELETE FROM knowledge.field_notes')
    summaries=h.request(f'/v1/projects/{project}/endpoints')[1]['endpoints']
    base=dict(endpoints=sorted(s['id'] for s in summaries),folders=[],placements=[],notes=[],fields=[],links=[])
    view=copy.deepcopy(base); author=dict(by='curate',**{'pass':'synthetic'},model='no-model-called'); at='2026-10-14T00:00:00Z'
    commands=[]; facts=[]; inputs={}
    for s in summaries:
        endpoint=s['id']; fields=[]; inputs[endpoint]=[]
        for n in range(per_endpoint):
            key=f'request_body:field{n:02}'; path=f'field{n:02}'
            context=dict(location={'in':'request_body'},field_path=path,display_name='合成字段'+str(n))
            text='合成审阅与应用性能夹具。'*42
            note=dict(endpoint=endpoint,path=key,text=text,values=[],context=context,author=author,at=at)
            view['fields'].append(note);commands.append({k:v for k,v in note.items() if k not in ['author','at']}|{'do':'put_field_note'})
            fields.append(dict(path=[{'key':path}],location={'in':'request_body'},types=['string'],labels=[],differs_between_environments=False,declared=None,conflict=None))
            inputs[endpoint].append(dict(key=key,path=path,location={'in':'request_body'},observations=None))
        facts.append(dict(summary=s,fields=fields,examples=[],aliases=[],addresses=[]))
    candidate=ident('review-apply-'+str(interfaces))
    frozen=dict(version=2,prepared=dict(facts=facts,inputs=inputs),synthetic_padding='合成冻结材料'*(140000 if interfaces >= 100 else 1000))
    batches=[dict(id=ident('review-batch'),author=author,state='done',commands=commands)]
    base_hash=hashlib.sha256(json.dumps(base,ensure_ascii=False,separators=(',',':')).encode()).hexdigest()
    h.sql(DB,f"INSERT INTO knowledge.candidates(id,project_id,state,created_at,updated_at,base_hash,base,view,batches,progress,execution_input,applied_rounds) VALUES('{candidate}','{project}','ready','{at}','{at}','{base_hash}',{literal(base)},{literal(view)},{literal(batches)},'{{}}',{literal(frozen)},'[]')")
    # Publish exactly the same typed representation as the normal candidate writer.
    typed=h.request(f'/v1/projects/{project}/curate/candidates/{candidate}')[1]
    h.sql(DB,f"UPDATE knowledge.candidates SET base={literal(typed['base'])},view={literal(typed['view'])},batches={literal(typed['batches'])} WHERE id='{candidate}'")
    return dict(project=project,candidate=candidate,interfaces=interfaces,commands=len(commands),base=base)

def reset(h, db):
    h.sql(db,"DELETE FROM knowledge.field_notes; DELETE FROM knowledge.revisions; DELETE FROM knowledge.rounds; UPDATE knowledge.candidates SET state='ready',applied_rounds='[]',operations='[]'")

def sql_measure(h, db, action):
    h.sql(db,"ALTER SYSTEM SET log_min_duration_statement=0; ALTER SYSTEM SET log_parameter_max_length=0; SELECT pg_reload_conf()")
    time.sleep(.05); log=h.tmp/'pg.log'; start=log.stat().st_size
    value,timing=action()
    time.sleep(.02); raw=log.read_bytes()[start:].decode()
    h.sql(db,"ALTER SYSTEM RESET log_min_duration_statement; SELECT pg_reload_conf()")
    statements=re.findall(r'duration: ([0-9.]+) ms  (?:execute [^:]+|statement): (.*)',raw)
    groups=collections.Counter()
    for ms,sql in statements:
        match=re.search(r'(?:FROM|INTO|UPDATE) (?:knowledge\.)?([a-z_]+)',sql,re.I)
        groups[(sql.split(' ',1)[0],match[1] if match else 'transaction')]+=1
    timing.update(sql_count=len(statements),sql_server_ms=sum(float(ms) for ms,_ in statements),sql_groups={" ".join(k):v for k,v in groups.items()})
    return value,timing

def normalize(value):
    if isinstance(value,dict):return {k:normalize(v) for k,v in value.items() if k not in ['at','round_id','id','reviewed_by','updated_at']}
    if isinstance(value,list):return [normalize(v) for v in value]
    return value

def formal(h, db):
    return json.loads(h.sql(db,"SELECT jsonb_build_object('fields',(SELECT jsonb_agg(to_jsonb(f) ORDER BY endpoint_id,path) FROM knowledge.field_notes f),'revisions',(SELECT jsonb_agg(jsonb_build_object('seq',seq,'target',target,'before',before) ORDER BY seq) FROM knowledge.revisions))"))

def run_case(h,db,binary,label,meta):
    h.start(db,binary,label)
    base=f"/v1/projects/{meta['project']}/curate/candidates/{meta['candidate']}"
    result={};value,timing=measure(h,base);result['get']=[timing]
    assert len(value['view']['fields'])==meta['commands']
    (OUT/(label+'-candidate.json')).write_text(json.dumps(value,ensure_ascii=False))
    result['candidate_sha256']=hashlib.sha256(encoded(value).encode()).hexdigest()
    for _ in range(2):result['get'].append(measure(h,base)[1])
    _,result['get_sql_probe']=sql_measure(h,db,lambda:measure(h,base))
    if 'optimized' in label:
        result['head']=[];result['status']=[]
        for _ in range(3):
            head,t=measure(h,base+'/review');result['head'].append(t)
            status,t=measure(h,base+'/review?part=status');result['status'].append(t)
        assert head['changes']['fields']==meta['commands'],head['changes']
        assert head['view']['fields']==[] and head['batches']==[]
        items=[]
        for offset in range(0,meta['commands'],60):
            page,t=measure(h,base+f'/review?part=changes&section=fields&version={head["version"]}&offset={offset}')
            items.extend(page['items'])
        assert len(items)==len({i['key'] for i in items})==meta['commands']
        expected={f['endpoint']+':'+f['path']:normalize(f) for f in value['view']['fields']}
        for item in items:
            # Typed Candidate omits legacy empty values; absence and [] render identically.
            after=normalize(item['after']);after.pop('values',None)
            want=expected[item['key']];want.pop('values',None)
            assert after==want,(item['key'],after,want)
        endpoint=head['view']['endpoints'][0]
        page,t=measure(h,base+f'/review?part=endpoint&endpoint={endpoint}&version={head["version"]}')
        assert len(page['notes'])==20 and len(page['facts']['fields'])==20
        result['endpoint']=t;result['pages_complete']=True
    result['apply']=[]
    for sample in range(3):
        reset(h,db)
        value,timing=measure(h,base+'/apply',{});assert value['status']=='applied'
        assert int(h.sql(db,'SELECT count(*) FROM knowledge.field_notes'))==meta['commands']
        result['apply'].append(timing)
    reset(h,db)
    _,result['apply_sql_probe']=sql_measure(h,db,lambda:measure(h,base+'/apply',{}))
    result['formal_sha256']=hashlib.sha256(encoded(normalize(formal(h,db))).encode()).hexdigest()
    before=h.sql(db,'SELECT count(*) FROM knowledge.revisions')
    assert h.request(base+'/apply',{})[0]==200
    assert h.sql(db,'SELECT count(*) FROM knowledge.revisions')==before
    result['idempotent']=True
    result.update(meta={k:v for k,v in meta.items() if k!='base'},model_calls=Upstream.calls,api_sha256=h.active_binary_sha,api_port=int(h.url.rsplit(':',1)[1]),pg_port=h.pg_port)
    (OUT/(label+'-measurements.json')).write_text(json.dumps(result,indent=2))
    h.stop()
    return result

def main():
    global DB
    label=sys.argv[1] if len(sys.argv)>1 else 'compare'
    OUT.mkdir(parents=True,exist_ok=True);pathlib.Path('/tmp/nexofolio-step04').mkdir(exist_ok=True)
    install_identity();h=Harness(OUT,ROOT/'target/debug/nexofolio-admin')
    results=json.loads((OUT/'comparison.json').read_text()) if label=='small' and (OUT/'comparison.json').exists() else {}
    try:
        for size in ([3] if label=='small' else [3,101]):
            DB='nexofolio_step04_review_'+str(size)
            h.create(DB);h.start(DB,OUT/'baseline-nexofolio-api','seed-'+str(size));meta=seed(h,size);h.stop()
            for variant,binary in [('baseline',OUT/'baseline-nexofolio-api'),('optimized',ROOT/'target/debug/nexofolio-api')]:
                db=DB+'_'+variant;h.create(db,DB)
                results[str(size)+'-'+variant]=run_case(h,db,binary,str(size)+'-'+variant,meta)
            before,after=results[str(size)+'-baseline'],results[str(size)+'-optimized']
            assert before['candidate_sha256']==after['candidate_sha256']
            assert before['formal_sha256']==after['formal_sha256'],'formal result/revisions differ'
        (OUT/'comparison.json').write_text(json.dumps(results,indent=2))
        print(json.dumps({k:{n:v[n] for n in ['get','apply']} for k,v in results.items()},indent=2))
    finally:
        h.cleanup()
        ports={h.pg_port,*[v['api_port'] for v in results.values() if 'api_port' in v]}
        closed={}
        for port in ports:
            with socket.socket() as sock:
                sock.settimeout(.2);closed[str(port)]=sock.connect_ex(('127.0.0.1',port))!=0
        assert all(closed.values()),closed
        (OUT/'cleanup-final.json').write_text(json.dumps(dict(ports_closed=closed,owned_directory_removed=not h.tmp.exists(),user_services_restarted=False,actual_development_database_accessed=False),indent=2))
if __name__=='__main__':main()
