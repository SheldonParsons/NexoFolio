-- Owner: knowledge. Old rows/history decode to empty contexts; all writes use the existing transaction.
ALTER TABLE endpoint_notes ADD COLUMN context jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE links ADD COLUMN context jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE links DROP CONSTRAINT links_relation_check;
ALTER TABLE links ADD CONSTRAINT links_relation_check CHECK (relation IN (
 'dictionary', 'list_to_detail', 'precondition', 'enum_subset',
 'parameter_source', 'entity_reference', 'authentication_use',
 'confirmed_prerequisite', 'conditional_prerequisite', 'recommended_check'
));

-- Distinct environments must not overwrite one another's directed evidence.
DROP INDEX links_identity;
CREATE UNIQUE INDEX links_identity ON links
 (from_endpoint, to_endpoint, relation, coalesce(from_field, ''), coalesce(to_field, ''),
  coalesce(context->>'environment', ''));
