#!/usr/bin/env python3
"""Own and clean only this acceptance's PG/API/Vite processes. Local synthetic data only."""
import json, os, pathlib, signal, socket, subprocess, tempfile, time
ROOT = pathlib.Path(__file__).resolve().parents[1]
FRONT = ROOT.parent / 'NexoFolio-Front'
BASE = pathlib.Path('/tmp/nexofolio-operations')
BASE.mkdir(exist_ok=True)
out = pathlib.Path(tempfile.mkdtemp(prefix='browser.', dir=BASE))
out.chmod(448)
pg = pathlib.Path(os.environ.get('PG_BIN', '/opt/homebrew/opt/postgresql@17/bin'))

def port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]

def run(*args):
    return subprocess.run([str(x) for x in args], check=True, stdout=subprocess.DEVNULL)

def wait_for(path, proc, timeout=180):
    until = time.monotonic() + timeout
    while not path.exists():
        if proc.poll() is not None:
            raise RuntimeError('fixture exited; inspect ' + str(out))
        if time.monotonic() > until:
            raise TimeoutError(str(path))
        time.sleep(0.2)
children = []
started = False
try:
    pg_port = port()
    front_port = port()
    env = os.environ.copy()
    run(pg / 'initdb', '-D', out / 'pg', '-A', 'trust', '--no-locale')
    run(pg / 'pg_ctl', '-D', out / 'pg', '-l', out / 'pg.log', '-o', f'-h 127.0.0.1 -p {pg_port} -k {out}', 'start')
    started = True
    run(pg / 'createdb', '-h', '127.0.0.1', '-p', pg_port, 'nexofolio_operations_browser')
    env.update(TEST_DATABASE_URL=f"postgres://{os.environ['USER']}@127.0.0.1:{pg_port}/nexofolio_operations_browser", OPERATIONS_EVIDENCE_DIR=str(out))
    log = open(out / 'fixture.log', 'w')
    proc = subprocess.Popen(['cargo', 'test', '-p', 'nexofolio-backend', '--test', 'operations', 'serve_operations_fixture', '--', '--ignored', '--exact', '--nocapture'], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    children.append(proc)
    wait_for(out / 'ready.json', proc)
    m = json.loads((out / 'ready.json').read_text())
    m.update(front=f'http://127.0.0.1:{front_port}', pg_port=pg_port, database='nexofolio_operations_browser')
    (out / 'browser.json').write_text(json.dumps(m))
    env.update(DEV_API_PROXY_TARGET=m['api'], VITE_WORKSPACE_ENABLED='true')
    vite_log = open(out / 'vite.log', 'w')
    vite = subprocess.Popen(['pnpm', 'exec', 'vite', '--host', '127.0.0.1', '--port', str(front_port), '--strictPort'], cwd=FRONT, env=env, stdout=vite_log, stderr=subprocess.STDOUT, start_new_session=True)
    children.append(vite)
    (BASE / 'current-browser-path').write_text(str(out))
    print(out, flush=True)
    until = time.monotonic() + 7000
    while not (out / 'finish').exists() and time.monotonic() < until:
        if proc.poll() is not None or vite.poll() is not None:
            raise RuntimeError('owned fixture stopped unexpectedly')
        time.sleep(0.5)
finally:
    (out / 'stop').touch()
    if children:
        try:
            children[0].wait(timeout=15)
        except subprocess.TimeoutExpired:
            pass
    for proc in reversed(children):
        if proc.poll() is None:
            os.killpg(proc.pid, signal.SIGTERM)
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
    if started:
        subprocess.run([str(pg / 'pg_ctl'), '-D', str(out / 'pg'), 'stop', '-m', 'fast'], stdout=subprocess.DEVNULL)
    import shutil
    shutil.rmtree(out / 'pg', ignore_errors=True)
    shutil.rmtree(out / 'chrome-profile', ignore_errors=True)
    shutil.rmtree(out / 'states-profile', ignore_errors=True)
    (out / 'api-runtime').unlink(missing_ok=True)
    print('Cleaned owned processes and private data; evidence: ' + str(out), flush=True)
