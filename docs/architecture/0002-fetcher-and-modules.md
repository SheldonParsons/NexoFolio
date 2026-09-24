# 0002 插件改造计划与后端模块划分

状态：已确认（2026-09-24），取代 `0001-restart.md`。阶段 0 已完成，见文末"阶段 0 交付记录"。

---

## 第一部分：公开收集合同（插件和后端共同遵守）

插件改造和后端 intake 模块都以这份合同为准，所以放在最前面。合同文件（JSON Schema + fixtures）以后端仓库的 [`contracts/collect/v1`](../../contracts/collect/v1/README.md) 为准，插件用现有的 `scripts/build-ingestion-contract.mjs` 复制并校验 SHA-256。本节的示例是示意，字段细节以 schema 为准。

### 1.1 端点

```
POST /v1/collect/batches        不需要 Authorization
```

- 调用方必须在请求体中填写 `platform`，例如 `nexofolio-fetcher`、`swagger-sync`。它只用来标记来源、做统计和限流，不代表身份，也不授予权限。
- 批次上限：50 条记录、整批 8 MiB、单条 4 MiB（与插件现有上限一致）。超过上限时，整批返回 413。
- 不再提供能力协商接口：上限直接写在合同里，版本由 `kind` 加 `version` 表达。

### 1.2 批次

```json
{
  "batch_id": "0b7a3c3e-5c55-4d52-9a57-7b2f3f0e4a11",
  "platform": "nexofolio-fetcher",
  "platform_version": "0.2.0",
  "target": { "project_id": "…", "environment": { "id": "…" }, "site": { "origin": "https://shop.example.com", "prefix": "/" } },
  "records": [ ... ]
}
```

| 字段 | 必填 | 用途 |
|---|---|---|
| `batch_id` | 是 | UUID，重试时保持不变。服务端在一段时间内记住已处理的批次，同一批次重发时直接返回原回执，不会重复计数。这就是全部的幂等规则，调用方只需要"生成一次 UUID、重试时复用"。 |
| `platform` | 是 | 来源标识 |
| `platform_version` | 否 | 用于排查是哪个客户端版本产生的数据 |
| `target` | 是 | 这批记录属于哪个项目、哪个环境，以及从哪个站点入口采集，见 1.4 |
| `records` | 是 | 1 到 50 条 |

### 1.3 记录（统一信封）

```json
{
  "id": "5d1f...",
  "kind": "http_exchange",
  "version": 1,
  "observed_at": "2026-09-24T08:00:00.123Z",
  "context": { ... },
  "payload": { ... }
}
```

`context` 可选。只有浏览器类客户端提供得了，后端的任何必需逻辑都不依赖它。

第一阶段只开放两种 kind：

**① `http_exchange`：实际发生过的一次调用**（来自插件、HAR 导入、代理等）

```json
{
  "request": {
    "method": "GET",
    "url": "https://test.crm.com/api/customer/1001/contacts?page=1",
    "headers": [["accept", "application/json"]],
    "body": { "state": "none" }
  },
  "response": {
    "status": 200,
    "headers": [["content-type", "application/json"]],
    "body": { "state": "full", "media_type": "application/json", "encoding": "utf8", "content": "{\"list\":[]}" }
  }
}
```

- `response` 可以不传，表示没有采集到响应。
- `body.state` 只有四个取值：
  - `full`：内容完整。
  - `truncated`：内容被截断，但前面部分可信。
  - `unreadable`：有内容，但读不到（超时、流式、跨域等），原因写在 `note` 里。
  - `none`：确实没有 body。
- 只有 `full` 的内容会参与"字段是否存在"的统计；`truncated` 只能用来补充结构，不能证明某个字段不存在。

**② `http_declaration`：声明出来的接口结构**（来自 Swagger/OpenAPI 同步、手工录入）

```json
{
  "method": "POST",
  "path": "/customer/{id}/contacts",
  "summary": "新增联系人",
  "request": {
    "path_params": { "id": { "type": "integer" } },
    "query": {},
    "body_schema": { "type": "object", "required": ["name"], "properties": { "name": { "type": "string" }, "phone": { "type": "string" } } },
    "example": null
  },
  "responses": {}
}
```

- 只有结构、没有示例、没有响应都是合法的。
- 路径模板由声明方直接给出，不需要推断。
- 声明中的 `required` 是字段"可选/必填"最直接的依据。

