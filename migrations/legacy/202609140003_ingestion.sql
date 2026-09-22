-- Owner: ingestion admission. Not formal API definitions or historical API versions.
CREATE TABLE ingestion_inbox (
 id uuid PRIMARY KEY,
 project_id uuid NOT NULL REFERENCES projects(id),
 actor_id uuid NOT NULL REFERENCES users(id),
 producer_id uuid NOT NULL,
 source_type text NOT NULL,
 record_id uuid NOT NULL,
 batch_id uuid NOT NULL,
 environment_id uuid NOT NULL REFERENCES environments(id),
 service_key text NOT NULL,
 identity_key text NOT NULL,
 structural_hash bytea,
 raw_record jsonb NOT NULL,
 status text NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','processing','completed','failed')),
 received_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX ingestion_pending ON ingestion_inbox(received_at) WHERE status='pending';
CREATE TABLE ingestion_heads (
 project_id uuid NOT NULL REFERENCES projects(id),
 environment_id uuid NOT NULL REFERENCES environments(id),
 service_key text NOT NULL,
 identity_key text NOT NULL,
 algorithm_version text NOT NULL,
 structural_hash bytea,
 ingestion_id uuid NOT NULL REFERENCES ingestion_inbox(id),
 PRIMARY KEY(project_id,environment_id,service_key,identity_key)
);
CREATE TABLE ingestion_receipts (
 actor_id uuid NOT NULL REFERENCES users(id),
 producer_id uuid NOT NULL,
 record_id uuid NOT NULL,
 content_hash bytea NOT NULL,
 status text NOT NULL CHECK(status IN ('accepted','ignored')),
 reason_code text NOT NULL,
 ingestion_id uuid NOT NULL REFERENCES ingestion_inbox(id),
 received_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(actor_id,producer_id,record_id)
);
