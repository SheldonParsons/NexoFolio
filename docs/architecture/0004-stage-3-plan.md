# 0004 阶段 3 计划：先让人看到接口，再让 AI 读到接口

状态：待确认（2026-09-29）

## 0. 这一阶段要解决什么

阶段 2 之后，接口已经采集下来并整理成结构，但只能在终端里用 `admin observe report` 查看。前端的接口文档页对接的还是旧后端的接口，打开就是 404；MCP 只有空壳，一个工具都没有。

这一阶段分三步，按顺序做：

| 步骤 | 内容 | 验收 |
|---|---|---|
| 3a | 后端：接口读取 HTTP（列表、详情、示例） | HTTP 测试通过；用本机真实采集的数据能取到内容 |
| 3b | 前端：接口文档页按新数据重做 | 你在页面上能看清一个项目的全部接口、字段和示例 |
| 3c | MCP：Token 和只读工具（**这一轮不做，设计先定在这里**） | agent 能列出和查看接口 |

这一轮不做：view 模块和目录树（"待分类"）、knowledge、接口描述、大模型。

已确认（2026-09-29）：先做前端，MCP 后补；示例值不脱敏，直接显示；正式目录、目录预览、知识维护三个页面先从导航隐藏，代码保留。

---

## 1. 3a 后端：接口读取

### 1.1 合同层变化（`crates/contracts/src/endpoint.rs`）

`EndpointReader` 已经有 `list` 和 `get`，这次只补"示例"：

- `EndpointFacts` 新增 `examples: Vec<ExampleSummary>`。每一种不同结构（一条指纹）对应一个示例：
  - `id`：示例 ID，16 位十六进制。由 环境 + 服务地址 + 指纹哈希 算出来：同一种结构可能在多个环境、多个地址上出现，所以这两项要算进去；接口 ID 不算进去，所以接口合并后示例 ID 不变。
  - `environment_id`、`address`
  - `status`：返回状态码，没有返回时为空
  - `calls`、`first_seen`、`last_seen`
- `EndpointReader` 新增 `example(endpoint, example_id) -> Option<Value>`：取一条示例的完整原始内容（请求方法、URL、请求头、请求体、返回状态、返回头、返回体）。

示例内容可能很大（整段返回体），所以详情里只带示例的摘要，原始内容点开时再单独取。

### 1.2 observe 内部变化

- `ObserveStore` 的 `FingerprintStats` 带上哈希；新增按 接口 + 哈希 读取 sample 的方法。已合并接口的示例跟着搬过去（现有 `move_traffic` 已经会搬指纹，不需要改）。
- 不需要新迁移：`fingerprints` 表里已经存着 `sample`。
- 内存 fake 和一致性测试同步补上。

### 1.3 HTTP 接口（apps/backend，新文件 `http/endpoints.rs`）

三个接口都要求登录，并且当前用户能访问这个项目，和 `service-addresses` 的检查方式一样。

1. `GET /v1/projects/{project_id}/endpoints?q=order`
   返回项目的全部接口，按路径模板排序。`q` 对路径模板做不区分大小写的普通文字匹配。
2. `GET /v1/projects/{project_id}/endpoints/{endpoint_id}`
   返回接口详情。如果它已经被合并进另一个接口，就直接返回合并后的那个，前端看到 `id` 不同会跳转过去。接口不属于这个项目时返回 404。
3. `GET /v1/projects/{project_id}/endpoints/{endpoint_id}/examples/{example_id}`
   返回一条示例的完整原始内容。

规则：

- 示例内容不脱敏，但**不进日志**，错误响应里也不带示例内容。
- 响应加 `Cache-Control: no-store`（沿用现有中间件）。
- 测试用的 fixtures 只用假数据。

### 1.4 返回格式（假数据示例）

列表：

```json
{
  "items": [
    {
      "id": "0192…",
      "method": "GET",
      "path_template": "/api/v1/orders/{id}",
      "external": false,
      "declared": false,
      "environments": [
        { "environment_id": "…", "calls": 128, "first_seen": "2026-09-26T08:00:00Z", "last_seen": "2026-09-28T10:12:00Z" }
      ]
    }
  ]
}
```

详情：

