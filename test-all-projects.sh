#!/bin/bash

TOKEN="nfi_aeb2236a759fcbd0b24bf557472c39c212e3612b1557388c1a1e2251b38d76e4"
API_URL="http://localhost:18080"

echo "=== 测试三个项目的详细访问 ==="
echo ""

# 商用订单管理系统
echo "1. 商用订单管理系统"
PROJECT_ID="57a3a86b-2fe4-4d4e-95b9-64a1a086b7a7"
RESPONSE=$(curl -s -w "\nHTTP_CODE:%{http_code}" \
    -H "Authorization: Bearer $TOKEN" \
    "$API_URL/v1/projects/$PROJECT_ID")
HTTP_CODE=$(echo "$RESPONSE" | grep "HTTP_CODE:" | cut -d: -f2)
BODY=$(echo "$RESPONSE" | sed '/HTTP_CODE:/d')
echo "状态码: $HTTP_CODE"
echo "$BODY" | jq '.' 2>/dev/null || echo "$BODY"
echo ""

# 禅道测试2.0
echo "2. 禅道测试2.0"
PROJECT_ID="8e0eebd3-38de-449e-8203-5fe374a12372"
RESPONSE=$(curl -s -w "\nHTTP_CODE:%{http_code}" \
    -H "Authorization: Bearer $TOKEN" \
    "$API_URL/v1/projects/$PROJECT_ID")
HTTP_CODE=$(echo "$RESPONSE" | grep "HTTP_CODE:" | cut -d: -f2)
BODY=$(echo "$RESPONSE" | sed '/HTTP_CODE:/d')
echo "状态码: $HTTP_CODE"
echo "$BODY" | jq '.' 2>/dev/null || echo "$BODY"
echo ""

# 客户资源管理系统CRM
echo "3. 客户资源管理系统CRM"
PROJECT_ID="ce5a40ff-40ae-4513-be05-bc725ac63c8c"
RESPONSE=$(curl -s -w "\nHTTP_CODE:%{http_code}" \
    -H "Authorization: Bearer $TOKEN" \
    "$API_URL/v1/projects/$PROJECT_ID")
HTTP_CODE=$(echo "$RESPONSE" | grep "HTTP_CODE:" | cut -d: -f2)
BODY=$(echo "$RESPONSE" | sed '/HTTP_CODE:/d')
echo "状态码: $HTTP_CODE"
echo "$BODY" | jq '.' 2>/dev/null || echo "$BODY"
