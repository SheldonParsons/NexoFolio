"""Stage 2e, one-off: replay the old HTTP captures into collect v1. Delete after acceptance.

Reads the stage-4 backup (pg_dump custom format + blob archive) directly, without restoring it,
converts old http_exchange events to collect v1 records and posts them to a running backend.
Only counts are printed; payloads and headers (they carry real tokens) are never logged.
The summary and the old interface list go to experiments/results/legacy-replay/ (gitignored).

    python3 replay.py --backup backups/stage4-current-input-20260917 \
        --base-url http://127.0.0.1:18080 --project-id <new project uuid>
"""

import argparse, json, re, tarfile, time, urllib.error, urllib.request, uuid, zlib
from datetime import datetime, timezone
from itertools import groupby
from pathlib import Path

BATCH = 50
NAMESPACE = uuid.UUID('6c0f3c55-2e1b-4d3a-9c55-2e0e5a7b1f00')  # stable batch ids make reruns idempotent


class Dump:
    """Reader for pg_dump custom format 1.14–1.16 (no pg_restore needed)."""

    def __init__(self, path):
        self.f = f = open(path, 'rb')
        assert f.read(5) == b'PGDMP', 'not a pg_dump custom archive'
        self.ver = tuple(f.read(3))
        self.intsize, self.offsize, _ = f.read(3)
        self.comp = f.read(1)[0] if self.ver >= (1, 15, 0) else self.int()
        for _ in range(7):
            self.int()  # creation time
        self.str(), self.str(), self.str()  # database, server and pg_dump versions
        self.entries = []
        for _ in range(self.int()):
            e = dict(id=self.int(), dumper=self.int(), tableoid=self.str(), oid=self.str(), tag=self.str(),
                     desc=self.str(), section=self.int(), defn=self.str(), drop=self.str(), copy=self.str(),
                     ns=self.str(), tbs=self.str())
            if self.ver >= (1, 14, 0):
                self.str()  # table access method
            if self.ver >= (1, 16, 0):
                self.int()  # relkind
            self.str(), self.str()  # owner, with_oids
            while self.str() is not None:
                pass  # dependencies
            f.read(1 + self.offsize)  # data state and offset
            self.entries.append(e)
        # A dump written to a pipe has no offsets, so the data blocks are indexed by scanning.
        self.blocks = {}
        while f.read(1):
            dump_id, chunks = self.int(), []
            while (n := self.int()) > 0:
                chunks.append(f.read(n))
            self.blocks[dump_id] = b''.join(chunks)

    def int(self):
        sign = self.f.read(1)[0]
        value = int.from_bytes(self.f.read(self.intsize), 'little')
        return -value if sign else value

    def str(self):
        n = self.int()
        return None if n < 0 else self.f.read(n).decode()

    def rows(self, table):
        """Rows of public.<table> as dicts; COPY text format, \\N is null."""
        e = next(e for e in self.entries if e['desc'] == 'TABLE DATA' and e['tag'] == table and e['ns'] == 'public')
        raw = self.blocks.get(e['id'], b'')
        if self.comp == 1:
            raw = zlib.decompress(raw)
        elif self.comp:
            raise ValueError(f'unsupported compression {self.comp}')
        columns = e['copy'].split('(', 1)[1].split(')', 1)[0].split(', ')
        for line in raw.decode().split('\n'):
            if line and line != '\\.':
                values = [None if v == '\\N' else unescape(v) for v in line.split('\t')]
                yield dict(zip(columns, values))


COPY_ESCAPES = {'\\': '\\', 'n': '\n', 't': '\t', 'r': '\r', 'b': '\b', 'f': '\f', 'v': '\v'}


def unescape(value):
    return re.sub(r'\\(.)', lambda m: COPY_ESCAPES.get(m.group(1), m.group(1)), value)


def iso(value):
    if isinstance(value, (int, float)):
        moment = datetime.fromtimestamp(value / 1000, timezone.utc)
    else:
        # pg_dump writes timestamptz in UTC as '2026-09-16 02:26:00.24+00'.
        assert value.endswith('+00'), value
        moment = datetime.strptime(value[:-3], '%Y-%m-%d %H:%M:%S.%f' if '.' in value else '%Y-%m-%d %H:%M:%S').replace(tzinfo=timezone.utc)
    return moment.astimezone(timezone.utc).isoformat(timespec='milliseconds').replace('+00:00', 'Z')


def media_type(headers):
    return next((v[:255] for k, v in headers.get('entries', []) if k.lower() == 'content-type'), None)


def body(old, headers):
    state = old.get('state')
    if state == 'complete':
        wire = {'state': 'full', 'encoding': 'utf8' if old.get('encoding') == 'text' else 'base64', 'content': old['content']}
        if isinstance(old.get('bytes'), int):
            wire['bytes'] = old['bytes']
    elif state == 'none':
        return {'state': 'none'}
    else:
        wire = {'state': 'unreadable', 'note': f'legacy body state: {state}'[:1024]}
    if (media := media_type(headers)):
        wire['media_type'] = media
    return wire


