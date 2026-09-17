-- Missing state with retained history is ambiguous: never choose a published version
-- or invent a system directory identity from the latest timestamp.
DO $$
BEGIN
 IF EXISTS (
  SELECT 1 FROM projects p
  WHERE NOT EXISTS (SELECT 1 FROM project_catalogs c WHERE c.project_id=p.id)
  AND (EXISTS (SELECT 1 FROM catalog_versions v WHERE v.project_id=p.id)
    OR EXISTS (SELECT 1 FROM knowledge_releases k WHERE k.project_id=p.id)
    OR EXISTS (SELECT 1 FROM catalog_activation_receipts r WHERE r.project_id=p.id)
    OR EXISTS (SELECT 1 FROM knowledge_activation_receipts r WHERE r.project_id=p.id)
    OR EXISTS (SELECT 1 FROM maintenance_runs m WHERE m.project_id=p.id)
    OR EXISTS (SELECT 1 FROM catalog_preview_tasks t WHERE t.project_id=p.id))
 ) THEN
  RAISE EXCEPTION 'MISSING_PROJECT_CATALOG_WITH_HISTORY: restore authoritative project state before repair';
 END IF;
 INSERT INTO project_catalogs(project_id)
 SELECT id FROM projects
 ON CONFLICT(project_id) DO NOTHING;
END $$;