**`context`（仅浏览器提供，全部可选）**

```json
{
  "page_url": "https://test.crm.com/admin/#/customer/list",
  "page_title": "客户列表",
  "page_id": "…", "frame_id": "…", "view_id": "…",
  "interaction_id": "…", "seq": 42
}
```

第一阶段后端只保存 `context`，不做推断。等以后做"参数来源关系""页面与接口对应"时才会使用。

**以后的 kind（现在不开放，只预留位置）**

| kind | 作用阶段 | 作用 |
|---|---|---|
| `page_context` | 整理 | 页面标题、面包屑、区域名，帮助 LLM 给接口起名、组织目录 |
| `ui_interaction` / `ui_snapshot` | 整理 | 控件标签和取值的对应关系，用于枚举说明 |
| `screenshot` | 整理 | 以哈希引用单独上传的图片（二进制不放进 JSON） |
| `openapi_document` | 录入 | 整份文档上传，由服务端拆成多条 `http_declaration` |

新增 kind 只需要改 intake 模块内部和合同 schema，不影响其他模块，见第三部分。

### 1.4 记录归属到哪个项目和环境

归属由客户端决定，服务端只校验，不猜测。每个批次都必须带 `target`。

这里要把三件事分开看，它们在旧稿里混在了一起：

| 概念 | 例子 | 作用 | 谁提供 |
|---|---|---|---|
| 站点入口（site） | `https://shop.example.com` | 记录"用户是在哪个页面访问到这个项目、这个环境的"，作为文档里的环境标注 | 插件提供；Swagger 可以不提供 |
| 服务地址（API base） | `https://shop.example.com/api`、`https://api.example.com` | 接口实际请求到的地址，作为环境标注 | 不需要客户端声明，observe 从 `request.url` 自动统计 |
| 接口身份 | 商用订单 · `GET /order/{id}` | 去重、建档 | observe 计算：项目 + 方法 + 路径模板 |

**站点入口只是标注，不参与接口身份的计算。** 所以 Swagger 没有域名，并不影响接口建档，只是少了一条"访问入口"标注。

#### 插件怎么填 target

插件现在的流程保持不变：用户第一次打开一个陌生页面时，自己选择"站点范围 → 项目 → 环境"。

区别在于这条映射改为保存在服务端的**站点登记表**里：

```
https://shop.example.com  /  →  商用订单 · 正式环境
```

- 插件打开页面时，先按"最长前缀"查询服务端登记表。
  - 查到了：直接使用，用户无感。另一台电脑、另一个同事打开同一个站点时，也不需要再绑定一次。
  - 查不到：才弹出现有的选择界面，选择结果写回登记表。
- 规则：
  - 一个站点范围只对应一个"项目 + 环境"。
  - 一个项目可以登记任意多个站点，多个互不相关的站点放进同一个项目也可以。
- 上传时，插件在 `target` 中填写**采集那一刻**使用的映射：

```json
{ "project_id": "…", "environment": { "id": "…" }, "site": { "origin": "https://shop.example.com", "prefix": "/" } }
```

服务端只校验项目存在、环境属于该项目，然后把站点入口记为这个环境的一条标注。

#### Swagger 类来源怎么填 target

```json
{ "project_id": "…", "environment": { "name": "正式环境" }, "site": null, "source_url": "https://shop.example.com/v3/api-docs" }
```

- `site` 可以缺省。Swagger 文档中有 `servers` 时，把它记为服务地址标注；没有就不记。
- `environment` 也可以缺省，因为 `http_declaration` 描述的是接口契约本身，不属于某个环境。缺省时，这份声明挂在项目级别，对所有环境都有效。
- 带 `environment.name` 时按名称查找，不存在就创建。
- `http_exchange` 必须带环境：实际流量一定发生在某个环境里。
- `source_url` 可选，用来标注声明来自哪份文档，也可以用于以后的增量同步。

#### 路径前缀怎么处理

接口身份使用路径模板，但同一个接口在不同环境可能挂在不同前缀下，例如正式环境是 `/api/order/list`，测试环境是 `/gateway/order/list`。

处理方式：

