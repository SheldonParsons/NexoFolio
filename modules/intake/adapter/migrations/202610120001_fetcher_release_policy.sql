-- Security high-water mark, not capture data. Recording resets must not delete it.
CREATE TABLE fetcher_release_policy (
    id integer PRIMARY KEY CHECK (id = 1),
    revision bigint NOT NULL,
    minimum integer[] NOT NULL,
    latest integer[] NOT NULL,
    fingerprint text NOT NULL
);
