#!/usr/bin/env python3
"""Private PG/API only: same-row-count small/large documents, exact query plans and bounded GET load."""
import concurrent.futures, hashlib, json, pathlib, re, subprocess, sys, time, urllib.error, urllib.request, urllib.parse, uuid
out = pathlib.Path(sys.argv[1]).resolve()
assert out.parent == pathlib.Path('/tmp/nexofolio-operations').resolve() and out.name.startswith('browser.')
m = json.loads((out / 'browser.json').read_text())
assert m['database'] == 'nexofolio_operations_browser'
assert urllib.parse.urlparse(m['api']).hostname == '127.0.0.1'
psql = ['/opt/homebrew/opt/postgresql@17/bin/psql', '-q', '-X', '-h', '127.0.0.1', '-p', str(m['pg_port']), '-d', m['database'], '-At', '-v', 'ON_ERROR_STOP=1']

def sql(q):
    return subprocess.run(psql, input=q, text=True, check=True, capture_output=True).stdout.strip()

def request(path, token=None, body=None):
    headers = {'content-type': 'application/json'}
    if token:
        headers['Authorization'] = 'Bearer ' + token
    req = urllib.request.Request(m['api'] + path, headers=headers, data=json.dumps(body).encode() if body else None)
    t = time.monotonic()
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            status, raw = (r.status, r.read())
    except urllib.error.HTTPError as e:
        status, raw = (e.code, e.read())
    return (status, raw, (time.monotonic() - t) * 1000)
status, raw, _ = request('/v1/auth/login', body={'account': 'a80646', 'password': 'fixture-pass'})
assert status == 200
token = json.loads(raw)['token']
seed = str(uuid.UUID(sql('SELECT id FROM knowledge.candidates ORDER BY created_at LIMIT 1')))
ids = [str(uuid.uuid4()) for _ in range(50)]
tasks = [str(uuid.uuid4()) for _ in range(50)]
large = str(uuid.uuid4())
columns = 'id,project_id,target,state,created_at,updated_at,base_hash,base,view,batches,progress,parent,applied_rounds,reviewed_by,execution_input,operations'

def clone(id, original):
    return f"INSERT INTO knowledge.candidates({columns}) SELECT '{id}',project_id,target,'ready',now(),now(),base_hash,base,view,batches,progress,NULL,'[]',NULL,NULL,'[]' FROM knowledge.candidates WHERE id='{original}';"

def quoted(values):
    return ','.join(("'" + str(uuid.UUID(v)) + "'" for v in values))
source = (pathlib.Path(__file__).resolve().parents[1] / 'modules/knowledge/adapter/src/postgres/operations.rs').read_text()
start = re.search('QueryBuilder::new\\(\\s*"([^"]+)"', source)[1]
end = re.search('query.push\\("(\\) SELECT [^"]+)"\\)', source)[1]

def query(limit):
    return start + ' ORDER BY created_at DESC,id DESC LIMIT ' + str(limit + 1) + end
result = {'source_sha256': hashlib.sha256(source.encode()).hexdigest(), 'boundary': '50 retained done candidates in private PG; up to 55 MB batch document AND oversized progress per candidate. No models, curation or apply calls. Identical row count and API limits before/after growth.', 'measurements': [], 'load_windows': []}

def plan(limit):
    raw = sql('SET search_path=knowledge; EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON) ' + query(limit))
    value = json.loads(raw)[0]
    return {'execution_ms': value['Execution Time'], 'shared_hit_blocks': value['Plan'].get('Shared Hit Blocks'), 'shared_read_blocks': value['Plan'].get('Shared Read Blocks'), 'plan': value}

def measure(label):
    cases = []
    for limit in [20, 50]:
        samples = []
        for _ in range(3):
            status, raw, elapsed = request('/v1/operations/tasks?limit=' + str(limit), token)
            body = json.loads(raw)
            assert status == 200, (label, limit, status)
            assert len(body['page']['items']) == limit
            assert all((t['checkpoint'] is None or t['checkpoint']['saved_batch_records'] is None for t in body['page']['items']))
            samples.append({'status': status, 'ms': elapsed, 'response_bytes': len(raw)})
        cases.append({'limit': limit, 'samples': samples, 'query': plan(limit)})
    result['measurements'].append({'scenario': label, 'cases': cases})
    print(label, [(c['limit'], round(max((s['ms'] for s in c['samples'])), 2), c['query']['shared_hit_blocks']) for c in cases], flush=True)