- observe 按"环境 × 服务地址"记录一个可选的 base path，默认为空，也就是完整路径直接参与模板计算。
- 如果 observe 发现两个环境下有大量路径尾部相同、结构也相同，只是前缀不同，就自动设定 base path，并合并对应的接口（发出合并事件，保留旧 ID 的别名）。
- 这个设定可以人工修改，但默认不需要人参与。

#### 第三方请求怎么办

一个页面上的请求不全是这个项目的，比如统计、埋点、地图 SDK。

- 插件不做过滤判断：绑定页面上发出的所有 fetch/XHR 都上传。过滤规则放在服务端，以后改起来不用重新发布插件。
- observe 按以下顺序判断一个服务地址是否属于本项目：
  1. **项目已知的服务地址**：被确认过或被人提升过的地址，直接算本项目的，与数据来源无关。
  2. **主域名相同**（可注册域名，按公共后缀列表计算，例如 `shop.example.com` 和 `api.example.com` 都是 `example.com`）：只在拿得到页面地址时使用。页面地址依次取批次的 `site`、请求头 `Origin`、请求头 `Referer`。
  3. **与域名无关的外部特征**：同一个地址在多个互不相关的项目里都出现，或者响应不是 JSON（1×1 gif、JS 脚本等），满足任一条即判为外部服务。
  4. **以上都判断不了**：正常建档。宁可多收；多收的可以一键降为外部服务。
- 判为外部服务的地址：它的接口仍然在项目唯一的那份文档里，照常建档、进入"待分类"、参与检索，只多一条"外部服务"标注，路径模板带上地址以免和本项目接口撞身份。结论可以人工修改，也可以预先登记，见 [`0003`](0003-stage-2-plan.md) 4.2。
- 声明来源（Swagger）不存在第三方的问题，全部正常建档。

#### 收集接口的鉴权

收集接口暂时不做鉴权，任何人都可以访问。以后会加一个专门的收集 Token，与账号登录 Token 无关。

站点登记表的写入需要登录，因为用户要从项目列表中选择项目，这沿用插件现有的登录；读取登记表不需要登录。

### 1.5 回执

```json
{ "batch_id": "…", "accepted": 18, "rejected": [ { "index": 3, "id": "…", "reason": "INVALID_RECORD" } ] }
```

- 回执中每条记录只有两种结果：
  - 被接收：计入 `accepted`，不论它带来了新结构，还是只让已有结构的计数加一，调用方都可以删掉本地副本。
  - 被拒绝：列在 `rejected` 里，都是不可重试的错误，调用方丢弃，或保留下来给人看。
- 可重试的情况不会逐条返回：服务端直接用 HTTP 429/503 拒绝整批，调用方用同一个 `batch_id` 重发即可。

---

## 第二部分：NexoFolio Fetcher 改造计划

### 2.1 现状

- **采集层**：可以保留。MAIN world 包装 fetch/XHR，由后台 manager 负责；有 1 MiB 上限和各种截断/不可读标记，以及上下文和交互采样。
- **绑定层**：只存在插件本地，按服务地址和用户 UUID 隔离，允许一个范围绑定多个项目，环境通过登录 Token 调用后端创建。
- **上传层**：
  - 队列：IndexedDB 队列 `nexofolio-upload-v1`。
  - 调用：先用登录 Token 调用 `/v1/ingestion/capabilities?schema_version=3`，再调用 `/v1/ingestion/batches`。
  - 回执：逐条核对回执。
  - 其他：同时维护 v1/v2/v3 三套 wire 格式，以及截图资产上传。
- **问题**：后端 main 上已经没有 `/v1/ingestion/*`，插件现在上传一定失败，数据只会在本地堆积。
- **提醒**：插件仓库目前有 52 处未提交的修改，包括整个 NexoFolio 品牌迁移、`src/upload`、`src/contracts` 等。建议你在改造开始前先自己提交一次，作为基线；按 AGENTS.md 的规定，我不会替你提交。

### 2.2 改造项

