ALTER TABLE fingerprints ADD COLUMN representatives jsonb NOT NULL DEFAULT '[]'::jsonb;
