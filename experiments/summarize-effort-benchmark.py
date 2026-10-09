#!/usr/bin/env python3
"""Aggregate only safe timing/usage/validation metadata; never export prompts or replies."""
import json,statistics,math
from pathlib import Path
root=Path(__file__).resolve().parent.parent
base=root/'experiments/results'
def load_records(name):
 return {json.loads(f.read_text())['summary']['case']:json.loads(f.read_text()) for f in (base/name).glob('*-flash-*.json')}
def elapsed(record):
 return (record['summary'].get('measurement') or {}).get('elapsed_ms',record.get('elapsed_ms'))
def numbers(records):
 times=[elapsed(r)/1000 for r in records]
 return {'calls':len(times),'mean_seconds':round(statistics.mean(times),3),'median_seconds':round(statistics.median(times),3),'max_seconds':round(max(times),3),'request_seconds_sum':round(sum(times),3),'reasoning_tokens':sum((r['summary'].get('measurement') or {}).get('reasoning_tokens') or 0 for r in records)}
initial=load_records('model-flash-high-20261009');retry=load_records('model-flash-high-retry-20261009');maximum=load_records('model-flash-max-20261009');high=initial|retry
assert len(initial)==15 and len(retry)==2 and len(maximum)==5
assert all(r['summary']['valid'] and r['summary']['same_prompt'] for r in high.values())
assert all(r['summary']['valid'] and r['summary']['same_prompt'] for r in maximum.values())
fields=[r for k,r in high.items() if k.startswith('field-')]
assert sum(r['summary']['expected_fields'] for r in fields)==63
assert all(r['summary']['missing']==0 and r['summary']['evidence_values_preserved'] for r in fields)
assert all(r['summary']['missing']==0 and r['summary']['evidence_values_preserved'] for k,r in maximum.items() if k.startswith('field-'))
assert all(high[f'stage-{i}']['summary']['commands']==1 for i in [0,1])
assert maximum['stage-1']['summary']['commands']==1
for record in list(high.values())+list(maximum.values()):
 r=record['summary'];assert record['prompt_sha256']==r['expected_prompt_sha256']
old=json.loads((root/'docs/audits/2026-10-09-model-comparison.json').read_text())
pro=[];low=[]
for case_id in maximum:
 kind,index=case_id.split('-');index=int(index)
 for variant,records in [('pro_low',pro),('flash_low',low)]:
  path=base/('model-comparison-20261009' if kind=='field' else 'model-stages-20261009')/(f'{index:02}-{variant}.json' if kind=='field' else f'{index}-{variant}.json')
  records.append(json.loads(path.read_text()))
initial_failures=[{'case':k,'elapsed_seconds':elapsed(r)/1000,'classification':'transport timeout at the configured 10-second connection limit; no generation token metrics available','retry_succeeded':retry[k]['summary']['valid']} for k,r in initial.items() if not r['summary']['valid']]
checks=[
 {'check':'Repeated scalar values across records must not imply multiple configurations in a single record','case':'field-06','previous_failure':'flash_low','high':'not reproduced','max':'not tested'},
 {'check':'No unsupported cross-system purpose for a null parentUniqueId','case':'field-07','previous_failure':'flash_disabled','high':'not reproduced','max':'not tested'},
 {'check':'Do not guess the enum meaning of evaluationStatus=0','case':'field-11','previous_failure':'flash_low','high':'not reproduced after connection retry','max':'not reproduced'},
 {'check':'Do not promise working pagination from page/size names alone when the response is one object','case':'stage-0','previous_failure':'flash_disabled','high':'purpose no longer promises pagination; description labels page/size inference','max':'not tested'},
 {'check':'An observed Authorization header is not a proven requirement','case':'stage-1','previous_failure':'pro_low also contains this unsupported requirement','high':'unsupported requirement remains','max':'uses observed header; required auth remains unknown'},
 {'check':'Numeric productionCodeId is not proof of a database primary key','case':'field-05','high':'unsupported primary-key assertion','max':'identity/uniqueness and source left unknown'},
 {'check':'Same observed processInstanceId in another API does not prove where this input should be obtained','case':'field-10','high':'says the value can be obtained from the query output','max':'candidate association; removes definite source assertion'},
 {'check':'A list of process nodes does not prove current workflow state or temporal ordering','case':'field-08','high':'suggests list can identify current stage; unverified','max':'does not make that claim; order, units and mapping stability unknown'},
 {'check':'A technical item ID need not be a confirmed database primary key','case':'field-11','previous_failure':'pro_low also asserts primary-key identity in details','high':'primary-key assertion remains','max':'business primary-key status left unknown'},
]
verification=[]
for name in ['model-flash-high-20261009','model-flash-high-retry-20261009','model-flash-max-20261009']:
 v=json.loads((base/name/'state-verification.json').read_text());assert v['candidate_unchanged'] and v['runtime_settings_unchanged']
 verification.append({'run':name,**v})