| # | 改动 | 涉及文件 | 说明 |
|---|---|---|---|
| F1 | 替换合同 | `src/contracts/capture/*`、`src/contracts/ingestion/*` 换成 `src/contracts/collect/*` | 从后端复制 schema 和 fixtures，由构建脚本校验哈希并生成 AJV 校验器。删除 v1/v2/v3 和 legacy 校验器。 |
| F2 | 新的上传客户端 | `src/api/nexofolio/ingestion.ts`、`capture.ts` 换成 `collect.ts` | 调用 `POST /v1/collect/batches`，不带 Authorization，body 中写 `platform`。删除能力协商和资产上传。 |
| F3 | 转换器 | `src/upload/converter.ts` | 把 Observation 转成新信封（映射见下方）。 |
| F4 | 组批规则 | `src/upload/manager.ts`、`contracts.ts` | 保持现有做法，按"服务地址 + 项目 + 环境 + 站点"分组，每批带上 `target`。回执校验简化为：用 `batch_id` 对应回执，`rejected` 里的记录标记失败，其余标记为已确认。 |
| F5 | 上传与登录解耦 | `src/upload/manager.ts` | Token 过期不再阻塞上传，队列照常发送；只有写入站点登记时才需要登录。 |
| F6 | 站点登记改为服务端数据 | `src/platforms/*` | 本地规则变成服务端登记表的缓存。打开页面时按最长前缀查询服务端；查不到时，才弹出现有的"站点范围 → 项目 → 环境"选择界面，选择结果写回服务端。去掉"同一范围多个项目"和 `ambiguous` 状态。 |
| F7 | 不做过滤 | `src/capture/manager.ts` | 绑定页面上的所有 fetch/XHR 照常上传，第三方请求由服务端识别为"外部服务"。 |
| F8 | 暂停其他 kind | `src/evidence/*` | 保留页面上下文和交互采样的代码，但第一阶段不上传 `page_context`、`ui_snapshot`、`interaction`，截图资产队列也停用。`http_exchange.context` 照常附带。 |
| F9 | 旧数据迁移 | `src/migrations/nexofolio.ts` | 本地绑定：一个范围只有一个项目的，登录后写入服务端登记表；一个范围有多个项目的，在界面上提示用户选一个。队列：还带着原始 Observation 的条目用新转换器重转；只有旧 wire 记录的条目，从 v3 字段映射；映射失败的标记为失败但保留，不自动删除。 |
| F10 | 界面文案 | `CaptureRow`、`PlatformPanel`、`AccountView` | 状态改为：采集中 / 待上传 / 已确认 / 被拒绝（附原因）。 |

**F3 的状态映射**

| 插件现有状态 | 新合同 |
|---|---|
| `complete` | `full` |
| `truncated` | `truncated` |
| `unreadable`、`timeout` | `unreadable`，原因写入 `note` |
| `none` | `none` |

以下字段放进 `context`：`sourcePage`、`transport`、`frame` 和现有的 EvidenceContext。

### 2.3 保持不变

- 采集钩子、上限、完整性标记。
- IndexedDB 队列本身（容量、持久化、退避重试）。
- 按主机申请的可选权限。
- 原始数据不脱敏；队列内容和凭据不写日志。

### 2.4 顺序和验收

1. 在后端仓库写定合同 schema 和 fixtures（第一部分）。
2. 插件完成 F1–F10，执行 `npm run build`。按 AGENTS.md 的规定不跑测试，也不做浏览器 QA，由你在真实项目里加载 `.output/chrome-mv3` 验证。
3. 后端完成 access 中的站点登记表和 intake/observe 模块（第三部分的阶段 1–2）之后，才能真正联调。

   在这之前，插件上传会收到 404，数据保留在队列里，不会丢失。

---

## 第三部分：后端模块划分

### 3.1 总体结构

```
crates/contracts     通用合同层：只有类型、trait、错误和事件，不含任何 IO
crates/common        ID、Secret 等最基础的工具（已有）

modules/access       身份、项目、权限、环境、前缀绑定、MCP Token
modules/intake       录入：公开协议、来源格式、归属解析、批次幂等
modules/observe      观测事实：接口身份、路径模板、合并结构、指纹统计、字段标签、样本
modules/knowledge    知识：目录树、接口位置、描述、字段说明、关联、修订、审核状态
modules/curate       整理：LLM 内容补全、目录整理循环、预算
modules/view         视图：把事实和知识组装成文档，提供浏览和关键字检索

apps/backend         装配根：HTTP 路由、MCP 工具、worker、CLI、下载接口
```

每个模块内部仍然采用 `core`（纯逻辑）+ `adapter`（数据库、外部服务）两层。

### 3.2 隔离规则

以下规则由架构测试强制检查，不靠约定。

