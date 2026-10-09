#!/bin/bash

# NexoFolio 登录测试脚本

if [ $# -lt 2 ]; then
    echo "用法: $0 <账号> <密码>"
    echo "示例: $0 zhangsan mypassword"
    exit 1
fi

ACCOUNT="$1"
PASSWORD="$2"
API_URL="http://localhost:18080"

echo "=== 登录到 NexoFolio ==="
echo "账号: $ACCOUNT"
echo "API: $API_URL"
echo ""

RESPONSE=$(curl -s -w "\nHTTP_CODE:%{http_code}" \
    -X POST "$API_URL/v1/auth/login" \
    -H "Content-Type: application/json" \
    -d "{\"account\":\"$ACCOUNT\",\"password\":\"$PASSWORD\"}")

HTTP_CODE=$(echo "$RESPONSE" | grep "HTTP_CODE:" | cut -d: -f2)
BODY=$(echo "$RESPONSE" | sed '/HTTP_CODE:/d')

echo "响应状态: $HTTP_CODE"
echo ""

if [ "$HTTP_CODE" = "200" ]; then
    echo "✓ 登录成功"
    echo ""
    echo "$BODY" | jq '.'

    # 提取 token
    TOKEN=$(echo "$BODY" | jq -r '.token')

    if [ "$TOKEN" != "null" ] && [ -n "$TOKEN" ]; then
        echo ""
        echo "=== 测试认证 ==="
        ME_RESPONSE=$(curl -s -w "\nHTTP_CODE:%{http_code}" \
            -H "Authorization: Bearer $TOKEN" \
            "$API_URL/v1/auth/me")

        ME_HTTP_CODE=$(echo "$ME_RESPONSE" | grep "HTTP_CODE:" | cut -d: -f2)
        ME_BODY=$(echo "$ME_RESPONSE" | sed '/HTTP_CODE:/d')

        if [ "$ME_HTTP_CODE" = "200" ]; then
            echo "✓ Token 验证成功"
            echo "$ME_BODY" | jq '.'
        else
            echo "✗ Token 验证失败 ($ME_HTTP_CODE)"
            echo "$ME_BODY"
        fi
    fi
else
    echo "✗ 登录失败"
    echo "$BODY" | jq '.' 2>/dev/null || echo "$BODY"
fi
