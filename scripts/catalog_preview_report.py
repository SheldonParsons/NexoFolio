"""Render catalog-show JSON as a plain-text review, without a frontend or model call."""
import argparse,json
from pathlib import Path

def render(task):
 snapshot=task['snapshot'];candidate=task.get('candidate');review=task.get('review')
 lines=[f"项目：{snapshot['project_name']}",f"任务：{task['task_id']}",f"状态：{task['status']}",f"快照接口数：{len(snapshot['interfaces'])}",f"快照时间：{task['snapshot_at']}","当前正式接口/目录不因本候选而改变。",""]
 if not candidate:return '\n'.join(lines+["尚无候选目录。"])
 if review:
  metrics=review['metrics'];lines.extend([f"结构检查：{'通过' if review['structurally_valid'] else '未通过'}（不代表业务分类正确）",f"目录数：{metrics['directory_count']}；最大深度：{metrics['max_depth']}；待分类：{metrics['unclassified_interfaces']}/{metrics['snapshot_interfaces']}"])
  for item in review['issues']+review['warnings']:lines.append(f"检查项：{item['code']} {item.get('subject') or ''}")
 if review and review['metrics'].get('logical_interfaces') is not None:lines.append(f"候选展示项：{review['metrics']['logical_interfaces']}；原始记录：{len(snapshot['interfaces'])}")
 for group in candidate.get('merge_groups',[]):
  lines.extend(['',f"候选合并：{group['path_template']}（{len(group['member_ids'])}个原始成员）",group['reason'],'原始成员：'+', '.join(group['member_ids'])])
 interfaces={i['interface_id']:i for i in snapshot['interfaces']}
 children={};assignments={}
 for node in candidate['nodes']:children.setdefault(node.get('parent'),[]).append(node)
 for item in candidate['assignments']:assignments.setdefault(item.get('directory_id'),[]).append(item)
 def members(directory,indent):
  for item in assignments.get(directory,[]):
   interface=interfaces.get(item['interface_id']);name=f"{interface['method']} {interface['path']}" if interface else item['interface_id']
   lines.append(f"{indent}- {name}：{item['reason']}")
 seen=set()
 def visit(node,depth):
  indent='  '*min(depth,20)
  if node['id'] in seen:lines.append(indent+'[重复/循环目录] '+node['name']);return
  seen.add(node['id']);lines.append(indent+node['name']+' — '+node['description']);members(node['id'],indent+'  ')
  if depth>=20:lines.append(indent+'[显示深度限制]');return
  for child in children.get(node['id'],[]):visit(child,depth+1)
 lines.extend(['','候选目录：'])
 for node in children.get(None,[]):visit(node,0)
 for node in candidate['nodes']:
  if node['id'] not in seen:lines.append('[未连接/异常节点]');visit(node,0)
 lines.extend(['','待分类：']);members(None,'')
 return '\n'.join(lines)+'\n'
if __name__=='__main__':
 parser=argparse.ArgumentParser();parser.add_argument('--input',required=True);parser.add_argument('--output',required=True);args=parser.parse_args()
 Path(args.output).write_text(render(json.loads(Path(args.input).read_text())))
 print('Catalog preview review written')
