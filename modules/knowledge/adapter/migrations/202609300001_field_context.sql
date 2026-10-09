-- Layered field knowledge and the evidence digest used by incremental curation.
ALTER TABLE field_notes ADD COLUMN context jsonb NOT NULL DEFAULT '{}'::jsonb;
