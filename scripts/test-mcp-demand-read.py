#!/usr/bin/env python3
"""Fixed synthetic protocol equivalence and three comparable 20-user API/PG runs."""
import argparse, concurrent.futures, hashlib, json, math, os, pathlib, subprocess, threading, time
from urllib.request import Request, urlopen
from urllib.error import HTTPError
from frozen_input_harness import Harness, Upstream, encoded, ident, PG, run
from mcp_read_fixture import install_identity, enrich, archives, literal


class Bench(Harness):
    def create(self, db, template=None):
        if template:
            return super().create(db, template)
        run([PG/'createdb','-h','127.0.0.1','-p',self.pg_port,'-E','UTF8','-T','template0',db], stdout=subprocess.DEVNULL)
        with (self.out/'migration.log').open('a') as log:
            run([self.admin,'migrate'],env=self.env(db,0),stdout=log,stderr=log)

    def request(self, path, body=None):
        status, value = super().request(path, body)
        if path == '/v1/projects' and status == 200:
            value['items'].sort(key=lambda p: p['name'])
        return status, value


def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2)+'\n')


def request(url, path, token, body=None):
    headers = {'Content-Type':'application/json','Accept':'application/json, text/event-stream','MCP-Protocol-Version':'2025-11-25'}
    if token: headers['Authorization'] = 'Bearer '+token
    try:
        with urlopen(Request(url+path, data=encoded(body).encode() if body is not None else None, headers=headers), timeout=120) as r:
            status, raw = r.status, r.read()
    except HTTPError as e:
        status, raw = e.code, e.read()
        if status == 429:
            value = json.loads(raw); value['_retry_after'] = float(e.headers.get('Retry-After','1'))
            return status, value
    return status, json.loads(raw)


def credential(h, account):
    status, login = request(h.url,'/v1/auth/login',None,dict(account=account,password='fixture-pass'))
    assert status == 200, (status,login)
    token = login['token']
    status, result = request(h.url,'/v1/auth/mcp-tokens',token,dict(name='synthetic reader'))
    assert status == 201, (status,result)
    return token, result['token']


def mcp(h, token, name, args, error=False):
    status, reply = request(h.url,'/mcp',token,dict(jsonrpc='2.0',id=1,method='tools/call',params=dict(name=name,arguments=args)))
    assert status == 200 and 'error' not in reply, (status,reply)
    result = reply['result']
    assert bool(result.get('isError')) == error, (name,args,result)
    return result.get('structuredContent') or json.loads(result['content'][0]['text'])


def normalize(value):
    # Revision implementation is opaque; compare every returned business item and continuation argument.
    v = json.loads(encoded(value))
    if isinstance(v,dict):
        v.pop('revision',None)
        if v.get('next'): v['next']['arguments'].pop('revision',None)
    return v


