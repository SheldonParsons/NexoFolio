# MCP 专用 Token 与工具契约 v1

日期：2026-09-14

状态：设计草案，尚未实现。v1 表示 NexoFolio 业务契约版本，不是 MCP 协议版本。

关联：[模块化架构](../architecture/0002-modular-backend.md)、[实施计划](../plans/0001-backend-foundation.md)

## 1. 独立的 MCP 凭证

本平台提供独立的 MCP Token 管理入口，登录用户在有签发权限时创建、查看元数据和撤销 Token。
管理入口使用本平台登录会话；MCP 请求使用单独签发的凭证，不接受禅道密码、超级密码或普通网页登录会话作为 MCP Token。
agent 配置 MCP 地址与 Token 后，通过 HTTPS 请求头传递：

```http
Authorization: Bearer <mcp_token>
```

Token 不放在 URL、工具参数、资源 URI 或提示词中。所有 MCP HTTP 请求，包括发现/列表、工具调用和资源读取都验证凭证。
业务调用不再请求禅道登录；服务端依据 Token 绑定的身份、项目范围和当前权限作出判断。
该方案定位为支持预配置 Bearer Token 的客户端接入；不能据此宣称已实现 OAuth 自动发现与登录。其他客户端的适配另行验证。

### 管理契约

| HTTP 接口 | 授权方式 | 功能 |
| --- | --- | --- |
| POST /v1/mcp-tokens | 平台会话 + 签发权限 | 签发指定项目/权限/有效期的 Token |
| GET /v1/mcp-tokens | 平台会话 + 管理范围 | 列表，只返回名称、前缀、范围、状态等元数据 |
| DELETE /v1/mcp-tokens/{token_id} | 平台会话 + 管理范围 | 撤销凭证 |

签发输入：

```typescript
type IssueMcpToken = {
  name: string;
  project_ids: string[];       // 必须是已同步、签发者有权限的项目
  scopes: ("knowledge.read" | "knowledge.write")[];
  expires_at: string;          // UTC 时间；界面可提供预设有效期
};
```

默认只读；签发写权限时同时具备 read。签发者不能授予超出自身权限的项目或能力。
每个 Token 的项目范围是显式集合，未来同步的新项目不会自动加入。
首轮使用有明确所有者的个人凭证；机器身份/服务账号以后通过相同 Principal 契约接入。

凭证使用密码学安全随机秘密，建议至少 256 bit 随机性；服务端只保存秘密摘要及查询前缀。
完整秘密仅在签发成功时展示一次；无法找回时重新签发。轮换以新建凭证、更新客户端、撤销旧凭证完成。
Token 元数据保存 owner_id、project_ids、scopes、created_at、expires_at、revoked_at 和聚合更新的 last_used_at。
有效权限取 Token 授权与当前本地保存的所有者项目权限的交集。所有者在本平台被禁用、本地项目权限撤销或 Token 撤销后停止访问。普通 MCP 请求不调用禅道；禅道端权限变化在下一次成功同步后反映到本地，不承诺实时生效。
内部登录 Token 的 3 个月有效期和登录时原样复用规则，不自动套用到这里的专用 MCP Token。
MCP 第一版每个请求查验权威 Token/授权状态；并非高频插件采集凭证，不必沿用采集路径的权限缓存优化。
若以后增加授权缓存，必须明确失效机制和撤销传播边界，不能以 TTL 替代即时撤销承诺。

### 所属模块

```text
crates/access/src/mcp_tokens/         # 签发、范围、过期、撤销规则
crates/application/src/mcp_access/    # 管理操作与鉴权编排
crates/infrastructure/src/postgres/   # Token 与 scope 持久化
apps/backend/src/http/mcp_tokens/    # 平台管理 API
apps/backend/src/mcp/auth/           # Bearer 解析与上下文接入
```

管理入口可随后由前端页面承载，当前不创建前端工程。MCP 工具列表不暴露签发/撤销凭证的能力。
Chrome 插件继续使用独立的采集授权；MCP Token 仅供 MCP 使用，不能作为登录密码或插件全量采集凭证。

## 2. 工具设计原则

1. 固定工具数量，项目和目录扩展不增加工具。
2. 先读目录/摘要，再按需读定义、样例或历史；默认结果有明确的大小上限。
3. 除发现项目外，project_id 必填。没有隐式的全局“当前项目”，并发调用不会串项目。
4. 修改通过稳定接口 ID 和基础修订定位，不能由模糊查询直接写入。
5. Token、调用者身份、有效权限由服务端注入，工具不能自报 user_id 或扩大权限。
6. 数据库、重构触发与模型选择封装在服务端，工具参数描述业务意图。
7. 所有业务数据入口校验输入和授权，annotations 只是客户端提示，不是权限控制。

