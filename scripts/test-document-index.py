#!/usr/bin/env python3
"""Step 07: reuse owned PG/API fixtures; serve actual production assets to isolated Chrome."""
import argparse, hashlib, json, pathlib, subprocess, threading
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from urllib.request import Request, urlopen
from urllib.error import HTTPError
import importlib.util
spec = importlib.util.spec_from_file_location('mcp_driver', pathlib.Path(__file__).with_name('test-mcp-demand-read.py'))
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)
Bench, request, save = driver.Bench, driver.request, driver.save
from frozen_input_harness import Upstream, ident
from mcp_read_fixture import install_identity, enrich, literal


def legacy(h, project):
    base = f'/v1/projects/{project}'
    def get(path):
        status, result = h.request(base + path)
        assert status == 200, (path, status, result)
        return result
    catalog = get('/catalog')
    summaries = {e['id']: e for e in get('/endpoints')['endpoints']}
    entries = []
    for folder in [f['id'] for f in catalog['folders']] + [None]:
        for page in range(1, 100):
            route = f'/catalog/interfaces?directory_id={folder}&' if folder else '/catalog/unclassified?'
            result = get(route + f'page={page}&limit=100')
            entries += [dict(e, folderId=folder, summary=summaries[e['interface_id']]) for e in result['items']]
            if page * result['limit'] >= result['total']:
                break
        else:
            raise AssertionError('pagination did not end')
    return dict(catalog=catalog, entries=entries, lifecycle=get('/document-lifecycle'))


def normalized(index):
    # Public projection deliberately omits author IDs/timestamps and private note context.
    folders = [{k: f[k] for k in ['id','parent','name','summary','position','endpoints','endpoints_deep']} for f in index['catalog']['folders']]
    entries = [dict(e, described_by=e.get('described_by')) for e in index['entries']]
    return dict(catalog=dict(index['catalog'], folders=folders), entries=sorted(entries,key=lambda e:e['interface_id']), lifecycle=index['lifecycle'])


def browser(h, assets, out, meta, label, login):
    class Web(SimpleHTTPRequestHandler):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, directory=str(assets), **kwargs)
        def log_message(self, *args):
            pass
        def do_GET(self):
            if self.path.startswith('/api/'):
                headers = {k:v for k,v in self.headers.items() if k.lower() in ['authorization','accept']}
                try:
                    reply = urlopen(Request(h.url + self.path[4:], headers=headers), timeout=120)
                except HTTPError as error:
                    reply = error
                data = reply.read()
                self.send_response(reply.code)
                self.send_header('Content-Type', reply.headers.get('content-type','application/json'))
                self.send_header('Content-Length', str(len(data)))
                self.send_header('Cache-Control','no-store')
                self.end_headers()
                try:
                    self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError):
                    pass
            else:
                if not (assets / self.path.lstrip('/').split('?')[0]).is_file():
                    self.path = '/index.html'
                super().do_GET()
    server = ThreadingHTTPServer(('127.0.0.1',0), Web)
    threading.Thread(target=server.serve_forever,daemon=True).start()
    try:
        config = dict(meta, api=h.url, front=f'http://127.0.0.1:{server.server_port}', label=label, out=str(out), login=login)
        result = subprocess.run(['node',str(pathlib.Path(__file__).with_name('test-document-index-browser.cjs'))],input=json.dumps(config),text=True)
        assert result.returncode == 0, label
    finally:
        server.shutdown()
        server.server_close()


def main():
    parser = argparse.ArgumentParser()
    for name in ['before','after','admin','front_before','front_after','out']:
        parser.add_argument('--'+name.replace('_','-'), required=True, type=pathlib.Path)
    parser.add_argument('--skip-browser', action='store_true')
    a = parser.parse_args()
    a.out.mkdir(parents=True,exist_ok=True)
    pathlib.Path('/tmp/nexofolio-step04').mkdir(exist_ok=True)
    install_identity()
    h = Bench(a.out,a.admin.resolve())
    try:
        seed = 'nexofolio_step04_document_seed'
        h.create(seed)
        h.start(seed,a.before,'seed')
        meta = enrich(h,seed,h.seed(seed,503))
        actor = dict(kind='system',id='synthetic',display_name='synthetic')
        state = dict(version=1,reconciled_version=1,decisions=[dict(target=dict(kind='endpoint',id=meta['first']),reason='synthetic',actor=actor,at='2026-10-14T00:00:00Z',version=1,retired=True,folders=[])])
        h.sql(seed,f"INSERT INTO knowledge.document_lifecycle(project_id,state) VALUES('{meta['project']}',{literal(state)}::jsonb)")
        # A second accessible project exercises route replacement; denied access is checked before this grant.
        save(a.out/'fixture.json',meta)
        h.stop()
        previous = None
        for label,binary,assets in [('before',a.before,a.front_before),('after',a.after,a.front_after)]:
            db = 'nexofolio_step04_document_'+label
            h.create(db,seed)
            h.start(db,binary,label)
            _, login = request(h.url,'/v1/auth/login',None,dict(account='reader-101',password='fixture-pass'))
            h.token = login['token']
            expected = legacy(h,meta['project']) if label == 'before' else previous
            public_status, public = request(h.url,f"/v1/public/projects/{meta['project']}/document",None)
            assert public_status == 200
            if label == 'after':
                status, actual = h.request(f"/v1/projects/{meta['project']}/document")
                assert status == 200 and normalized(actual) == normalized(expected)
                assert public == previous_public
                save(a.out/'aggregate-index.json', actual)
                save(a.out/'public-after.json', public)
                for token, project, status in [(None,meta['project'],401),(h.token,meta['forbidden_project'],403)]:
                    actual_status, denied = request(h.url,f'/v1/projects/{project}/document',token)
                    assert actual_status == status, (actual_status, status)
                h.sql(db,'ALTER TABLE knowledge.folders RENAME TO denied_probe')
                assert request(h.url,f"/v1/projects/{meta['forbidden_project']}/document",h.token)[0] == 403
                h.sql(db,'ALTER TABLE knowledge.denied_probe RENAME TO folders')
                save(a.out/'equivalence.json',dict(entries=len(actual['entries']),equal=True,public_unchanged=True,authorization_before_storage=True))
            else:
                previous,previous_public = expected,public
                save(a.out/'legacy-index.json',expected)
                save(a.out/'public-before.json',public)
            # Reader login is real and ordinary; only synthetic local membership is extended for the switch check.
            h.sql(db,f"INSERT INTO access.user_project_access(user_id,project_id) SELECT id,'{meta['forbidden_project']}' FROM access.users WHERE account='reader-101' ON CONFLICT DO NOTHING")
            save(a.out/(label+'-runtime.json'),dict(api_sha256=h.active_binary_sha,assets={str(p.relative_to(assets)):hashlib.sha256(p.read_bytes()).hexdigest() for p in assets.rglob('*') if p.is_file()}))
            if not a.skip_browser:
                browser(h,assets,a.out,meta,label,login)
            h.stop()
        assert not Upstream.calls
    finally:
        h.cleanup()
        save(a.out/'cleanup.json',dict(owned_pg_removed=not h.tmp.exists(),owned_processes_stopped=True,paid_model_calls=0))

if __name__ == '__main__':
    main()
