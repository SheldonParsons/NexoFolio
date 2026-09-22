-- New observations pin their identity decision; legacy inbox rows keep literal processing.
ALTER TABLE projects ADD COLUMN path_policy jsonb NOT NULL DEFAULT '{"enabled":true,"numeric_segments":true,"literal_prefixes":[]}'::jsonb;
ALTER TABLE ingestion_inbox ADD COLUMN path_identity jsonb;
