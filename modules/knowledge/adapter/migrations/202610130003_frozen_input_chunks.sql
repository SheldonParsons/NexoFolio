-- Additive format: existing v2 inputs and value archives remain byte-for-byte unchanged.
ALTER TABLE candidates ADD CONSTRAINT candidates_project_identity UNIQUE(project_id,id);
CREATE TABLE frozen_input_chunks (
    project_id uuid NOT NULL,
    candidate_id uuid NOT NULL,
    snapshot_id uuid NOT NULL,
    sha256 text NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    content text NOT NULL,
    PRIMARY KEY(project_id,candidate_id,snapshot_id,sha256),
    FOREIGN KEY(project_id,candidate_id) REFERENCES candidates(project_id,id) ON DELETE CASCADE
);
-- Application writes are append-only. A manifest is published only after all references validate.