def correctness(h, db, binary, label, meta, arc):
    h.start(db,binary,label)
    web, token = credential(h,'reader-101')
    q = dict(project_id=meta['project'],environment_id=meta['environment'],search_in='fields',query='remark',limit=20)
    outputs = {}
    status, listing = request(h.url,'/mcp',token,dict(jsonrpc='2.0',id=1,method='tools/list'))
    assert status == 200 and len(listing['result']['tools']) == 11
    outputs['tools'] = listing['result']
    for mode in ['fields','interface']:
        for text in ['','remark','MiXeD','100%_','100X_','%','_','\\','ΟΣ','İ','K','中文\u2003词语','no-such-value']:
            query = dict(q,search_in=mode,query=text)
            outputs[mode+':'+text] = normalize(mcp(h,token,'search_interfaces',query))
    for tag, extra in [('environment',dict(environment_id=meta['other_environment'])),('method',dict(method='get')),
                       ('scope',dict(business_scope='/capacity/')),('directory',dict(directory_id=meta['directory'],recursive=True)),
                       ('child',dict(directory_id=meta['child']))]:
        outputs[tag] = normalize(mcp(h,token,'search_interfaces',dict(q,**extra)))
    first = mcp(h,token,'search_interfaces',q)
    assert first['total'] == 501 and len(first['items']) == 20
    items, page = [], first
    while True:
        items += page['items']
        if not page['next']: break
        page = mcp(h,token,page['next']['tool'],page['next']['arguments'])
    assert len(items) == 501 and len({i['endpoint_id'] for i in items}) == 501
    outputs['all_pages'] = items
    # Live traffic counts do not invalidate either search mode.
    h.sql(db,'UPDATE observe.fingerprints SET calls=calls+1,last_seen=last_seen+interval \'1 second\'')
    continuation = mcp(h,token,first['next']['tool'],first['next']['arguments'])
    assert continuation['revision'] == first['revision']
    for section in ['field_summary','field','relations','fields','overview','samples']:
        query = dict(project_id=meta['project'],environment_id=meta['environment'],endpoint_id=meta['target'],section=section)
        if section in ['field_summary','field','relations']: query['field_key']='request_body:remark'
        outputs[section] = normalize(mcp(h,token,'read_interface',query))
    for version in ['2','3','4']:
        for env in [meta['environment'],meta['other_environment']]:
            query = dict(project_id=meta['project'],endpoint_id=meta['target'],archive_id=arc[version],field_key='request_body:remark',environment_id=env)
            outputs['values'+version+env] = mcp(h,token,'read_field_values',query)
            full = mcp(h,token,'read_field_values',dict(query,value_id='one'))
            assert json.loads(full['data']) == [('精确正文' if version=='4' else '精确\0正文'),900719925474099312345,1.23456789012345]
            outputs['value'+version+env] = full
    denied = mcp(h,token,'search_interfaces',dict(q,project_id=meta['forbidden_project']),True)
    assert 'PROJECT_ACCESS_DENIED' in encoded(denied)
    h.sql(db, f"UPDATE knowledge.field_notes SET text=text||'changed' WHERE endpoint_id='{items[-1]['endpoint_id']}' AND path='request_body:remark'")
    changed = mcp(h,token,first['next']['tool'],first['next']['arguments'],True)
    assert 'SOURCE_CHANGED_RESTART_READ' in encoded(changed)
    # Existing lifecycle API rules drive retired endpoint, field subtree and directory scope.
    actor = dict(kind='system',id='synthetic',display_name='synthetic')
    decisions = []
    for i,target in enumerate([dict(kind='endpoint',id=meta['first']),dict(kind='folder',id=meta['child']),
                              dict(kind='field',endpoint=meta['target'],location={'in':'request_body'},path=[{'key':'remark'}])]):
        decisions.append(dict(target=target,reason='synthetic',actor=actor,at='2026-10-14T00:00:00Z',version=i+1,retired=True,folders=[]))
    lifecycle = dict(version=3,reconciled_version=3,decisions=decisions)
    h.sql(db,f"INSERT INTO knowledge.document_lifecycle(project_id,state) VALUES('{meta['project']}',{literal(lifecycle)}::jsonb)")
    for include in [False,True]:
        outputs['retired'+str(include)] = normalize(mcp(h,token,'search_interfaces',dict(q,include_retired=include)))
        outputs['retired-unclassified'+str(include)] = normalize(mcp(h,token,'browse_directory',dict(project_id=meta['project'],environment_id=meta['environment'],section='interfaces',include_retired=include,unclassified=True)))
    if label == 'after':
        exact = dict(project_id=meta['project'],endpoint_id=meta['target'],archive_id=arc['3'],field_key='request_body:remark',value_id='one')
        h.sql(db,f"UPDATE knowledge.frozen_input_chunks SET content='invalid-unused' WHERE candidate_id='{arc['3']}' AND sha256='{arc['unused_hash']}'")
        assert json.loads(mcp(h,token,'read_field_values',exact)['data'])[0] == '精确\0正文'
        h.sql(db,f"UPDATE knowledge.frozen_input_chunks SET content='invalid-required' WHERE candidate_id='{arc['3']}' AND sha256='{arc['required_hash']}'")
        assert 'VALUE_UNAVAILABLE' in encoded(mcp(h,token,'read_field_values',exact,True))
        assert 'VALUE_UNAVAILABLE' in encoded(mcp(h,token,'read_field_values',dict(exact,endpoint_id=meta['first']),True))
    # Revoked membership rejects at access before knowledge storage (even if it is unavailable).
    h.sql(db,"DELETE FROM access.user_project_access WHERE user_id=(SELECT id FROM access.users WHERE account='reader-101'); ALTER TABLE knowledge.field_notes RENAME TO denied_probe")
    assert 'PROJECT_ACCESS_DENIED' in encoded(mcp(h,token,'search_interfaces',q,True))
    h.sql(db,'ALTER TABLE knowledge.denied_probe RENAME TO field_notes')
    save(h.out/(label+'-correctness.json'),outputs)
    h.stop()
    return outputs


