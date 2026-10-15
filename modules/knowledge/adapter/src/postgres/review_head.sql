WITH c AS MATERIALIZED (SELECT base,view,base_hash,reviewed_by,batches,coalesce(execution_input,evidence_input) AS input FROM candidates WHERE project_id=$1 AND id=$2)
SELECT jsonb_build_object(
    'base',jsonb_build_object('lifecycle',coalesce(base->'lifecycle','{}'),'endpoints',base->'endpoints','folders',base->'folders','placements',base->'placements','notes','[]'::jsonb,'fields','[]'::jsonb,'links','[]'::jsonb),
    'view',jsonb_build_object('lifecycle',coalesce(view->'lifecycle','{}'),'endpoints',view->'endpoints','folders',view->'folders','placements',view->'placements',
        'notes',coalesce((SELECT jsonb_agg(jsonb_build_object('endpoint',n->'endpoint','name',n->'name','purpose',left(n->>'purpose',160))) FROM jsonb_array_elements(view->'notes') n),'[]'),
        'fields','[]'::jsonb,'links','[]'::jsonb),
    'counts',jsonb_build_object('links',jsonb_array_length(view->'links'),'fields',jsonb_array_length(view->'fields')),
    'changes',(SELECT jsonb_object_agg(section,total) FROM (
        SELECT section,(SELECT count(*) FROM review_diff(base,view,section,$3->section)) total
        FROM unnest(ARRAY['folders','placements','notes','fields','links']) section) counts),
    'summaries',CASE WHEN input->>'version'='3' THEN input->'summaries'
        ELSE coalesce((SELECT jsonb_agg(f->'summary') FROM jsonb_array_elements(input#>'{prepared,facts}') f),'[]') END,
    'commands',(SELECT coalesce(sum(jsonb_array_length(b->'commands')),0) FROM jsonb_array_elements(batches) b),
    'base_hash',base_hash,'batches','[]'::jsonb,'reviewed_by',reviewed_by,'operations','[]'::jsonb
) FROM c