```json
{
  "id": "0192…",
  "method": "GET",
  "path_template": "/api/v1/orders/{id}",
  "external": false,
  "declared": false,
  "aliases": [],
  "environments": [ … ],
  "addresses": [
    { "environment_id": "…", "address": "https://api.example.com", "base_path": "/gateway" }
  ],
  "fields": [
    {
      "location": { "in": "path" },
      "path": "id",
      "types": ["string"],
      "labels": [ { "environment_id": "…", "label": "always" } ],
      "differs_between_environments": false,
      "declared": null,
      "conflict": null
    },
    {
      "location": { "in": "response_body", "status": 200 },
      "path": "data.items[].sku",
      "types": ["string"],
      "labels": [ { "environment_id": "…", "label": "optional" } ],
      "differs_between_environments": false,
      "declared": null,
      "conflict": null
    }
  ],
  "examples": [
    { "id": "9f3a…", "environment_id": "…", "address": "https://api.example.com", "status": 200, "calls": 120, "first_seen": "…", "last_seen": "…" }
  ]
}
```

`path` 用可读形式（`data.items[].sku`），不暴露内部的分段数组。环境名前端用现有的环境接口来查，这里只给 ID。

---

## 2. 3b 前端：接口文档页

### 2.1 范围

- 项目内的接口页（`/projects/:id/interfaces`）按新接口重做。旧的"定义 / 观测记录 / 评估"数据层和对应组件删除，因为它们对应的是旧模型。
- 正式目录、目录预览、知识维护：从导航和入口隐藏，路由和代码保留。
- 本地开发用的假数据示例（fixtures）按新格式重写，只用假数据。

### 2.2 页面内容

- 左侧是接口列表：请求方法、路径模板、外部服务标记，支持搜索和键盘上下切换。
- 右侧是接口详情：
  - 页头：请求方法、路径模板、服务地址、各环境的调用次数和最近一次调用时间。
  - 字段：按"路径参数 / 查询参数 / 请求体 / 返回体（按状态码分组）"排列，嵌套字段按层级缩进。每个字段标出类型，以及它在各环境是"一直有 / 可选 / 新增 / 已删除 / 从不出现 / 观察中"中的哪一种。环境之间不一致、或者和声明冲突的字段要能一眼看出来。
  - 示例：列出每种结构的示例（环境、状态码、调用次数），点开后显示完整的请求和返回。

### 2.3 设计要求和做法

核心内容（字段和示例）要干净整洁：以内容为主，不加装饰性的外框、标签和编号；信息层级靠字号、字重、间距和对齐来区分，颜色只用来标出需要注意的地方（环境差异、冲突）。

做法：先用假数据把页面做出来，截图给你看，确认后再接真实接口。

---

## 3. 3c MCP（下一轮做，设计先定）

### 3.1 MCP Token（access 模块）

- 新增 Token 表：库里只存哈希，明文只在创建时显示一次；带名称、创建时间、最近使用时间、吊销时间。
- Token 属于某个用户，权限跟着用户走：用户能看哪些项目，Token 就能看哪些。
- HTTP：创建、列出、吊销。前端的设置页加一个 Token 管理区，附 MCP 配置示例。
- 实现现有的 `McpTokenVerifier`，替换现在"永远拒绝"的占位实现。

### 3.2 只读工具（apps/backend/src/mcp）

直接复用 3a 的 `EndpointReader` 和 `ServiceAddresses`，不写新的业务逻辑：

- `list_projects`：当前 Token 能看的项目
- `list_endpoints(project, query?)`：接口列表，按路径做普通文字匹配
- `get_endpoint(project, endpoint)`：字段、类型、各环境标签
- `get_example(project, endpoint, example)`：一条示例的原始内容
- `list_service_addresses(project)`：服务地址和外部服务标记

检索不经过大模型。等阶段 4、5 有了 view 和目录树，再把这些工具切过去。

---

## 4. 实施顺序

1. 3a-1：合同 + observe（store、fake、一致性测试、Postgres 实现）
2. 3a-2：HTTP 接口 + 测试 + 用本机数据试一遍 → **把真实返回格式发给你确认**
3. 3b-1：前端数据层 + 假数据 + 隐藏三个页面
4. 3b-2：接口文档页设计和实现 → **截图给你确认**
5. 3b-3：接真实接口，测试、类型检查、构建
