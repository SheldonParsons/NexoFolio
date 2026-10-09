-- Owner: knowledge. Folder identity, manual protection and rollback share provenance.
ALTER TABLE folders ADD COLUMN author jsonb;

-- Legacy history has no write timestamp, so do not guess a latest manual
-- writer from round start time. Only all-curation histories are safe to mark
-- generated. Any manual/unknown history remains NULL and is protected.
WITH generated AS (
 SELECT DISTINCT ON (r.target->>'id') r.target->>'id' AS folder_id, q.author
 FROM revisions r JOIN rounds q ON q.id = r.round_id
 WHERE r.target->>'of' = 'folder' AND q.state <> 'rolled_back'
   AND q.author->>'by' = 'curate'
 ORDER BY r.target->>'id', q.started_at DESC, r.seq DESC
)
UPDATE folders f SET author = g.author FROM generated g
WHERE f.id::text = g.folder_id AND NOT EXISTS (
 SELECT 1 FROM revisions r JOIN rounds q ON q.id = r.round_id
 WHERE r.target->>'of' = 'folder' AND r.target->>'id' = f.id::text
   AND q.state <> 'rolled_back' AND q.author->>'by' IS DISTINCT FROM 'curate'
);
