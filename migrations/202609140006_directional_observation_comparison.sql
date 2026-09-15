-- Current comparison projection only. Original observations, receipts and definitions unchanged.
ALTER TABLE ingestion_heads ADD COLUMN structural_projection jsonb;
COMMENT ON COLUMN ingestion_heads.structural_projection IS 'Directional schema comparison; empty-array observations never replace a richer current head';
