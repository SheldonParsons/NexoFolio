#!/usr/bin/env python3
"""Owned PG/API, synthetic bodies; no shared database or model traffic."""
import argparse, concurrent.futures, hashlib, http.client, json, pathlib, socket, statistics, subprocess, threading, time, uuid
from urllib.parse import urlparse
from frozen_input_harness import Harness, Upstream, encoded


class Bench(Harness):
    def env(self, db, port):
        return dict(super().env(db,port), NEXOFOLIO_COLLECT_RATE_PER_MINUTE='60000', NEXOFOLIO_COLLECT_BURST='10000',NEXOFOLIO_COLLECT_IN_FLIGHT=str(getattr(self,'limit',4)))


def call(h, body=None, client=None, path='/v1/collect/batches'):
    target = urlparse(h.url)
    headers = {'Host': 'localhost', 'Content-Type': 'application/json', 'Authorization': 'Bearer '+h.token, 'Connection': 'close'}
    if client is not None: headers['X-NexoFolio-Client-Id'] = client
    if body is not None: headers.update({'Content-Length':str(len(body)), 'Expect':'100-continue'})
    start = time.monotonic()
    with socket.create_connection((target.hostname,target.port),timeout=120) as conn:
        conn.sendall((('POST' if body is not None else 'GET')+' '+path+' HTTP/1.1\r\n'+''.join(k+': '+v+'\r\n' for k,v in headers.items())+'\r\n').encode())
        stream=conn.makefile('rb')
        def response():
            line=stream.readline().decode();assert line,line
            status=int(line.split()[1]);headers={}
            while True:
                line=stream.readline().decode()
                if line=='\r\n':break
                assert line,'truncated headers'
                key,value=line.split(':',1);headers[key.lower()]=value.strip()
            return status,headers
        status,headers=response()
        if status==100:
            conn.sendall(body);status,headers=response()
        raw=stream.read(int(headers['content-length']))
        return dict(status=status,ms=round((time.monotonic()-start)*1000,2),retry=headers.get('retry-after'),body=json.loads(raw))



