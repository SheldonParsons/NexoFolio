-- Owner: knowledge. The catalogue tree, where endpoints sit, the words written
-- about them, the links between them, and the rounds that wrote it all.
--
-- Observe owns what traffic proves; this owns what people and curation say.
-- Endpoint ids come from observe and are not foreign keys: modules do not
-- share tables, and an endpoint observe forgets simply stops being read here.

-- One run of curation, or one sitting of manual edits. Every write belongs to a
-- round, so undo has a single shape no matter who wrote it.
CREATE TABLE rounds (
 id uuid PRIMARY KEY,
 project_id uuid NOT NULL,
 author jsonb NOT NULL,
 started_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 finished_at timestamptz,
 state text NOT NULL,
 summary text,
 CHECK (state IN ('running', 'done', 'failed', 'rolled_back')),
 CHECK ((state = 'running') = (finished_at IS NULL))
);
CREATE INDEX rounds_project ON rounds(project_id, started_at DESC);

-- Depth is capped at 3 by the core, which knows the whole path; the database
-- only keeps parents honest.
CREATE TABLE folders (
 id uuid PRIMARY KEY,
 project_id uuid NOT NULL,
 parent_id uuid REFERENCES folders(id) ON DELETE CASCADE,
 name text NOT NULL,
 summary text,
 position integer NOT NULL DEFAULT 0,
 CHECK (id <> parent_id)
);
CREATE INDEX folders_project ON folders(project_id, parent_id, position);
CREATE UNIQUE INDEX folders_sibling_name ON folders(project_id, parent_id, name) WHERE parent_id IS NOT NULL;
CREATE UNIQUE INDEX folders_top_level_name ON folders(project_id, name) WHERE parent_id IS NULL;

-- One row per endpoint at most: the tree is a real tree. An endpoint with no
-- row here is unplaced, which is how it shows up under 待分类 without anyone
-- writing anything.
CREATE TABLE placements (
 endpoint_id uuid PRIMARY KEY,
 project_id uuid NOT NULL,
 folder_id uuid NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
 author jsonb NOT NULL,
 at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX placements_folder ON placements(folder_id);
CREATE INDEX placements_project ON placements(project_id);

-- name and purpose are written separately: the describe pass may fill one
-- without clearing what someone edited by hand.
CREATE TABLE endpoint_notes (
 endpoint_id uuid PRIMARY KEY,
 project_id uuid NOT NULL,
 name text,
 purpose text,
 author jsonb NOT NULL,
 at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX endpoint_notes_project ON endpoint_notes(project_id);

-- path is the field path as observe reports it, so a note survives as long as
-- the field keeps its place.
CREATE TABLE field_notes (
 endpoint_id uuid NOT NULL,
 path text NOT NULL,
 project_id uuid NOT NULL,
 text text NOT NULL,
 values_seen jsonb NOT NULL DEFAULT '[]'::jsonb,
 author jsonb NOT NULL,
 at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY (endpoint_id, path)
);
CREATE INDEX field_notes_project ON field_notes(project_id);

-- Found by matching real values across samples, so every link carries the
-- evidence that produced it. Identity is the two ends plus the relation:
-- rediscovering a link refreshes its evidence instead of adding a duplicate.
CREATE TABLE links (
 id uuid PRIMARY KEY,
 project_id uuid NOT NULL,
 from_endpoint uuid NOT NULL,
 from_field text,
 to_endpoint uuid NOT NULL,
 to_field text,
 relation text NOT NULL,
 evidence text NOT NULL,
 author jsonb NOT NULL,
 at timestamptz NOT NULL DEFAULT clock_timestamp(),
 CHECK (relation IN ('dictionary', 'list_to_detail', 'precondition', 'enum_subset'))
);
CREATE UNIQUE INDEX links_identity ON links
 (from_endpoint, to_endpoint, relation, coalesce(from_field, ''), coalesce(to_field, ''));
CREATE INDEX links_from ON links(from_endpoint);
CREATE INDEX links_to ON links(to_endpoint);
CREATE INDEX links_project ON links(project_id);

-- What one write replaced: the previous value only, never a snapshot of the
-- project. A round over 47 endpoints costs tens of kilobytes, so this needs no
-- retention policy of its own.
--
-- before IS NULL means the write created something that did not exist, so
-- rolling back removes it.
CREATE TABLE revisions (
 round_id uuid NOT NULL REFERENCES rounds(id) ON DELETE CASCADE,
 seq bigint NOT NULL,
 target jsonb NOT NULL,
 before jsonb,
 PRIMARY KEY (round_id, seq)
);
