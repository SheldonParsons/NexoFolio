#!/usr/bin/env python3
"""Complete aggregate regression. Owned PG/API/Chrome only; no cleanup requests or .env."""
import hashlib
import json
import mimetypes
import os
import pathlib
import subprocess
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from frozen_input_harness import Harness, PG, ROOT, Upstream, ident
from mcp_read_fixture import install_identity

OUT = ROOT / 'docs/audits/2026-10-14-storage-panel-fixes'
FRONT = ROOT.parent / 'NexoFolio-Front'
NODE = pathlib.Path('/Users/sheldon/.nvm/versions/node/v22.22.0/bin/node')
DB = 'nexofolio_step04_storage_statistics'
OUT.mkdir(parents=True, exist_ok=True)
pathlib.Path('/tmp/nexofolio-step04').mkdir(exist_ok=True)
install_identity()
h = Harness(OUT, ROOT / 'target/debug/nexofolio-admin')
server = lock = None
results = {'scales': {}, 'failures': {}}

def read(project):
    code, body = h.request(f'/v1/projects/{project}/storage')
    assert code == 200, (code, body)
    return body['snapshot']

def refresh(project):
    assert h.request(f'/v1/projects/{project}/storage/refresh', {})[0] == 200

def wait_sample(project, predicate):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        value = read(project)
        if predicate(value):
            return value
        time.sleep(.1)
    raise AssertionError(('sampling deadline', project, value))

# Independent sums per physical source; combine scalar results in the test, not the adapter UNION.
SOURCES = [
    ('observe.fingerprints f JOIN observe.endpoints e ON e.id=f.endpoint_id', 'e.project_id', 'coalesce(pg_column_size(f.sample),0)+coalesce(pg_column_size(f.structure),0)'),
    ('observe.declarations d JOIN observe.endpoints e ON e.id=d.endpoint_id', 'e.project_id', 'pg_column_size(d.structure)'),
    ('observe.fingerprints f JOIN observe.endpoints e ON e.id=f.endpoint_id', 'e.project_id', 'pg_column_size(f.representatives)'),
    ('knowledge.candidates', 'project_id', 'pg_column_size(execution_input)'),
    ('knowledge.frozen_input_chunks', 'project_id', 'pg_column_size(content)'),
    ('knowledge.field_value_archives', 'project_id', 'pg_column_size(archive)'),
    ('knowledge.candidates', 'project_id', 'pg_column_size(evidence_input)'),
    ('knowledge.candidates', 'project_id', 'coalesce(pg_column_size(base),0)+coalesce(pg_column_size(view),0)+coalesce(pg_column_size(batches),0)'),
    ('knowledge.revisions v JOIN knowledge.rounds r ON r.id=v.round_id', 'r.project_id', 'coalesce(pg_column_size(v.target),0)+coalesce(pg_column_size(v.before),0)'),
    ('knowledge.field_notes', 'project_id', 'coalesce(pg_column_size(text),0)+coalesce(pg_column_size(context),0)+coalesce(pg_column_size(values_seen),0)'),
    ('knowledge.endpoint_notes', 'project_id', 'coalesce(pg_column_size(name),0)+coalesce(pg_column_size(purpose),0)'),
]

def expected(project):
    values = []
    for table, key, expression in SOURCES:
        query = f"SELECT count(*),coalesce(sum(({expression})::bigint),0) FROM {table} WHERE {key}='{project}'"
        values.append(list(map(int, h.sql(DB, query).split('|'))))
    return [[sum(values[i][column] for i in indices) for column in [0, 1]]
            for indices in [[0, 1], [2], [3, 4], [5, 6], [7, 8], [9, 10]]]

def assert_complete(snapshot, wanted):
    groups = [g for module in snapshot['modules'] for g in module['value']['groups']]
    assert [[g['rows'], g['column_bytes']] for g in groups] == wanted
    assert all(g['logical_bytes'] is None for g in groups)
    return groups

