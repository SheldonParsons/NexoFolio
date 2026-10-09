#!/bin/bash

if [ $# -lt 1 ]; then
    echo "用法: $0 <token>"
    exit 1
fi

TOKEN="$1"
API_URL="http://localhost:18080"

echo "=== 获取项目列表 ==="
curl -s -X GET "$API_URL/v1/projects" \
    -H "Authorization: Bearer $TOKEN" | jq '.'

echo ""
echo "=== 获取第一个项目详情 ==="
PROJECT_ID="9568a877-eb48-42ee-acbc-f9217f052386"
curl -s -X GET "$API_URL/v1/projects/$PROJECT_ID" \
    -H "Authorization: Bearer $TOKEN" | jq '.'
