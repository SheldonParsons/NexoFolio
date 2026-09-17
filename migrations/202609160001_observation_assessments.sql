-- Append diagnostics; preserve original observations, receipt hashes and current definitions.
ALTER TABLE interface_observations ADD COLUMN assessment jsonb;
ALTER TABLE interface_observations ADD COLUMN extractor_version text;
COMMENT ON COLUMN interface_observations.assessment IS 'Immutable classification at processing time; NULL means historical/unassessed, never duplicate';
