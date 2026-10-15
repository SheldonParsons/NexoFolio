#!/usr/bin/env python3
"""Final binary smoke: real timeout release and quota isolation on owned PG."""
import importlib.util,json,pathlib,socket,sys,time,uuid
from urllib.parse import urlparse
from frozen_input_harness import Harness,Upstream
spec=importlib.util.spec_from_file_location('driver',pathlib.Path(__file__).with_name('test-collect-admission.py'))
d=importlib.util.module_from_spec(spec);spec.loader.exec_module(d)
root=pathlib.Path(__file__).resolve().parents[1];out=root/'docs/audits/2026-10-14-collect-admission/final-smoke';out.mkdir(parents=True,exist_ok=True)
class Smoke(Harness):
    def env(self,db,port):return dict(super().env(db,port),NEXOFOLIO_COLLECT_BURST='1',NEXOFOLIO_REQUEST_TIMEOUT_MS='1000',NEXOFOLIO_DB_TIMEOUT_MS='200')
h=Smoke(out,root/'target/debug/nexofolio-admin');result={}
try:
    db='nexofolio_step04_final_collect';h.create(db);h.start(db,root/'target/debug/nexofolio-api','final')
    project=h.request('/v1/projects')[1]['items'][0]['project_id']
    result['prebody']=d.prebody(h)
    body=d.batch(project,4096,'quota');a=str(uuid.uuid4());b=str(uuid.uuid4())
    responses=[d.call(h,body,client) for client in [a,a,b,None,None]]
    assert [r['status'] for r in responses]==[200,429,200,200,429],responses
    assert responses[0]['body']==responses[2]['body']==responses[3]['body']
    result['quota']=responses
    url=urlparse(h.url);s=socket.create_connection((url.hostname,url.port),timeout=3)
    s.sendall(b'POST /v1/collect/batches HTTP/1.1\r\nHost: localhost\r\nContent-Length: 8000000\r\n\r\n')
    started=time.monotonic();response=s.recv(4096).decode();s.close()
    assert '408 Request Timeout' in response,response
    result['timeout']=dict(response=response,ms=(time.monotonic()-started)*1000,released=d.call(h,b'{}')['status'])
    assert result['timeout']['released']==400
    result['ledger']=int(h.sql(db,'SELECT count(*) FROM intake.batches'));result['calls']=int(h.sql(db,'SELECT sum(calls) FROM observe.fingerprints'))
    assert result['ledger']==result['calls']==1
    result['binary']=h.active_binary_sha
    assert not Upstream.calls
finally:
    h.cleanup();result['cleanup']=not h.tmp.exists();(out/'results.json').write_text(json.dumps(result,indent=2)+'\n')
print(result)
