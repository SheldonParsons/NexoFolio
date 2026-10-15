-- Synthetic owned database only. Each UNION arm has records, no maintenance is enabled.
CREATE TEMP TABLE fixture_keys AS
SELECT n, md5('__PROJECT__' || ':' || n)::uuid AS id
FROM generate_series(1, __COUNT__) n;
INSERT INTO access.projects(id,instance,external_id,name,status,last_sync_generation)
SELECT '__PROJECT__',instance,'statistics-__COUNT__','统计修复 __COUNT__','doing',0 FROM access.projects LIMIT 1;
INSERT INTO observe.endpoints(id,project_id,method,path_template)
SELECT id,'__PROJECT__','GET','/statistics/'||n FROM fixture_keys;
INSERT INTO observe.fingerprints(endpoint_id,environment_id,address,hash,structure,sample,representatives,calls,first_seen,last_seen)
SELECT id,'__PROJECT__','https://synthetic.invalid',decode(md5(n::text)||md5(n::text),'hex'),
       '{"structure":"synthetic"}',jsonb_build_object('payload',repeat('synthetic',512)),
       jsonb_build_array(n),1,'2025-01-01T00:00:00Z','2025-01-01T00:00:00Z' FROM fixture_keys;
INSERT INTO observe.declarations(endpoint_id,environment_id,platform,structure,updated_at)
SELECT id,NULL,'synthetic',jsonb_build_object('n',n),'2025-01-01T00:00:00Z' FROM fixture_keys;
INSERT INTO knowledge.candidates(id,project_id,state,created_at,updated_at,base_hash,base,view,batches,progress,applied_rounds,execution_input,evidence_input)
SELECT id,'__PROJECT__','discarded','2025-01-01T00:00:00Z','2025-01-01T00:00:00Z','synthetic',
       jsonb_build_object('n',n),'{"view":"synthetic"}','[]','{}','[]',
       jsonb_build_object('version',2,'n',n),jsonb_build_object('version',3,'n',n) FROM fixture_keys;
INSERT INTO knowledge.frozen_input_chunks(project_id,candidate_id,snapshot_id,sha256,content)
SELECT '__PROJECT__',id,id,repeat('a',64),repeat('synthetic text',512) FROM fixture_keys;
INSERT INTO knowledge.field_value_archives(project_id,candidate_id,archive)
SELECT '__PROJECT__',id,repeat('synthetic archive',256) FROM fixture_keys;
INSERT INTO knowledge.rounds(id,project_id,author,started_at,finished_at,state)
SELECT id,'__PROJECT__','{"by":"person"}','2025-01-01T00:00:00Z','2025-01-01T00:00:00Z','done' FROM fixture_keys;
INSERT INTO knowledge.revisions(round_id,seq,target,before)
SELECT id,1,jsonb_build_object('n',n),'{"previous":"synthetic"}' FROM fixture_keys;
INSERT INTO knowledge.field_notes(endpoint_id,project_id,path,text,author,context,values_seen)
SELECT id,'__PROJECT__','request_body:field','synthetic '||n,'{"by":"person"}',jsonb_build_object('n',n),'[]' FROM fixture_keys;
INSERT INTO knowledge.endpoint_notes(endpoint_id,project_id,name,purpose,author)
SELECT id,'__PROJECT__','synthetic '||n,'fixture','{"by":"person"}' FROM fixture_keys;
DROP TABLE fixture_keys;