report={'date':'2026-10-09','scope':'Follow-up comparison only; no production configuration edits and no full project rerun','calls':{'high_initial':15,'high_connection_retries':2,'max_selected_hard_cases':5,'total_attempts':22,'successful':20,'transport_failures':2},'design':{'same_original_prompts':True,'high_unique_cases':15,'high_fields':63,'max_unique_cases':5,'max_fields':24,'outer_concurrency':3,'automatic_retries':0,'manual_retries':'Exactly once for each of two high transport failures','prompt_or_parser_changes':False,'compared_at_different_times':True},'high_field_results':numbers(fields),'high_effective_request_seconds_per_field_batch_including_two_failures':round((sum(elapsed(r) for r in fields)+sum(elapsed(r) for r in initial.values() if not r['summary']['valid']))/12/1000,3),'previous_same_12_field_cases':{k:{x:old['results'][k][x] for x in ['mean_seconds','median_seconds','max_seconds']} for k in ['pro_low','flash_low','flash_disabled']},'high_other_stages_seconds':{k:round(elapsed(r)/1000,3) for k,r in high.items() if k.startswith('stage-')},'paired_hard_case_comparison':{'case_ids':sorted(maximum),'pro_low':numbers(pro),'flash_low':numbers(low),'flash_high':numbers([high[k] for k in maximum]),'flash_max':numbers(list(maximum.values()))},'failures':initial_failures,'manual_quality_review':{'scope':'Manual comparison against the input evidence and known counterexamples. Not a gold-labelled accuracy score.','checks':checks,'conclusion':'High corrected several previous low/disabled errors and kept a large observed speed advantage over Pro low. Max improved the targeted counterexamples at higher latency. Pro low shares some unsupported claims; no evidence that it is universally more reliable.'},'decision':{'production_switch_made':False,'current_local_model':'deepseek-v4-pro','current_local_effort':'low','recommendation':'Flash high is a credible candidate for ordinary candidate-generation work; consider max for difficult evidence interpretation only after broader validation and explicit evidence checks. This experiment did not implement automatic semantic failure detection or model routing.'},'limitations':['Each configuration has one successful sample per case; initial transport failures were retried once, not repeated quality trials.','High used all original 15 cases; max only selected 5 hard cases. Max mean is compared only against those same cases.','No 8-concurrent real-provider throughput test and no full-round elapsed-time guarantee.','The gateway accepted the high/max request parameters and returned usage; dated model ID does not independently prove provider-internal effort or exact weights.','Qualitative corrections in these examples do not prove all business semantics correct.','Test output/candidate generation remains separate from formal document application.'],'verification':verification,'request_metrics':[r['summary'] for r in high.values()],'max_request_metrics':[r['summary'] for r in maximum.values()]}
path=root/'docs/audits/2026-10-09-flash-effort-comparison.json';path.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
print(json.dumps({'calls':report['calls'],'high_fields':report['high_field_results'],'high_including_failures':report['high_effective_request_seconds_per_field_batch_including_two_failures'],'paired_hard_cases':report['paired_hard_case_comparison'],'high_stages':report['high_other_stages_seconds']},indent=2))
print('Comparison report saved; all prompt hashes, field coverage, evidence/value preservation and unchanged candidate/config checks passed')
