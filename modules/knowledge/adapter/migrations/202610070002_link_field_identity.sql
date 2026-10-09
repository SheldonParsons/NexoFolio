-- Owner: knowledge. Keep legacy unknown locations distinct from qualified fields.
-- JSON locations include response status, so 200 and 400 never share an identity.
DROP INDEX links_identity;
CREATE UNIQUE INDEX links_identity ON links
 (from_endpoint, to_endpoint, relation, coalesce(from_field, ''), coalesce(to_field, ''),
  coalesce(context->>'environment', ''), coalesce(context->'from_location', 'null'::jsonb),
  coalesce(context->'to_location', 'null'::jsonb));
