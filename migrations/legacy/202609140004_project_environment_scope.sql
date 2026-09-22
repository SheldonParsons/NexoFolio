-- v2: capture scope maps to project/environment; no service partition.
-- Preserve queued v1 hashes/receipts and raw observations. Consolidate only current heads.
WITH ranked AS (
 SELECT h.ctid AS row_id,row_number() OVER (
  PARTITION BY h.project_id,h.environment_id,h.identity_key
  ORDER BY i.received_at DESC,i.id DESC
 ) AS rank
 FROM ingestion_heads h JOIN ingestion_inbox i ON i.id=h.ingestion_id
)
DELETE FROM ingestion_heads h USING ranked r WHERE h.ctid=r.row_id AND r.rank>1;
ALTER TABLE ingestion_heads DROP CONSTRAINT ingestion_heads_pkey;
ALTER TABLE ingestion_heads DROP COLUMN service_key;
ALTER TABLE ingestion_heads ADD PRIMARY KEY(project_id,environment_id,identity_key);
ALTER TABLE ingestion_inbox RENAME COLUMN service_key TO legacy_service_key;
ALTER TABLE ingestion_inbox ALTER COLUMN legacy_service_key DROP NOT NULL;
COMMENT ON COLUMN ingestion_inbox.legacy_service_key IS 'v1 provenance only; not part of the dedup scope';