## 3. 第一版工具集合与签名

下面使用 TypeScript 风格表示 JSON 参数，问号表示可选。实际 MCP 使用 JSON Schema inputSchema/outputSchema，由 Rust DTO 生成并做契约测试。
固定工具名：list_projects、list_directory、search_interfaces、read_interface、create_interface、update_interface、get_change。
只读 Token 发现 5 个读取工具；写 Token 再发现新增和修改工具。即使客户端缓存了旧列表，执行时也重新鉴权。

### 3.1 list_projects：先确定项目

```typescript
list_projects({
  query?: string,
  limit?: number,              // 默认 20，范围 1..50
  cursor?: string
}) -> ProjectPage
```

只返回有效授权项目。每项包含 project_id、名称、禅道来源 ID、项目状态、root_directory_id、当前目录版本、可用操作。
这是 MCP 的凭证范围规则。平台网页登录后的项目列表另行允许展示已同步项目的基础卡片和 can_access 状态；该展示规则不扩大 MCP Token 权限。
query 用于名称筛选；重名返回多个候选，agent 必须使用稳定 ID。
本工具不创建或同步项目。

### 3.2 list_directory：逐层浏览

```typescript
list_directory({
  project_id: string,
  directory_id?: string,       // 省略为项目根目录
  catalog_version?: string,    // 省略为当前正式版本
  limit?: number,              // 默认 20，范围 1..50
  cursor?: string
}) -> DirectoryPage
```

返回当前目录的名称、范围摘要、业务别名和同级导航提示，以及一层子目录/接口卡片。
子项统一标注 kind=directory 或 interface，提供稳定 ID、简短用途、可继续读取的位置。
根目录包含待分类入口。目录没有子项也返回摘要和明确的空结果。
返回实际采用的 catalog_version，后续导航可以固定该版本；普通 Token 只访问当前或仍保留的已发布版本，不能访问后台候选。
不默认递归输出整棵树。目录迁移不改变接口 ID；旧路径解析不在第一版参数中承担定位职责。

### 3.3 search_interfaces：不清楚目录时搜索

```typescript
search_interfaces({
  project_id: string,
  query: string,              // 1..500 字符；业务词、接口名、路径或字段
  directory_id?: string,
  catalog_version?: string,
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS",
  service_id?: string,
  limit?: number,             // 默认 10，范围 1..50
  cursor?: string
}) -> SearchPage
```

默认搜索项目内当前可见接口，包含 observed/draft/confirmed，返回时明确标记状态。
默认覆盖待分类接口；仅当指定 directory_id 时按该目录子树限制。
每项返回 interface_id、名称、method/path、service_id、简短用途、当前修订、归属、matched_on 和 evidence_refs。
matched_on 说明命中了路径、字段还是业务说明，不把模型分数包装成事实置信度。
返回 coverage 说明本次是穷尽精确匹配还是候选检索，以及降级/限额影响；候选列表翻页完毕不等于证明整个知识库不存在相关内容。
catalog_version 控制目录过滤和归属视图，接口内容默认当前版本，二者分别标识。

### 3.4 read_interface：按需读取正文

```typescript
read_interface({
  project_id: string,
  interface_id: string,
  view?: "overview" | "definition" | "examples" | "history", // 默认 overview
  revision?: string,           // 默认当前修订
  max_chars?: number,          // 默认 8000，范围 1000..20000
  cursor?: string
}) -> InterfaceReadResult
```

overview：用途、方法/路径、关键参数、状态、来源、是否有待裁决变化，以及进一步读取入口。
definition：该修订的完整定义，包含身份、请求参数/Schema、响应Schema、鉴权说明、业务说明等。
examples：与该修订关联的脱敏代表样例、采集环境、时间、来源和原始数据是否完整。
history：修订清单、变更摘要和来源；此视图不允许同时传 revision，以免混淆“历史清单”和“某个修订正文”。

返回 metadata、view、content_format、content、chunk_offset_chars、complete、truncated、next_cursor。
overview 渲染 Markdown，其他视图可输出 JSON 文本。长正文采用固定修订的文本分块，content_format=json_fragment 时片段不能独立解析为完整 JSON，需按游标拼接。
续读游标绑定 project_id、interface_id、revision、view、内容哈希和偏移，客户端不得改变读取对象。
内容本来就被采集截断时标注 source_complete=false；读完已保存片段不代表原始响应完整。
修改 request/responses 等整组字段前，应读完整对应定义，不能将截断片段当作完整旧值。

### 3.5 create_interface：建立接口草稿

