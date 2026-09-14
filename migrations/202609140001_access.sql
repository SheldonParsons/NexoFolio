-- Owner: access. No knowledge, directory or MCP-token business tables here.
CREATE SEQUENCE access_sync_generation;
CREATE TABLE users (
 id uuid PRIMARY KEY,
 instance text NOT NULL,
 external_id text NOT NULL,
 account text NOT NULL,
 display_name text NOT NULL,
 enabled boolean NOT NULL DEFAULT true,
 grants_synced boolean NOT NULL DEFAULT false,
 last_sync_generation bigint NOT NULL DEFAULT 0,
 created_at timestamptz NOT NULL DEFAULT now(),
 last_login_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(instance, external_id)
);
CREATE UNIQUE INDEX users_canonical_account ON users(instance, lower(account));
CREATE TABLE internal_sessions (
 user_id uuid PRIMARY KEY REFERENCES users(id),
 token_hash bytea NOT NULL UNIQUE,
 token_encrypted bytea NOT NULL,
 issued_at timestamptz NOT NULL DEFAULT now(),
 expires_at timestamptz NOT NULL,
 revoked_at timestamptz,
 CHECK(expires_at > issued_at)
);
CREATE TABLE projects (
 id uuid PRIMARY KEY,
 instance text NOT NULL,
 external_id text NOT NULL,
 name text NOT NULL,
 status text NOT NULL,
 last_sync_generation bigint NOT NULL,
 last_synced_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(instance, external_id)
);
CREATE TABLE user_project_access (
 user_id uuid NOT NULL REFERENCES users(id),
 project_id uuid NOT NULL REFERENCES projects(id),
 PRIMARY KEY(user_id, project_id)
);
CREATE TABLE login_audit (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 user_id uuid NOT NULL REFERENCES users(id),
 method text NOT NULL CHECK(method IN ('zentao', 'emergency')),
 created_at timestamptz NOT NULL DEFAULT now()
);
