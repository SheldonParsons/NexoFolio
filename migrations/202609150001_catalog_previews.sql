-- Candidate-only directory work. Does not alter current definitions or classification.
CREATE TABLE catalog_preview_tasks (
 id uuid PRIMARY KEY,
 project_id uuid NOT NULL REFERENCES projects(id),
 candidate_id uuid NOT NULL UNIQUE,
 contract_version text NOT NULL,
 status text NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','running','ready','rejected','failed')),
 snapshot_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 snapshot jsonb NOT NULL,
 snapshot_sha256 text NOT NULL,
 generation bigint NOT NULL DEFAULT 0,
 lease_until timestamptz,
 generator jsonb,
 error_code text,
 candidate jsonb,
 review jsonb,
 finished_at timestamptz,
 UNIQUE(id,candidate_id)
);
CREATE INDEX catalog_preview_project_time ON catalog_preview_tasks(project_id,snapshot_at DESC);
CREATE TABLE catalog_preview_nodes (
 candidate_id uuid NOT NULL REFERENCES catalog_preview_tasks(candidate_id),
 id uuid NOT NULL,
 parent_id uuid,
 name text NOT NULL,
 description text NOT NULL,
 PRIMARY KEY(candidate_id,id),
 FOREIGN KEY(candidate_id,parent_id) REFERENCES catalog_preview_nodes(candidate_id,id) DEFERRABLE INITIALLY DEFERRED
);
CREATE TABLE catalog_preview_assignments (
 candidate_id uuid NOT NULL REFERENCES catalog_preview_tasks(candidate_id),
 interface_id uuid NOT NULL REFERENCES interface_documents(id),
 directory_id uuid,
 reason text NOT NULL,
 PRIMARY KEY(candidate_id,interface_id),
 FOREIGN KEY(candidate_id,directory_id) REFERENCES catalog_preview_nodes(candidate_id,id) DEFERRABLE INITIALLY DEFERRED
);
