"""Semantic and transaction acceptance. Owned PostgreSQL, synthetic projects only."""
import importlib.util, json, pathlib, hashlib, subprocess, os, time, sys
from urllib.parse import urlencode
from frozen_input_harness import ROOT, PG, Harness, Upstream, ident, encoded
from mcp_read_fixture import literal, install_identity
spec=importlib.util.spec_from_file_location('review_fixture',ROOT/'scripts/test-review-apply.py')
f=importlib.util.module_from_spec(spec);spec.loader.exec_module(f)
OUT=f.OUT

def v3(h,db,meta):
    p,c=meta['project'],meta['candidate']; snapshot=ident('review-frozen-v3')
    input=json.loads(h.sql(db,f"SELECT execution_input FROM knowledge.candidates WHERE id='{c}'"))
    legacy=input['prepared']; refs={}
    def chunk(value):
        content=encoded(value); ref=dict(sha256=hashlib.sha256(content.encode()).hexdigest(),bytes=len(content.encode()))
        h.sql(db,f"INSERT INTO knowledge.frozen_input_chunks(project_id,candidate_id,snapshot_id,sha256,content) VALUES('{p}','{c}','{snapshot}','{ref['sha256']}',{chr(39)+content.replace(chr(39),chr(39)*2)+chr(39)}) ON CONFLICT DO NOTHING")
        return ref
    empty=chunk([])
    for facts in legacy['facts']:
        e=facts['summary']['id'];inputs=legacy['inputs'][e]
        doc=dict(source=dict(endpoint=e,example='synthetic-evidence',environment=(facts['summary']['environments'][0]['environment_id'] if facts['summary']['environments'] else None)),observed_at=None,observation_key=None,representation='json',state='full',root={'value':'合成历史观测'})
        values=dict(version=1,documents={},fields={e+'/request_body:field00':dict(values=[dict(id='one',value_type='string',representation='json',occurrences=[dict(document='d',pointer='/value')])],coverage=[],hash='synthetic')})
        refs[e]=dict(facts=chunk(dict(facts=facts,authentication=[])),values=chunk(values),documents={'d':chunk(doc)},inputs=[dict(keys=[x['key'] for x in inputs[n:n+25]],content=chunk(inputs[n:n+25])) for n in range(0,len(inputs),25)])
    header=dict(version=3,lifecycle_version=0,summaries=[x['summary'] for x in legacy['facts']],manifest=dict(version=1,project=p,candidate=c,snapshot=snapshot,links=empty,endpoints=refs))
    header['manifest_hash']=hashlib.sha256(encoded(header).encode()).hexdigest()
    h.sql(db,f"UPDATE knowledge.candidates SET execution_input={literal(header)} WHERE id='{c}'")
    return header

