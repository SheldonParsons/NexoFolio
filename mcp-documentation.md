# NexoFolio MCP 设计说明

## 1. MCP 是什么

NexoFolio 通过 Model Context Protocol (MCP) 将项目的接口知识暴露给 AI Agent（如 Claude Desktop、Cursor 等），让 Agent 能够查询、理解和推荐项目中的 API。

## 2. 认证方式

- **MCP Token**：用户在 NexoFolio 中生成专用的 MCP Token
- Token 具有项目级权限：每个 token 可以访问特定项目的接口知识
- Token 格式：`nfm_` 前缀 + 64 字符十六进制
- 安全性：Token 在数据库中以哈希形式存储，仅在生成时显示一次

## 3. 对外暴露的 MCP Tools

### 3.1 `nexofolio_catalog`

**用途：** 获取项目的接口目录树

**输入参数：**
- `project_id` (string, required): 项目 ID

**返回内容：**
- 目录树结构（folders）
- 每个目录包含：id, name, summary（目录说明）, parent_id, position
- 未分类接口数量（unplaced_count）

**典型使用场景：**
- Agent 需要了解项目整体 API 结构
- 查找特定功能模块的接口组织

### 3.2 `nexofolio_folder_endpoints`

**用途：** 获取某个目录下的接口列表，或获取未分类接口

**输入参数：**
- `project_id` (string, required): 项目 ID
- `folder_id` (string, optional): 目录 ID，不传则返回未分类接口
- `page` (integer, optional, default=1): 页码
- `limit` (integer, optional, default=100): 每页数量

**返回内容：**
- 接口 ID 列表（items: EndpointId[]）
- 分页信息：total, page, limit

**典型使用场景：**
- 浏览特定模块的接口
- 查看新增的未分类接口

### 3.3 `nexofolio_endpoint`

**用途：** 获取单个接口的完整知识

**输入参数：**
- `endpoint_id` (string, required): 接口 ID

**返回内容：**

**知识部分（knowledge）：**
- note: 名称（name）、用途说明（purpose）
- placement: 所在目录位置
- path: 目录层级路径（面包屑）
- fields: 字段说明列表（path, text, values）
- links: 关联接口（关联类型 + 证据）

**事实部分（facts，来自流量观测）：**
- method, path_template
- 请求参数、请求体、响应结构
- 调用统计：call_count, last_called_at
- 真实调用示例（examples）

**典型使用场景：**
- 理解某个接口的用途和参数
- 查看接口的真实调用示例
- 发现接口之间的关联关系

### 3.4 `nexofolio_search_endpoints`

**用途：** 按关键词搜索接口（按 method + path 字面匹配）

**输入参数：**
- `project_id` (string, required): 项目 ID
- `query` (string, required): 搜索关键词

**返回内容：**
- 匹配的接口列表（method, path, id）

**典型使用场景：**
- "帮我找用户登录相关的接口"
- "这个项目有订单导出的 API 吗？"

## 4. 知识来源说明

NexoFolio 的接口知识由两部分构成：

### 4.1 自动观测（observe 模块）

- 从真实流量中自动提取接口结构
- 包括：method, path, 参数类型、请求响应结构、调用次数
- 持续更新，反映最新的接口行为

### 4.2 人工整理 + LLM 补全（knowledge 模块）

- 接口名称和用途说明（由 LLM 基于真实示例生成，人工可修订）
- 目录分类（由 LLM 基于全项目接口列表生成目录树，人工可调整）
- 字段说明（对重要字段的业务含义解释）
- 接口关联（标注哪些接口之间存在调用关系、数据依赖等）

## 5. MCP 使用流程

### 步骤 1：生成 Token

- 在 NexoFolio 项目设置中生成 MCP Token
- 选择要授权的项目

### 步骤 2：配置 MCP Client

示例配置（Claude Desktop）：

```json
{
  "mcpServers": {
    "nexofolio": {
      "command": "npx",
      "args": ["-y", "@nexofolio/mcp-client"],
      "env": {
        "NEXOFOLIO_API_URL": "https://your-nexofolio-instance.com",
        "NEXOFOLIO_MCP_TOKEN": "nfm_your_token_here"
      }
    }
  }
}
```

### 步骤 3：Agent 调用

Agent 可以通过上述 tools 查询接口知识，例如：

- "这个项目有哪些用户管理相关的接口？"
- "POST /api/orders 这个接口的参数是什么？"
- "帮我找到和订单状态变更相关的所有接口"

## 6. 典型使用场景

### 场景 1：理解陌生项目

Agent 先调用 `nexofolio_catalog` 了解整体结构，再通过 `nexofolio_folder_endpoints` 浏览各模块接口。

### 场景 2：集成开发

开发者问 "如何调用订单创建接口？"，Agent 通过 `nexofolio_search_endpoints` 找到接口，再用 `nexofolio_endpoint` 获取参数说明和真实示例，直接生成调用代码。

### 场景 3：影响面分析

需要修改某个数据结构时，Agent 通过 `nexofolio_endpoint` 的 links 字段发现所有关联接口，评估改动影响范围。

## 7. 与其他 API 文档工具的区别

| 特性 | Swagger/OpenAPI | NexoFolio MCP |
|------|----------------|---------------|
| 数据来源 | 人工编写/代码注解 | 真实流量自动提取 + LLM 整理 |
| 内容时效性 | 易过期 | 持续同步线上行为 |
| 业务语义 | 需要人工维护 | LLM 基于真实示例推断 |
| 接口关联 | 不支持 | 自动发现并标注 |
| AI 友好性 | 需要解析 YAML/JSON | 原生 MCP 工具调用 |
| 真实示例 | 手动编写示例 | 直接展示生产调用记录 |

---

**以上内容供 Codex 更新 MCP 文档页面使用。重点是：认证方式、4 个核心 tools 的用途和参数、知识的双重来源（观测 + 整理）、典型使用场景。**
