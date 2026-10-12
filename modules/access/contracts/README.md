# 登录和项目 HTTP API

状态：已实现。真实禅道使用已验证的 REST API v1 适配器。账号级 MCP Token 管理见下文；独立同步接口不在本次范围。

## 配置与迁移

配置 `DATABASE_URL`，运行 `nexofolio-admin migrate`。
启用本组路由必须同时设置：

- `NEXOFOLIO_ZENTAO_BASE_URL`：实际禅道 REST v1 基地址，不含账号密码、query 或 fragment。
- `NEXOFOLIO_SESSION_KEY`：32 字节随机密钥的 64 位十六进制表示，例如通过 `openssl rand -hex 32` 生成。保存在部署密钥配置中，重启必须复用同一密钥。
- 可选 `NEXOFOLIO_EMERGENCY_PASSWORD_HASH`：Argon2id PHC 字符串；不设置则禁用超级密码分支。

只设置基地址或密钥之一会启动失败。两者均不设置时，不注册登录与项目路由，保持骨架原来的 404 行为。
密钥加密数据库内的 Token，不能直接删除或随重启随机更换；损坏密文或错误密钥返回错误，不偷偷刷新用户现有 Token。
业务只通过明确的迁移命令建表，API 不自动迁移。

## POST /v1/auth/login

```json
{ "account": "你的禅道账号", "password": "你的密码" }
```

- 普通登录始终先请求禅道 `/tokens` 和 `/user`，即使本地已有有效 Token，也不能跳过禅道验证。
- 身份按禅道实例和 profile.id 唯一映射，保存禅道返回的 account 与显示名；本次验证的大小写规范化不会创建重复用户。
- 如果输入密码匹配配置的超级密码哈希，直接查找该实例的已有本地用户，不访问禅道、不创建新用户、不增加项目权限。
- 每用户一个有效内部 Token；未过期且未撤销时原样返回，不改变签发和到期时间。没有或已过期/撤销时签发新 Token，有效期为 UTC 签发时点起 3 个自然月，月底按 PostgreSQL interval 规则处理。
- 内部 Token 使用 `nfi_` 前缀和 256 bit 随机秘密，持久化 SHA-256 摘要及绑定用户 ID 的 AES-256-GCM 密文；普通密码和禅道 Token 不落库。
- 行锁保证同一用户并发登录不会重复签发。账号已在本地禁用时拒绝登录。
- 登录审计独立记录普通/超级密码方式，不为记录登录方式而改变 Token。

成功 HTTP 200 示例：

```json
{
  "user": {
    "id": "本地用户UUID",
    "account": "规范账号",
    "display_name": "显示名",
    "enabled": true
  },
  "token": "仅在响应中交给调用方的内部Token",
  "token_type": "Bearer",
  "expires_at": "2026-12-14T00:00:00+00:00",
  "token_reused": false,
  "project_sync": {
    "status": "completed",
    "counts": { "created": 2, "updated": 0, "skipped": 8 }
  }
}
```

项目同步失败时登录仍成功，project_sync 返回 `{"status":"failed","code":"PROJECT_SYNC_FAILED"}`，不覆盖旧项目或完整授权。
超级密码登录返回 `{"status":"skipped","reason":"EMERGENCY_LOGIN_NO_ZENTAO_CREDENTIAL"}`，使用已有本地权限；从未取得权限时项目内容保持不可访问。
响应设 `Cache-Control: no-store`。密码只接受非空值，账号最多 128 字节、密码最多 1024 字节；请求体最多 16 KiB，未知字段拒绝。
进程内最多同时执行 4 个登录，忙时返回 429/LOGIN_BUSY 和 Retry-After；这是有界并发保护，不是跨实例的完整暴力登录防护。
禅道认证全链路最长 15 秒，项目分页全链路最长 20 秒；默认 HTTP 请求截止时间 60 秒。

## 登录后的项目同步

每次普通登录成功都同步一次，包括复用旧 Token 的登录。

1. 请求用户可见的 `/projects?status=all&page=...&limit=50`，按实际返回分页读取完整集合。
2. 不存在且 doing：创建本地项目；不存在且其他状态：跳过。
3. 已存在：只更新状态和同步观测信息，保留原项目 ID、名称和已有知识，不重复创建。
4. 本地项目访问集合来自完整返回集合与 profile.view.projects 的交集；profile.view 缺失时使用该用户完整项目列表；明确空 view 不解释为全部权限。
5. 不在本次结果里的项目不删除、不擅自改状态；完整同步可移除该用户不再拥有的本地访问关系。
6. 失败页、重复 ID、不一致的 total、异常空页或超过条数/页数/响应上限，整次同步返回失败，不提交部分权限。
7. 同步代次在请求禅道前分配，数据库拒绝旧代次覆盖该用户新快照；共享项目状态也只接受更大的观测代次。

