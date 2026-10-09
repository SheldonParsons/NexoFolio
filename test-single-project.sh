#!/bin/bash

TOKEN="nfi_aeb2236a759fcbd0b24bf557472c39c212e3612b1557388c1a1e2251b38d76e4"
PROJECT_ID="57a3a86b-2fe4-4d4e-95b9-64a1a086b7a7"

echo "=== 测试商用订单管理系统访问 ==="
curl -v "http://localhost:18080/v1/projects/$PROJECT_ID" \
    -H "Authorization: Bearer $TOKEN" 2>&1
