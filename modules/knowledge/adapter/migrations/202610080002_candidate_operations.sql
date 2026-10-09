-- Identity snapshots belong to successful proposal lifecycle transactions.
ALTER TABLE candidates ADD COLUMN operations jsonb NOT NULL DEFAULT '[]'::jsonb;
