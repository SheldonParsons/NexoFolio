"""Synthetic step-06 cases on the existing owned-PG harness."""
import hashlib, json
from frozen_input_harness import Upstream, ident, encoded


def literal(value):
    return "'" + encoded(value).replace("'", "''") + "'"


def install_identity():
    def get(self):
        if self.path.startswith('/api.php/v1/user'):
            account = self.headers.get('Token', 'A80646')
            number = int(account.rsplit('-', 1)[-1]) if account.startswith('reader-') else 1
            self.reply({'profile': dict(id=number, account=account, realname='合成读取用户', deleted='0', view={'projects': '11'})})
        elif self.path.startswith('/api.php/v1/projects'):
            self.reply(dict(page=1, limit=100, total=2, projects=[dict(id=n, name='合成项目'+str(n), status='doing') for n in [11,12]]))
        else: self.reply({}, 404)
    def post(self):
        body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', '0'))))
        if self.path == '/api.php/v1/tokens': self.reply({'token': body['account']})
        else:
            Upstream.calls.append({'unexpected': self.path})
            self.reply({'error':'models disabled'}, 503)
    Upstream.do_GET, Upstream.do_POST = get, post


def enrich(h, db, meta):
    project = meta['project']
    # Keep the original stress query at 501 hits, with the same 500 x 50 capacity fields.
    h.sql(db, """INSERT INTO knowledge.field_notes(endpoint_id,project_id,path,text,author,context)
    SELECT id,project_id,'request_body:remark','合成测试备注',jsonb_build_object('by','person','user',(SELECT id FROM access.users LIMIT 1)),
    '{"location":{"in":"request_body"},"field_path":"remark","display_name":"备注"}' FROM observe.endpoints WHERE path_template='/synthetic/orders'""")
    endpoints = json.loads(h.sql(db, "SELECT json_agg(id ORDER BY id) FROM observe.endpoints WHERE path_template LIKE '/capacity/%'"))
    env = h.sql(db, 'SELECT id FROM access.environments LIMIT 1')
    other = ident('other-env')
    h.sql(db, f"INSERT INTO access.environments(id,project_id,name) VALUES('{other}','{project}','第二环境')")
    target = h.sql(db, "SELECT id FROM observe.endpoints WHERE path_template='/synthetic/orders'")
    h.sql(db, f"INSERT INTO observe.fingerprints(endpoint_id,environment_id,address,hash,structure,sample,calls,first_seen,last_seen) SELECT endpoint_id,'{other}',address,hash,structure,jsonb_set(sample,'{{environment_id}}',to_jsonb('{other}'::text)),calls,first_seen,last_seen FROM observe.fingerprints WHERE endpoint_id='{target}'")
    parent, child = ident('directory'), ident('child-directory')
    h.sql(db, f"INSERT INTO knowledge.folders(id,project_id,name,position) VALUES('{parent}','{project}','父目录',0); INSERT INTO knowledge.folders(id,project_id,parent_id,name,position) VALUES('{child}','{project}','{parent}','子目录',0)")
    h.sql(db, f"INSERT INTO knowledge.placements(endpoint_id,project_id,folder_id,author) SELECT id,project_id,'{child}',jsonb_build_object('by','person','user',(SELECT id FROM access.users LIMIT 1)) FROM observe.endpoints WHERE id IN ('{endpoints[0]}','{target}')")
    # Unicode case/whitespace and literal wildcard characters; search uses full text, output keeps previews.
    text = 'MiXeD 100%_literal \\ Ελληνικά ΟΣ İ K 中文\u2003词语 ' + '完整语义。'*180
    h.sql(db, "UPDATE knowledge.field_notes SET text=" + "'" + text.replace("'", "''") + "'" + f" WHERE endpoint_id='{endpoints[0]}' AND path='request_body:field01'")
    h.sql(db, f"INSERT INTO knowledge.endpoint_notes(endpoint_id,project_id,name,purpose,author) SELECT id,project_id,'100%_Interface','MiXeD ΟΣ 中文完整用途',jsonb_build_object('by','person','user',(SELECT id FROM access.users LIMIT 1)) FROM observe.endpoints WHERE id='{endpoints[0]}'")
    meta.update(environment=env, other_environment=other, target=target, first=endpoints[0], directory=parent, child=child,
                forbidden_project=h.sql(db, "SELECT id FROM access.projects WHERE external_id='12'"))
    return meta


def archives(h, db, meta):
    project, endpoint, env = meta['project'], meta['target'], meta['environment']
    field = endpoint+'/request_body:remark'
    doc = dict(source=dict(endpoint=endpoint, example='synthetic', environment=env), observed_at=None,
               observation_key=None, representation='json', state='full', root={'v': ['精确\0正文', 900719925474099312345, 1.23456789012345]})
    document2 = dict(doc, source=dict(doc['source'], environment=meta['other_environment']))
    documents = {'d1':doc, 'd2':document2, 'unused':dict(doc, root='无关大文档'*200000)}
    values = [dict(id='one', value_type='array', representation='json', occurrences=[dict(document=d, pointer='/v') for d in ['d1','d2']])]
    archive = dict(version=1, documents=documents, fields={field:dict(values=values, coverage=[dict(source=doc['source'],state='full')], hash='synthetic-field-hash')})
    result = {}
    for version in [2,3,4]:
        candidate = ident('read-archive-'+str(version))
        h.sql(db, f"INSERT INTO knowledge.candidates(id,project_id,state,created_at,updated_at,base_hash,base,view,batches,progress,applied_rounds) VALUES('{candidate}','{project}','discarded',now(),now(),'synthetic','{{}}','{{}}','[]','{{}}','[]')")
        if version == 4:
            archive = json.loads(encoded(archive))
            for value in archive['documents'].values():
                if isinstance(value['root'], dict): value['root']['v'][0] = '精确正文'
        if version != 3:
            h.sql(db, f"INSERT INTO knowledge.field_value_archives(project_id,candidate_id,archive) VALUES('{project}','{candidate}',{literal(archive)})")
        else:
            snapshot = ident('read-snapshot')
            def chunk(value):
                content = encoded(value)
                ref = dict(sha256=hashlib.sha256(content.encode()).hexdigest(), bytes=len(content.encode()))
                h.sql(db, f"INSERT INTO knowledge.frozen_input_chunks(project_id,candidate_id,snapshot_id,sha256,content) VALUES('{project}','{candidate}','{snapshot}','{ref['sha256']}',{literal(value)}) ON CONFLICT DO NOTHING")
                return ref
            empty = chunk([])
            docs = {key:chunk(value) for key,value in documents.items()}
            manifest = dict(version=1,project=project,candidate=candidate,snapshot=snapshot,links=empty,
                            endpoints={endpoint:dict(facts=empty,inputs=[],values=chunk(dict(archive,documents={})),documents=docs)})
            header = dict(version=3,manifest=manifest)
            header['manifest_hash'] = hashlib.sha256(encoded(header).encode()).hexdigest()
            h.sql(db, f"UPDATE knowledge.candidates SET execution_input={literal(header)}::jsonb WHERE id='{candidate}'")
            result['unused_hash'] = docs['unused']['sha256']
            result['required_hash'] = docs['d1']['sha256']
        result[str(version)] = candidate
    snapshot = dict(endpoints=[endpoint],folders=[],placements=[],notes=[],fields=[],links=[])
    h.sql(db,f"UPDATE knowledge.candidates SET base={literal(snapshot)}::jsonb,view={literal(snapshot)}::jsonb")
    return result
