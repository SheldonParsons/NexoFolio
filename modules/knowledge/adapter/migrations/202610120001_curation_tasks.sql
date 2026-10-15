CREATE TABLE curation_tasks (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL,
    target uuid,
    parent uuid,
    candidate_id uuid,
    actor jsonb NOT NULL,
    state text NOT NULL CHECK (state IN ('queued','running','done','failed','stopped')),
    generation bigint NOT NULL DEFAULT 0,
    run_id uuid,
    lease_until timestamptz,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    last_started_at timestamptz,
    message text
);
CREATE UNIQUE INDEX curation_task_scope ON curation_tasks(project_id, target) NULLS NOT DISTINCT WHERE state IN ('queued','running');
CREATE UNIQUE INDEX curation_task_candidate ON curation_tasks(candidate_id) WHERE candidate_id IS NOT NULL;
CREATE UNIQUE INDEX curation_task_project_running ON curation_tasks(project_id) WHERE state='running';
CREATE INDEX curation_task_queue ON curation_tasks(created_at,id) WHERE state IN ('queued','running');
CREATE INDEX curation_task_turn ON curation_tasks(project_id,last_started_at DESC NULLS LAST);
-- Adopt the single active legacy candidate per project. Older expired duplicates
-- remain readable, but cannot become a second executor after upgrade.
WITH ranked AS (
 SELECT id, row_number() OVER (PARTITION BY project_id ORDER BY updated_at DESC,id) AS n
 FROM candidates WHERE state='running'
)
UPDATE candidates SET state='interrupted',progress=coalesce(progress,'{}'::jsonb)||jsonb_build_object('status','failed','active','[]'::jsonb,'message','升级时发现同项目旧执行记录，已保留批次，请手动续跑') WHERE id IN (SELECT id FROM ranked WHERE n>1);
INSERT INTO curation_tasks(id,project_id,target,candidate_id,actor,state,generation,run_id,lease_until,created_at,updated_at,last_started_at)
SELECT id,project_id,target,id,
 coalesce((SELECT op->'actor' FROM jsonb_array_elements(operations) op WHERE op->'actor'->>'kind'='user' ORDER BY op->>'at' LIMIT 1), '{"kind":"unknown"}'::jsonb),
 'running',1,CASE WHEN progress->>'run_id' ~ '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$' THEN (progress->>'run_id')::uuid END,updated_at+interval '120 seconds',created_at,updated_at,created_at
FROM candidates WHERE state='running';
