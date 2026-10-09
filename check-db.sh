#!/bin/bash

# 检查用户的 grants_synced 状态
echo "=== 检查用户同步状态 ==="
docker exec nexofolio-local-db-1 psql -U nexofolio -d nexofolio -c "
SELECT id, account, display_name, grants_synced, enabled
FROM users
WHERE account = 'A80646';"

echo ""
echo "=== 检查项目列表 ==="
docker exec nexofolio-local-db-1 psql -U nexofolio -d nexofolio -c "
SELECT id, name, status
FROM projects
ORDER BY name
LIMIT 10;"

echo ""
echo "=== 检查项目访问权限 ==="
docker exec nexofolio-local-db-1 psql -U nexofolio -d nexofolio -c "
SELECT u.account, p.name as project_name
FROM user_project_access a
JOIN users u ON u.id = a.user_id
JOIN projects p ON p.id = a.project_id
WHERE u.account = 'A80646'
ORDER BY p.name;"

echo ""
echo "=== 检查访问权限数量 ==="
docker exec nexofolio-local-db-1 psql -U nexofolio -d nexofolio -c "
SELECT
  (SELECT COUNT(*) FROM user_project_access WHERE user_id = (SELECT id FROM users WHERE account = 'A80646')) as user_grants,
  (SELECT COUNT(*) FROM projects) as total_projects;"