1. **依赖方向**：模块只能依赖 `contracts` 和 `common`。模块之间不允许有任何 Cargo 依赖，只有 `apps/backend` 可以同时依赖多个模块。这条沿用并扩展现有的 `tests/architecture/dependencies.rs`。
2. **数据归属**：
   - 每个模块只使用自己的 PostgreSQL schema（例如 `observe.*`、`knowledge.*`），迁移文件放在模块自己的目录。
   - 架构测试会扫描迁移文件和 SQL，禁止一个模块引用另一个模块的 schema。
   - 不做跨模块的外键和 JOIN。跨模块只通过 ID 引用。
3. **交互方式只有两种**，都定义在 `contracts` 中：
   - **同步调用**：一个模块实现某个 trait，另一个模块通过这个 trait 调用；由 apps 注入具体实现。
   - **变更流**：事件发出方把事件写进自己的 outbox 表，并通过 contracts 中的 `ChangeFeed` trait 按游标对外提供；订阅方记录自己读到的游标。

     这样做的好处：发出方不需要知道谁在订阅；订阅方失败了可以从游标处重放。
4. **合同变更可追踪**：
   - 合同中的类型带版本号。修改 trait 或类型时，编译器会指出所有受影响的实现方和调用方。
   - 架构测试维护一张"谁实现、谁使用"的清单，合同改动必须同步更新这张清单。
5. **可以单独测试**：`contracts` 带 `testing` feature，提供两类东西：
   - 每个 trait 的内存假实现：模块测试时用它代替其他模块。
   - 一套一致性测试：每个真实实现都必须通过同一套测试。

   只要一个模块在内部修改后仍然通过自己的测试和一致性测试，其他模块就不受影响。

### 3.3 合同层里有什么

| 合同 | 定义内容 | 实现方 | 使用方 |
|---|---|---|---|
| `scope` | `TargetResolver`：校验 target（项目存在、环境属于该项目），按名称确保环境存在，登记站点入口；`SiteRegistry`：按最长前缀查询站点，以及写入站点 | access | intake、apps（给插件用的站点查询和写入接口） |
| `permission` | `ProjectGate`：某个主体能否读或写某个项目 | access | apps（HTTP/MCP 在请求入口统一检查） |
| `observation` | 标准化观测 `CanonicalObservation`（与来源无关）；`ObservationSink` | observe | intake |
| `endpoint` | 接口事实的只读模型（身份、各环境的合并结构、字段标签、样本摘要）；`EndpointReader`；变更事件 `EndpointEvent`（新增、结构变化、合并、拆分） | observe | knowledge、curate、view |
| `knowledge` | 文档读取模型；写入命令（放入目录、改描述、加关联、移动目录……，每条都带作者和来源）；`KnowledgeStore`；变更事件 | knowledge | curate、view、apps（MCP 修改） |
| `curation` | 整理任务状态、预算；`LanguageModel` 端口 | curate 实现任务，模型端口由 apps 注入适配器 | apps |
| `view` | 浏览树节点、接口文档、检索结果；`DocumentView` | view | apps（HTTP、MCP） |

### 3.4 为什么这样划分

这次划分遵循一条原则：**变化的原因不同，就分开；两个部分总是一起改，就合并。** 下面逐个说明各模块的变化原因。

**access**：变化原因是"谁能进来、属于哪个项目"。

- 禅道、会话、项目同步本来就在这里。
- 站点登记表放在这里，因为它和项目、环境是同一类归属数据；环境改名不影响登记，因为登记表存的是环境 ID。
- 顺带要清理：
  - `PlatformAccess` 太臃肿，按用途拆成几个 trait。
  - 删除死合同。
  - 补上 MCP 专用 Token（推迟到阶段 3，和 MCP 只读工具一起做）。

**intake 与 observe 分开**：两者变化的原因完全不同。

- intake 面对外部世界。新增一种客户端（HAR、Postman、OpenAPI 整份文档）、新增一种 kind、调整上限或限流，都只改 intake。它的输出是与来源无关的 `CanonicalObservation`，其中 `http_exchange` 和 `http_declaration` 都会转成这种统一形式。
- observe 面对的是"结构事实怎么算"。路径模板识别、JSON 结构合并、指纹统计、标签规则，这些是最常需要调优的算法。改这些不应该碰到协议。
- 分开之后，这两部分可以各自单独测试：
  - intake：给它一批原始 JSON，在假 sink 里检查它产出的标准化观测。
  - observe：给它一串标准化观测，检查最后得到的接口和标签。这正好可以用真实采集备份来重放。
