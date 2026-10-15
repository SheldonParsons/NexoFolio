#!/usr/bin/env python3
"""Synthetic regression against owned PostgreSQL/API; never reads user traffic."""
import argparse, base64, hashlib, json, pathlib
from frozen_input_harness import Harness, Upstream, ident, encoded

p = argparse.ArgumentParser()
p.add_argument('--before', required=True, type=pathlib.Path)
p.add_argument('--after', required=True, type=pathlib.Path)
p.add_argument('--admin', required=True, type=pathlib.Path)
p.add_argument('--out', required=True, type=pathlib.Path)
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=True)
pathlib.Path('/tmp/nexofolio-step04').mkdir(exist_ok=True)
h = Harness(a.out, a.admin.resolve())
db = 'nexofolio_step04_collect_content'
results = {}
try:
    h.create(db)
    h.start(db, a.before.resolve(), 'collect-before')
    status, projects = h.request('/v1/projects')
    assert status == 200
    project = projects['items'][0]['project_id']
    cases = [
        ('json', 'full', 'utf8', encoded({'ordinary': '正文', 'escaped_nul': '\0'}), 'application/json'),
        ('nul-full', 'full', 'utf8', 'PK\x03\x04\0收到的正文�', 'application/zip'),
        ('nul-truncated', 'truncated', 'utf8', 'PK\0前缀', 'application/octet-stream'),
        ('nul-partial', 'partial', 'utf8', '摘要\0保留', 'text/plain'),
        ('binary', 'full', 'base64', base64.b64encode(b'PK\x03\x04\0\xff\xfe').decode(), 'application/zip'),
    ]
    records = []
    for name, state, encoding, content, media in cases:
        body = dict(state=state, encoding=encoding, content=content, media_type=media)
        if state == 'truncated': body['bytes'] = 10000
        if state == 'partial': body['note'] = 'synthetic summary'
        records.append(dict(id=ident('collect-'+name), kind='http_exchange', version=1,
            observed_at='2026-10-14T00:00:00Z', payload=dict(request=dict(method='POST',
            url='https://synthetic.example.invalid/collect/'+name, body=body))))
    batch = dict(batch_id=ident('collect-content-batch'), platform='nexofolio-content-synthetic',
        target=dict(project_id=project, environment=dict(name='合成环境')), records=records)
    status, receipt = h.request('/v1/collect/batches', batch)
    assert status == 503, (status, receipt)
    assert h.sql(db, 'SELECT count(*) FROM observe.fingerprints') == '0'
    assert h.sql(db, 'SELECT count(*) FROM intake.batches') == '0'
    results['before'] = dict(status=status, fingerprints=0, ledger=0, binary_sha=h.active_binary_sha)
    h.stop()
    h.start(db, a.after.resolve(), 'collect-after')
    status, receipt = h.request('/v1/collect/batches', batch)
    assert status == 200 and receipt['accepted'] == len(cases) and not receipt['rejected'], (status, receipt)
    rows = json.loads(h.sql(db, "SELECT jsonb_agg(jsonb_build_object('path',e.path_template,'sample',f.sample,'calls',f.calls) ORDER BY e.path_template) FROM observe.fingerprints f JOIN observe.endpoints e ON e.id=f.endpoint_id"))
    for name, state, encoding, content, media in cases:
        row = next(r for r in rows if r['path'] == '/collect/'+name)
        body = row['sample']['fact']['request']['body']
        assert body['state'] == state and body['media_type'] == media and row['calls'] == 1
        saved = body['content']
        original = base64.b64decode(content) if encoding == 'base64' else content.encode()
        recovered = bytes(saved['data']) if saved['encoding'] == 'binary' else saved['data'].encode()
        assert original == recovered
        assert saved['encoding'] == ('binary' if encoding == 'base64' or '\0' in content else 'text')
        if state == 'truncated': assert body['original_bytes'] == 10000
        if state == 'partial': assert body['note'] == 'synthetic summary'
    status, repeated = h.request('/v1/collect/batches', batch)
    assert status == 200 and repeated == receipt
    assert h.sql(db, 'SELECT sum(calls) FROM observe.fingerprints') == str(len(cases))
    assert h.sql(db, 'SELECT count(*) FROM intake.batches') == '1'
    # Sink deduplication also covers a lost ledger acknowledgement.
    h.sql(db, 'DELETE FROM intake.batches')  # owned synthetic database only
    status, retry = h.request('/v1/collect/batches', batch)
    assert status == 200 and retry == receipt
    assert h.sql(db, 'SELECT sum(calls) FROM observe.fingerprints') == str(len(cases))
    results['after'] = dict(status=status, accepted=receipt['accepted'], cases=len(cases),
        sample_bytes_exact=True, states_exact=True, ledger_retry_count=len(cases), binary_sha=h.active_binary_sha)
    assert not Upstream.calls
finally:
    h.cleanup()
    results['cleanup'] = dict(owned_pg_removed=not h.tmp.exists(), paid_model_calls=0)
    (a.out/'collect-results.json').write_text(json.dumps(results, ensure_ascii=False, indent=2)+'\n')
print(json.dumps(results, ensure_ascii=False))
