#!/usr/bin/env python3
"""Bounded process/API/DB A-B measurement; panel arm replays its two real GETs every 5s."""
import concurrent.futures, hashlib, http.client, json, os, pathlib, signal, socket, statistics, subprocess, tempfile, threading, time, urllib.request, shutil
ROOT = pathlib.Path(__file__).resolve().parents[1]
PG = pathlib.Path('/opt/homebrew/opt/postgresql@17/bin')
BASE = pathlib.Path('/tmp/nexofolio-operations')
fixture = pathlib.Path(__import__('sys').argv[1])
m = json.loads((fixture / 'browser.json').read_text())
out = pathlib.Path(tempfile.mkdtemp(prefix='overhead.', dir=BASE))
out.chmod(448)

def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]

def run(args, **kw):
    return subprocess.run([str(v) for v in args], check=True, **kw)

def source_sql(q):
    return subprocess.check_output([str(PG / 'psql'), '-h', '127.0.0.1', '-p', str(m['pg_port']), '-d', m['database'], '-At', '-c', q], text=True).strip()
instance = source_sql("SELECT instance FROM access.users WHERE lower(account)='a80646' LIMIT 1")
login = json.load(urllib.request.urlopen(urllib.request.Request(m['api'] + '/v1/auth/login', data=json.dumps({'account': 'A80646', 'password': 'fixture-pass'}).encode(), headers={'Content-Type': 'application/json'})))
token = login['token']
pid = None
started = False
port = free_port()
api_port = free_port()
db = 'nexofolio_operations_overhead'
run([PG / 'pg_dump', '-h', '127.0.0.1', '-p', m['pg_port'], '-d', m['database'], '-f', out / 'fixture.sql'])
(out / 'prepared').touch()
(BASE / 'current-overhead-path').write_text(str(out))
print(out, flush=True)

def sql(q):
    return subprocess.check_output([str(PG / 'psql'), '-h', '127.0.0.1', '-p', str(port), '-d', db, '-At', '-c', q], text=True).strip()

def cpu(text):
    parts = text.strip().split(':')
    return sum((float(v) * 60 ** i for i, v in enumerate(reversed(parts))))

def proc_sample(p):
    raw = subprocess.check_output(['ps', '-p', str(p), '-o', 'rss=', '-o', 'time='], text=True).split()
    return {'at': time.time(), 'rss_bytes': int(raw[0]) * 1024, 'cpu_seconds': cpu(raw[1])}

def db_sample():
    value = sql("SELECT json_build_object('calls',(SELECT coalesce(sum(calls),0) FROM public.pg_stat_statements WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database())),'execution_ms',(SELECT coalesce(sum(total_exec_time),0) FROM public.pg_stat_statements WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database())),'connections',(SELECT count(*) FROM pg_stat_activity WHERE datname=current_database())-1)")
    return json.loads(value)
