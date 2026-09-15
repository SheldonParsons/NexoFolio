-- Capture receipt/evidence transactions are independent from model execution.
CREATE TABLE capture_blobs(project_id uuid NOT NULL REFERENCES projects(id),sha256 text NOT NULL,bytes bigint NOT NULL,media_type text NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(project_id,sha256));
CREATE TABLE capture_assets(project_id uuid NOT NULL REFERENCES projects(id),id uuid NOT NULL,sha256 text NOT NULL,actor_id uuid NOT NULL REFERENCES users(id),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(project_id,id),FOREIGN KEY(project_id,sha256) REFERENCES capture_blobs(project_id,sha256));
CREATE TABLE capture_events(
 id uuid PRIMARY KEY,project_id uuid NOT NULL REFERENCES projects(id),environment_id uuid NOT NULL REFERENCES environments(id),actor_id uuid NOT NULL REFERENCES users(id),producer_id uuid NOT NULL,source_type text NOT NULL,
 record_id uuid NOT NULL,kind text NOT NULL,payload_version text NOT NULL,captured_at timestamptz NOT NULL,context jsonb,raw_hash text,content_hash bytea NOT NULL,
 ingestion_id uuid REFERENCES ingestion_inbox(id),structure text NOT NULL,evidence_status text NOT NULL DEFAULT 'pending' CHECK(evidence_status IN ('pending','processing','completed','failed')),
 generation bigint NOT NULL DEFAULT 0,attempts integer NOT NULL DEFAULT 0,lease_until timestamptz,retry_at timestamptz NOT NULL DEFAULT clock_timestamp(),error_code text,processed_at timestamptz,received_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(actor_id,producer_id,record_id),FOREIGN KEY(project_id,raw_hash) REFERENCES capture_blobs(project_id,sha256)
);
CREATE TABLE capture_receipts(actor_id uuid NOT NULL REFERENCES users(id),producer_id uuid NOT NULL,record_id uuid NOT NULL,content_hash bytea NOT NULL,result jsonb NOT NULL,PRIMARY KEY(actor_id,producer_id,record_id));
CREATE INDEX capture_pending ON capture_events(retry_at,received_at) WHERE evidence_status IN ('pending','processing');
CREATE INDEX capture_project_time ON capture_events(project_id,environment_id,captured_at,id);
CREATE TABLE evidence_facts(id uuid PRIMARY KEY,project_id uuid NOT NULL REFERENCES projects(id),environment_id uuid NOT NULL REFERENCES environments(id),kind text NOT NULL,subject jsonb NOT NULL,data jsonb NOT NULL,key_hash text NOT NULL,observations bigint NOT NULL DEFAULT 1,first_seen timestamptz NOT NULL,last_seen timestamptz NOT NULL,UNIQUE(project_id,environment_id,kind,key_hash));
CREATE INDEX evidence_subject ON evidence_facts(project_id,environment_id,kind);
CREATE TABLE evidence_samples(fact_id uuid NOT NULL REFERENCES evidence_facts(id),event_id uuid NOT NULL REFERENCES capture_events(id),first_sample boolean NOT NULL DEFAULT false,counterexample boolean NOT NULL DEFAULT false,PRIMARY KEY(fact_id,event_id));
CREATE TABLE evidence_value_index(event_id uuid NOT NULL REFERENCES capture_events(id),project_id uuid NOT NULL,environment_id uuid NOT NULL,producer_id uuid NOT NULL,page_instance_id uuid NOT NULL,frame_instance_id uuid NOT NULL,view_id uuid NOT NULL,interaction_id uuid,field_key text NOT NULL,field_ref jsonb NOT NULL,pointer text NOT NULL,value jsonb NOT NULL,match_key text NOT NULL,direction text NOT NULL,started_at bigint NOT NULL,available_at bigint NOT NULL,PRIMARY KEY(event_id,field_key,pointer));
CREATE INDEX evidence_value_lookup ON evidence_value_index(project_id,environment_id,producer_id,page_instance_id,frame_instance_id,view_id,match_key,available_at);
CREATE TABLE evidence_relation_pairs(source_event uuid NOT NULL,target_event uuid NOT NULL,source_key text NOT NULL,target_key text NOT NULL,fact_id uuid NOT NULL REFERENCES evidence_facts(id),PRIMARY KEY(source_event,target_event,source_key,target_key));
CREATE TABLE evidence_pins(owner_kind text NOT NULL,owner_id uuid NOT NULL,event_id uuid NOT NULL REFERENCES capture_events(id),PRIMARY KEY(owner_kind,owner_id,event_id));
CREATE TABLE evidence_ui_bindings(project_id uuid NOT NULL,environment_id uuid NOT NULL,actor_id uuid NOT NULL,producer_id uuid NOT NULL,browser_instance_id uuid NOT NULL,page_instance_id uuid NOT NULL,frame_instance_id uuid NOT NULL,view_id uuid NOT NULL,element_id text NOT NULL,field_key text NOT NULL,field_ref jsonb NOT NULL,PRIMARY KEY(project_id,environment_id,actor_id,producer_id,browser_instance_id,page_instance_id,frame_instance_id,view_id,element_id,field_key));
CREATE TABLE evidence_ui_pairs(http_event uuid NOT NULL,ui_event uuid NOT NULL,field_key text NOT NULL,fact_id uuid NOT NULL REFERENCES evidence_facts(id),PRIMARY KEY(http_event,ui_event,field_key));
ALTER TABLE ingestion_inbox ALTER COLUMN raw_record DROP NOT NULL;
ALTER TABLE evidence_value_index ADD COLUMN actor_id uuid NOT NULL REFERENCES users(id);
ALTER TABLE evidence_value_index ADD COLUMN browser_instance_id uuid NOT NULL;
CREATE INDEX evidence_value_owner_lookup ON evidence_value_index(project_id,environment_id,actor_id,producer_id,browser_instance_id,page_instance_id,frame_instance_id,view_id,match_key,available_at);
CREATE TABLE evidence_relation_values(fact_id uuid NOT NULL REFERENCES evidence_facts(id),value_hash text NOT NULL,PRIMARY KEY(fact_id,value_hash));
ALTER TABLE evidence_facts ADD COLUMN subject_hash text NOT NULL DEFAULT '';
CREATE INDEX evidence_fact_subject_lookup ON evidence_facts(project_id,environment_id,kind,subject_hash);

CREATE INDEX capture_context_lookup ON capture_events(project_id,environment_id,actor_id,producer_id,(context->>'browser_instance_id'),(context->>'page_instance_id'),(context->>'frame_instance_id'),(context->>'interaction_id'),captured_at);
CREATE TABLE evidence_sample_groups(fact_id uuid NOT NULL REFERENCES evidence_facts(id),support_key text NOT NULL,event_ids uuid[] NOT NULL,observed_at timestamptz NOT NULL,first_sample boolean NOT NULL DEFAULT false,counterexample boolean NOT NULL DEFAULT false,PRIMARY KEY(fact_id,support_key));
CREATE TABLE capture_backlog(project_id uuid PRIMARY KEY REFERENCES projects(id),pending bigint NOT NULL CHECK(pending>=0));
INSERT INTO capture_backlog(project_id,pending) SELECT project_id,count(*) FROM capture_events WHERE evidence_status<>'completed' GROUP BY project_id;