def batch(project, size, name):
    records = []
    for n in range(3 if size > 1024*1024 else 1):
        content = encoded({'blob': 'x'*(size//(3 if size > 1024*1024 else 1)), 'nul': '\0'})
        records.append(dict(id=str(uuid.uuid4()), kind='http_exchange', version=1, observed_at='2026-10-14T00:00:00Z', payload=dict(request=dict(method='POST', url='https://synthetic.invalid/'+name+str(n), body=dict(state='full', encoding='utf8', media_type='application/json', content=content)))))
    return encoded(dict(batch_id=str(uuid.uuid4()), platform='step08-synthetic', target=dict(project_id=project, environment=dict(name='隔离合成环境')), records=records)).encode()


def window(h, bodies, ids, out, label):
    stop = threading.Event()
    samples, reads = [], []
    def sample():
        while not stop.is_set():
            row = subprocess.check_output(['ps','-o','rss=,%cpu=','-p',str(h.api.pid)], text=True).split()
            samples.append(dict(at=time.monotonic(), rss_kib=int(row[0]), cpu=float(row[1])))
            stop.wait(.05)
    def read():
        while not stop.is_set():
            reads.append(call(h, path='/v1/projects'))
            stop.wait(.05)
    threads = [threading.Thread(target=f) for f in (sample, read)]
    for t in threads: t.start()
    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(bodies)) as pool:
            results = list(pool.map(lambda pair:call(h,*pair), zip(bodies,ids)))
    finally:
        stop.set()
        for t in threads: t.join()
    data = dict(requests=results, resources=samples, reads=reads, sizes=list(map(len,bodies)), binary=h.active_binary_sha)
    (out/(label+'.json')).write_text(json.dumps(data,indent=2)+'\n')
    assert all(r['status'] in ([200,429,503] if label.startswith('before') else [200,429]) for r in results), results
    assert all(r['status']==200 for r in reads), reads
    return data


def prebody(h):
    target=urlparse(h.url)
    held=[]
    client=str(uuid.uuid4())
    def hold(identity):
        s=socket.create_connection((target.hostname,target.port),timeout=2)
        s.sendall(f'POST /v1/collect/batches HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 8000000\r\nX-NexoFolio-Client-Id: {identity}\r\n\r\n'.encode())
        held.append(s)
        time.sleep(.08)
        return s
    try:
        hold(client);hold(client)
        rejected=hold(client).recv(4096).decode()
        assert '429 Too Many Requests' in rejected and 'retry-after: 1' in rejected.lower(),rejected
        held.pop().close()
        hold(str(uuid.uuid4()));hold(str(uuid.uuid4()))
        global_reply=hold(str(uuid.uuid4())).recv(4096).decode()
        assert '429 Too Many Requests' in global_reply,global_reply
        return dict(client_headers_only=rejected, global_headers_only=global_reply, body_bytes_sent=0)
    finally:
        for s in held:s.close()
        time.sleep(.25)
        assert call(h,b'{}',str(uuid.uuid4()))['status']==400


def main():
    p=argparse.ArgumentParser()
    for name in ['before','after','admin','out']:p.add_argument('--'+name,type=pathlib.Path,required=True)
    p.add_argument('--reuse-before',type=pathlib.Path)
    p.add_argument('--only-before',action='store_true')
    p.add_argument('--limit',type=int,default=4)
    a=p.parse_args();a.out.mkdir(parents=True,exist_ok=True)
    pathlib.Path('/tmp/nexofolio-step04').mkdir(exist_ok=True)
    h=Bench(a.out,a.admin.resolve());h.limit=a.limit; report={'parameters':dict(rate_per_minute=60000,burst=10000,in_flight=a.limit,reason='isolate resource admission from token refill in the finite measurement window')}
    try:
        seed='nexofolio_step04_admission_seed';h.create(seed);h.start(seed,a.before.resolve(),'seed')
        project=h.request('/v1/projects')[1]['items'][0]['project_id'];h.stop()
        fixture={name:[batch(project,size,name+str(i)) for i in range(3)] for name,size in [('small',4096),('medium',256*1024),('large',6*1024*1024)]}
        burst=[batch(project,6*1024*1024,'burst'+str(i)) for i in range(12)]
        if a.reuse_before: report['before']=json.loads(a.reuse_before.read_text())['before'];report['reused_before']=str(a.reuse_before)
        for label,binary in ([('before',a.before)] if a.only_before else [('after',a.after)] if a.reuse_before else [('before',a.before),('after',a.after)]):
            db='nexofolio_step04_admission_'+label;h.create(db,seed);h.start(db,binary.resolve(),label)
            report[label]={}
            for name,bodies in fixture.items():
                runs=[window(h,[body],[str(uuid.uuid4())],a.out,label+'-'+name+str(i)) for i,body in enumerate(bodies)]
                report[label][name]=dict(bytes=len(bodies[0]),ms=[r['requests'][0]['ms'] for r in runs],peak_rss_kib=max(s['rss_kib'] for r in runs for s in r['resources']))
            result=window(h,burst,[str(uuid.uuid4()) for _ in burst],a.out,label+'-burst')
            report[label]['burst']=dict(statuses=[r['status'] for r in result['requests']],ms=[r['ms'] for r in result['requests']],peak_rss_kib=max(s['rss_kib'] for s in result['resources']),read_ms=[r['ms'] for r in result['reads']])
            # Drain bounded refusals and repeat all original wires: database calls must count once.
            for body in burst:
                for _ in range(6):
                    response=call(h,body,str(uuid.uuid4()))
                    if response['status']==200:break
                    assert response['status']==429
                    time.sleep(int(response['retry']))
                assert response['status']==200 and response['body']['accepted']==3,response
            assert h.sql(db,'SELECT count(*) FROM intake.batches')=='21'
            assert h.sql(db,'SELECT sum(calls) FROM observe.fingerprints')=='51'
            # Concurrent duplicates exercise the existing same-project transaction lock.
            dup=batch(project,4096,'duplicate')
            with concurrent.futures.ThreadPoolExecutor(2) as pool:
                replies=list(pool.map(lambda _:call(h,dup,str(uuid.uuid4())),range(2)))
            assert replies[0]['body']==replies[1]['body'] and all(r['status']==200 for r in replies),replies
            assert h.sql(db,'SELECT sum(calls) FROM observe.fingerprints')=='52'
            if label=='after':
                report['prebody']=prebody(h)
                report['compatibility']=[call(h,batch(project,4096,'compat'+str(i)),identity)['status'] for i,identity in enumerate([None,'invalid',str(uuid.uuid4())])]
                assert report['compatibility']==[200]*3
                assert call(h,b'x'*(8*1024*1024+1),str(uuid.uuid4()))['status']==413
            report[label]['db']=dict(ledger=int(h.sql(db,'SELECT count(*) FROM intake.batches')),calls=int(h.sql(db,'SELECT sum(calls) FROM observe.fingerprints')),duplicate_receipts=replies)
            h.stop()
        assert not Upstream.calls
    finally:
        h.cleanup();report['cleanup']=dict(owned_pg_removed=not h.tmp.exists(),model_calls=0)
        (a.out/'results.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))


if __name__=='__main__':main()