多用户登录逐步汇集项目，项目库不代表禅道全量项目。
用户可见范围不是写入、裁决或发布授权；本次只实现项目内容的读取准入。

## 内部鉴权

后续请求使用 `Authorization: Bearer <内部Token>`。只查询本地会话有效期、撤销状态、用户状态和项目授权，不请求禅道。
权限依照最近一次成功同步的本地记录，禅道变更不实时生效；没有权限快照 TTL 或后台自动刷新。
专用 MCP Token 与该 Token 分开，内部 Token 不能访问 `/mcp`，MCP Token 也不能用于网页登录或管理凭证。

## 账号级 MCP 凭证

前端 `/settings/mcp` 的头像菜单和顶部接口文档菜单入口共用同一页面。用户只能管理自己创建的凭证；不绑定项目、不复制权限快照。访问所有项目时仍按该账号当前本地授权检查，禅道侧变更沿用上文的登录同步机制。

- `GET /v1/auth/mcp-tokens?page=1&limit=20`：仅返回当前账号的凭证元数据，包含 `items/total/page/limit`。page 范围 1..100000，limit 范围 1..100。
- `GET /v1/auth/mcp-connection`：返回 `transport`、MCP handler 声明支持的 `protocol_versions`，以及当前账号的 `accessible_projects` 数量。未建立权限快照时数量为 null，不等同于 0；只计入同实例且已授权项目。前端服务地址沿用实际 API 基础地址及其代理前缀，追加 `/mcp`。
- `POST /v1/auth/mcp-tokens`：请求为 `name`、`starts_at`、`expires_at`，拒绝 `user_id`、项目选择等未知字段。名称为 1～64 字；时间使用带时区的 RFC 3339。`starts_at` 缺省或 null 表示立即生效，`expires_at` 缺省或 null 表示无限期；结束时间必须晚于开始时间及服务端当前时间。
- 创建返回 HTTP 201，含 `credential` 元数据、`token` 和 `token_type: Bearer`。完整 token 仅此时返回，之后无法取回。前端仅保存在当前弹窗内存中，关闭、退出账号或离开页面即清除；创建超时不会自动重试。
- `DELETE /v1/auth/mcp-tokens/{id}`：撤销自己的凭证，成功返回 204，重复撤销仍返回 204；不存在或属于他人的凭证返回 404。撤销不会影响其他 token。

元数据包含 `id/name/issued_at/starts_at/expires_at/revoked_at/status`，状态为 `scheduled/active/expired/revoked`。列表不返回原文、摘要或业务系统凭据。所有管理接口要求有效网页登录态，响应设 `Cache-Control: no-store`。

新 token 使用 `nfm_` 前缀和 256 bit 随机秘密，只保存 SHA-256 摘要。旧 MCP token 仍通过独立凭证表校验，不根据前缀将网页登录 token 当作 MCP token。未生效、到期、撤销、账号禁用或权限未建立时拒绝 MCP 访问；每次工具调用也重新检查项目权限和凭证状态。

部署本功能需先执行 access 迁移 `202610090001_mcp_token_management.sql`，再更新后端；API 不自动迁移。该迁移保留旧凭证，以原签发时间回填生效时间，并允许无到期时间。开发验证仅使用独立测试库，不代表运行中的实例已经迁移或部署。

### MCP 项目与环境发现

客户端连接 `/mcp` 后可从 `list_projects` 开始，无需已知项目 ID。它只返回 token 所属账号当前已授权、同一实例的项目，字段为 `project_id/name/status`。按名称确认项目后调用 `list_project_environments`，传入 `project_id`，返回 `project_id/environment_id/name`。这些 ID 可以直接传给后续接口工具；不会通过项目名称或环境名称猜测 UUID。

两个工具均接受 `offset`（默认 0）、`limit`（默认 20、范围 1～50），返回 `items/total/offset/limit/has_more/next_offset`。`has_more=true` 时使用返回的 `next_offset` 续读。查询使用一致的只读事务，且每次重新检查 token 生效时间、到期、撤销、账号状态与当前授权；权限缺失、存储故障与成功的空列表是不同结果。无权项目与不存在项目统一拒绝环境读取，不泄露环境数量或名称。

