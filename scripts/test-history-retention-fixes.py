#!/usr/bin/env python3
"""Review regressions, using the same owned PG/API harness and synthetic probe shapes."""
import hashlib, json, os, pathlib, subprocess, uuid
from frozen_input_harness import Harness, Upstream, ROOT, encoded

out = ROOT / 'docs/audits/2026-10-14-history-retention-fixes'
pathlib.Path('/tmp/nexofolio-step04').mkdir(exist_ok=True)
h = Harness(out, ROOT / 'target/debug/nexofolio-admin')
result = {'scope': 'owned synthetic PostgreSQL/API only'}
def uid(n):
    return str(uuid.UUID(int=n))
def lit(value):
    return "'" + encoded(value).replace("'", "''") + "'"
try:
    db = 'nexofolio_step04_retention_fixes'
    h.create(db)
    h.start(db, ROOT/'target/debug/nexofolio-api', 'fixes')
    meta = h.seed(db, 1)
    p = meta['project']
    e = h.sql(db, "SELECT id FROM observe.endpoints WHERE path_template='/synthetic/orders'")
    snapshot = dict(endpoints=[e],folders=[],placements=[],notes=[],fields=[],links=[])
    commands = [dict(do='put_endpoint_note',endpoint=e,name='PRIVATE_NAME',purpose=None,context={}),
                dict(do='put_endpoint_note',endpoint=e,name=None,purpose='PRIVATE_BODY',context={})]
    batches = [dict(id=uid(1000),author=dict(by='curate',**{'pass':'describe'},model=None),state='done',commands=commands)]
    for i in range(1,9):
        age = 60 if i <= 3 else 1
        h.sql(db,f"INSERT INTO knowledge.candidates(id,project_id,state,created_at,updated_at,base_hash,base,view,batches,progress,applied_rounds,execution_input) VALUES('{uid(i)}','{p}','discarded',now()-interval '{age} days',now()-interval '{age} days','synthetic',{lit(snapshot)}::jsonb,{lit(snapshot)}::jsonb,{lit(batches if i==1 else [])}::jsonb,'{{\"message\":\"password=PRIVATE_SECRET\",\"api_key\":\"sk-private\",\"history_message\":\"PRIVATE_HISTORY\"}}','[]','{{\"version\":2,\"prepared\":{{\"inputs\":{{}}}}}}')")
    h.sql(db,f"INSERT INTO knowledge.curation_tasks(id,project_id,candidate_id,actor,state,generation,created_at,updated_at) VALUES('{uid(900)}','{p}','{uid(1)}','{{\"kind\":\"unknown\"}}','done',1,now(),now())")
    def diag(path):
        status, data = h.request(path)
        assert status==200,(status,data)
        text = json.dumps(data)
        for private in ['PRIVATE_NAME','PRIVATE_BODY','PRIVATE_SECRET','sk-private','PRIVATE_HISTORY']:
            assert private not in text, private
        return {key:data[key] for key in ['subject','candidate','history','batches','agent_prompt']}
    candidate_route=f'/v1/audit/curations/{p}/{uid(1)}'
    task_route=f'/v1/audit/curations/{p}/tasks/{uid(900)}'
    result['before']={key:diag(path) for key,path in [('candidate',candidate_route),('task',task_route)]}
    for data in result['before'].values():
        assert data['history']['available'] is True
        assert data['history']['retained_commands']==2 and data['batches']['recorded']==1
    # Exact review-sized population remains present throughout all subsequent calls.
    h.sql(db,f"INSERT INTO knowledge.retention_projects(project_id,enabled_at) VALUES('{p}',now()-interval '8 days')")
    h.sql(db,f"INSERT INTO knowledge.field_notes(endpoint_id,project_id,path,text,author,context) SELECT '{e}','{p}','retention_probe_'||g,'synthetic','{{\"by\":\"curate\",\"pass\":\"fixture\",\"model\":null}}','{{}}' FROM generate_series(1,20001) g")
    def invariant():
        return h.sql(db,"SELECT jsonb_build_object('fields',(SELECT md5(string_agg(to_jsonb(f)::text,',' ORDER BY path)) FROM knowledge.field_notes f),'samples',(SELECT md5(string_agg(to_jsonb(f)::text,',' ORDER BY endpoint_id)) FROM observe.fingerprints f))")
    material_before = invariant()
    env={'PATH':os.environ['PATH'],'NEXOFOLIO_RETENTION_DATABASE_URL':h.env(db,0)['DATABASE_URL'],'NEXOFOLIO_RETENTION_ENABLED':'1'}
    def maintain(after=None,limit=1):
        cmd=[str(h.admin),'retain-history','--project',p,'--limit',str(limit),'--execute']
        if after:
            cmd+=['--after',after]
        done=subprocess.run(cmd,env=env,capture_output=True,text=True,check=True,timeout=10)
        return json.loads(done.stdout)
    result['many_rows']=[maintain(),maintain(uid(1)),maintain(uid(2))]
    for index,r in enumerate(result['many_rows'],1):
        assert r['report']['committed'] and r['report']['inputs_released']==1, r
        assert r['report']['next']==uid(index)
    result['repeat']=maintain()
    assert result['repeat']['report']['inputs_released']==0
    assert h.sql(db,"SELECT count(*) FROM knowledge.field_notes WHERE path LIKE 'retention_probe_%'")=='20001'
    assert h.sql(db,"SELECT count(*) FROM knowledge.candidates WHERE id IN ('"+"','".join(uid(i) for i in [1,2,3])+"') AND execution_input IS NOT NULL")=='0'
    result['after']={key:diag(path) for key,path in [('candidate',candidate_route),('task',task_route),('operations_task',f'/v1/operations/tasks/{uid(900)}/diagnostics')]}
    for data in result['after'].values():
        assert data['history']['available'] is False and data['history']['reason_code']=='HISTORY_EXPIRED'
        assert data['history']['retained_commands']==2 and data['history']['retained_batches']==1
        assert data['candidate']['commands']==2 and data['candidate']['history_available'] is False
        assert data['batches']['available'] is False and data['batches']['included']==0
        assert data['batches']['recorded']==1 and data['batches']['truncated'] is None
        assert 'included=0 不等于原本没有批次' in data['agent_prompt']
    status,listing=h.request(f'/v1/audit/curations/{p}')
    assert status==200
    rows=listing['items']
    row=next(x for x in rows if x['id']==uid(1))
    assert row['commands']==2 and row['history_available'] is False
    result['diagnostic_list_item']=row
    status,history=h.request(f'/v1/projects/{p}/curate/candidates')
    assert status==200
    rows=history if isinstance(history,list) else history['items']
    assert next(x for x in rows if x['id']==uid(1))['commands']==2
    assert h.request(f'/v1/projects/{p}/curate/candidates/{uid(1)}')[0]==410
    h.sql(db,f"UPDATE knowledge.candidates SET progress=progress-'retained_batches' WHERE id='{uid(1)}'")
    result['legacy_compacted']=diag(candidate_route)
    assert result['legacy_compacted']['history']['retained_batches'] is None
    assert result['legacy_compacted']['batches']['recorded'] is None
    assert result['legacy_compacted']['history']['retained_commands']==2
    result['originally_empty']=diag(f'/v1/audit/curations/{p}/{uid(4)}')
    assert result['originally_empty']['history']['available'] is True
    assert result['originally_empty']['batches']['recorded']==0
    assert result['originally_empty']['batches']['truncated'] is False
    # Sixteen older prepared inputs, each with 24 field samples, exceed 128 MiB.
    # They are real stored JSONB, not a fabricated reported size or timing mock.
    prepared=f"(SELECT jsonb_build_object('version',2,'prepared',jsonb_build_object('inputs',jsonb_build_object('{e}',jsonb_agg(jsonb_build_object('key','request_body:field'||n,'path','field'||n,'location','{{\"in\":\"request_body\"}}'::jsonb,'types','[\"string\"]'::jsonb,'values','[]'::jsonb,'observations',NULL,'value_examples',jsonb_build_array(repeat('observed ',44000)),'evidence','[]'::jsonb,'parent','','siblings','[]'::jsonb))))) FROM generate_series(1,24) n)"
    for n in range(100,116):
        h.sql(db,f"INSERT INTO knowledge.candidates(id,project_id,state,created_at,updated_at,base_hash,base,view,batches,progress,applied_rounds,execution_input) VALUES('{uid(n)}','{p}','discarded',now()-interval '60 days',now()-interval '60 days','synthetic',{lit(snapshot)}::jsonb,{lit(snapshot)}::jsonb,'[]','{{}}','[]',{prepared})")
    size=int(h.sql(db,f"SELECT sum(octet_length(execution_input::text)) FROM knowledge.candidates WHERE id>='{uid(100)}'"))
    assert size>128*1024*1024,size
    calls=[]
    cursor=uid(99)
    for _ in range(8):
        r=maintain(cursor,2)
        calls.append(r)
        assert r['report']['committed'] and r['report']['inputs_released']==2,r
        assert r['elapsed_ms']<5000,r
        cursor=r['report']['next']
    assert h.sql(db,f"SELECT count(*) FROM knowledge.candidates WHERE id>='{uid(100)}' AND execution_input IS NOT NULL")=='0'
    result['large_history']={'logical_input_bytes_before':size,'calls':calls,'remaining_inputs':0}
    assert material_before==invariant()
    assert not Upstream.calls
    result.update(api_sha256=h.active_binary_sha,admin_sha256=hashlib.sha256(h.admin.read_bytes()).hexdigest(),paid_model_calls=0,material_unchanged=True)
finally:
    h.cleanup()
    result['owned_cluster_removed']=not h.tmp.exists()
    (out/'probe-results.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
print(json.dumps({'many_rows_released':3,'large_history_released':16,'diagnostics':'before/after/legacy-unknown/true-zero and candidate/task passed','model_calls':0}))