try:
    run([PG / 'initdb', '-D', out / 'pg', '-A', 'trust', '--no-locale'], stdout=subprocess.DEVNULL)
    run([PG / 'pg_ctl', '-D', out / 'pg', '-l', out / 'pg.log', '-o', f'-h 127.0.0.1 -p {port} -k {out} -c shared_preload_libraries=pg_stat_statements', 'start'], stdout=subprocess.DEVNULL)
    started = True
    run([PG / 'createdb', '-h', '127.0.0.1', '-p', port, db])
    run([PG / 'psql', '-h', '127.0.0.1', '-p', port, '-d', db, '-f', out / 'fixture.sql'], stdout=subprocess.DEVNULL, stderr=open(out / 'restore.log', 'w'))
    sql('CREATE EXTENSION pg_stat_statements')
    sql("INSERT INTO knowledge.curation_tasks(id,project_id,target,actor,state,created_at,updated_at) SELECT md5('operations-benchmark-'||n)::uuid,'" + m['projects'][0] + '\'::uuid,md5(\'operations-target-\'||n)::uuid,\'{"kind":"unknown"}\'::jsonb,\'done\',now()-interval \'1 day\'-(n*interval \'1 second\'),now()-interval \'1 day\' FROM generate_series(1,25000) n')
    sql('ANALYZE knowledge.curation_tasks')
    task_rows = int(sql('SELECT count(*) FROM knowledge.curation_tasks'))
    until = time.monotonic() + 1200
    while not (out / 'start').exists():
        if time.monotonic() > until:
            raise TimeoutError('waiting for explicit measurement start')
        time.sleep(0.2)
    binaries = {'baseline': BASE / 'baseline-api', 'current': ROOT / 'target/debug/nexofolio-api'}
    manifest = {k: hashlib.sha256(p.read_bytes()).hexdigest() for k, p in binaries.items()}
    rows = []

    def start(kind):
        global pid
        env = {'DATABASE_URL': f"postgres://{os.environ['USER']}@127.0.0.1:{port}/{db}", 'NEXOFOLIO_BIND_ADDR': f'127.0.0.1:{api_port}', 'NEXOFOLIO_LOG': 'info', 'NEXOFOLIO_ZENTAO_BASE_URL': instance, 'NEXOFOLIO_SESSION_KEY': 'ab' * 32, 'NEXOFOLIO_CATALOG_MODEL_BASE_URL': instance.removesuffix('/api.php/v1') + '/v1', 'NEXOFOLIO_CATALOG_MODEL': 'fixture', 'NEXOFOLIO_CATALOG_MODEL_API_KEY': 'fixture-model-key', 'NEXOFOLIO_MODEL_MAX_CONCURRENCY': '2'}
        pid = subprocess.Popen([str(binaries[kind])], env=env, stdout=open(out / (kind + '-api.log'), 'a'), stderr=subprocess.STDOUT, start_new_session=True)
        for _ in range(100):
            try:
                with urllib.request.urlopen(f'http://127.0.0.1:{api_port}/health/ready', timeout=0.3) as r:
                    if r.status == 200:
                        break
            except Exception:
                time.sleep(0.1)
        else:
            raise RuntimeError('API not ready')
        time.sleep(6)

    def stop():
        global pid
        if pid is not None:
            pid.send_signal(signal.SIGTERM)
            pid.wait(timeout=10)
            pid = None

    def window(kind, panel, label):
        start(kind)
        latencies = []
        failures = []
        monitor_latencies = []
        resource = []
        before = db_sample()
        initial = proc_sample(pid.pid)
        deadline = time.monotonic() + 20
        begin = time.monotonic()

        def call(conn, path, dest):
            t = time.perf_counter()
            try:
                conn.request('GET', path, headers={'Authorization': 'Bearer ' + token})
                r = conn.getresponse()
                r.read()
                if r.status != 200:
                    failures.append({'path': path, 'status': r.status})
                dest.append((time.perf_counter() - t) * 1000)
            except Exception as e:
                failures.append({'path': path, 'error': type(e).__name__})

        def reader():
            conn = http.client.HTTPConnection('127.0.0.1', api_port, timeout=3)
            while time.monotonic() < deadline:
                t = time.monotonic()
                call(conn, f"/v1/projects/{m['projects'][0]}/endpoints", latencies)
                time.sleep(max(0, 0.05 - (time.monotonic() - t)))
            conn.close()

        def watcher():
            conn = http.client.HTTPConnection('127.0.0.1', api_port, timeout=3)
            while time.monotonic() < deadline:
                call(conn, '/v1/operations/summary', monitor_latencies)
                call(conn, '/v1/operations/tasks?limit=20', monitor_latencies)
                time.sleep(min(5, max(0, deadline - time.monotonic())))
            conn.close()

        def sampler():
            while time.monotonic() < deadline:
                resource.append(proc_sample(pid.pid))
                time.sleep(1)
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            jobs = [pool.submit(reader), pool.submit(reader), pool.submit(sampler)]
            if panel:
                jobs.append(pool.submit(watcher))
            for job in jobs:
                job.result()
        elapsed = time.monotonic() - begin
        final = proc_sample(pid.pid)
        after = db_sample()
        values = sorted(latencies)
        row = {'label': label, 'binary': kind, 'panel_polling': panel, 'window_seconds': elapsed, 'requests': len(values), 'throughput_rps': len(values) / elapsed, 'p50_ms': statistics.median(values), 'p95_ms': values[int(len(values) * 0.95)], 'mean_ms': statistics.mean(values), 'cpu_core_average': (final['cpu_seconds'] - initial['cpu_seconds']) / elapsed, 'rss_mean_bytes': statistics.mean((r['rss_bytes'] for r in resource)), 'rss_max_bytes': max((r['rss_bytes'] for r in resource)), 'db_calls_delta': after['calls'] - before['calls'], 'db_execution_ms_delta': after['execution_ms'] - before['execution_ms'], 'db_connections_end': after['connections'], 'panel_requests': len(monitor_latencies), 'panel_mean_ms': statistics.mean(monitor_latencies) if monitor_latencies else None, 'errors': failures, 'resource_samples': resource, 'latency_ms': values}
        if kind == 'current':
            req = urllib.request.Request(f'http://127.0.0.1:{api_port}/v1/operations/summary', headers={'Authorization': 'Bearer ' + token})
            row['final_summary'] = json.load(urllib.request.urlopen(req))
        stop()
        assert not failures, failures[:3]
        rows.append(row)
        (out / 'results.json').write_text(json.dumps({'method': 'Two business readers, 50ms period each; panel-equivalent real GET polling every 5s. Not browser rendering cost or production capacity.', 'task_rows': task_rows, 'binary_sha256': manifest, 'windows': rows}, indent=2))
        print(label, round(row['p95_ms'], 3), 'ms', flush=True)
    for kind, panel, label in [('baseline', False, 'A1-no-sampler'), ('current', False, 'B1-sampler-closed'), ('current', True, 'C1-sampler-panel'), ('current', True, 'C2-sampler-panel'), ('current', False, 'B2-sampler-closed'), ('baseline', False, 'A2-no-sampler')]:
        window(kind, panel, label)
finally:
    if pid is not None and pid.poll() is None:
        pid.terminate()
        pid.wait(timeout=10)
    if started:
        subprocess.run([str(PG / 'pg_ctl'), '-D', str(out / 'pg'), 'stop', '-m', 'fast'], stdout=subprocess.DEVNULL)
    shutil.rmtree(out / 'pg', ignore_errors=True)
    (out / 'fixture.sql').unlink(missing_ok=True)
    print('Cleaned; overhead evidence: ' + str(out), flush=True)
