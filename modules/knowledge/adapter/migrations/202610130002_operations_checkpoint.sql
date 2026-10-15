-- A bounded observation summary, not a candidate/body/snapshot projection.
-- No backfill: pre-existing progress remains unknown until its next normal write.
ALTER TABLE candidates ADD COLUMN operations_checkpoint jsonb;

CREATE FUNCTION operations_checkpoint_summary(p jsonb) RETURNS jsonb
LANGUAGE sql IMMUTABLE PARALLEL SAFE
SET search_path = pg_catalog
AS $$
    SELECT jsonb_object_agg(k, CASE
        WHEN k = 'stage' AND p->>k IN ('describe','fields','organize') THEN p->k
        WHEN k = 'run_id' AND jsonb_typeof(p->k) = 'string'
            AND p->>k ~ '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$'
            THEN p->k
        WHEN k = 'updated_at' AND jsonb_typeof(p->k) = 'string'
            AND length(p->>k) <= 40
            AND p->>k ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}([.][0-9]{1,9})?(Z|[+-][0-9]{2}:[0-9]{2})$' THEN p->k
        WHEN k IN ('completed','total','completed_batches')
            AND jsonb_typeof(p->k) = 'number'
            AND p->>k ~ '^[0-9]{1,20}$' THEN p->k
        ELSE 'null'::jsonb END)
    FROM unnest(ARRAY['run_id','stage','updated_at','completed','total','completed_batches']) AS keys(k)
$$;

CREATE FUNCTION capture_operations_checkpoint() RETURNS trigger
LANGUAGE plpgsql SET search_path = knowledge, pg_catalog AS $$
BEGIN
    NEW.operations_checkpoint := knowledge.operations_checkpoint_summary(NEW.progress);
    RETURN NEW;
END;
$$;

CREATE TRIGGER candidate_operations_checkpoint
BEFORE INSERT OR UPDATE OF progress, operations_checkpoint ON candidates
FOR EACH ROW EXECUTE FUNCTION capture_operations_checkpoint();

-- The function only emits fixed keys, short strings and bounded integer literals.
-- Prevent a future change from silently reintroducing a TOAST-sized polling payload.
ALTER TABLE candidates ADD CONSTRAINT operations_checkpoint_bounded
CHECK (operations_checkpoint IS NULL OR
       (jsonb_typeof(operations_checkpoint) = 'object' AND
        pg_column_size(operations_checkpoint) <= 1024));