环境列表来自项目配置，可能尚无采集数据。多个名称匹配时由用户选择；这两个发现工具不创建项目、环境，不同步外部权限，也不读取业务认证凭据。此扩展无需新增数据库迁移。

## GET /v1/auth/me

返回当前本地用户基础信息。凭证缺失、过期、撤销或用户禁用返回 401。可用于应用恢复登录状态，调用不会同步项目或续期。

## GET /v1/projects?page=1&limit=20

返回当前禅道实例下已同步的项目，包括当前用户无权进入的项目。page 范围 1..100000，limit 范围 1..100。
列表包含 total、page、limit、items，每项：

```json
{
  "project_id": "本地项目UUID",
  "name": "项目名称",
  "status": "doing",
  "can_access": false,
  "access_state": "denied",
  "reason_code": "PROJECT_ACCESS_DENIED"
}
```

access_state 为 allowed、denied 或 unknown。未知权限返回 can_access=false 和 PROJECT_ACCESS_UNAVAILABLE。
无权限卡片只包含以上基础字段，不返回成员、白名单、目录或接口内容。项目状态可以为 closed/wait 等，状态变化不等于本次自动改成只读策略。

## GET /v1/projects/{project_id}

已登录且具有该项目有效访问权限时返回项目卡片；明确无权限返回 403/PROJECT_ACCESS_DENIED；本地权限尚未建立返回 503/PROJECT_ACCESS_UNAVAILABLE；不存在或不同实例返回 404。
此端点是当前的“进入项目”校验。后续知识端点必须同样调用 ProjectAccess::require_project，不能认为进过一次项目就不再检查。
不提供 POST /v1/projects、独立项目同步 API 或项目编辑/删除 API。

### A80646 的全项目访问

已认证账号 `A80646`（忽略大小写）可访问同一禅道实例下所有已同步到 NexoFolio 的项目，包括以后新增同步的项目，无需加入项目成员。它沿用现有项目功能权限；这是 NexoFolio 的本地访问策略，不改变禅道成员或禅道本身的权限。

策略只读取已认证用户在数据库中保存的账号，不接受请求中自报的账号，也不会向 `user_project_access` 补写成员关系。该账号不依赖成员权限同步成功，仍需正常登录并保持账号启用；停用或改名后会在后续校验中立即失效。其他账号仍遵循原有同步关系和权限未知时拒绝访问的规则，跨禅道实例仍不可访问。

页面、环境读写、MCP 项目/环境发现、知识读取和凭证页的可访问项目数统一使用 access 模块内的实时权限视图。发布时须先用新版本执行 access 数据库迁移 `202610100001_project_access_policy.sql`，再启动新 API；不需要修改前端或添加账号成员记录。

## 网页全局只读

独立业务项目和仅限指定管理账号的创建、清空接口，见 `docs/architecture/0014-local-projects-and-content-reset.md`。本地项目通过 `effective_project_access` 继承真实禅道项目的授权，网页操作与 MCP 使用同一判断；普通项目成员不获得创建或清空权限。

网页可通过独立 `/v1/public/projects/{project_id}/document` 和公开接口详情路由读取正式文档，不要求加入项目。`ProjectDocuments` 只提供当前配置实例下的项目基本信息和环境，不创建成员关系，也不参与 `ProjectAccess` / `McpProjectAccess` 的放行判断。

原有成员 API、真实样本、观测原值、候选结果和所有修改操作继续要求现有身份与项目权限。MCP 不使用公开文档策略；网页可以只读不代表 MCP 有权读取。实现说明见 `docs/architecture/0013-public-document-reading.md`。

## 验证

管理中心已注册用户的只读端口 `UserDirectory` 与 `/v1/user-management/users` 见 `docs/architecture/0016-registered-user-directory.md`。它只查询当前实例中首次登录已留档的用户和展示资料，沿用管理中心服务端权限，不涉及用户修改、MCP 或公开文档放行。

- 常规：cargo test --workspace --locked。
- 隔离真实 PostgreSQL：设置 TEST_DATABASE_URL 后执行 cargo test -p nexofolio-backend --test access_flow --test postgres --locked -- --ignored。
- access_flow 通过真实 HTTP 调用本地模拟禅道和 Rust API，并验证数据库持久化、完整分页、失败回退、并发复用、过期换新、超级密码、列表可见/详情拒绝和服务停机后本地访问。
- 实际用户提供的禅道账号另行完成真实外部端到端验收；模拟权限用例不冒充真实多账号权限隔离验收。
