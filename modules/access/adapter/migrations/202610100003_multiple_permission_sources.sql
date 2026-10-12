-- Owner: access. Keep project identity/data, normalize permission bindings.
ALTER TABLE projects ADD COLUMN management_revision bigint NOT NULL DEFAULT 1 CHECK (management_revision > 0);
ALTER TABLE projects ADD CONSTRAINT projects_origin_identity UNIQUE(instance,id,origin);

CREATE TABLE project_permission_sources (
    instance text NOT NULL,
    project_id uuid NOT NULL,
    source_project_id uuid NOT NULL,
    project_origin text NOT NULL DEFAULT 'local' CHECK (project_origin='local'),
    source_origin text NOT NULL DEFAULT 'zentao' CHECK (source_origin='zentao'),
    PRIMARY KEY(project_id,source_project_id),
    FOREIGN KEY(instance,project_id,project_origin) REFERENCES projects(instance,id,origin) ON DELETE CASCADE,
    FOREIGN KEY(instance,source_project_id,source_origin) REFERENCES projects(instance,id,origin),
    CHECK(project_id<>source_project_id)
);
CREATE INDEX project_permission_sources_source ON project_permission_sources(source_project_id,project_id);

INSERT INTO project_permission_sources(instance,project_id,source_project_id)
SELECT instance,id,permission_source_project_id FROM projects WHERE origin='local';

CREATE OR REPLACE VIEW effective_project_access AS
SELECT u.id AS user_id, p.id AS project_id
FROM project_access_principals u
JOIN projects p ON p.instance=u.instance
WHERE u.ready AND (
    u.all_projects OR
    (p.origin='zentao' AND EXISTS (
        SELECT 1 FROM user_project_access a WHERE a.user_id=u.id AND a.project_id=p.id
    )) OR
    (p.origin='local' AND EXISTS (
        SELECT 1 FROM project_permission_sources s
        JOIN user_project_access a ON a.project_id=s.source_project_id
        WHERE s.project_id=p.id AND a.user_id=u.id
    ))
);

ALTER TABLE projects DROP CONSTRAINT projects_origin;
ALTER TABLE projects DROP COLUMN permission_source_project_id;
ALTER TABLE projects ADD CONSTRAINT projects_origin CHECK (
    (origin='zentao' AND external_id IS NOT NULL) OR
    (origin='local' AND external_id IS NULL)
);
