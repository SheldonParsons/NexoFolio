"""Compare database review paging/counts with the unchanged frontend semantic rule."""
import copy, json, pathlib, subprocess
from frozen_input_harness import ROOT, ident
from mcp_read_fixture import literal
NODE='/Users/sheldon/.nvm/versions/node/v22.22.0/bin/node'
FRONT=ROOT.parent/'NexoFolio-Front'
def check(h,db,meta,out):
    p,c=meta['project'],meta['candidate'];url=f'/v1/projects/{p}/curate/candidates/{c}'
    original=h.request(url)[1];base=copy.deepcopy(original['base']);view=copy.deepcopy(original['view'])
    e1,e2=base['endpoints'][:2];author=copy.deepcopy(view['fields'][0]['author']);at='2026-10-14T00:00:00Z'
    folder=dict(id=ident('review-folder-a'),author=author,parent=None,name='旧目录',summary='目录说明',position=0,endpoints=0,endpoints_deep=0)
    base['folders']=[folder];view['folders']=[dict(folder,name='新目录'),dict(folder,id=ident('review-folder-b'),name='另一目录')]
    base['placements']=[dict(endpoint=e1,folder=folder['id'],author=author,at=at)]
    view['placements']=[dict(base['placements'][0],folder=view['folders'][1]['id'])]
    note=dict(endpoint=e1,name='旧名称',purpose='用途',context={},author=author,at=at)
    base['notes']=[note];view['notes']=[dict(note,name='新名称'),dict(note,endpoint=e2,name='第二接口')]
    base['fields']=copy.deepcopy(view['fields'][:30])
    # 10 metadata-only rewrites, 10 semantic updates, 10 removals, remaining additions.
    for field in view['fields'][:10]:field['at']='2026-10-14T01:00:00Z';field['context']['input_hash']='different'
    for field in view['fields'][10:20]:field['text']='更新后的完整说明'
    del view['fields'][20:30]
    link=dict(id=ident('old-link'),**{'from':e1},from_field=None,to=e2,to_field=None,relation='precondition',evidence='证据',context={},author=author,at=at)
    base['links']=[link];view['links']=[dict(link,id=ident('new-link'),at='2026-10-14T01:00:00Z')]
    h.sql(db,f"UPDATE knowledge.candidates SET base={literal(base)},view={literal(view)} WHERE id='{c}'")
    typed=h.request(url)[1]
    # Keep one genuinely old row's missing context defaults. The old typed read
    # fills them; the read projection must agree without inventing a change.
    h.sql(db,f"UPDATE knowledge.candidates SET base={literal(typed['base'])},view={literal(typed['view'])} WHERE id='{c}'")
    h.sql(db,f"UPDATE knowledge.candidates SET base=jsonb_set(base,'{{fields,0,context}}',(base#>'{{fields,0,context}}') - 'meaning' - 'evidence') WHERE id='{c}'")
    typed=h.request(url)[1];head=h.request(url+'/review')[1];pages={}
    for section in ['folders','placements','notes','fields','links']:
        rows=[]
        for offset in range(0,max(1,head['changes'][section]),60):
            code,page=h.request(url+f'/review?part=changes&section={section}&offset={offset}&version={head["version"]}')
            assert code==200,(section,code,page);rows+=page['items']
        pages[section]=rows
    fixture=h.tmp/'semantic.json';fixture.write_text(json.dumps(dict(candidate=typed,head=head,pages=pages),ensure_ascii=False))
    bundle=h.tmp/'changes.cjs'
    subprocess.run([NODE,'-e',"require(process.argv[1]).buildSync({entryPoints:[process.argv[2]],bundle:true,platform:'node',format:'cjs',outfile:process.argv[3]})",str(FRONT/'node_modules/esbuild'),str(FRONT/'src/features/curation/changes.ts'),str(bundle)],check=True)
    script="""const fs=require('node:fs'),assert=require('node:assert/strict');const {candidateChanges}=require(process.argv[1]);const x=JSON.parse(fs.readFileSync(process.argv[2]));const expected=candidateChanges(x.candidate);for(const [section,items] of Object.entries(x.pages)){const scoped={...x.candidate,base:{...x.candidate.base,[section]:items.flatMap(i=>i.before?[i.before]:[])},view:{...x.candidate.view,[section]:items.flatMap(i=>i.after?[i.after]:[])}};const actual=candidateChanges(scoped).filter(c=>c.section===section), wanted=expected.filter(c=>c.section===section);assert.equal(x.head.changes[section],wanted.length,section+' count');assert.deepEqual(actual,wanted,section+' complete content');}console.log(JSON.stringify({sections:5,changes:expected.length,legacyDefaults:true,fullBodiesEqual:true}));"""
    result=subprocess.check_output([NODE,'-e',script,str(bundle),str(fixture)],text=True)
    (out/'semantic-parity.json').write_text(result)
    h.sql(db,f"UPDATE knowledge.candidates SET base={literal(original['base'])},view={literal(original['view'])} WHERE id='{c}'")
    return json.loads(result)
