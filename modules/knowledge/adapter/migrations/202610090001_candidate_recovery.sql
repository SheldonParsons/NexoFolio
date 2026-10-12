-- Recovery scans only unfinished workers, without reading their large inputs.
CREATE INDEX candidates_running_recovery ON candidates (id, updated_at)
WHERE state = 'running';
