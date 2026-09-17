"""Destructive-script verification against disposable labelled containers only."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('recording_data', ROOT / 'scripts/recording_data.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
run = module.run
project = 'nexo-reset-test-' + uuid.uuid4().hex[:10]
volume = project + '-capture'
password = uuid.uuid4().hex
containers = []
with tempfile.TemporaryDirectory(prefix='nexo-recording-test-') as directory:
    temporary = Path(directory)
    try:
        run(['docker', 'volume', 'create', '--label', f'com.docker.compose.project={project}', volume])
        for service in ('db', 'api', 'worker'):
            args = ['docker', 'run', '-d', '--name', f'{project}-{service}', '--label', f'com.docker.compose.project={project}', '--label', f'com.docker.compose.service={service}']
            if service == 'db':
                args += ['-e', 'POSTGRES_USER=nexofolio', '-e', 'POSTGRES_DB=nexofolio', '-e', f'POSTGRES_PASSWORD={password}', '-p', '127.0.0.1::5432', 'postgres:17-bookworm']
            else:
                args += ['--mount', f'type=volume,src={volume},dst=/var/lib/nexofolio/blobs', '--entrypoint', 'sleep', 'postgres:17-bookworm', 'infinity']
            containers.append(run(args).decode().strip())
        for _ in range(60):
            if subprocess.run(['docker','exec',containers[0],'pg_isready','-h','127.0.0.1','-U','nexofolio'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
                break
            time.sleep(.2)
        port = run(['docker','port',containers[0],'5432/tcp']).decode().strip().rsplit(':',1)[1]
        env = os.environ.copy() | {'DATABASE_URL': f'postgres://nexofolio:{password}@127.0.0.1:{port}/nexofolio'}
        subprocess.run([str(ROOT/'target/debug/nexofolio-admin'), 'migrate'], env=env, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        store = module.Store(project)
        user, proj, environment, document, event = [str(uuid.uuid4()) for _ in range(5)]
        store.sql(f"""INSERT INTO users(id,instance,external_id,account,display_name) VALUES('{user}','test','u','test','Synthetic');
        INSERT INTO internal_sessions(user_id,token_hash,token_encrypted,issued_at,expires_at) VALUES('{user}','\\x01','\\x02',now(),now()+interval '3 months');
        INSERT INTO projects(id,instance,external_id,name,status,last_sync_generation) VALUES('{proj}','test','p','Synthetic','doing',1);
        INSERT INTO user_project_access VALUES('{user}','{proj}');
        INSERT INTO environments(id,project_id,name) VALUES('{environment}','{proj}','Test');
        INSERT INTO environment_names VALUES('{proj}','Test','{environment}');
        INSERT INTO interface_documents(id,project_id,identity_key,method,path) VALUES('{document}','{proj}','GET /test','GET','/test');
        INSERT INTO capture_blobs(project_id,sha256,bytes,media_type) VALUES('{proj}','synthetic',7,'application/json');
        INSERT INTO capture_events(id,project_id,environment_id,actor_id,producer_id,record_id,source_type,kind,payload_version,captured_at,raw_hash,content_hash,structure) VALUES('{event}','{proj}','{environment}','{user}','{user}','{event}','test','page_context','1',now(),'synthetic','\\x03','not_applicable');""")
        store.volume_command(['sh','-c','mkdir -p /data/originals && printf sample > /data/originals/payload && chown 501:20 /data && chmod 700 /data'])
        catalog_before = json.loads(store.sql("SELECT row_to_json(c) FROM project_catalogs c"))
        baseline = temporary/'baseline'
        with store.offline():
            manifest = store.snapshot(baseline, pin=True)
        assert store.sql("SELECT count(*) FROM evidence_pins WHERE owner_kind='recording_dataset'") == '1'
        protected = {t: store.sql(f'SELECT coalesce(json_agg(row_to_json(t)),\'[]\'::json) FROM public."{t}" t') for t in module.PRESERVE}
        # Exercise CLI automatic backup, stopped writers and restart, not only internal helpers.
        result = run(['python3',str(ROOT/'scripts/recording_data.py'),'--project',project,'reset']).decode()
        safety = Path(next(line.split(': ',1)[1] for line in result.splitlines() if line.startswith('Automatic safety backup:')))
        assert module.read_manifest(safety)['counts']['interface_documents'] == 1
        assert store.sql('SELECT count(*) FROM interface_documents') == '0'
        assert all(store.counts()[t] == 0 for t in module.RECORDING - {'project_catalogs'})
        catalog_after = json.loads(store.sql("SELECT row_to_json(c) FROM project_catalogs c"))
        assert catalog_after['unclassified_id'] == catalog_before['unclassified_id']
        assert catalog_after['generation'] == catalog_before['generation'] + 1
        assert catalog_after['current_version_id'] is None and catalog_after['current_knowledge_version_id'] is None
        with store.offline():
            store.reset()
        repeated_catalog = json.loads(store.sql("SELECT row_to_json(c) FROM project_catalogs c"))
        assert repeated_catalog['unclassified_id'] == catalog_before['unclassified_id']
        assert repeated_catalog['generation'] == catalog_after['generation'] + 1
        for table, before in protected.items():
            assert store.sql(f'SELECT coalesce(json_agg(row_to_json(t)),\'[]\'::json) FROM public."{table}" t') == before
        assert store.volume_command(['find','/data','-mindepth','1']).strip() == b'/data/.initialized'
        # Recreating a real backend mount used to reset the empty volume to UID10001,
        # even though the configured service UID501 had been preserved before reset.
        run(['docker','run','--rm','--network','none','--user','501:20',
             '--mount',f'type=volume,src={volume},dst=/var/lib/nexofolio/blobs',
             '--entrypoint','sh',os.environ.get('BACKEND_IMAGE','nexofolio-backend:foundation'),
             '-c','test -w /var/lib/nexofolio/blobs && touch /var/lib/nexofolio/blobs/.write-probe && rm /var/lib/nexofolio/blobs/.write-probe'])
        with store.offline():
            store.restore(baseline, module.read_manifest(baseline))
        assert store.counts() == manifest['counts']
        assert store.volume_command(['cat','/data/originals/payload']) == b'sample'
        assert store.volume_command(['stat','-c','%u:%g %a','/data']).strip() == b'501:20 700'
        # A missing state with no history is safely repaired; healthy state is byte-for-byte stable.
        repair = (ROOT/'migrations/202609160003_repair_missing_project_catalogs.sql').read_text()
        intact = store.sql("SELECT row_to_json(c) FROM project_catalogs c")
        store.sql(repair)
        assert store.sql("SELECT row_to_json(c) FROM project_catalogs c") == intact
        with store.offline():
            store.reset()
            store.sql('TRUNCATE project_catalogs')
            store.sql(repair)
            repaired = store.sql("SELECT row_to_json(c) FROM project_catalogs c")
            store.sql(repair)
            assert store.sql("SELECT row_to_json(c) FROM project_catalogs c") == repaired
            # A retained release makes choosing a new system folder unsafe: refuse atomically.
            store.sql(f"INSERT INTO knowledge_releases(id,project_id,annotations,origin) VALUES('{uuid.uuid4()}','{proj}','[]','audit'); TRUNCATE project_catalogs")
            try:
                store.sql(repair)
                raise AssertionError('ambiguous historical state was repaired blindly')
            except RuntimeError:
                pass
            assert store.sql('SELECT count(*) FROM project_catalogs') == '0'
            # Reset is explicitly clearing that history, and must recreate the existing project state.
            store.reset()
            assert store.sql('SELECT count(*) FROM project_catalogs') == '1'
            store.restore(baseline, module.read_manifest(baseline))
        # Corrupt snapshots reject before the target is stopped or changed.
        damaged = temporary/'damaged'
        shutil.copytree(baseline, damaged)
        with (damaged/'database.dump').open('ab') as f:
            f.write(b'bad')
        try:
            module.read_manifest(damaged)
            raise AssertionError('corrupt archive accepted')
        except RuntimeError:
            pass
        # An unknown table refuses even all-data resets rather than broadening scope.
        store.sql('CREATE TABLE future_business_table(id int)')
        try:
            module.Store(project)
            raise AssertionError('unknown table accepted')
        except RuntimeError:
            pass
        store.sql('DROP TABLE future_business_table')
        with store.offline():
            all_counts = store.reset(all_data=True)
            assert all(v == 0 for t,v in all_counts.items() if t != '_sqlx_migrations')
            assert all_counts['_sqlx_migrations'] == manifest['counts']['_sqlx_migrations']
            store.reset(all_data=True)  # repeat-safe
            store.restore(baseline, module.read_manifest(baseline))
        assert store.counts() == manifest['counts']
        states = json.loads(run(['docker','inspect',*containers]))
        assert all(c['State']['Running'] for c in states)
        kept = run(['python3',str(ROOT/'scripts/recording_data.py'),'--project',project,'--keep-stopped','reset']).decode()
        kept_backup = Path(next(line.split(': ',1)[1] for line in kept.splitlines() if line.startswith('Automatic safety backup:')))
        states = json.loads(run(['docker','inspect',*containers]))
        assert states[0]['State']['Running'] and not any(c['State']['Running'] for c in states[1:])
        shutil.rmtree(kept_backup)
        shutil.rmtree(safety)
        print('PASS: preserved system-folder IDs, repaired missing catalog state, history guard, snapshot+pins, auto-backup, recording reset, protected rows unchanged, full reset, repeated reset, exact DB/blob restore+permissions, corrupt snapshot rejection, unknown-table rejection, service restart')
    finally:
        for container in containers:
            subprocess.run(['docker','rm','-fv',container],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        subprocess.run(['docker','volume','rm',volume],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
