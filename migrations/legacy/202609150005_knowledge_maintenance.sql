CREATE TABLE maintenance_runs (
 id uuid PRIMARY KEY, project_id uuid NOT NULL REFERENCES projects(id), actor_id uuid NOT NULL REFERENCES users(id),
 request_id uuid NOT NULL, created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 snapshot_id uuid NOT NULL UNIQUE, snapshot jsonb NOT NULL, snapshot_sha256 text NOT NULL,
 base_generation bigint NOT NULL, status text NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','running','ready','incomplete','failed')),
 phase text NOT NULL DEFAULT 'snapshot', coverage jsonb NOT NULL, candidate jsonb,
 generation bigint NOT NULL DEFAULT 0, lease_until timestamptz, settings_hash text,
 model_calls integer NOT NULL DEFAULT 0, read_count integer NOT NULL DEFAULT 0, error_code text,
 UNIQUE(actor_id,project_id,request_id)
);
CREATE INDEX maintenance_claim ON maintenance_runs(status,created_at);
CREATE TABLE maintenance_checkpoints (
 run_id uuid NOT NULL REFERENCES maintenance_runs(id), id text NOT NULL, value jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(), PRIMARY KEY(run_id,id)
);
CREATE TABLE maintenance_calls (
 id uuid PRIMARY KEY, run_id uuid NOT NULL REFERENCES maintenance_runs(id), generation bigint NOT NULL,
 request jsonb NOT NULL, request_sha256 text NOT NULL, response jsonb,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(), finished_at timestamptz
);
ALTER TABLE catalog_versions ALTER COLUMN source_task_id DROP NOT NULL;
ALTER TABLE catalog_versions ADD COLUMN maintenance_run_id uuid REFERENCES maintenance_runs(id);
ALTER TABLE catalog_versions ADD CONSTRAINT catalog_source CHECK ((source_task_id IS NULL) <> (maintenance_run_id IS NULL));
CREATE TABLE knowledge_releases (
 id uuid PRIMARY KEY, project_id uuid NOT NULL REFERENCES projects(id), catalog_version_id uuid REFERENCES catalog_versions(id),
 source_run_id uuid REFERENCES maintenance_runs(id), annotations jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(), actor_id uuid REFERENCES users(id), origin text NOT NULL,
 UNIQUE(project_id,id)
);
ALTER TABLE project_catalogs ADD COLUMN current_knowledge_version_id uuid REFERENCES knowledge_releases(id);
CREATE TABLE knowledge_activation_receipts (
 project_id uuid NOT NULL REFERENCES projects(id), actor_id uuid NOT NULL REFERENCES users(id), request_id uuid NOT NULL,
 command jsonb NOT NULL, result jsonb NOT NULL, PRIMARY KEY(project_id,actor_id,request_id)
);
-- Preserve any pre-existing directory state when enabling unified knowledge history.
INSERT INTO knowledge_releases(id,project_id,catalog_version_id,annotations,origin)
 SELECT gen_random_uuid(),project_id,current_version_id,'[]'::jsonb,'legacy_baseline' FROM project_catalogs WHERE current_version_id IS NOT NULL;
UPDATE project_catalogs p SET current_knowledge_version_id=k.id FROM knowledge_releases k WHERE k.project_id=p.project_id;
