-- Owner: intake. Processed collect batches, kept 7 days so a retry gets the first receipt back.
CREATE TABLE batches (
 batch_id uuid PRIMARY KEY,
 content_hash bytea NOT NULL,
 platform text NOT NULL,
 receipt jsonb NOT NULL,
 recorded_at timestamptz NOT NULL
);
CREATE INDEX batches_recorded_at ON batches(recorded_at);
