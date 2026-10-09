-- Owner: knowledge. Generated knowledge stays here until explicitly reviewed.
CREATE TABLE candidates (
 id uuid PRIMARY KEY,
 project_id uuid NOT NULL,
 target uuid,
 state text NOT NULL CHECK (state IN ('running','ready','partial','applied','discarded','interrupted')),
 created_at timestamptz NOT NULL,
 updated_at timestamptz NOT NULL,
 base_hash text NOT NULL,
 base jsonb NOT NULL,
 view jsonb NOT NULL,
 batches jsonb NOT NULL,
 progress jsonb NOT NULL,
 parent uuid REFERENCES candidates(id),
 applied_rounds jsonb NOT NULL,
 reviewed_by uuid
);
CREATE INDEX candidates_project ON candidates(project_id, created_at DESC);
