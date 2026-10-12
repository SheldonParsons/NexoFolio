-- Historical folders keep their original names and IDs. Name uniqueness is
-- enforced for the effective tree by knowledge core under the existing project
-- advisory lock (writes, rollback, candidate application and lifecycle undo).
-- The old indexes incorrectly included retired historical rows.
DROP INDEX folders_sibling_name;
DROP INDEX folders_top_level_name;