def load_window(label, panel):
    deadline = time.monotonic() + 15
    business, monitoring = ([], [])

    def worker(path, interval, records):
        while time.monotonic() < deadline:
            start = time.monotonic()
            status, _, ms = request(path, token)
            assert status == 200
            records.append(ms)
            time.sleep(max(0, interval - (time.monotonic() - start)))

    def poll():
        while time.monotonic() < deadline:
            for route in ['/v1/operations/summary', '/v1/operations/tasks?limit=50']:
                status, _, ms = request(route, token)
                assert status == 200
                monitoring.append(ms)
            time.sleep(min(5, max(0, deadline - time.monotonic())))
    begun = time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
        work = [pool.submit(worker, f"/v1/projects/{m['projects'][0]}/endpoints", 0.05, business) for _ in range(2)]
        if panel:
            work.append(pool.submit(poll))
        for future in work:
            future.result()
    elapsed = time.monotonic() - begun
    business.sort()
    result['load_windows'].append({'scenario': label, 'panel': panel, 'duration_seconds': elapsed, 'business_requests': len(business), 'rps': len(business) / elapsed, 'p95_ms': business[int(len(business) * 0.95)], 'monitor_requests': len(monitoring), 'monitor_ms': monitoring, 'business_ms': business})
try:
    sql('BEGIN;' + ''.join((clone(id, seed) + f"""INSERT INTO knowledge.curation_tasks(id,project_id,candidate_id,actor,state,created_at,updated_at) SELECT '{task}',project_id,id,'{{"kind":"unknown"}}','done',now(),now() FROM knowledge.candidates WHERE id='{id}';""" for id, task in zip(ids, tasks))) + 'COMMIT;')
    measure('50_small_candidates')
    load_window('small_closed', False)
    load_window('small_panel', True)
    sql(f"WITH template AS (SELECT b FROM knowledge.candidates c CROSS JOIN LATERAL jsonb_array_elements(c.batches) b WHERE c.id='{seed}' AND b->'commands'->0->>'do'='put_field_note' LIMIT 1),\n    commands AS (SELECT n,(n-1)/25 AS batch,jsonb_set(jsonb_set(jsonb_set(b->'commands'->0,'{{path}}',to_jsonb('request_body:field_'||n)),'{{context,field_path}}',to_jsonb('field_'||n)),'{{text}}',to_jsonb((SELECT string_agg(md5(n::text||':'||j::text),'') FROM generate_series(1,32) j))) AS command FROM template CROSS JOIN generate_series(1,25000) n),\n    batches AS (SELECT batch,jsonb_build_object('id',md5('review-batch-'||batch)::uuid,'state','done','author',jsonb_build_object('by','curate','pass','fields','model','synthetic-review'),'commands',jsonb_agg(command ORDER BY n)) AS b FROM commands GROUP BY batch),\n    large AS (SELECT jsonb_agg(b ORDER BY batch) AS batches FROM batches)\n    INSERT INTO knowledge.candidates({columns}) SELECT '{large}',project_id,target,'ready',now(),now(),base_hash,base,view,large.batches,progress,NULL,'[]',NULL,NULL,'[]' FROM knowledge.candidates c CROSS JOIN large WHERE c.id='{seed}';")
    result['batch_size'] = json.loads(sql(f"SELECT jsonb_build_object('records',jsonb_array_length(batches),'logical_bytes',octet_length(batches::text),'stored_bytes',pg_column_size(batches)) FROM knowledge.candidates WHERE id='{large}'"))
    for count in [1, 4, 20, 50]:
        sql(f"UPDATE knowledge.candidates SET batches=(SELECT batches FROM knowledge.candidates WHERE id='{large}') WHERE id IN ({quoted(ids[:count])}); UPDATE knowledge.curation_tasks SET created_at=clock_timestamp() WHERE id IN ({quoted(tasks[:count])});")
        measure(str(count) + '_large_batches')
    sql(f"UPDATE knowledge.candidates SET progress=progress || jsonb_build_object('diagnostic_history',(SELECT batches FROM knowledge.candidates WHERE id='{large}')) WHERE id IN ({quoted(ids)});")
    result['progress_size'] = json.loads(sql(f"SELECT jsonb_build_object('logical_bytes',octet_length(progress::text),'stored_bytes',pg_column_size(progress),'summary_bytes',pg_column_size(operations_checkpoint),'summary',operations_checkpoint) FROM knowledge.candidates WHERE id='{ids[0]}'"))
    measure('50_large_batches_and_large_progress')
    load_window('large_closed', False)
    load_window('large_panel', True)
    legacy = f"SELECT jsonb_array_length(batches),progress->'completed',progress->'updated_at' FROM knowledge.candidates WHERE id='{ids[0]}'"
    result['legacy_one_row_plan'] = json.loads(sql('EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON) ' + legacy))[0]
    result['database_bytes'] = int(sql('SELECT pg_database_size(current_database())'))
finally:
    sql(f'BEGIN;DELETE FROM knowledge.curation_tasks WHERE id IN ({quoted(tasks)});DELETE FROM knowledge.candidates WHERE id IN ({quoted(ids + [large])});COMMIT;')
    result['own_large_rows_removed'] = True
    (out / 'large-candidates-fixed.json').write_text(json.dumps(result, indent=2))
