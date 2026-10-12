-- Separate from candidate state and observation facts. All writers use the
-- existing project advisory lock; version and decisions land atomically.
CREATE TABLE document_lifecycle (
    project_id uuid PRIMARY KEY,
    state jsonb NOT NULL
);
CREATE TABLE document_lifecycle_history (
    project_id uuid NOT NULL,
    version bigint NOT NULL,
    state jsonb NOT NULL,
    PRIMARY KEY (project_id, version)
);
