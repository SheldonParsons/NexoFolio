-- Owner: access. MCP credentials are separate from web/business-system sessions.
CREATE TABLE mcp_tokens (
 id uuid PRIMARY KEY,
 user_id uuid NOT NULL REFERENCES users(id),
 token_hash bytea NOT NULL UNIQUE,
 issued_at timestamptz NOT NULL DEFAULT now(),
 expires_at timestamptz NOT NULL,
 revoked_at timestamptz,
 CHECK (expires_at > issued_at)
);
CREATE INDEX mcp_tokens_owner ON mcp_tokens(user_id);
