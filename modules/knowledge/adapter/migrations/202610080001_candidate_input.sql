ALTER TABLE candidates ADD COLUMN execution_input jsonb;
CREATE INDEX candidates_running ON candidates(project_id) WHERE state='running';
