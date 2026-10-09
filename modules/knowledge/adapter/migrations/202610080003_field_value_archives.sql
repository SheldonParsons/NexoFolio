-- Immutable captured-value documents and references, saved once instead of copied into every field/batch.
CREATE TABLE field_value_archives (
 candidate_id uuid PRIMARY KEY REFERENCES candidates(id) ON DELETE CASCADE,
 project_id uuid NOT NULL,
 archive text NOT NULL
);
CREATE INDEX field_value_archives_project ON field_value_archives(project_id);