def collection(meta, user, sequence):
    body = dict(state='full',media_type='application/json',encoding='utf8',content=encoded(dict(orderId=101,remark='合成测试备注')))
    records = [dict(id=ident(f'load-{user}-{sequence}-{n}'),kind='http_exchange',version=1,observed_at='2026-10-14T00:00:00Z',
                    payload=dict(request=dict(method='POST',url='https://synthetic.example.invalid/synthetic/orders',body=body))) for n in range(2)]
    return dict(batch_id=ident(f'load-batch-{user}-{sequence}'),platform='nexofolio-mcp-synthetic',
                target=dict(project_id=meta['project'],environment=dict(name='合成环境')),records=records)


def measure(h, db, binary, label, meta, seconds):
    pid = h.start(db,binary,label)
    credentials = [credential(h,'reader-'+str(200+i)) for i in range(20)]
    q = dict(project_id=meta['project'],environment_id=meta['environment'],query='remark',search_in='fields',limit=20)
    # Identical warm-up per binary, outside the sample window.
    for _,token in credentials:
        found=mcp(h,token,'search_interfaces',q); action=found['items'][0]['matched_field']['open'];mcp(h,token,action['tool'],action['arguments'])
    samples, resources, errors, traffic = [], [], [], []
    initial_calls = int(h.sql(db,'SELECT sum(calls) FROM observe.fingerprints'))
    stop = threading.Event()
    started = time.monotonic()
    def monitor():
        while not stop.is_set():
            resources.append(dict(seconds=time.monotonic()-started,rss_bytes=int(subprocess.check_output(['ps','-o','rss=','-p',str(pid)]))*1024))
            stop.wait(.1)
    sampler = threading.Thread(target=monitor);sampler.start()
    def worker(number):
        web,token=credentials[number]
        local=[]
        sequence, accepted, limited, pending, next_attempt = 0, 0, 0, None, 0
        while time.monotonic()-started<seconds:
            # Original mixed flow: four web reads, bounded collection retry, two MCP reads.
            for path in ['/v1/auth/me',f"/v1/projects/{meta['project']}/endpoints",f"/v1/projects/{meta['project']}/catalog",f"/v1/projects/{meta['project']}/curate"]:
                status,_=request(h.url,path,web)
                if status!=200: errors.append(dict(user=number,path=path,status=status))
            if time.monotonic() >= next_attempt:
                if pending is None: pending=collection(meta,number,sequence)
                status,receipt=request(h.url,'/v1/collect/batches',None,pending)
                if status == 429:
                    limited += 1
                    next_attempt = time.monotonic()+max(1,receipt['_retry_after'])+(number%5)*.1
                elif status==200 and receipt['accepted']==2 and not receipt['rejected']:
                    accepted += 1; sequence += 1; pending=None
                else: errors.append(dict(user=number,path='/v1/collect/batches',status=status,receipt=receipt))
            for kind in ['search','field']:
                begin=time.monotonic()
                try:
                    if kind=='search':
                        found=mcp(h,token,'search_interfaces',q)
                        assert found['total']==501 and len(found['items'])==20
                        action=found['items'][0]['matched_field']['open']
                    else: mcp(h,token,action['tool'],action['arguments'])
                    local.append(dict(user=number,kind=kind,ms=(time.monotonic()-begin)*1000,seconds=begin-started))
                except Exception as e:
                    detail = e.args[0][-1] if e.args and isinstance(e.args[0],tuple) and isinstance(e.args[0][-1],dict) else {'error':str(e)[:300]}
                    errors.append(dict(user=number,kind=kind,error=detail.get('structuredContent',detail)))
            stop.wait(1)
        traffic.append(dict(user=number,accepted_batches=accepted,limited=limited,pending=pending is not None))
        return local
    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=20) as pool:
            for result in pool.map(worker,range(20)): samples.extend(result)
    finally:
        stop.set();sampler.join()
    elapsed=time.monotonic()-started
    accepted = sum(t['accepted_batches'] for t in traffic)
    final_calls = int(h.sql(db,'SELECT sum(calls) FROM observe.fingerprints'))
    assert final_calls-initial_calls == accepted*2, (initial_calls,final_calls,accepted)
    def stats(rows):
        values=sorted(x['ms'] for x in rows)
        return dict(n=len(values),p95_ms=values[math.ceil(.95*len(values))-1],mean_ms=sum(values)/len(values))
    result=dict(label=label,seconds=elapsed,users=20,binary_sha256=h.active_binary_sha,load=os.getloadavg(),
                combined=stats(samples),search=stats([x for x in samples if x['kind']=='search']),field=stats([x for x in samples if x['kind']=='field']),
                rss_peak_bytes=max(x['rss_bytes'] for x in resources),errors=errors,rate_limited=sum(t['limited'] for t in traffic),collection=traffic,accepted_records=accepted*2,verified_call_delta=final_calls-initial_calls)
    save(h.out/(label+'-samples.json'),samples);save(h.out/(label+'-resources.json'),resources);save(h.out/(label+'-result.json'),result)
    h.stop()
    if label.startswith("after"):
        assert not errors,errors[:2]
    return result


