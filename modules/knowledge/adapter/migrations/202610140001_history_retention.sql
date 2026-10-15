-- Explicit maintenance only. Migration never starts a cleanup or backfills user rows.
CREATE TABLE retention_projects (
    project_id uuid PRIMARY KEY,
    enabled_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
ALTER TABLE candidates ADD COLUMN history_pruned_at timestamptz;
ALTER TABLE candidates ADD COLUMN execution_released_at timestamptz;
ALTER TABLE candidates ADD COLUMN evidence_input jsonb;
ALTER TABLE candidates ADD COLUMN archive_unreferenced_since timestamptz;
ALTER TABLE candidates ADD COLUMN archive_reclaimed_at timestamptz;
ALTER TABLE rounds ADD COLUMN history_pruned_at timestamptz;
CREATE INDEX candidates_archive_grace ON candidates(project_id)
    WHERE archive_unreferenced_since IS NOT NULL;

-- These paths mirror FieldContext, KnowledgeSnapshot, CandidateBatch/Command,
-- and FrozenInput v2. Never match prose, example values, or source documents.
CREATE FUNCTION retention_refs(kind text, doc jsonb) RETURNS SETOF uuid
LANGUAGE sql IMMUTABLE AS $$
    SELECT DISTINCT (v #>> '{}')::uuid FROM (
        SELECT jsonb_path_query(doc, '$.observations.archive_id') v WHERE kind='context'
        UNION ALL
        SELECT jsonb_path_query(doc, '$.fields[*].context.observations.archive_id') WHERE kind='snapshot'
        UNION ALL
        SELECT jsonb_path_query(doc, '$[*].commands[*].context.observations.archive_id') WHERE kind='batches'
        UNION ALL
        SELECT jsonb_path_query(doc, '$.prepared.inputs.*[*].observations.archive_id') WHERE kind='input'
        UNION ALL
        SELECT jsonb_path_query(doc, '$[*].observations.archive_id') WHERE kind='field_inputs'
    ) refs WHERE v <> 'null'::jsonb
$$;

-- Same project lock as application, candidate application and recovery. A new
-- reference both rejects tombstones and cancels an earlier unreferenced grace.
-- Maintenance rechecks under this lock; no saved preview authorizes deletion.
CREATE FUNCTION retention_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=knowledge,pg_temp AS $$
DECLARE p uuid; refs uuid[]; own uuid;
BEGIN
    IF TG_TABLE_NAME='revisions' THEN
        SELECT project_id INTO STRICT p FROM rounds WHERE id=NEW.round_id;
    ELSE p := NEW.project_id;
    END IF;
    -- Disabled projects do not scan candidate JSON on their ordinary write path.
    IF NOT EXISTS (SELECT 1 FROM retention_projects WHERE project_id=p) THEN
        RETURN NEW;
    END IF;
    PERFORM pg_advisory_xact_lock(1802379265,hashtext(p::text));
    IF TG_TABLE_NAME='revisions' THEN
        SELECT array_agg(r) INTO refs FROM retention_refs('context',NEW.before->'context') r;
    ELSE
        p := NEW.project_id;
        IF TG_TABLE_NAME='field_notes' THEN
            SELECT array_agg(r) INTO refs FROM retention_refs('context',NEW.context) r;
        ELSIF TG_TABLE_NAME='candidates' THEN
            own := NEW.id;
            NEW.archive_unreferenced_since := NULL;
            IF TG_OP='UPDATE' AND OLD.execution_released_at IS NOT NULL
                AND NEW.state NOT IN ('applied','discarded') THEN
                RAISE EXCEPTION 'released candidate cannot resume';
            END IF;
            SELECT array_agg(r) INTO refs FROM (
                SELECT retention_refs('snapshot',NEW.base) r
                UNION SELECT retention_refs('snapshot',NEW.view)
                UNION SELECT retention_refs('batches',NEW.batches)
                UNION SELECT retention_refs('input',NEW.execution_input)
            ) s;
        ELSIF TG_TABLE_NAME='curation_tasks' THEN
            own := NEW.candidate_id;
        END IF;
    END IF;
    IF EXISTS (SELECT 1 FROM candidates WHERE project_id=p
               AND id=ANY(refs) AND archive_reclaimed_at IS NOT NULL) THEN
        RAISE EXCEPTION 'archive reference has expired';
    END IF;
    UPDATE candidates SET archive_unreferenced_since=NULL
        WHERE project_id=p AND archive_unreferenced_since IS NOT NULL
          AND (id=ANY(refs) OR id=own);
    RETURN NEW;
END $$;
CREATE TRIGGER retention_field_write BEFORE INSERT OR UPDATE OF context ON field_notes
    FOR EACH ROW EXECUTE FUNCTION retention_guard();
CREATE TRIGGER retention_candidate_write BEFORE INSERT OR UPDATE OF state,base,view,batches,execution_input ON candidates
    FOR EACH ROW EXECUTE FUNCTION retention_guard();
CREATE TRIGGER retention_revision_write BEFORE INSERT OR UPDATE OF before ON revisions
    FOR EACH ROW EXECUTE FUNCTION retention_guard();
CREATE TRIGGER retention_task_write BEFORE INSERT OR UPDATE OF state,candidate_id ON curation_tasks
    FOR EACH ROW EXECUTE FUNCTION retention_guard();