def check_api(h,db,meta):
    p,c=meta['project'],meta['candidate'];base=f'/v1/projects/{p}/curate/candidates/{c}'
    original=h.request(base)[1];checks={}
    head=h.request(base+'/review')[1];v=head['version'];e=head['view']['endpoints'][0]
    assert head['changes']['fields']==meta['commands']
    first=h.request(base+f'/review?part=endpoint&version={v}&endpoint={e}')[1]
    second=h.request(base+f'/review?part=endpoint&version={v}&endpoint={e}&offset=100')[1]
    assert first['fields_total']==120 and len(first['facts']['fields'])==100 and len(second['facts']['fields'])==20
    assert len({n['path'] for n in first['notes']+second['notes']})==120
    h.sql(db,"UPDATE knowledge.candidates SET updated_at=clock_timestamp(),progress='{\"completed\":10}'")
    assert h.request(base+'/review?part=status')[1]['version']==v
    h.sql(db,f"UPDATE observe.endpoints SET path_template='/later/identity' WHERE id='{e}'")
    assert h.request(base+'/review')[1]['summaries']==head['summaries']
    merged=head['view']['endpoints'][1]
    h.sql(db,f"UPDATE observe.endpoints SET merged_into='{merged}' WHERE id='{e}'")
    current=h.request(f'/v1/projects/{p}/endpoints')[1]['endpoints']
    assert e not in [row['id'] for row in current]
    retained=h.request(base+f'/review?part=endpoint&version={v}&endpoint={e}')[1]
    assert retained['facts']['summary']==first['facts']['summary']
    assert h.request(base+'/review')[1]['summaries']==head['summaries']
    h.sql(db,f"UPDATE observe.endpoints SET merged_into=NULL WHERE id='{e}'")
    checks['merged_historical_identity']=True
    h.sql(db,f"UPDATE knowledge.candidates SET view=jsonb_set(view,'{{fields,0,text}}','\"content changed\"') WHERE id='{c}'")
    assert h.request(base+f'/review?part=changes&section=fields&version={v}')[0]==409
    h.sql(db,f"UPDATE knowledge.candidates SET view={literal(original['view'])} WHERE id='{c}'")
    checks['v2_paging_history_and_version']=True
    token=h.token;h.token=None;assert h.request(base+'/review')[0]==401;h.token=token
    other=h.sql(db,f"SELECT id FROM access.projects WHERE id<>'{p}' LIMIT 1")
    assert h.request(f'/v1/projects/{other}/curate/candidates/{c}/review')[0]==404
    checks['permission_scope']=True
    header=v3(h,db,meta);head=h.request(base+'/review')[1];v=head['version']
    page=h.request(base+f'/review?part=endpoint&version={v}&endpoint={e}')[1]
    assert page['facts']['summary']['path_template']==first['facts']['summary']['path_template']
    assert page['facts']['fields']==first['facts']['fields']
    assert page['inputs']==first['inputs']
    checks['v3_facts_inputs_equal_v2']=True
    ref=header['manifest']['links'];old=h.sql(db,f"SELECT content FROM knowledge.frozen_input_chunks WHERE candidate_id='{c}' AND sha256='{ref['sha256']}'")
    h.sql(db,f"UPDATE knowledge.frozen_input_chunks SET content='[1]' WHERE candidate_id='{c}' AND sha256='{ref['sha256']}'")
    assert h.request(base+'/apply',{})[0]==422
    assert h.sql(db,'SELECT count(*) FROM knowledge.revisions')=='0'
    h.sql(db,f"UPDATE knowledge.frozen_input_chunks SET content='{old}' WHERE candidate_id='{c}' AND sha256='{ref['sha256']}'")
    checks['v3_integrity_not_skipped']=True
    h.sql(db,"CREATE FUNCTION knowledge.synthetic_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.path='request_body:field110' THEN RAISE EXCEPTION 'synthetic mid-transaction failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER synthetic_failure BEFORE INSERT ON knowledge.field_notes FOR EACH ROW EXECUTE FUNCTION knowledge.synthetic_failure()")
    assert h.request(base+'/apply',{})[0]==503
    assert h.sql(db,"SELECT (SELECT count(*) FROM knowledge.field_notes)||(SELECT count(*) FROM knowledge.revisions)::text||(SELECT count(*) FROM knowledge.rounds)::text")=='000'
    assert h.request(base+'/review?part=status')[1]['state']=='ready'
    h.sql(db,'DROP TRIGGER synthetic_failure ON knowledge.field_notes; DROP FUNCTION knowledge.synthetic_failure()')
    checks['real_sql_failure_full_rollback']=True
    chunks_before=h.sql(db,f"SELECT md5(string_agg(sha256||content,'' ORDER BY sha256)) FROM knowledge.frozen_input_chunks WHERE candidate_id='{c}'")
    assert h.request(base+'/apply',{})[0]==200
    assert h.request(base)[1]['execution_input']==header
    assert h.sql(db,f"SELECT md5(string_agg(sha256||content,'' ORDER BY sha256)) FROM knowledge.frozen_input_chunks WHERE candidate_id='{c}'")==chunks_before
    checks['applied_frozen_header_and_chunks_unchanged']=True
    before=h.sql(db,'SELECT count(*) FROM knowledge.revisions');assert int(before)==meta['commands']
    assert h.request(base+'/apply',{})[0]==200 and h.sql(db,'SELECT count(*) FROM knowledge.revisions')==before
    checks['v3_apply_and_repeat']=True
    h.sql(db,f"UPDATE knowledge.candidates SET state='ready' WHERE id='{c}'")
    assert h.request(base+'/apply',{})[0]==409
    checks['stale_rejected']=True
    # Keep the historic evidence header but release execution-only chunks, as step 09 does.
    keep=[r['sha256'] for ep in header['manifest']['endpoints'].values() for r in [ep['values'],*ep['documents'].values()]]
    h.sql(db,f"UPDATE knowledge.candidates SET state='applied',evidence_input=execution_input,execution_input=NULL,execution_released_at=now() WHERE id='{c}'; DELETE FROM knowledge.frozen_input_chunks WHERE candidate_id='{c}' AND sha256 NOT IN ({','.join(chr(39)+x+chr(39) for x in keep)})")
    head=h.request(base+'/review')[1];v=head['version']
    page=h.request(base+f'/review?part=endpoint&version={v}&endpoint={e}')[1]
    assert not page['facts_available'] and page['notes'] and not head['resume_available']
    code,values=h.request(f'/v1/projects/{p}/endpoints/{e}/field-values?'+urlencode(dict(archive=c,field='request_body:field00')))
    assert code==200 and values['items'],(code,values)
    checks['released_execution_retained_evidence']=True
    h.sql(db,f"UPDATE knowledge.candidates SET history_pruned_at=now() WHERE id='{c}'")
    assert h.request(base+'/review')[0]==410
    assert h.request(base+'/review?part=status')[1]['history_available'] is False
    checks['history_expired']=True
    return checks

