-- Review identity changes with content, never with a heartbeat/checkpoint.
ALTER TABLE candidates ADD COLUMN review_version bigint NOT NULL DEFAULT 0;
CREATE FUNCTION candidate_review_version() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.base IS DISTINCT FROM OLD.base OR NEW.view IS DISTINCT FROM OLD.view
       OR NEW.execution_input IS DISTINCT FROM OLD.execution_input
       OR NEW.evidence_input IS DISTINCT FROM OLD.evidence_input
       OR NEW.history_pruned_at IS DISTINCT FROM OLD.history_pruned_at THEN
        NEW.review_version := OLD.review_version + 1;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER candidate_review_version BEFORE UPDATE OF base,view,execution_input,evidence_input,history_pruned_at
ON candidates FOR EACH ROW EXECUTE FUNCTION candidate_review_version();

-- The review's existing semantic comparison: recursively ignore only these
-- provenance keys, preserve array order, and ignore a link's generated row ID.
CREATE FUNCTION review_semantic(doc jsonb) RETURNS jsonb LANGUAGE plpgsql IMMUTABLE STRICT AS $$
BEGIN
    IF jsonb_typeof(doc) = 'object' THEN
        RETURN coalesce((SELECT jsonb_object_agg(key,review_semantic(value)) FROM jsonb_each(doc)
            WHERE key NOT IN ('at','input_hash','recipe')), '{}'::jsonb);
    ELSIF jsonb_typeof(doc) = 'array' THEN
        RETURN coalesce((SELECT jsonb_agg(review_semantic(value) ORDER BY ord)
            FROM jsonb_array_elements(doc) WITH ORDINALITY a(value,ord)), '[]'::jsonb);
    END IF;
    RETURN doc;
END $$;
CREATE FUNCTION review_key(section text, item jsonb) RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE section WHEN 'links' THEN jsonb_build_array(item->'from',item->'from_field',
        item#>'{context,from_location}',item->'to',item->'to_field',item#>'{context,to_location}',
        item->'relation',item#>'{context,environment}')::text
    WHEN 'fields' THEN (item->>'endpoint')||':'||(item->>'path')
    WHEN 'folders' THEN item->>'id' ELSE item->>'endpoint' END
$$;
-- Fill serde defaults before comparing legacy rows to recently saved rows.
-- Defaults are supplied by the Rust contract, not a separate SQL schema.
CREATE FUNCTION review_fill(defaults jsonb, doc jsonb) RETURNS jsonb LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
    IF jsonb_typeof(defaults)='object' AND jsonb_typeof(doc)='object' THEN
        RETURN (SELECT jsonb_object_agg(key,CASE WHEN doc ? key THEN review_fill(defaults->key,doc->key) ELSE CASE jsonb_typeof(defaults->key) WHEN 'array' THEN '[]'::jsonb WHEN 'object' THEN review_fill(defaults->key,'{}') ELSE defaults->key END END)
            FROM jsonb_object_keys(defaults || doc) key);
    ELSIF jsonb_typeof(defaults)='array' AND jsonb_array_length(defaults)>0 AND jsonb_typeof(doc)='array' THEN
        RETURN coalesce((SELECT jsonb_agg(review_fill(defaults->0,v) ORDER BY ord)
            FROM jsonb_array_elements(doc) WITH ORDINALITY x(v,ord)),'[]');
    END IF;
    RETURN doc;
END $$;
CREATE FUNCTION review_diff(a jsonb,b jsonb,section text,defaults jsonb)
RETURNS TABLE(key text,before jsonb,after jsonb,ordinal bigint) LANGUAGE sql IMMUTABLE AS $$
    WITH old AS (SELECT DISTINCT ON (review_key(section,v)) review_key(section,v) k,v,ord
        FROM jsonb_array_elements(coalesce(a->section,'[]')) WITH ORDINALITY x(v,ord)
        ORDER BY review_key(section,v),ord DESC),
    new AS (SELECT DISTINCT ON (review_key(section,v)) review_key(section,v) k,v,ord
        FROM jsonb_array_elements(coalesce(b->section,'[]')) WITH ORDINALITY x(v,ord)
        ORDER BY review_key(section,v),ord DESC)
    SELECT coalesce(old.k,new.k),old.v,new.v,coalesce(old.ord,1000000000+new.ord)
    FROM old FULL JOIN new USING(k)
    WHERE review_semantic(review_fill(defaults,CASE WHEN section='links' THEN old.v-'id' ELSE old.v END))
        IS DISTINCT FROM review_semantic(review_fill(defaults,CASE WHEN section='links' THEN new.v-'id' ELSE new.v END))
$$;
