WITH c AS MATERIALIZED (SELECT view,execution_released_at,coalesce(execution_input,evidence_input) AS input FROM candidates WHERE project_id=$1 AND id=$2),
f AS (SELECT f FROM c,jsonb_array_elements(input#>'{prepared,facts}') f WHERE f#>>'{summary,id}'=$3),
fields AS MATERIALIZED (SELECT f,ord FROM c,jsonb_array_elements(view->'fields') WITH ORDINALITY a(f,ord) WHERE f->>'endpoint'=$3)
SELECT jsonb_build_object(
    'included',coalesce(view->'endpoints' ? $3,false),
    'note',(SELECT n FROM jsonb_array_elements(view->'notes') n WHERE n->>'endpoint'=$3 LIMIT 1),
    'notes',coalesce((SELECT jsonb_agg(f ORDER BY ord) FROM (SELECT * FROM fields ORDER BY ord LIMIT 100 OFFSET $4) page),'[]'),
    'notes_total',(SELECT count(*) FROM fields),
    'facts',(SELECT (f-'fields') || jsonb_build_object('fields',coalesce((SELECT jsonb_agg(v ORDER BY ord) FROM (SELECT v,ord FROM jsonb_array_elements(f->'fields') WITH ORDINALITY x(v,ord) ORDER BY ord LIMIT 100 OFFSET $4) page),'[]')) FROM f),
    'fields_total',(SELECT jsonb_array_length(f->'fields') FROM f),
    'inputs',coalesce((SELECT jsonb_agg(v ORDER BY ord) FROM (SELECT v,ord FROM jsonb_array_elements(input#>ARRAY['prepared','inputs',$3]) WITH ORDINALITY x(v,ord) ORDER BY ord LIMIT 100 OFFSET $4) page),'[]'),
    'input',CASE WHEN input->>'version'='3' THEN input END,
    'execution_released',execution_released_at IS NOT NULL
) FROM c