def core_tests(h):
    binary=pathlib.Path(os.environ.get('REVIEW_TEST_BINARY',str(ROOT/'target/debug/deps/candidate_review-43973bec51d3c1b8')))
    names=subprocess.check_output([str(binary),'--list'],text=True).splitlines()
    results=[]
    for i,line in enumerate(names):
        if not line.endswith(': test'):continue
        name=line[:-6];db='nexofolio_step04_review_core_'+str(i);h.create(db)
        env={**os.environ,'TEST_DATABASE_URL':f'postgres://{os.environ["USER"]}@127.0.0.1:{h.pg_port}/{db}'}
        with (OUT/('core-'+name+'.log')).open('w') as log:
            subprocess.run([str(binary),name,'--ignored','--exact','--nocapture'],env=env,stdout=log,stderr=log,check=True)
        results.append(name)
    return results

def main():
    pathlib.Path('/tmp/nexofolio-step04').mkdir(exist_ok=True);install_identity();h=Harness(OUT,ROOT/'target/debug/nexofolio-admin')
    db='nexofolio_step04_review_acceptance';f.DB=db
    try:
        h.create(db);h.start(db,ROOT/'target/debug/nexofolio-api','acceptance');meta=f.seed(h,3,120)
        from review_semantic_acceptance import check
        semantics=check(h,db,meta,OUT)
        checks=check_api(h,db,meta)
        checks['semantic_parity']=semantics
        h.stop()
        if '--skip-core' in sys.argv:
            checks['core_tests_reused']=json.loads((OUT/'api-acceptance-before-timing.json').read_text())['checks']['core_tests']
        else:checks['core_tests']=core_tests(h)
        assert not Upstream.calls
        (OUT/'api-acceptance.json').write_text(json.dumps(dict(checks=checks,model_calls=0,api_sha256=h.active_binary_sha),indent=2))
        print(json.dumps(checks))
    finally:h.cleanup()
if __name__=='__main__':main()
