-- Owner: access. Existing MCP credentials keep their identity and validity.
ALTER TABLE mcp_tokens ADD COLUMN name text NOT NULL DEFAULT '已有 MCP 凭证';
ALTER TABLE mcp_tokens ADD COLUMN starts_at timestamptz;
UPDATE mcp_tokens SET starts_at = issued_at;
ALTER TABLE mcp_tokens ALTER COLUMN starts_at SET DEFAULT now();
ALTER TABLE mcp_tokens ALTER COLUMN starts_at SET NOT NULL;
ALTER TABLE mcp_tokens ALTER COLUMN expires_at DROP NOT NULL;
ALTER TABLE mcp_tokens ADD CONSTRAINT mcp_tokens_valid_window
    CHECK (expires_at IS NULL OR expires_at > starts_at);
ALTER TABLE mcp_tokens ADD CONSTRAINT mcp_tokens_name_length
    CHECK (char_length(btrim(name)) BETWEEN 1 AND 64);
CREATE INDEX mcp_tokens_owner_created ON mcp_tokens(user_id, issued_at DESC, id DESC);