try:
    h.create(DB)
    h.start(DB, ROOT / 'target/debug/nexofolio-api', 'statistics')
    projects = {n: ident(f'statistics-{n}') for n in [999, 1001, 3000, 12000]}
    fixture = (ROOT / 'scripts/storage-statistics-fixture.sql').read_text()
    for n, project in projects.items():
        h.sql(DB, fixture.replace('__PROJECT__', project).replace('__COUNT__', str(n)))
    h.sql(DB, 'ANALYZE')
    for project in projects.values():
        refresh(project)
    samples = {}
    for n, project in projects.items():
        value = wait_sample(project, lambda x: all(m['value'] for m in x['modules']))
        wanted = expected(project)
        groups = assert_complete(value, wanted)
        samples[n] = value
        results['scales'][n] = {'project': project, 'independent_sql': wanted, 'groups': groups,
                                'snapshot_bytes': len(json.dumps(value).encode())}
    target = projects[3000]
    # GET-only calls after the queue drained: statements must remain auth/project reads.
    h.sql(DB, "ALTER SYSTEM SET log_statement='all'; SELECT pg_reload_conf()")
    time.sleep(.2)
    pg_log = h.tmp / 'pg.log'
    offset = pg_log.stat().st_size
    for _ in range(6):
        assert read(target) == samples[3000]
    time.sleep(.2)
    statements = pg_log.read_bytes()[offset:].decode()
    assert 'pg_column_size' not in statements and 'retention_projects' not in statements
    results['cache_gets'] = {'count': 6, 'storage_statements': 0}
    h.sql(DB, "ALTER SYSTEM RESET log_statement; SELECT pg_reload_conf()")

    config, dist = h.tmp / 'vite.config.mjs', h.tmp / 'dist'
    config.write_text("import vue from '" + str(FRONT / 'node_modules/@vitejs/plugin-vue/dist/index.mjs') +
                      "'; export default { root:" + json.dumps(str(FRONT)) + ",envDir:" + json.dumps(str(h.tmp)) +
                      ",plugins:[vue()],resolve:{alias:{'@':" + json.dumps(str(FRONT / 'src')) +
                      "}},build:{outDir:" + json.dumps(str(dist)) + ",emptyOutDir:true}}")
    with (OUT / 'frontend-build.log').open('w') as log:
        subprocess.run([str(NODE), str(FRONT / 'node_modules/vite/bin/vite.js'), 'build', '--config', str(config)],
                       cwd=FRONT, env={'PATH': str(NODE.parent) + ':' + os.environ['PATH']},
                       stdout=log, stderr=log, check=True)
    results['frontend_assets'] = {str(p.relative_to(dist)): hashlib.sha256(p.read_bytes()).hexdigest()
                                  for p in dist.rglob('*') if p.is_file()}

    class Front(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass
        def do_GET(self):
            self.serve()
        def do_POST(self):
            self.serve()
        def serve(self):
            if self.path.startswith('/api/'):
                body = self.rfile.read(int(self.headers.get('Content-Length', 0))) if self.command == 'POST' else None
                headers = {k: v for k, v in self.headers.items() if k.lower() in ['authorization', 'content-type']}
                try:
                    response = urlopen(Request(h.url + self.path[4:], data=body, headers=headers, method=self.command), timeout=20)
                except HTTPError as error:
                    response = error
                data, status, mime = response.read(), response.status, response.headers.get('Content-Type')
            else:
                file = dist / self.path.split('?')[0].lstrip('/')
                if not file.is_file():
                    file = dist / 'index.html'
                data, status, mime = file.read_bytes(), 200, mimetypes.guess_type(file)[0] or 'application/octet-stream'
            self.send_response(status)
            self.send_header('Content-Type', mime)
            self.end_headers()
            try:
                self.wfile.write(data)
            except (BrokenPipeError, ConnectionResetError):
                pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), Front)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    browser_meta = dict(front=f'http://127.0.0.1:{server.server_port}', project=target,
                        token=h.token, out=str(OUT), expected=expected(target))
    (h.tmp / 'browser.json').write_text(json.dumps(browser_meta))
    def browser(stage):
        subprocess.run([str(NODE), str(ROOT / 'scripts/test-storage-statistics-browser.cjs'), str(h.tmp), stage], check=True)
    browser('complete')

    # Change an earlier knowledge category, then block the last one. No partial module update may escape.
    prior = read(target)
    h.sql(DB, f"UPDATE knowledge.candidates SET execution_input='{{\"changed\":\"must not escape failed snapshot\"}}' WHERE id=(SELECT id FROM knowledge.candidates WHERE project_id='{target}' LIMIT 1)")
    h.sql(DB, f"UPDATE observe.fingerprints SET representatives='[\"available module updated\"]' WHERE endpoint_id=(SELECT id FROM observe.endpoints WHERE project_id='{target}' LIMIT 1)")
    lock = subprocess.Popen([str(PG / 'psql'), '-XqAt', '-v', 'ON_ERROR_STOP=1', '-h', '127.0.0.1', '-p', str(h.pg_port), '-d', DB], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    lock.stdin.write("BEGIN; LOCK TABLE knowledge.field_notes IN ACCESS EXCLUSIVE MODE; SELECT 'locked';\n")
    lock.stdin.flush()
    assert lock.stdout.readline().strip() == 'locked'
    time.sleep(15)  # existing per-project refresh throttle, not a changed statistic budget
    refresh(target)
    failed = wait_sample(target, lambda x: x['modules'][1]['error'] is not None)
    assert failed['modules'][1]['value'] == prior['modules'][1]['value']
    assert failed['modules'][1]['sampled_at'] == prior['modules'][1]['sampled_at']
    assert failed['modules'][0]['error'] is None
    assert failed['modules'][0]['sampled_at'] > prior['modules'][0]['sampled_at']
    assert failed['modules'][0]['value'] != prior['modules'][0]['value']
    results['failures']['retained'] = failed
    fresh = h.sql(DB, "SELECT id FROM access.projects WHERE external_id='11'")
    refresh(fresh)
    first_failed = wait_sample(fresh, lambda x: x['modules'][1]['error'] is not None)
    assert first_failed['modules'][1]['value'] is None and first_failed['modules'][1]['sampled_at'] is None
    assert first_failed['modules'][0]['value']['groups'][0]['rows'] == 0
    results['failures']['first_sample'] = first_failed
    browser('timeout')
    lock.stdin.write('ROLLBACK;\n\\q\n')
    lock.stdin.flush()
    lock.wait(timeout=5)
    lock = None
    time.sleep(15)
    refresh(target)
    recovered = wait_sample(target, lambda x: x['modules'][1]['error'] is None and x['modules'][1]['sampled_at'] > prior['modules'][1]['sampled_at'])
    assert_complete(recovered, expected(target))
    results['failures']['recovered'] = True
    results['failures']['database_timeout_errors'] = (h.tmp / 'pg.log').read_text().count('canceling statement due to statement timeout')
    assert results['failures']['database_timeout_errors'] >= 2
    assert h.sql(DB, 'SELECT count(*) FROM knowledge.retention_projects') == '0'
    assert not Upstream.calls
    results.update(api_sha256=h.active_binary_sha, admin_sha256=hashlib.sha256(h.admin.read_bytes()).hexdigest(),
                   model_calls=0, ports=dict(pg=h.pg_port, api=h.url, front=server.server_port))
    (OUT / 'probe-results.json').write_text(json.dumps(results, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'scales': list(projects), 'complete_categories': 24, 'lock_failure_and_recovery': 'passed'}))
finally:
    if lock and lock.poll() is None:
        lock.stdin.write('ROLLBACK;\n\\q\n')
        lock.stdin.flush()
        lock.wait(timeout=5)
    if server:
        server.shutdown()
    h.cleanup()
