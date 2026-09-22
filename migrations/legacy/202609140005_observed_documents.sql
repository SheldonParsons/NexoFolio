-- Owner: knowledge. Observed definitions, not adjudicated or inferred business rules.
CREATE TABLE interface_documents (
 id uuid PRIMARY KEY,
 project_id uuid NOT NULL REFERENCES projects(id),
 identity_key text NOT NULL,
 method text NOT NULL,
 path text NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(project_id,identity_key),
 UNIQUE(project_id,id)
);
CREATE TABLE interface_observed_revisions (
 id uuid PRIMARY KEY,
 project_id uuid NOT NULL,
 interface_id uuid NOT NULL,
 environment_id uuid NOT NULL,
 definition jsonb NOT NULL,
 definition_hash bytea NOT NULL,
 origin_ingestion_id uuid NOT NULL UNIQUE REFERENCES ingestion_inbox(id),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 FOREIGN KEY(project_id,interface_id) REFERENCES interface_documents(project_id,id),
 FOREIGN KEY(project_id,environment_id) REFERENCES environments(project_id,id),
 UNIQUE(interface_id,environment_id,id)
);
CREATE TABLE interface_environment_current (
 interface_id uuid NOT NULL REFERENCES interface_documents(id),
 environment_id uuid NOT NULL REFERENCES environments(id),
 current_revision_id uuid NOT NULL,
 PRIMARY KEY(interface_id,environment_id),
 FOREIGN KEY(interface_id,environment_id,current_revision_id) REFERENCES interface_observed_revisions(interface_id,environment_id,id)
);
CREATE TABLE interface_observed_differences (
 id uuid PRIMARY KEY,
 interface_id uuid NOT NULL,
 environment_id uuid NOT NULL,
 base_revision_id uuid NOT NULL,
 proposed_definition jsonb NOT NULL,
 definition_hash bytea NOT NULL,
 origin_ingestion_id uuid NOT NULL REFERENCES ingestion_inbox(id),
 status text NOT NULL DEFAULT 'pending' CHECK(status='pending'),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 FOREIGN KEY(interface_id,environment_id,base_revision_id) REFERENCES interface_observed_revisions(interface_id,environment_id,id),
 UNIQUE(base_revision_id,definition_hash)
);
CREATE TABLE interface_observations (
 ingestion_id uuid PRIMARY KEY REFERENCES ingestion_inbox(id),
 interface_id uuid NOT NULL,
 environment_id uuid NOT NULL,
 compared_revision_id uuid NOT NULL,
 difference_id uuid REFERENCES interface_observed_differences(id),
 outcome text NOT NULL CHECK(outcome IN ('created','unchanged','difference_recorded')),
 processed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 FOREIGN KEY(interface_id,environment_id,compared_revision_id) REFERENCES interface_observed_revisions(interface_id,environment_id,id)
);
CREATE INDEX interface_observation_lookup ON interface_observations(interface_id,environment_id,processed_at DESC);
ALTER TABLE ingestion_inbox ADD COLUMN processing_generation bigint NOT NULL DEFAULT 0;
ALTER TABLE ingestion_inbox ADD COLUMN processing_attempts integer NOT NULL DEFAULT 0;
ALTER TABLE ingestion_inbox ADD COLUMN lease_until timestamptz;
ALTER TABLE ingestion_inbox ADD COLUMN retry_at timestamptz NOT NULL DEFAULT clock_timestamp();
ALTER TABLE ingestion_inbox ADD COLUMN last_error_code text;
ALTER TABLE ingestion_inbox ADD COLUMN completed_at timestamptz;
CREATE INDEX ingestion_processing_order ON ingestion_inbox(project_id,environment_id,identity_key,received_at,id) WHERE status<>'completed';
