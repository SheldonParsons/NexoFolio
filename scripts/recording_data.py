#!/usr/bin/env python3
"""Offline local-Compose recording reset/snapshot/restore. No business payload logging."""
import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import tarfile
import uuid

ROOT = Path(__file__).resolve().parents[1]
PRESERVE = set('users internal_sessions projects user_project_access login_audit environments environment_names'.split())
RECORDING = set('''capture_assets capture_backlog capture_blobs capture_events capture_receipts
catalog_activation_receipts catalog_preview_assignments catalog_preview_nodes catalog_preview_tasks
catalog_version_assignments catalog_version_nodes catalog_versions evidence_facts evidence_pins
evidence_relation_pairs evidence_relation_values evidence_sample_groups evidence_samples
 evidence_ui_bindings evidence_ui_pairs evidence_value_index ingestion_heads ingestion_inbox
 ingestion_receipts interface_documents interface_environment_current interface_observations
 interface_observed_differences interface_observed_revisions knowledge_activation_receipts
 knowledge_releases maintenance_calls maintenance_checkpoints maintenance_runs project_catalogs'''.split())


def run(args, *, input=None, stdout=subprocess.PIPE, stdin=None):
    result = subprocess.run(args, input=input, stdin=stdin, stdout=stdout, stderr=subprocess.PIPE)
    if result.returncode:
        # Do not print pg_restore/SQL errors which may contain original data.
        raise RuntimeError(f'{args[0]} command failed (exit {result.returncode}); writers remain stopped if maintenance had begun')
    return result.stdout


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def validate_archive(path):
    with tarfile.open(path, 'r:gz') as archive:
        for entry in archive:
            name = PurePosixPath(entry.name)
            if name.is_absolute() or '..' in name.parts or not (entry.isfile() or entry.isdir()):
                raise RuntimeError('Unsafe or unsupported entry in volume archive')