- "入库前去重"在 observe 的入口完成：先算出指纹；如果指纹已知，只累加计数，不写入原始数据。

**observe 与 knowledge 分开**：一边是机器算出的事实，一边是人和 LLM 写的解释。

- 事实可以重算。比如改进路径模板算法后，可以用样本重放，重新生成事实。
- 知识不能重算，每次修改都要有修订、来源和审核状态。
- 如果把两者混在一起，重算事实就可能冲掉人写的描述。
- 两者之间只通过接口 ID 连接：
  - observe 合并或拆分接口时发出事件，knowledge 订阅后迁移自己的引用。
  - 字段说明通过"字段路径"关联到事实；字段消失后说明仍然保留，并标记为"对应字段已不存在"。

**curate 单独成模块**：它是唯一调用 LLM 的地方，成本高、耗时长、最需要反复实验。

- 它只读 observe 和 knowledge 的只读模型，只通过 knowledge 的写入命令写入，写入内容带上"待审核"状态和来源标记。
- 因此以后把"人工审核"改成"自动生效"，只是 knowledge 里一条策略的变化，curate 不用改。
- 更换模型、调整 prompt、调整"摘要 → 诊断 → 执行 → 校验"循环或预算，都只改 curate。
- 如果 curate 关闭或出故障，录入和检索照常工作。

**view 单独成模块**：读取路径和写入路径的变化节奏不同。

- 给 agent 看的排版、目录浏览方式、检索排序、以后加上的 embedding，都会频繁调整，但它们不应该影响数据正确性。
- view 不写入任何事实或知识，只维护自己的读取模型或检索索引，而且这些都可以从 observe 和 knowledge 重建。
- "待分类"虚拟目录在这里计算：observe 里有、但 knowledge 里还没有安排位置的接口，都显示在"待分类"下。因此新接口一被 observe 接收，就能立即被浏览和检索到，不需要等 knowledge 或 curate。

**apps/backend 只做装配**：HTTP 路由、MCP 工具定义、worker 调度、配置。

- MCP 的"直接修改"只是把请求转成 knowledge 的写入命令，apps 里不写业务逻辑。
- 下载接口暂时留在 apps，因为它和知识库没有关系。以后如果变大，可以独立成模块。

**没有继续拆细的部分**：

- 路径模板识别没有单独成模块：它和结构合并共享同一份统计数据，总是一起改。
- 目录整理和内容补全没有拆成两个模块：它们共享 LLM 端口、预算和任务调度。在 curate 内部，它们是两条独立流水线。

### 3.5 一条接口调用走过哪些模块

以插件上传的一次调用为例：

1. 插件发送 `http_exchange`。apps 收到请求，交给 intake。
2. intake 校验 schema，然后通过 `TargetResolver`（由 access 实现）校验 target：商用订单 / 正式环境，站点入口 `https://shop.example.com`。
3. intake 生成 `CanonicalObservation`，交给 `ObservationSink`（由 observe 实现）。
4. observe 处理这条观测：
   - 判断服务地址：同一主域名的按相对路径建档，外部服务的接口带上地址建档，都在同一份文档里。
   - 识别路径模板：`/order/{id}/items`。
   - 计算结构指纹。
   - 如果指纹已知，只更新计数和最后出现时间，结束。
   - 如果是新接口或新结构，合并结构、更新标签、保留一个样本，并在 outbox 中写入 `EndpointEvent`。
5. 调用方收到回执。
6. view 从变更流读到这个事件，更新读取模型。这时新接口已经出现在"待分类"下，MCP 可以检索到。
7. curate 从变更流读到事件，把接口加入内容补全的待办。当新接口积累到一定程度，或到了预定时间，curate 发起一轮目录整理。产出通过 knowledge 的写入命令提交，状态为待审核。
8. 审核通过后，knowledge 发出事件，view 更新。

### 3.6 "接口更新了"还是"参数本来就可选"

这个问题由 observe 自动判断，不需要 LLM。

observe 对每个"接口 × 环境 × 字段路径"记录以下统计：出现次数、缺失次数、首次出现时间和最后出现时间，以及最近一次在出现和缺失之间切换的时间点。

