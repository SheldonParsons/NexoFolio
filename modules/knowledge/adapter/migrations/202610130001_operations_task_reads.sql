-- Private read pagination only. Existing scheduling indexes and state remain unchanged.
CREATE INDEX curation_task_recent ON curation_tasks(created_at DESC,id DESC);
CREATE INDEX curation_task_state_recent ON curation_tasks(state,created_at DESC,id DESC);
CREATE INDEX curation_task_project_recent ON curation_tasks(project_id,created_at DESC,id DESC);
CREATE INDEX curation_task_project_state_recent ON curation_tasks(project_id,state,created_at DESC,id DESC);
