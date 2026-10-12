-- Owner: access. Effective authorization is separate from synchronized membership.
-- Only the authenticated, enabled user's stored account participates in this policy.
CREATE VIEW project_access_principals AS
SELECT id, instance,
       lower(account) = 'a80646' AS all_projects,
       grants_synced OR lower(account) = 'a80646' AS ready
FROM users
WHERE enabled;

-- Live views also cover newly synchronized projects, without creating member rows.
-- The instance boundary applies to both ordinary grants and the account exception.
CREATE VIEW effective_project_access AS
SELECT u.id AS user_id, p.id AS project_id
FROM project_access_principals u
JOIN projects p ON p.instance = u.instance
WHERE u.ready AND (
    u.all_projects OR EXISTS (
        SELECT 1 FROM user_project_access a
        WHERE a.user_id = u.id AND a.project_id = p.id
    )
);