```typescript
create_interface({
  project_id: string,
  definition: InterfaceDraft,
  source_ids?: string[],
  reason: string,              // 1..1000 字符
  idempotency_key: string      // 1..128 字符
}) -> ChangeReceipt
```

```typescript
type InterfaceDraft = {
  service_id?: string;         // 多个服务可选时必须明确；单一映射时服务端可补全
  method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS";
  path: string;               // 服务内路径，不含域名和 query
  api_version?: string;       // 省略表示未声明，不猜测版本
  summary: string;
  description?: string;
  tags?: string[];
  request?: RequestDefinition;
  responses?: ResponseDefinition[];
  business_notes?: string;
};

type RequestDefinition = {
  parameters?: {name: string; in: "path" | "query" | "header" | "cookie";
                schema: object; required?: boolean; description?: string}[];
  body?: {media_type: string; schema: object; required?: boolean};
};

type ResponseDefinition = {
  status: string;             // 精确 HTTP 状态码或 default
  media_type: string;
  schema: object;             // 支持范围内的 JSON Schema
  description?: string;
};
```

默认创建 draft，由 agent/操作者身份记载来源，不将生成内容自动标记为人工已确认。
未提供的请求或响应部分表示未知，不表示已证明不存在。Schema 校验和定义确认是不同概念。
服务歧义返回 SERVICE_REQUIRED 和受权限限制的候选；不会自动创建服务或项目。
同一身份已存在且内容相同返回 unchanged，存在冲突则创建/关联待裁决提案。source_ids 必须属于同项目且调用者可读，不接受伪造来源。
不自动执行该接口或访问正文中的外部 URL。插件高频流量使用专用批量入口，不通过此工具上传。

### 3.6 update_interface：精确修改

```typescript
update_interface({
  project_id: string,
  interface_id: string,
  base_revision: string,
  changes: InterfaceChanges,
  source_ids?: string[],
  reason: string,
  idempotency_key: string
}) -> ChangeReceipt

type InterfaceChanges = {
  summary?: string;
  description?: string;
  tags?: string[];
  request?: RequestDefinition;
  responses?: ResponseDefinition[];
  business_notes?: string;
};
```

changes 至少一个字段。未提供的字段保持原值；提供的字段整体替换。空字符串或空数组仅在字段定义允许时表示清空。
request/responses 内部不隐式做递归合并；语义必须由文档、JSON Schema 描述和测试固定。
第一版此工具不修改 project、service、method/path 身份，也不暴露删除/强制覆盖/直接确认按钮。
基础修订匹配且当前编辑规则允许时生成新修订；过期基础修订或冲突进入 pending_adjudication，不能自动重试成覆盖最新版本。
状态匹配但违反输入或业务约束是工具错误，不自动创建无效提案。

### 3.7 get_change：读取操作结果

```typescript
get_change({
  project_id: string,
  change_id: string
}) -> ChangeStatus
```

返回 applied、unchanged、pending_adjudication 或 rejected，以及目标 ID、基础修订、应用结果修订、冲突摘要和审计时间。
只展示当前调用者可见的内容。待裁决状态可查询但不会通过此工具完成裁决。
不需要高频轮询；首次新增/修改已经返回回执，只有后续需要确认裁决结果时再读取。

## 4. 返回契约与版本

所有工具以 JSON 对象返回 structuredContent，并声明 outputSchema；为兼容需要文本的客户端，同时返回同内容的紧凑 JSON TextContent。
不重复输出一份长 Markdown 解读，避免无谓放大上下文。
业务版本字段 contract_version="1" 与传输协议版本分开。

列表结果包含 data、request_id、contract_version 和 page：

```typescript
type PageInfo = {
  complete: boolean;           // 当前范围下本次结果集是否已经读完
  truncated: boolean;          // 是否因内容预算裁剪；普通分页本身不等于截断
  next_cursor: string | null;
};
```

游标绑定原参数、授权主体和读取快照，必须再次验证当前 Token 项目范围。不能用旧游标恢复已经撤销的权限。
同一主体换新凭证后也必须重新鉴权；跨主体游标不可复用。游标过期返回 CURSOR_EXPIRED，要求从入口刷新。
search 的 page.complete 仅表示候选结果集读完；coverage.exhaustive 才说明搜索覆盖是否穷尽，不能混用。

写回执示例：

```json
{
  "contract_version": "1",
  "request_id": "req_example",
  "data": {
    "change_id": "chg_example",
    "status": "pending_adjudication",
    "interface_id": "api_example",
    "base_revision": "rev_12",
    "current_revision": "rev_14",
    "applied_revision": null,
    "reason_code": "BASE_REVISION_CHANGED"
  }
}
```

