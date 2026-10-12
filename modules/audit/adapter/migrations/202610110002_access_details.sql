-- Historical hour buckets remain unchanged: their missing exact timestamps cannot be recovered.
CREATE TABLE access_events (
    id bigserial PRIMARY KEY,
    at timestamptz NOT NULL,
    kind varchar(64) NOT NULL,
    actor varchar(64) NOT NULL,
    project varchar(36) NOT NULL,
    resource varchar(36) NOT NULL,
    route varchar(200) NOT NULL,
    action varchar(64) NOT NULL,
    method varchar(64) NOT NULL,
    background boolean NOT NULL,
    data jsonb NOT NULL CHECK(octet_length(data::text)<=2048)
);
CREATE INDEX access_events_time ON access_events(at DESC,id DESC);
CREATE INDEX access_events_scope ON access_events(kind,project,resource,at DESC);
ALTER TABLE access_events SET (autovacuum_vacuum_scale_factor=0.02,autovacuum_analyze_scale_factor=0.02);
ALTER TABLE counters ADD COLUMN access_events bigint NOT NULL DEFAULT 0;
ALTER TABLE counters ADD COLUMN evicted_access_events bigint NOT NULL DEFAULT 0;
ALTER TABLE counters ADD COLUMN access_details_since timestamptz NOT NULL DEFAULT now();
