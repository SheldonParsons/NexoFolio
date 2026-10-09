-- Owner: access. Local presentation overrides never change identity or permissions.
CREATE TABLE profile_preferences (
 user_id uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 display_name text,
 bio text NOT NULL DEFAULT '',
 avatar_url text,
 version bigint NOT NULL DEFAULT 0 CHECK (version >= 0),
 updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 CHECK (display_name IS NULL OR char_length(display_name) BETWEEN 1 AND 40),
 CHECK (char_length(bio) <= 160)
);
