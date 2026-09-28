-- Owner: observe. Endpoints, their fingerprints and declarations, address verdicts and the change outbox.

-- Delivered batches, kept 7 days so a redelivery counts once.
CREATE TABLE seen_batches (
 batch_id uuid PRIMARY KEY,
 seen_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX seen_batches_seen_at ON seen_batches(seen_at);

-- Per project and service address: the verdict and how much it is called.
CREATE TABLE service_addresses (
 project_id uuid NOT NULL,
 address text NOT NULL,
 manual_verdict text,
 auto_verdict text,
 auto_reason text,
 calls bigint NOT NULL DEFAULT 0,
 last_seen timestamptz,
 PRIMARY KEY (project_id, address),
 CHECK ((auto_verdict IS NULL) = (auto_reason IS NULL))
);
CREATE INDEX service_addresses_address ON service_addresses(address) WHERE calls > 0;

-- The leading path an own address serves every endpoint under.
CREATE TABLE base_paths (
 project_id uuid NOT NULL,
 environment_id uuid NOT NULL,
 address text NOT NULL,
 base_path text NOT NULL,
 PRIMARY KEY (project_id, environment_id, address)
);

-- merged_into set: an alias kept so old IDs still resolve.
CREATE TABLE endpoints (
 id uuid PRIMARY KEY,
 project_id uuid NOT NULL,
 method text NOT NULL,
 path_template text NOT NULL,
 merged_into uuid REFERENCES endpoints(id),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE UNIQUE INDEX endpoints_identity ON endpoints(project_id, method, path_template) WHERE merged_into IS NULL;
CREATE INDEX endpoints_merged_into ON endpoints(merged_into) WHERE merged_into IS NOT NULL;

-- One row per distinct structure; sample is the first observation with it, whole.
CREATE TABLE fingerprints (
 endpoint_id uuid NOT NULL REFERENCES endpoints(id),
 environment_id uuid NOT NULL,
 address text NOT NULL,
 hash bytea NOT NULL,
 structure jsonb NOT NULL,
 sample jsonb NOT NULL,
 calls bigint NOT NULL,
 first_seen timestamptz NOT NULL,
 last_seen timestamptz NOT NULL,
 PRIMARY KEY (endpoint_id, environment_id, address, hash)
);

-- environment_id NULL: applies to every environment.
CREATE TABLE declarations (
 endpoint_id uuid NOT NULL REFERENCES endpoints(id),
 environment_id uuid,
 platform text NOT NULL,
 source_url text,
 structure jsonb NOT NULL,
 updated_at timestamptz NOT NULL,
 UNIQUE NULLS NOT DISTINCT (endpoint_id, environment_id, platform, source_url)
);

-- The endpoint change feed.
CREATE TABLE outbox (
 seq bigserial PRIMARY KEY,
 event jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
