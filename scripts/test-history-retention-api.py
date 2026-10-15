#!/usr/bin/env python3
"""One actual API/PG smoke using existing synthetic helpers, never application .env."""
import hashlib, json, os, pathlib, subprocess
from urllib.parse import urlencode
from frozen_input_harness import Harness, Upstream, ident, encoded, ROOT
from mcp_read_fixture import archives, literal

out = ROOT / 'docs/audits/2026-10-14-history-retention'
pathlib.Path('/tmp/nexofolio-step04').mkdir(exist_ok=True)
h = Harness(out, ROOT / 'target/debug/nexofolio-admin')
db = 'nexofolio_step04_retention'
result = {}
try:
    h.create(db)
    h.start(db, ROOT / 'target/debug/nexofolio-api', 'retention')
    meta = h.seed(db, 1)
    p = meta['project']
    e = h.sql(db, "SELECT id FROM observe.endpoints WHERE path_template='/synthetic/orders'")
    env = h.sql(db, 'SELECT id FROM access.environments LIMIT 1')
    meta.update(target=e, environment=env, other_environment=env)
    ids = archives(h, db, meta)
    # Current fields explicitly refer to both legacy and v3 evidence families.
    for version in ['2','3','4']:
        context = dict(observations=dict(archive_id=ids[version], total=1, hash='synthetic-field-hash', states={}))
        h.sql(db, f"UPDATE knowledge.field_notes SET context={literal(context)}::jsonb WHERE endpoint_id='{e}' AND path='request_body:manual{version}'")
    h.sql(db, "UPDATE knowledge.candidates SET created_at=now()-interval '60 days',updated_at=now()-interval '60 days'")
    snapshot = dict(endpoints=[e], folders=[], placements=[], notes=[], fields=[], links=[])
    for n in range(5):
        h.sql(db, f"INSERT INTO knowledge.candidates(id,project_id,state,created_at,updated_at,base_hash,base,view,batches,progress,applied_rounds) VALUES('{ident('retention-recent-'+str(n))}','{p}','discarded',now(),now(),'synthetic',{literal(snapshot)}::jsonb,{literal(snapshot)}::jsonb,'[]','{{}}','[]')")
    # Establish source/sample/manual-state invariance before maintenance.
    def invariants():
        return h.sql(db, "SELECT jsonb_build_object('samples',(SELECT jsonb_agg(to_jsonb(x) ORDER BY endpoint_id) FROM observe.fingerprints x),'manual',(SELECT jsonb_agg(to_jsonb(x) ORDER BY endpoint_id,path) FROM knowledge.field_notes x),'lifecycle',(SELECT jsonb_agg(to_jsonb(x)) FROM knowledge.document_lifecycle x))")
    before = invariants()
    command = [str(h.admin), 'retain-history', '--project', p]
    envs = {'PATH':os.environ['PATH'], 'NEXOFOLIO_RETENTION_DATABASE_URL':h.env(db,0)['DATABASE_URL']}
    def maintenance(execute=True):
        done=subprocess.run(command+(['--execute'] if execute else []),env={**envs,'NEXOFOLIO_RETENTION_ENABLED':'1'},capture_output=True,text=True,check=True)
        return json.loads(done.stdout)
    denied=subprocess.run(command+['--execute'],env=envs,capture_output=True,text=True)
    assert denied.returncode != 0 and 'NEXOFOLIO_RETENTION_ENABLED=1' in denied.stderr
    result['enable_guard']=denied.stderr.strip()
    result['preview_before_enable']=maintenance(False)
    assert h.sql(db,'SELECT count(*) FROM knowledge.retention_projects')=='0'
    result['first_enable']=maintenance()
    assert result['first_enable']['report']['compacted']==0
    # Synthetic clock boundary is separately covered by Rust; only this owned
    # fixture activation time is moved back for the wall-clock CLI/API smoke.
    h.sql(db, f"UPDATE knowledge.retention_projects SET enabled_at=now()-interval '7 days' WHERE project_id='{p}'")
    result['cleanup']=maintenance()
    assert result['cleanup']['report']['compacted']==3
    assert result['cleanup']['report']['inputs_released']==1, 'empty legacy input is not counted as released payload'
    result['reads']={}
    for version in ['2','3','4']:
        root=f'/v1/projects/{p}/curate/candidates/{ids[version]}'
        status, detail=h.request(root)
        assert status==410 and detail['error']['code']=='HISTORY_EXPIRED', (status,detail)
        query=urlencode(dict(archive=ids[version],field='request_body:remark'))
        values=f'/v1/projects/{p}/endpoints/{e}/field-values?{query}'
        status,page=h.request(values);assert status==200,(status,page)
        status,value=h.request(values+'&value_id=one');assert status==200,(status,value)
        decoded=json.loads(value['json']);assert decoded[1]==900719925474099312345
        assert decoded[0]==('精确正文' if version=='4' else '精确\0正文')
        assert page['items'][0]['sources'][0]['example']=='synthetic'
        result['reads'][version]=dict(detail_status=410,summary=page['summary'],sources=page['items'][0]['sources'],value=value)
    status, listing=h.request(f'/v1/projects/{p}/curate/candidates')
    assert status==200
    result['history_list']=listing
    assert before==invariants(), 'samples, manual knowledge and lifecycle unchanged'
    assert not Upstream.calls, 'no model calls'
    result['api_sha256']=h.active_binary_sha
    result['admin_sha256']=hashlib.sha256(h.admin.read_bytes()).hexdigest()
    result['unchanged_material_sha256']=hashlib.sha256(before.encode()).hexdigest()
    result['model_calls']=0
    (out/'api-results.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps(dict(compacted=3,read_formats=3,http_expired=410,model_calls=0)))
finally:
    h.cleanup()
