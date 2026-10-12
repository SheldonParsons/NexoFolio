-- Owner: access. Local business projects inherit one real ZenTao project's grants.
ALTER TABLE projects ADD COLUMN origin text NOT NULL DEFAULT 'zentao';
ALTER TABLE projects ADD COLUMN permission_source_project_id uuid;
ALTER TABLE projects ALTER COLUMN external_id DROP NOT NULL;
ALTER TABLE projects ALTER COLUMN last_sync_generation SET DEFAULT 0;
ALTER TABLE projects ADD CONSTRAINT projects_instance_identity UNIQUE(instance,id);
ALTER TABLE projects ADD CONSTRAINT projects_permission_source FOREIGN KEY(instance,permission_source_project_id) REFERENCES projects(instance,id);
ALTER TABLE projects ADD CONSTRAINT projects_origin CHECK (
    (origin='zentao' AND external_id IS NOT NULL AND permission_source_project_id IS NULL) OR
    (origin='local' AND external_id IS NULL AND permission_source_project_id IS NOT NULL AND permission_source_project_id<>id)
);
CREATE UNIQUE INDEX local_project_names ON projects(instance,lower(name)) WHERE origin='local';
CREATE INDEX projects_permission_source ON projects(permission_source_project_id) WHERE permission_source_project_id IS NOT NULL;

CREATE OR REPLACE VIEW effective_project_access AS
SELECT u.id AS user_id, p.id AS project_id
FROM project_access_principals u
JOIN projects p ON p.instance=u.instance
WHERE u.ready AND (
    u.all_projects OR EXISTS (
        SELECT 1 FROM user_project_access a
        WHERE a.user_id=u.id AND a.project_id=coalesce(p.permission_source_project_id,p.id)
    )
);