class Store:
    def __init__(self, project):
        self.project = project
        ids = run(['docker', 'ps', '-aq', '--filter', f'label=com.docker.compose.project={project}']).decode().split()
        if not ids:
            raise RuntimeError('No containers for this Compose project')
        containers = json.loads(run(['docker', 'inspect', *ids]))
        self.services = {}
        for service in ('db', 'api', 'worker'):
            matches = [c for c in containers if c['Config']['Labels'].get('com.docker.compose.service') == service
                       and c['Config']['Labels'].get('com.docker.compose.oneoff', 'false').lower() != 'true']
            if len(matches) != 1:
                raise RuntimeError(f'Expected exactly one {service} container')
            self.services[service] = matches[0]
        self.db = self.services['db']['Id']
        if not self.services['db']['State']['Running']:
            raise RuntimeError('Database must already be running')
        volumes = []
        for service in ('api', 'worker'):
            mounts = [m for m in self.services[service]['Mounts'] if m['Destination'] == '/var/lib/nexofolio/blobs' and m['Type'] == 'volume']
            if len(mounts) != 1:
                raise RuntimeError('Expected named capture volume mounted on API and worker')
            volumes.append(mounts[0]['Name'])
        if volumes[0] != volumes[1]:
            raise RuntimeError('API/worker capture volumes differ')
        self.volume = volumes[0]
        labels = json.loads(run(['docker', 'volume', 'inspect', self.volume]))[0].get('Labels') or {}
        if labels.get('com.docker.compose.project') != project:
            raise RuntimeError('Capture volume is not owned by the selected project')
        self.image = self.services['db']['Image']
        self.running = [c['Id'] for name, c in self.services.items() if name != 'db' and c['State']['Running']]
        self.tables = json.loads(self.sql("SELECT coalesce(json_agg(tablename ORDER BY tablename),'[]'::json) FROM pg_tables WHERE schemaname='public'"))
        unknown = set(self.tables) - PRESERVE - RECORDING - {'_sqlx_migrations'}
        if unknown:
            raise RuntimeError(f'Unreviewed tables; update the explicit reset scope first: {sorted(unknown)}')

    def pg(self, executable, args, **kwargs):
        # Credentials stay inside the already configured DB container.
        command = 'exec "$@" -U "$POSTGRES_USER" -d "$POSTGRES_DB"'
        return run(['docker', 'exec', '-i', self.db, 'sh', '-c', command, 'db-tool', executable, *args], **kwargs)

    def sql(self, query):
        return self.pg('psql', ['-X', '-q', '-A', '-t', '-v', 'ON_ERROR_STOP=1'], input=query.encode()).decode().strip()

    def counts(self):
        if not self.tables:
            return {}
        query = ' UNION ALL '.join(f'SELECT \'{t}\' AS name,count(*) AS count FROM public."{t}"' for t in self.tables)
        return json.loads(self.sql(f'SELECT json_object_agg(name,count) FROM ({query}) counts'))

    def volume_command(self, args, **kwargs):
        return run(['docker', 'run', '--rm', '-i', '--network', 'none', '--user', '0:0', '--mount',
                    f'type=volume,src={self.volume},dst=/data', '--entrypoint', args[0], self.image, *args[1:]], **kwargs)

    @contextmanager
    def offline(self, restart=True):
        if self.running:
            run(['docker', 'stop', '--time', '15', *self.running])
        # Another writer to this volume or database invalidates the backup boundary.
        users = run(['docker', 'ps', '-q', '--filter', f'volume={self.volume}']).decode().split()
        if users or self.sql("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND backend_type='client backend' AND pid<>pg_backend_pid()") != '0':
            raise RuntimeError('Other volume/database clients are still active; no data was cleared')
        yield
        if restart and self.running:
            run(['docker', 'start', *self.running])
        # On failure do not start a possibly mixed database/volume state.

    def snapshot(self, destination, pin=False):
        destination.mkdir(parents=True, exist_ok=False, mode=0o700)
        dataset = str(uuid.uuid4())
        if pin:
            required = {'capture_events', 'evidence_pins'}
            if not required <= set(self.tables):
                raise RuntimeError('Capture tables required to preserve a recording dataset')
            self.sql(f"INSERT INTO evidence_pins(owner_kind,owner_id,event_id) SELECT 'recording_dataset','{dataset}',id FROM capture_events WHERE raw_hash IS NOT NULL ON CONFLICT DO NOTHING")
        dump, archive = destination / 'database.dump', destination / 'blobs.tar.gz'
        with dump.open('wb') as stream:
            self.pg('pg_dump', ['--format=custom', '--no-owner', '--no-privileges'], stdout=stream)
        with archive.open('wb') as stream:
            self.volume_command(['tar', '-C', '/data', '-czf', '-', '.'], stdout=stream)
        validate_archive(archive)
        # Ensure the DB archive is readable before any reset is allowed.
        with dump.open('rb') as stream:
            self.pg('pg_restore', ['--list'], stdin=stream)
        manifest = {'format': 1, 'dataset_id': dataset, 'created_at': datetime.now(timezone.utc).isoformat(),
                    'project': self.project, 'volume': self.volume, 'recording_pinned': pin,
                    'counts': self.counts(), 'missing_raw_events': int(self.sql('SELECT count(*) FROM capture_events WHERE raw_hash IS NULL')) if 'capture_events' in self.tables else 0, 'images': {k: v['Image'] for k, v in self.services.items()},
                    'files': {p.name: digest(p) for p in (dump, archive)}}
        (destination / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        return manifest

    def reset(self, all_data=False):
        selected = sorted(set(self.tables) & (RECORDING | (PRESERVE if all_data else set())))
        if selected:
            # Keep the system folder identity, but invalidate the cleared catalog generation.
            # The table must participate in TRUNCATE because it references cleared versions.
            rebuild_catalog = not all_data and 'project_catalogs' in selected
            save_catalog = ('CREATE TEMP TABLE reset_catalog_basis ON COMMIT DROP AS '
                            'SELECT project_id,unclassified_id,generation FROM project_catalogs; '
                            if rebuild_catalog else '')
            restore_catalog = ("INSERT INTO project_catalogs(project_id,unclassified_id,generation) "
                               "SELECT p.id,coalesce(b.unclassified_id,gen_random_uuid()),coalesce(b.generation+1,0) "
                               "FROM projects p LEFT JOIN reset_catalog_basis b ON b.project_id=p.id; "
                               if rebuild_catalog else '')
            # No CASCADE: a newly introduced dependency must fail rather than broaden deletion.
            self.sql('BEGIN; ' + save_catalog + 'TRUNCATE ' +
                     ','.join(f'public."{t}"' for t in selected) +
                     ' RESTART IDENTITY; ' + restore_catalog + 'COMMIT;')
        # A nonempty volume prevents Docker copy-up from replacing its UID/mode
        # with the next backend image's default owner when containers are recreated.
        self.volume_command(['sh', '-c', 'find /data -mindepth 1 ! -path /data/.initialized -delete && touch /data/.initialized'])
        counts = self.counts()
        cleared = set(selected) - ({'project_catalogs'} if not all_data else set())
        if any(counts[t] for t in cleared):
            raise RuntimeError('Reset verification failed')
        if not all_data and 'project_catalogs' in selected:
            missing = self.sql("SELECT count(*) FROM projects p LEFT JOIN project_catalogs c ON c.project_id=p.id WHERE c.project_id IS NULL OR c.current_version_id IS NOT NULL OR c.current_knowledge_version_id IS NOT NULL")
            if missing != '0' or counts['project_catalogs'] != counts['projects']:
                raise RuntimeError('Project catalog initialization failed after reset')
        return counts

    def restore(self, directory, manifest):
        snapshot_tables = set(manifest['counts'])
        if self.tables and set(self.tables) != snapshot_tables:
            raise RuntimeError('Target schema differs: restore into an empty isolated DB, then apply newer migrations')
        with (directory / 'database.dump').open('rb') as stream:
            self.pg('pg_restore', ['--clean', '--if-exists', '--single-transaction', '--no-owner', '--no-privileges', '--exit-on-error'], stdin=stream)
        self.volume_command(['find', '/data', '-mindepth', '1', '-delete'])
        with (directory / 'blobs.tar.gz').open('rb') as stream:
            self.volume_command(['tar', '--numeric-owner', '-C', '/data', '-xzf', '-'], stdin=stream)
        self.tables = sorted(snapshot_tables)
        if self.counts() != manifest['counts']:
            raise RuntimeError('Restored database counts differ from snapshot')


def read_manifest(directory):
    manifest = json.loads((directory / 'manifest.json').read_text())
    if manifest.get('format') != 1 or set(manifest.get('files', {})) != {'database.dump', 'blobs.tar.gz'}:
        raise RuntimeError('Unsupported or incomplete snapshot')
    for name, expected in manifest['files'].items():
        if digest(directory / name) != expected:
            raise RuntimeError(f'Snapshot checksum mismatch: {name}')
    validate_archive(directory / 'blobs.tar.gz')
    return manifest


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project', default='nexofolio-local', help='Exact Docker Compose project label')
    parser.add_argument('--keep-stopped', action='store_true', help='Keep API/worker stopped after maintenance (for plugin queue cleanup or isolated replay)')
    sub = parser.add_subparsers(dest='action', required=True)
    sub.add_parser('inspect')
    reset = sub.add_parser('reset')
    reset.add_argument('--scope', choices=['recording', 'all'], default='recording')
    sub.add_parser('snapshot').add_argument('directory', type=Path)
    sub.add_parser('restore').add_argument('directory', type=Path)
    args = parser.parse_args()
    backups = ROOT / 'backups'
    backups.mkdir(exist_ok=True, mode=0o700)
    # Same-host maintenance runs on the same Compose project must not overlap.
    lock_name = hashlib.sha256(args.project.encode()).hexdigest()[:16]
    with (backups / f'.{lock_name}.lock').open('w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        store = Store(args.project)
        if args.action == 'inspect':
            print(json.dumps({'project': args.project, 'volume': store.volume, 'counts': store.counts()}, indent=2))
            return
        manifest = read_manifest(args.directory.resolve()) if args.action == 'restore' else None
        with store.offline(restart=not args.keep_stopped):
            before = store.counts()
            if args.action == 'snapshot':
                store.snapshot(args.directory.resolve(), pin=True)
                print(f'Recording snapshot: {args.directory.resolve()}')
            else:
                safety = backups / (datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ') + '-' + uuid.uuid4().hex[:8])
                store.snapshot(safety)
                print(f'Automatic safety backup: {safety}', flush=True)
                if args.action == 'reset':
                    after = store.reset(args.scope == 'all')
                    if args.scope == 'recording' and any(after.get(t) != before.get(t) for t in PRESERVE):
                        raise RuntimeError('Protected account/project/environment counts changed')
                    print(json.dumps({'scope': args.scope, 'counts': after}, indent=2))
                else:
                    store.restore(args.directory.resolve(), manifest)
                    print(f'Restored dataset: {manifest["dataset_id"]}')
        print('Completed; API/worker remain stopped.' if args.keep_stopped else 'Completed; originally running API/worker restarted.')
        print('Plugin queues are not modified.')


if __name__ == '__main__':
    try:
        main()
    except (RuntimeError, OSError, ValueError, tarfile.TarError) as exc:
        raise SystemExit(str(exc))
