"""Read-only current-data audit. Keeps raw bodies in memory; emits aggregate findings only.
Run with --container against an explicitly authorized local PostgreSQL container.
"""
import argparse,base64,collections,json,subprocess,urllib.parse,re
from pathlib import Path

def query(container,sql):
 r=subprocess.run(['docker','exec',container,'psql','-U','nexofolio','-d','nexofolio','-Atc',sql],capture_output=True,text=True,check=True)
 return json.loads(r.stdout)

def validate_shape(value,schema):
 if not isinstance(schema,dict):return False
 if schema.get('unknown'):return True
 if 'anyOf' in schema:return any(validate_shape(value,s) for s in schema['anyOf'])
 kind=schema.get('type')
 if value is None:return kind=='null'
 if isinstance(value,bool):return kind=='boolean'
 if isinstance(value,(int,float)):return kind=='number'
 if isinstance(value,str):return kind=='string'
 if isinstance(value,list):return kind=='array' and all(validate_shape(item,schema.get('items')) for item in value)
 if isinstance(value,dict):
  props=schema.get('properties',{})
  return kind=='object' and set(props)==set(value) and all(validate_shape(v,props[k]) for k,v in value.items())
 return False

def check_definition(raw,definition):
 errors=[];stats=collections.Counter()
 payload=raw['payload'];req=payload['request'];res=payload['response']
 parsed=urllib.parse.urlsplit(req['url'])
 template=definition['path'];a=template.split('/');b=parsed.path.split('/')
 declared={p['name'] for p in definition['request']['parameters'] if p['in']=='path'}
 used=set()
 def segment_matches(x,y):
  if x==y:return True
  if re.fullmatch(r'\{param[0-9]+\}',x):
   used.add(x[1:-1]);return bool(re.fullmatch(r'[0-9]+|[0-9a-fA-F]{32}|[0-9a-fA-F]{64}|[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}',y))
  return False
 if definition['method']!=req['method'] or len(a)!=len(b) or not all(segment_matches(x,y) for x,y in zip(a,b)) or used!=declared:errors.append('IDENTITY_MISMATCH')
 expected={k for k,v in urllib.parse.parse_qsl(parsed.query,keep_blank_values=True)}
 if {p['name'] for p in definition['request']['parameters'] if p['in']=='query'}!=expected:errors.append('QUERY_FIELD_MISMATCH')
 for side in ['request','response']:
  source=payload[side];stored=definition[side]
  if {h['name'] for h in stored['headers']}!={k.lower() for k,v in source['headers']['entries']}:errors.append('HEADER_NAME_MISMATCH')
  body=source['body'];d=stored['body'];state=body['state'];stats[f'{side}_{state}']+=1
  if d['state']!=state:errors.append('BODY_STATE_MISMATCH')
  if state=='none':continue
  if state!='complete':
   if d.get('observed_schema') is not None:errors.append('INCOMPLETE_PRESENTED_AS_SCHEMA')
   continue
  media=next((v.split(';')[0].strip().lower() for k,v in source['headers']['entries'] if k.lower()=='content-type'),'')
  stats[f'{side}_media:{media or "unspecified"}']+=1
  if media=='application/json' or media.endswith('+json'):
   try:
    content=base64.b64decode(body['content'],validate=True) if body['encoding']=='base64' else body['content']
    value=json.loads(content)
   except (ValueError,UnicodeError):
    if d.get('observed_schema') is not None:errors.append('INVALID_JSON_PRESENTED_AS_SCHEMA')
    continue
   if 'STRUCTURE_EXTRACTION_LIMIT' in definition['limitations']:stats['extraction_limited']+=1
   elif not validate_shape(value,d.get('observed_schema')):errors.append('JSON_FIELD_OR_TYPE_MISMATCH')
   else:stats[f'{side}_json_verified']+=1
  elif d.get('observed_schema') is not None:errors.append('NON_JSON_INVENTED_SCHEMA')
 return errors,stats

def changes(a,b,path=''):
 if type(a)!=type(b):return [path]
 if isinstance(a,dict):
  result=[]
  for key in sorted(a.keys()|b.keys()):
   child=path+'/'+key.replace('~','~0').replace('/','~1')
   result.extend([child] if key not in a or key not in b else changes(a[key],b[key],child))
  return result
 if isinstance(a,list):
  if len(a)!=len(b):return [path]
  return [p for i,(x,y) in enumerate(zip(a,b)) for p in changes(x,y,path+'/'+str(i))]
 return [] if a==b else [path]

if __name__=='__main__':
 parser=argparse.ArgumentParser();parser.add_argument('--container',required=True);parser.add_argument('--output',required=True);args=parser.parse_args()
 # One consistent read statement for all current comparison evidence.
 data=query(args.container,"""SELECT json_build_object(
 'definitions',(SELECT coalesce(json_agg(json_build_object('id',r.id,'definition',r.definition,'raw',i.raw_record,'still_current',EXISTS(SELECT 1 FROM interface_environment_current c WHERE c.current_revision_id=r.id))), '[]') FROM interface_observed_revisions r JOIN ingestion_inbox i ON i.id=r.origin_ingestion_id),
 'observations',(SELECT coalesce(json_agg(json_build_object('id',i.id,'raw',i.raw_record,'baseline',r.definition,'proposed',f.proposed_definition,'outcome',o.outcome)), '[]') FROM interface_observations o JOIN ingestion_inbox i ON i.id=o.ingestion_id JOIN interface_observed_revisions r ON r.id=o.compared_revision_id LEFT JOIN interface_observed_differences f ON f.id=o.difference_id),
 'inbox_count',(SELECT count(*) FROM ingestion_inbox),
 'completed_count',(SELECT count(*) FROM ingestion_inbox WHERE status='completed'),
 'unlinked_completed',(SELECT count(*) FROM ingestion_inbox i WHERE i.status='completed' AND NOT EXISTS(SELECT 1 FROM interface_observations o WHERE o.ingestion_id=i.id)))""")
 issues=[];stats=collections.Counter();difference_paths=[]
 for item in data['definitions']:
  errors,count=check_definition(item['raw'],item['definition']);stats.update(count)
  if errors:issues.append({'revision_id':item['id'],'codes':errors})
 for item in data['observations']:
  definition=item['proposed'] if item['outcome']=='difference_recorded' else item['baseline']
  errors,_=check_definition(item['raw'],definition)
  if errors:issues.append({'ingestion_id':item['id'],'codes':errors})
  if item['outcome']=='difference_recorded':
   paths=changes(item['baseline'],item['proposed'])
   difference_paths.append({'ingestion_id':item['id'],'changed_schema_paths':paths[:30],'path_count':len(paths),'has_actual_definition_difference':bool(paths)})
 report={'read_only':True,'inbox_count':data['inbox_count'],'completed_count':data['completed_count'],'definitions_checked':len(data['definitions']),'observations_checked':len(data['observations']),'unlinked_completed':data['unlinked_completed'],'all_initial_baselines_still_current':all(x['still_current'] for x in data['definitions']),'definition_coverage':dict(stats),'findings':issues,'differences':difference_paths}
 Path(args.output).write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
 print(json.dumps(report,ensure_ascii=False,indent=2))
 if issues or data['unlinked_completed']:raise SystemExit(1)
