#!/usr/bin/env python3
"""Show generated explanations for manual review, masking observed values and identifiers."""
import argparse,json,re
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--effort',choices=['high','max'],required=True);p.add_argument('--cases',required=True);p.add_argument('--output-name');args=p.parse_args()
root=Path(__file__).resolve().parent.parent/'experiments/results'
cases=json.loads((root/'model-comparison-20261009/cases.json').read_text())
for case_id in args.cases.split(','):
 file=root/(args.output_name or f'model-flash-{args.effort}-20261009')/f'{case_id}-flash-{args.effort}.json'
 if not file.exists():continue
 data=json.loads(file.read_text());secret_values=set()
 def collect(value):
  if isinstance(value,str):
   if len(value)>=3:secret_values.add(value)
   try:
    decoded=json.loads(value)
    if decoded!=value:collect(decoded)
   except (ValueError,TypeError):pass
  elif isinstance(value,dict):
   for v in value.values():collect(v)
  elif isinstance(value,list):
   for v in value:collect(v)
 def safe(value):
  text=json.dumps(value,ensure_ascii=False)
  for secret in sorted(secret_values,key=len,reverse=True):text=text.replace(secret,'[观测值]')
  text=re.sub(r'https?://[^\s"，。]+','[URL]',text)
  text=re.sub(r'[a-zA-Z0-9_=/+\-]{24,}','[long identifier]',text)
  return re.sub(r'\b\d{6,}\b','[number]',text)
 print('CASE',case_id)
 if case_id.startswith('field-'):
  case=cases[int(case_id.split('-')[1])]
  for field in case['inputs']:
   collect(field.get('values',[]));collect(field.get('value_examples',[]))
  print('PATHS',[f['path'] for f in case['inputs']])
  print('TYPES',[f['types'] for f in case['inputs']])
 if 'reply' not in data:
  print('No final reply; validation',data['summary'].get('valid'));continue
 reply=json.loads(data['reply'])
 if case_id.startswith('field-'):
  for field in reply.get('fields',[]): print(safe({k:v for k,v in field.items() if k in ['key','display_name','description','details','unknowns','name_unknown','description_unknown']}))
 elif case_id=='stage-2':
  print('top folders',len(reply.get('folders',[])),'unplaced',len(reply.get('unplaced',[])))
 else:
  for key in ['name','purpose','unknowns']:print(key,safe(reply.get(key)))
  description=reply.get('description','')
  print('description markers',{term:term in description for term in ['需 Authorization','必须','分页','推测','未确认','仅代表本次观测']})