def headers(old):
    # The old plugin only saw page-visible headers, which is what collect calls partial.
    return {'state': 'partial', 'entries': [[k, v] for k, v in old.get('entries', [])]}


def record(event, blob):
    c = json.loads(event['context'] or '{}')
    request, response = blob['request'], blob.get('response') or {}
    context = {
        'page_url': c.get('page_url') or blob.get('source_page'),
        'page_id': c.get('page_instance_id'), 'frame_id': c.get('frame_instance_id'),
        'view_id': c.get('view_id'), 'interaction_id': c.get('interaction_id'),
        'seq': c.get('event_seq'),
        'transport': blob.get('transport') if blob.get('transport') in ('xhr', 'fetch') else 'other',
        'frame': 'iframe' if blob.get('frame') == 'iframe' else 'page',
        'started_at': iso(c['request_started_at_ms']) if c.get('request_started_at_ms') else None,
        'completed_at': iso(c['response_completed_at_ms']) if c.get('response_completed_at_ms') else None,
    }
    payload = {'request': {'method': request['method'], 'url': request['url'],
                           'headers': headers(request['headers']), 'body': body(request['body'], request['headers'])}}
    if request.get('url_truncated'):
        payload['request']['url_truncated'] = True
    if response.get('state') == 'complete' and isinstance(response.get('status'), int):
        payload['response'] = {'status': response['status'], 'headers': headers(response['headers']),
                               'body': body(response['body'], response['headers'])}
    return {'id': event['record_id'], 'kind': 'http_exchange', 'version': 1, 'observed_at': iso(event['captured_at']),
            'context': {k: v for k, v in context.items() if v is not None}, 'payload': payload}


def post(base_url, batch):
    data = json.dumps(batch, ensure_ascii=False).encode()
    while True:
        request = urllib.request.Request(base_url.rstrip('/') + '/v1/collect/batches', data, {'content-type': 'application/json'})
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return response.status, json.load(response)
        except urllib.error.HTTPError as error:
            if error.code in (429, 503):
                time.sleep(int(error.headers.get('retry-after') or 1))
                continue
            return error.code, json.loads(error.read() or b'{}')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--backup', type=Path, required=True)
    parser.add_argument('--base-url', required=True)
    parser.add_argument('--project-id', required=True, help='project id in the new backend')
    parser.add_argument('--out', type=Path, default=Path(__file__).resolve().parent.parent / 'results' / 'legacy-replay')
    args = parser.parse_args()

    dump = Dump(args.backup / 'database.dump')
    environments = {row['id']: row['name'] for row in dump.rows('environments')}
    events = list(dump.rows('capture_events'))
    exchanges = sorted((e for e in events if e['kind'] == 'http_exchange'), key=lambda e: (e['environment_id'], e['captured_at']))
    with tarfile.open(args.backup / 'blobs.tar.gz') as archive:
        blobs = {Path(m.name).name: archive.extractfile(m).read() for m in archive.getmembers() if m.isfile()}

    summary = {'events': len(events), 'kinds': {}, 'environments': {}, 'batches': 0, 'accepted': 0, 'rejected': [], 'errors': []}
    for event in events:
        summary['kinds'][event['kind']] = summary['kinds'].get(event['kind'], 0) + 1
    for environment_id, group in groupby(exchanges, key=lambda e: e['environment_id']):
        name = environments[environment_id]
        records = [record(e, json.loads(blobs[e['raw_hash']])) for e in group]
        summary['environments'][name] = len(records)
        for start in range(0, len(records), BATCH):
            chunk = records[start:start + BATCH]
            batch_id = str(uuid.uuid5(NAMESPACE, ','.join(r['id'] for r in chunk)))
            batch = {'batch_id': batch_id, 'platform': 'nexofolio-fetcher', 'platform_version': 'legacy-replay',
                     'target': {'project_id': args.project_id, 'environment': {'name': name}}, 'records': chunk}
            status, receipt = post(args.base_url, batch)
            summary['batches'] += 1
            if status == 200:
                summary['accepted'] += receipt['accepted']
                summary['rejected'] += [{'id': r['id'], 'reason': r['reason'], 'message': r.get('message')} for r in receipt['rejected']]
            else:
                summary['errors'].append({'batch_id': batch_id, 'status': status, 'code': receipt.get('code')})
            print(f'{name}: batch {summary["batches"]} status {status}', flush=True)

    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / 'summary.json').write_text(json.dumps(summary, ensure_ascii=False, indent=2) + '\n')
    old = sorted(f"{row['method']} {row['path']}" for row in dump.rows('interface_documents'))
    (args.out / 'old-interfaces.txt').write_text('\n'.join(old) + '\n')
    print(f"accepted {summary['accepted']}, rejected {len(summary['rejected'])}, failed batches {len(summary['errors'])}; old interfaces {len(old)}")


if __name__ == '__main__':
    main()