只有 `full` 状态的 body 才会计入"缺失"：截断或不可读的 body 不能证明字段不存在。

判定规则如下：

| 观察到的模式 | 标签 |
|---|---|
| 同一时间段里，有的调用有这个字段，有的没有，反复交替 | 可选 |
| 某个时间点之前一直有，之后连续 N 次调用（且超过 M 天）都没有 | 疑似移除，属于接口更新 |
| 某个时间点之前一直没有，之后一直有 | 新增，属于接口更新 |
| A 环境一直有，B 环境一直没有 | 环境差异 |
| 声明来源（Swagger）给出了 required | 以声明为准；实际流量与声明矛盾时，才进入待裁决 |
| 列表为空或有值 | 不属于这个问题：空列表的元素结构记为"未知"，与已知结构合并，不产生冲突，也不产生标签 |
| 同一字段的类型变了（例如字符串变成对象） | 有明确的切换时间点时，判为更新；同一时间段里两种类型交替出现时，判为多态字段 |
| 样本太少 | 标记为"观察中"，不进入待裁决 |

N 和 M 按项目调用频率设定默认值，可以配置。

进入人工待裁决的只有一种情况：两个可信来源互相矛盾。其余情况都由规则自动判定，而且规则集中在 observe 内部，调整规则不影响其他模块。

### 3.7 开发顺序

每个阶段都可以单独验收：

| 阶段 | 内容 | 单独验收方式 |
|---|---|---|
| 0 | `contracts` 骨架和扩展后的架构测试 | 架构测试能拦住违规依赖和跨 schema 的 SQL |
| 1 | access 整理：拆分 trait、删除死合同、增加站点登记表 | 现有测试加上站点最长前缀匹配测试 |
| 2 | intake + observe，以及插件改造 | 用真实插件录入；用历史备份重放；检查接口数量、模板、标签是否符合预期 |
| 3 | view + MCP 只读 | agent 能浏览"待分类"并检索接口 |
| 4 | knowledge + MCP 修改 | 修改有修订记录、可以回退、可以审核 |
| 5 | curate | 对真实项目运行一轮，看描述和目录的质量，同时控制时间和调用次数 |

---

## 确认记录

已确认（2026-09-24）：

- 收集接口暂时不鉴权，以后使用专门的收集 Token，不使用账号 Token。
- 一个项目可以有多个站点入口，互不相关的站点也可以放进同一个项目。
- 第一阶段只开放两种 kind；插件暂停上传其他 kind 和截图。
- 模块划分为 access / intake / observe / knowledge / curate / view，加上 apps 和 contracts。

已确认（第二轮）：

- 归属按 1.4 的新方案处理：
  - 插件按页面选择站点范围，映射保存在服务端登记表，各客户端共享。
  - 站点入口只是环境标注，不参与接口身份。
  - Swagger 可以不提供站点和环境。
  - 一个站点范围只对应一个"项目 + 环境"。
- 第三方请求由服务端按多个信号依次判断，插件不做过滤。
- 不同环境的路径前缀差异，由 observe 自动识别并合并接口。
- 接口身份固定为"项目 + 方法 + 路径模板"，不含域名和环境。同一项目内的相同身份一律视为同一个接口，各环境的差异作为标注；不做"疑似不同系统"检测，也不设"服务"层。不同的平台应该绑定到不同的项目，这由用户在绑定站点时负责。

---

## 阶段 0 交付记录（2026-09-24，分支 `restart/stage-0`）

| 交付物 | 位置 | 验收 |
|---|---|---|
| 收集合同 v1 | `contracts/collect/v1`：batch / receipt / error 三个 schema、35 个虚构数据 fixtures、`manifest.json`、README | `cargo test -p nexofolio-backend --test contracts`：合法 fixtures 必须通过，非法 fixtures 必须失败，manifest 哈希必须与磁盘一致，上限和 kind 列表必须与 schema 一致 |
| 通用合同层 | `crates/contracts`（`nexofolio-contracts`）：`scope`（`TargetResolver`、`SiteRegistry`、`SiteScope`）、`observation`（`CanonicalObservation`、`ObservationSink`） | 单元测试；`testing` feature 提供 `InMemoryScope`、`RecordingSink` 和三套一致性测试，假实现本身先通过 |
| 依赖方向检查 | `tests/architecture/dependencies.rs` | 模块登记表 `MODULES`；按 common / contracts / 模块 contracts / core / adapter / backend 角色检查；模块之间任何层都不允许依赖；带故意违规的用例 |
| 数据归属检查 | `tests/architecture/sql_ownership.rs` | 扫描模块的迁移文件和 Rust 字符串中的 SQL：只能在自己的 schema 建表；不能引用其他模块的 schema、`public` 或其他模块的表；`modules/` 下不能有未登记目录；带故意违规的用例 |

