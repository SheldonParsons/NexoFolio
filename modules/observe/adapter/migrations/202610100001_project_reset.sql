-- Owner: observe. Reject retained pre-reset observations when an old client buffer retries.
CREATE TABLE project_resets (
 project_id uuid PRIMARY KEY,
 cleared_at timestamptz NOT NULL
);
