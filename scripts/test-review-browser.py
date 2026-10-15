#!/usr/bin/env python3
"""Real browser acceptance, compiled bundles, private API/PG/proxy only."""
import json, pathlib, os, subprocess, shutil, threading, mimetypes, sys, hashlib, tempfile
from types import SimpleNamespace
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.request import Request, urlopen
from urllib.error import HTTPError
from review_apply_acceptance import f, v3, OUT
from frozen_input_harness import ROOT, Harness, Upstream, PG
from mcp_read_fixture import install_identity, literal
FRONT=ROOT.parent/'NexoFolio-Front';NODE=pathlib.Path('/Users/sheldon/.nvm/versions/node/v22.22.0/bin/node')

def build(h,variant):
    root=FRONT
    if variant=='baseline':
        root=h.tmp/'baseline-front';root.mkdir()
        shutil.copytree(FRONT/'src',root/'src')
        shutil.copytree(OUT/'baseline-source/frontend/src',root/'src',dirs_exist_ok=True)
        for directory in ['assets','content']:
            if (FRONT/'src'/directory).exists():shutil.copytree(FRONT/'src'/directory,root/'src'/directory,dirs_exist_ok=True)
        for name in ['index.html','package.json']:shutil.copy2(FRONT/name,root/name)
        for name in ['node_modules','vendor','public']:
            if (FRONT/name).exists():(root/name).symlink_to(FRONT/name,target_is_directory=True)
    root=root.resolve()
    dist=(h.tmp/('dist-'+variant)).resolve();config=h.tmp/('vite-'+variant+'.mjs')
    config.write_text("import vue from '"+str(FRONT/'node_modules/@vitejs/plugin-vue/dist/index.mjs')+"';export default {root:"+json.dumps(str(root))+",envDir:"+json.dumps(str(h.tmp))+",plugins:[vue()],resolve:{alias:{'@':"+json.dumps(str(root/'src'))+"}},build:{outDir:"+json.dumps(str(dist))+",emptyOutDir:true},define:{'import.meta.env.VITE_WORKSPACE_ENABLED':'true'}}")
    with (OUT/('browser-'+variant+'-build.log')).open('w') as log:
        subprocess.run([str(NODE),str(FRONT/'node_modules/vite/bin/vite.js'),'build','--config',str(config)],cwd=root,env={'PATH':str(NODE.parent)+':'+os.environ['PATH']},stdout=log,stderr=log,check=True)
    (OUT/('browser-'+variant+'-bundle.json')).write_text(json.dumps({str(p.relative_to(dist)):hashlib.sha256(p.read_bytes()).hexdigest() for p in dist.rglob('*') if p.is_file()},indent=2))
    return dist

def main():
    if '--build-only' in sys.argv:
        tmp=pathlib.Path(tempfile.mkdtemp(prefix='review-build.'))
        try:build(SimpleNamespace(tmp=tmp),'optimized')
        finally:shutil.rmtree(tmp)
        return
    pathlib.Path('/tmp/nexofolio-step04').mkdir(exist_ok=True);install_identity();h=Harness(OUT,ROOT/'target/debug/nexofolio-admin');server=None
    db='nexofolio_step04_review_browser';f.DB=db
    try:
        scenario_only='--scenarios-only' in sys.argv or '--commit-only' in sys.argv
        bundles={variant:build(h,variant) for variant in (['optimized'] if scenario_only else ['baseline','optimized'])}
        h.create(db);h.start(db,OUT/'baseline-nexofolio-api','browser-baseline');meta=f.seed(h,101)
        dist=bundles.get('baseline',bundles['optimized'])
        class Front(BaseHTTPRequestHandler):
            def log_message(self,*args):pass
            def do_GET(self):self.serve()
            def do_POST(self):self.serve()
            def serve(self):
                if self.path.startswith('/api/'):
                    body=self.rfile.read(int(self.headers.get('Content-Length','0'))) if self.command=='POST' else None
                    try:r=urlopen(Request(h.url+self.path[4:],data=body,headers={k:v for k,v in self.headers.items() if k.lower() in ['authorization','content-type']},method=self.command),timeout=195)
                    except HTTPError as e:r=e
                    self.send_response(r.status);self.send_header('Content-Type',r.headers.get('Content-Type','application/json'));self.end_headers()
                    try:self.wfile.write(r.read())
                    except (BrokenPipeError,ConnectionResetError):pass
                else:
                    file=dist/self.path.split('?')[0].lstrip('/')
                    if not file.is_file():file=dist/'index.html'
                    self.send_response(200);self.send_header('Content-Type',mimetypes.guess_type(str(file))[0] or 'application/octet-stream');self.end_headers();self.wfile.write(file.read_bytes())
        server=ThreadingHTTPServer(('127.0.0.1',0),Front);threading.Thread(target=server.serve_forever,daemon=True).start()
        def browser(stage):
            (h.tmp/'browser.json').write_text(json.dumps(dict(front=f'http://127.0.0.1:{server.server_port}',project=meta['project'],candidate=meta['candidate'],token=h.token,out=str(OUT),db=db,pg_port=h.pg_port,psql=str(PG/'psql'))))
            subprocess.run([str(NODE),str(ROOT/'scripts/test-review-browser.cjs'),str(h.tmp),stage],check=True)
        if not scenario_only:browser('baseline')
        h.stop();h.start(db,ROOT/'target/debug/nexofolio-api','browser-optimized');dist=bundles['optimized']
        if not scenario_only:browser('optimized')
        body=json.loads(h.sql(db,"SELECT execution_input FROM knowledge.candidates LIMIT 1"))
        for inputs in body['prepared']['inputs'].values():
            inputs[0]['observations']=dict(archive_id=meta['candidate'],total=1,hash='synthetic',states={'full':1})
        h.sql(db,f"UPDATE knowledge.candidates SET execution_input={literal(body)}")
        header=v3(h,db,meta)
        browser('commit' if '--commit-only' in sys.argv else 'scenarios')
        assert not Upstream.calls
        print('real browser baseline/optimized/scenarios passed; no model calls')
    finally:
        if server:server.shutdown()
        h.cleanup()
if __name__=='__main__':main()
