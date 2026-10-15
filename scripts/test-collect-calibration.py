#!/usr/bin/env python3
"""A single finite burst at each candidate concurrency; same owned harness."""
import argparse, importlib.util, json, pathlib, uuid
spec=importlib.util.spec_from_file_location('admission',pathlib.Path(__file__).with_name('test-collect-admission.py'))
driver=importlib.util.module_from_spec(spec);spec.loader.exec_module(driver)
p=argparse.ArgumentParser()
for name in ['before','after','admin','out']:p.add_argument('--'+name,type=pathlib.Path,required=True)
a=p.parse_args();a.out.mkdir(parents=True,exist_ok=True)
pathlib.Path('/tmp/nexofolio-step04').mkdir(exist_ok=True)
h=driver.Bench(a.out,a.admin.resolve());results={}
try:
    seed='nexofolio_step04_calibration_seed';h.create(seed);h.start(seed,a.before.resolve(),'seed')
    project=h.request('/v1/projects')[1]['items'][0]['project_id'];h.stop()
    bodies=[driver.batch(project,6*1024*1024,'burst'+str(i)) for i in range(12)]
    ids=[str(uuid.uuid4()) for _ in bodies]
    for label,binary,limit in [('before',a.before,4),('after2',a.after,2),('after4',a.after,4),('after8',a.after,8)]:
        db='nexofolio_step04_calibration_'+label;h.limit=limit;h.create(db,seed);h.start(db,binary.resolve(),label)
        # All cases use Expect:100-continue to read intentional early refusals.
        # Keep failures explicit at candidate values that exceed the local pool.
        data=driver.window(h,bodies,ids,a.out,'before-'+label)
        results[label]=dict(limit=limit,data=data,ledger=int(h.sql(db,'SELECT count(*) FROM intake.batches')),calls=int(h.sql(db,'SELECT coalesce(sum(calls),0) FROM observe.fingerprints')))
        accepted=sum(r['status']==200 for r in data['requests'])
        assert results[label]['ledger']==accepted and results[label]['calls']==accepted*3
        h.stop()
    assert not driver.Upstream.calls
finally:
    h.cleanup();results['cleanup']=dict(owned_pg_removed=not h.tmp.exists(),models=0)
    (a.out/'results.json').write_text(json.dumps(results,indent=2)+'\n')
print({label:{'statuses':[r['status'] for r in row['data']['requests']],'rss_kib':max(r['rss_kib'] for r in row['data']['resources'])} for label,row in results.items() if label!='cleanup'})
