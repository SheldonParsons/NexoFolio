SELECT id FROM (
SELECT retention_refs('context',context) AS id FROM field_notes WHERE project_id=$1 AND context#>'{observations,archive_id}' IS NOT NULL
UNION SELECT retention_refs('snapshot',base) FROM candidates WHERE project_id=$1 AND history_pruned_at IS NULL
UNION SELECT retention_refs('snapshot',view) FROM candidates WHERE project_id=$1 AND history_pruned_at IS NULL
UNION SELECT retention_refs('batches',batches) FROM candidates WHERE project_id=$1 AND history_pruned_at IS NULL
UNION SELECT retention_refs('input',execution_input) FROM candidates WHERE project_id=$1
UNION SELECT id FROM candidates WHERE project_id=$1 AND (state NOT IN ('applied','discarded') OR execution_released_at IS NULL OR history_pruned_at IS NULL)
UNION SELECT retention_refs('context',v.before->'context') FROM revisions v
    JOIN rounds r ON r.id=v.round_id WHERE r.project_id=$1 AND r.history_pruned_at IS NULL
) refs WHERE id=ANY($2)
