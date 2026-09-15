-- Owner: project environment metadata. Stable identity across renaming.
CREATE TABLE environments (
 id uuid PRIMARY KEY,
 project_id uuid NOT NULL REFERENCES projects(id),
 name text NOT NULL CHECK(char_length(name) BETWEEN 1 AND 64),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(project_id,name),
 UNIQUE(project_id,id)
);
CREATE TABLE environment_names (
 project_id uuid NOT NULL REFERENCES projects(id),
 name text NOT NULL,
 environment_id uuid NOT NULL,
 PRIMARY KEY(project_id,name),
 FOREIGN KEY(project_id,environment_id) REFERENCES environments(project_id,id)
);
-- Retain old names as aliases: queued name-based uploads still resolve after rename.
