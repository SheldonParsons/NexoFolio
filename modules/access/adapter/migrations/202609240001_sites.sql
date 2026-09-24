-- Owner: access. Site scopes (origin + path prefix) are where users browse while collecting.
-- The registry every client shares: one scope binds to exactly one environment.
CREATE TABLE sites (
 origin text NOT NULL,
 prefix text NOT NULL,
 project_id uuid NOT NULL,
 environment_id uuid NOT NULL,
 bound_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(origin,prefix),
 FOREIGN KEY(project_id,environment_id) REFERENCES environments(project_id,id)
);
-- Where an environment's data was collected. An annotation only; never part of endpoint identity.
CREATE TABLE environment_sites (
 environment_id uuid NOT NULL REFERENCES environments(id),
 origin text NOT NULL,
 prefix text NOT NULL,
 first_seen_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(environment_id,origin,prefix)
);
