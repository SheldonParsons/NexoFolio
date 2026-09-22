-- Older evidence has unknown extraction coverage. Do not invent completeness or rewrite raw observations.
ALTER TABLE capture_events ADD COLUMN evidence_coverage jsonb;
ALTER TABLE evidence_ui_bindings ADD COLUMN evidence_rule_version text NOT NULL DEFAULT 'legacy';
-- Historical facts are not silently promoted to the new provenance/association guarantees.
UPDATE evidence_facts SET data=data||'{"evidence_rule_version":"legacy","needs_reassessment":true}'::jsonb;