与合同草稿的差异：环境名规则与 access 现有的 `validate_environment_name` 对齐（首尾不能有空白、不能有控制字符、区分大小写）。

留给后续阶段的事项：

- ~~access 的表目前仍在 `public`，架构测试用 `OWNS_PUBLIC` 暂时豁免。~~ 阶段 1 已完成，豁免已删除，见下方阶段 1 记录。
- ~~每个模块不能共用同一张 `_sqlx_migrations` 账本。~~ 阶段 1 已完成：每个模块的账本在自己的 schema 里。
- 新模块加入时，把名字加入 `tests/architecture/dependencies.rs` 的 `MODULES`。

---

## 阶段 1 交付记录（2026-09-24，分支 `restart/stage-0`）

| 交付物 | 位置 | 验收 |
|---|---|---|
| 每个模块一个 schema、一份账本 | `modules/access/adapter/src/postgres.rs`：连接的 `search_path` 只有 `access`；迁移器先建 schema，账本为 `access._sqlx_migrations` | `tests/integration/postgres.rs`：全部表在 `access`，`public` 里没有表，迁移可以重复执行 |
| 删除 legacy 账本 | 删除 `migrations/legacy`、`migrations/README.md`，以及 Dockerfile 和 `build.rs` 里对它的引用 | 迁移只剩模块自己的目录 |
| 数据归属检查收紧 | `tests/architecture/sql_ownership.rs`：删除 `OWNS_PUBLIC` 豁免；模块 SQL 不写 schema 前缀（由 `search_path` 决定），写了前缀就只能是自己的 schema；不能建其他 schema，不能引用 `public` | 带故意违规的用例 |
| 拆分 `PlatformAccess` | `modules/access/contracts`：`LoginStore`、`Sessions`、`ProjectAccess`、`Environments`、`McpTokenVerifier`，每个 trait 一个文件；`modules/access/adapter/src/store/` 每个 trait 一个实现文件；HTTP 层只拿到自己用到的 trait | 原有测试全部通过 |
| 删除死合同 | `ExternalAuthenticator`、`EmergencyAuthenticator`、`UserDirectory`、`ProjectSource`、`ProjectPermissionSource`、MCP 的 grant/scope/签发相关类型、`EnvironmentRef` 等；access contracts 不再依赖 `schemars` | 编译和 clippy 无警告 |
| 站点登记表和目标校验 | 迁移 `202609240001_sites.sql`：`sites`（一个站点范围对应一个项目 + 环境）、`environment_sites`（环境的采集站点标注）；`store/scope.rs` 实现 `TargetResolver`、`SiteRegistry`；`SiteScope::origin_of` | `modules/access/adapter/tests/scope.rs`：Postgres 实现通过两套一致性测试（包括最长前缀匹配），同一站点标注只记一次 |

说明：

- 迁到 `access` schema 没有写"搬表"迁移。已执行的两份迁移原样保留，因为 SQL 不写 schema 前缀，由连接的 `search_path` 决定落在哪里。本地库可以丢弃，所以旧库 `public` 下的表不迁数据、也不删除，新迁移不会再读它们。
- MCP grant 相关合同随死合同一起删除；`McpPrincipal` 只剩用户和 Token ID。MCP 专用 Token 推迟到阶段 3。
- 收集接口、插件使用的站点 HTTP 接口推迟到阶段 2；`TargetResolver`、`SiteRegistry` 的 Postgres 实现届时直接装配。

留给后续阶段的事项：

- 阶段 2 在 backend 里装配 `PostgresAccess` 作为 `TargetResolver` 和 `SiteRegistry`，并提供站点登记的 HTTP 接口，与插件对齐。
- 新模块沿用同样的做法：自己的 schema、`search_path`、账本表，并加入 `tests/architecture/dependencies.rs` 的 `MODULES`。