pending_adjudication 是成功接收了提案，MCP isError=false，但绝不能描述为已经修改成功。
applied_revision 标识本次实际写入的修订。后续查询中的 current_revision 可能已再次变化，两者分别返回。
每个写请求都返回可查询的 change_id，unchanged 也具有操作回执；幂等键作用域为主体、项目、工具名和键，绑定规范化请求摘要。
同键同请求返回原回执，同键不同请求返回 IDEMPOTENCY_CONFLICT；凭证轮换不应让同一主体的重试创建第二次变更。

## 5. MCP JSON Schema 示例

以下仅展示只读工具输入，实际实现必须为七个工具分别提供完整 inputSchema/outputSchema：

```json
{
  "name": "read_interface",
  "description": "按项目和稳定接口 ID 读取摘要、完整定义、样例或历史。先用 overview 判断相关性；内容未读完时使用 next_cursor 续读。",
  "inputSchema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "project_id": {"type": "string", "minLength": 1},
      "interface_id": {"type": "string", "minLength": 1},
      "view": {"type": "string", "enum": ["overview", "definition", "examples", "history"], "default": "overview"},
      "revision": {"type": "string", "minLength": 1},
      "max_chars": {"type": "integer", "minimum": 1000, "maximum": 20000, "default": 8000},
      "cursor": {"type": "string", "minLength": 1}
    },
    "required": ["project_id", "interface_id"]
  },
  "annotations": {
    "readOnlyHint": true,
    "destructiveHint": false,
    "idempotentHint": true,
    "openWorldHint": false
  }
}
```

默认值由服务端实际实现，不能只写在 Schema 中。revision 与 history 的互斥也必须在业务校验中检查。
create/update 标记 readOnlyHint=false；update 因整组字段可能被清空，destructiveHint=true。幂等保证依赖必填幂等键及服务端事务。
所有工具仅操作本知识库，openWorldHint=false；未来若增加外部调用能力需独立工具和权限。

## 6. 错误及客户端行为

- 缺失/错误/过期/撤销 Token：在 HTTP 层拒绝，返回 401。有效凭证但操作范围不足按访问层与工具错误契约返回，不能泄露不可见对象是否存在。
- 已进入工具调用的业务校验错误：isError=true，给出稳定 error.code、message、retryable 和受限的 details。
- 典型错误：VALIDATION_ERROR、FORBIDDEN、NOT_FOUND、SERVICE_REQUIRED、CATALOG_VERSION_EXPIRED、CURSOR_EXPIRED、IDEMPOTENCY_CONFLICT、RETRYABLE_FAILURE。
- 修改遇到基础版本变化按提案处理，返回 pending_adjudication；不以网络失败诱导客户端反复重试。
- 限流或临时不可用可以重试；写请求必须沿用相同幂等键。Token 失效应在客户端更新凭证，不在工具参数中补密码。

## 7. 典型 agent 使用顺序

查询：list_projects → list_directory 或 search_interfaces → read_interface(overview) → 按需读取 definition/examples。
已知项目和接口 ID 时直接 read_interface，无需每次从根目录开始。
修改：读取当前完整相关定义 → update_interface(base_revision=读取到的版本) → 根据回执报告 applied 或待裁决。
新增：确定项目与服务 → create_interface → 使用回执中的 ID 读取结果。
目录重构、项目同步、Token 管理和裁决属于平台管理能力，第一版普通 MCP Token 不拥有这些操作。

## 8. 实施与验收补充

- access 增加独立 mcp_tokens 模块、Token 元数据/授权表、签发与撤销服务。
- HTTP 管理 API 使用平台会话；MCP middleware 使用专用 Bearer Token，二者凭证不能互换。
- 验证签发者不能扩大授权；撤销/过期/所有者禁用/项目权限变化在后续请求生效。
- 验证只读凭证发现/执行工具范围；缓存旧工具列表不能绕过写权限。
- 七个工具分别校验 Schema、未知字段、默认值、限额和错误形态；实际生成 Schema 与文档示例保持一致。
- 验证并发不同 project_id 不串项目，读旧游标不能越权，目录发布后稳定 ID 仍可读。
- 验证长内容续读无丢字/重复、原始截断信息不丢失；搜索候选耗尽不误报不存在。
- 验证整组替换和未提供字段保持语义；并发修改进入待裁决；丢响应重试返回同一回执。
- 使用真实 HTTP MCP 客户端验证 Token 配置与读取/写入。兼容性结论按客户端与版本分别记录。

参考资料：[MCP 工具和结构化返回](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)、[MCP HTTP 授权](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization)。使用对象型 Schema 保留兼容面；这些资料不代表本方案已实现完整 OAuth 流程。
