CREATE TABLE counters (
    id boolean PRIMARY KEY DEFAULT true CHECK (id),
    events bigint NOT NULL DEFAULT 0,
    buckets bigint NOT NULL DEFAULT 0,
    evicted_events bigint NOT NULL DEFAULT 0,
    evicted_buckets bigint NOT NULL DEFAULT 0
);
INSERT INTO counters (id) VALUES (true);
CREATE TABLE events (
    id bigserial PRIMARY KEY,
    at timestamptz NOT NULL,
    kind varchar(64) NOT NULL,
    actor varchar(64) NOT NULL,
    request_id varchar(36) NOT NULL,
    project varchar(36) NOT NULL,
    data jsonb NOT NULL CHECK (octet_length(data::text) <= 2048)
);
CREATE INDEX events_at ON events(at, id);
CREATE INDEX events_request ON events(request_id);
CREATE INDEX events_actor ON events(actor, at DESC);
CREATE INDEX events_project ON events(project, at DESC);
CREATE TABLE buckets (
    key char(64) PRIMARY KEY,
    hour timestamptz NOT NULL,
    kind varchar(64) NOT NULL,
    actor varchar(64) NOT NULL,
    project varchar(36) NOT NULL,
    route varchar(200) NOT NULL,
    method varchar(64) NOT NULL,
    status integer NOT NULL,
    hits bigint NOT NULL,
    duration_ms bigint NOT NULL,
    max_duration_ms bigint NOT NULL
);
CREATE INDEX buckets_hour ON buckets(hour, key);
CREATE INDEX buckets_actor ON buckets(actor, hour DESC);
-- Keep dead tuples reusable under sustained retention/capacity pruning.
ALTER TABLE events SET (autovacuum_vacuum_scale_factor = 0.02, autovacuum_analyze_scale_factor = 0.02);
ALTER TABLE buckets SET (autovacuum_vacuum_scale_factor = 0.02, autovacuum_analyze_scale_factor = 0.02);