def main():
    p=argparse.ArgumentParser()
    for name in ['before','after','admin','out']: p.add_argument('--'+name,required=True,type=pathlib.Path)
    p.add_argument('--skip-correctness',action='store_true');p.add_argument('--rounds',type=int,default=3);p.add_argument('--seconds',type=int,default=180)
    a=p.parse_args();a.out.mkdir(parents=True,exist_ok=True);pathlib.Path('/tmp/nexofolio-step04').mkdir(exist_ok=True)
    install_identity();h=Bench(a.out,a.admin.resolve());results=[]
    try:
        seed='nexofolio_step04_mcp_seed';h.create(seed);h.start(seed,a.before.resolve(),'seed')
        meta=h.seed(seed,501);meta=enrich(h,seed,meta);arc=archives(h,seed,meta)
        save(a.out/'fixture.json',dict(meta,archives=arc));h.stop()
        if not a.skip_correctness:
            previous=None
            for mode,binary in [('before',a.before),('after',a.after)]:
                db='nexofolio_step04_check_'+mode;h.create(db,seed)
                actual=correctness(h,db,binary.resolve(),mode,meta,arc)
                if previous is not None:
                    assert actual==previous, [k for k in actual if actual[k]!=previous[k]]
                previous=actual
                print(mode+' correctness passed',flush=True)
            save(a.out/'equivalence.json',dict(cases=len(previous),all_equal=True,tools=11))
        for i in range(a.rounds):
            for mode,binary in [('before',a.before),('after',a.after)]:
                label=mode+'-'+str(i+1);db='nexofolio_step04_'+label.replace('-','_');h.create(db,seed)
                result=measure(h,db,binary.resolve(),label,meta,a.seconds);results.append(result);save(a.out/'results.json',results)
                print(encoded(result),flush=True)
        assert not Upstream.calls
    finally:
        h.cleanup();save(a.out/'cleanup.json',dict(owned_pg_removed=not h.tmp.exists(),owned_processes_stopped=True,paid_model_calls=0))

if __name__=='__main__': main()
