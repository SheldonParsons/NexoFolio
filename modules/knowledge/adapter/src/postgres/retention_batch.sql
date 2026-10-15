SELECT id,state,updated_at,execution_released_at,history_pruned_at,applied_rounds,
    coalesce(history_pruned_at IS NOT NULL OR (jsonb_typeof(base->'fields')='array'
        AND jsonb_typeof(view->'fields')='array' AND jsonb_typeof(batches)='array'),false) AS history_shape_valid,
    coalesce((execution_input->>'version'='3' AND octet_length(execution_input::text)>4194304)
        OR octet_length(evidence_input::text)>4194304,false) AS header_oversized,
    CASE WHEN execution_input->>'version'='3' THEN
        CASE WHEN octet_length(execution_input::text)<=4194304 THEN execution_input END
    WHEN execution_input IS NOT NULL THEN jsonb_build_object('version',execution_input->'version') END AS execution_input,
    CASE WHEN octet_length(evidence_input::text)<=4194304 THEN evidence_input END AS evidence_input
FROM candidates WHERE project_id=$1 AND ($2::uuid IS NULL OR id>$2) ORDER BY id LIMIT $3
