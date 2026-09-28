# 0003 阶段 2 方案：intake、observe 与插件改造

状态：已确认（2026-09-24），2a、2b 已完成，见文末交付记录。依据 [`0002-fetcher-and-modules.md`](0002-fetcher-and-modules.md)。本文只写阶段 2 怎么做，0002 已确认的规则不再重复。

---

## 0. 拆分和顺序

阶段 2 拆成六步。每步单独验收，做完一步向你汇报，不自动进入下一步。

| 步 | 内容 | 验收 |
|---|---|---|
| 2a | 合同层补上 `endpoint`：接口 ID、事件、变更流、只读模型 | 单元测试；假实现通过一致性测试 |
| 2b | intake 模块、`POST /v1/collect/batches`、站点登记的 HTTP 接口 | 合同 fixtures 全部按预期通过或拒绝；幂等、限流、target 错误的集成测试 |
| 2c | observe 第一版 | 用构造的观测序列测试模板、结构合并、标签、幂等 |
| 2d | 插件 F1–F10 | 你在真实项目里加载插件，上传成功，回执正确 |
| 2e | 历史备份重放和整体验收 | 接口数量、模板、标签和旧系统对照，由你抽查 |
| 2f | 跨环境前缀自动合并、按取值多样性识别路径参数 | 看 2e 的数据再决定要不要做、怎么做 |

2b 做完后，后端就能收数据，插件可以开始联调。2c 之前，intake 先接一个只记录的 sink，方便单独测试。

---

## 1. 合同层新增（2a）

在 `crates/contracts` 增加 `endpoint` 模块，只放 observe 对外提供的东西：

- `EndpointId`（UUID，放在 `crates/common`）。
- `EndpointEvent { project_id, endpoint, change }`，`change` 只放阶段 2 真正会产生的三种：
  - `Created { method, path_template }`
  - `StructureChanged { environment_id }`
  - `MergedInto { into }`，用于声明到达后按前缀重新归并（见 4.3），以及地址结论改变
  拆分等事件等真正产生时再加。
- `ChangeFeed<E>`：`read(after: Cursor, limit) -> Vec<Change<E>>`。这个 trait 做成泛型，阶段 4 的 knowledge 可以直接复用。
- `EndpointReader`：只放验收用得到的两个方法，阶段 3 的 view 再按需扩展。
  - `list(project)`：接口列表，含方法、模板、各环境调用次数和最后出现时间。
  - `get(id)`：各环境的合并结构、字段标签、服务地址、声明来源。
- `ServiceAddresses`：列出项目的服务地址，以及写入人工结论（见 4.2）。
- `testing` feature 提供内存版 `ChangeFeed` 和一致性测试，覆盖三点：游标单调、从任意游标可以重放、`limit` 生效。

---

## 2. intake（2b）

### 2.1 结构

```
modules/intake/core      纯逻辑：解析、校验、转换、限流、提交流程
modules/intake/adapter   Postgres：intake schema，批次账本
```

intake 不单独建 contracts crate。它对外的合同就是 `contracts/collect/v1` 的 JSON。apps 只调用 core 暴露的一个服务：

```rust
Intake::submit(raw: &[u8]) -> Result<Receipt, CollectError>
```

它依赖四个端口，都由 apps 注入：`TargetResolver`（access）、`ObservationSink`（observe）、`BatchLedger`（intake adapter），以及一个时钟。

### 2.2 解析：用 serde 类型，不在运行时跑 JSON Schema

- wire 类型和 schema 一一对应，全部 `deny_unknown_fields`。
- 为了保证两者不走样，合同测试让 `contracts/collect/v1/fixtures` 里的每个文件都走一遍 intake 解析：
  - 合法的 fixture 必须被接收。
  - 非法的 fixture 必须以 fixture 期望的错误码被拒绝。
- 这样既省掉运行时校验的开销，又不会出现"schema 允许、代码拒绝"的偏差。

### 2.3 一个批次的处理流程

| 步骤 | 做什么 | 失败时 |
|---|---|---|
| 1 | 请求体上限 8 MiB（axum 的 body limit） | 413 `BATCH_TOO_LARGE` |
| 2 | 按 `platform` 限流（见 2.5） | 429 `RATE_LIMITED`，带 `Retry-After` |
| 3 | 解析外层：`batch_id`、`platform`、`target`、`records` 的数量 | 400 `INVALID_BATCH` |
| 4 | 查账本：同一 `batch_id` 内容相同，直接返回原回执；内容不同，拒绝 | 409 `BATCH_ID_REUSED` |
| 5 | `TargetResolver` 校验 target：批次里有 exchange 却没有环境时直接 400 | 404 `UNKNOWN_PROJECT` / `UNKNOWN_ENVIRONMENT`，503 |
| 6 | 逐条检查记录（单条 4 MiB、kind、version、payload），转成 `CanonicalObservation` | 不合格的记录进入 `rejected`，其余照常处理 |
| 7 | `ObservationSink::accept`，整批一次调用 | 503 `UNAVAILABLE` |
| 8 | 账本写入 `batch_id`、内容哈希、`platform`、回执、时间 | 503（客户端重发，见 2.4） |

内容哈希按规范化后的 JSON 计算（键排序、去掉空白），不按原始字节计算，这样客户端重新序列化也不会被误判为"内容不同"。

### 2.4 幂等：不加锁，靠 observe 按批次去重

第 7 步和第 8 步之间可能崩溃，同一批次也可能被并发重发。处理方式如下：

- observe 在处理一批观测的**同一个事务**里，先插入 `observe.seen_batches(batch_id)`。
  - 主键冲突，说明这批处理过，整个事务什么都不做，直接返回成功。
  - 并发的第二个事务会等第一个提交，然后撞上主键，同样什么都不做。
- 于是 intake 可以放心"至少交付一次"，实际效果是恰好一次，intake 这边不需要锁，也不需要"处理中"状态。
- `seen_batches` 和 intake 账本一样保留 7 天，由 worker 每小时清理一次。

### 2.5 限流

- 每个 `platform` 一个进程内令牌桶，默认每分钟 120 批，突发 30 批，可以配置。
- 目前后端只部署一个实例，进程内就够了。以后多实例部署，再换成数据库计数，只改 intake 内部。
- 限流在读账本之前执行，所以同一批次重发也会被限流。客户端按 `Retry-After` 重发即可，不会丢数据。

### 2.6 日志

日志只记这几项：`batch_id`、`platform`、记录数、接收数、拒绝的原因码、耗时。任何请求体、请求头和 target 以外的内容都不写进日志。

---

## 3. apps/backend 装配（2b）

| 接口 | 鉴权 | 说明 |
|---|---|---|
| `POST /v1/collect/batches` | 无 | 交给 `Intake::submit` |
| `GET /v1/sites/lookup?url=<页面地址>` | 无 | 按最长前缀返回 `{ site, project: {id, name}, environment: {id, name} }`，查不到返回 `null` |
| `PUT /v1/sites` | 登录 | 请求体 `{ site, project_id, environment_id }`；先用 `ProjectAccess` 确认当前用户能访问该项目，再调用 `SiteRegistry::bind` |

- 查询接口返回项目名和环境名，因为插件要把它们显示出来。代价是：知道站点地址的人，不登录也能看到项目名和环境名。见第 7 节第 4 问。
- 装配：
  - `PostgresAccess` 作为 `TargetResolver` 和 `SiteRegistry`。
  - observe 的 store 作为 `ObservationSink`、`EndpointReader`，以及 `ChangeFeed<EndpointEvent>`。
- worker：新增"清理 7 天前的批次账本和 `seen_batches`"，每小时一次。
- admin CLI：新增 `admin observe report --project <id>`，打印接口列表、模板、各环境字段标签和待裁决项。它是阶段 2 的验收工具，阶段 3 有了 view 之后可以删掉。
- 把 `intake`、`observe` 加进 `tests/architecture/dependencies.rs` 的 `MODULES`。

---

## 4. observe 第一版（2c）

```
modules/observe/core      纯逻辑：地址归属、路径模板、结构提取、指纹、标签
modules/observe/adapter   Postgres：observe schema
```

### 4.1 处理一条 exchange

一批观测在一个事务里处理完，包括前面说的 `seen_batches`。对每条 exchange 依次做以下几步：

1. **服务地址** = `request.url` 的 scheme + host + port，判断是否属于本项目（4.2）。
2. **路径模板**（4.3）。query 不参与接口身份。
3. **结构提取**。
   - 分三部分提取：query 的键、请求 body、响应（状态码 + body）。
   - JSON body 递归提取形状：
     - 对象：键 → 形状。
     - 数组：所有元素的形状合并；空数组的元素形状记为"未知"。
     - 标量：只记 string、number、boolean、null。第一版不推断格式。
   - 表单 body 只取键。
   - 其他媒体类型只记媒体类型。
   - 每个 body 记一个标记：它能不能证明字段缺失（只有 `full` 能）。
4. **指纹** = 哈希（接口、环境、状态码、各部分形状、各部分的证明标记）。
5. **指纹已知**：调用次数加一，更新最后出现时间，结束。原始数据不落库。
6. **指纹未知**：
   - 写入指纹，连同它的形状和首次出现时间。
   - 保存这条观测作为这个指纹的样本。
   - 重算"接口 × 环境"的合并结构：各指纹形状的并集；"未知"与任何形状合并，结果都是那个形状。
   - 在 outbox 里写 `StructureChanged`。如果是新接口，还要写 `Created`。

### 4.2 服务地址归属

表 `service_addresses(project_id, address, verdict, decided_by, reason)`。`verdict` 取 `own` 或 `external`，`decided_by` 取 `auto` 或 `manual`。

判断顺序与 0002 相同：

1. 有人工结论的，以人工结论为准。
2. 取页面地址，依次用批次的 `site.origin`、请求头 `Origin`、请求头 `Referer`。取到后，用公共后缀列表（`psl` crate，编译时内置）计算可注册域名。两边相同，判为 `own`。
3. 以下任一条成立，判为 `external`：
   - 这个地址在 3 个以上项目里出现过。
   - 响应既不是 JSON 媒体类型，也解析不出 JSON。

   注意第 3 条排在第 2 条后面，所以同域名下的文件下载接口不会被误判为外部服务。
4. 都判断不了的，判为 `own`，宁可多收。

**外部地址不单独存放，和项目自己的接口放在同一套数据里，同一份文档。** 区别只在路径模板怎么写：

- 本项目地址：模板是相对路径，例如 `POST /collect`。这就是 0002 的"项目 + 方法 + 路径模板"。
- 外部地址：模板带上地址，写成绝对形式，例如 `POST https://analytics.example.net/collect`。

这样接口身份的规则不变，还是"项目 + 方法 + 路径模板"，只是外部接口的模板本身带域名。所以第三方的 `/collect` 不会和项目自己的 `/collect` 撞成同一个接口。

项目从始至终只有一份文档。外部接口就是这份文档里的普通接口，没有第二份文档、第二套存储或第二种接口类型，也不降级：

- 去重、统计、打标签、留样本、发事件，和其他接口完全一样。
- 进入"待分类"，参与目录整理和检索，排序也不降低。
- 唯一的区别是一条"外部服务"标注，和环境、站点标注属于同一类信息。

地址结论改变时，复用 4.3 的归并机制，不需要重放：

- 外部改为本项目：`POST https://analytics.example.net/collect` 归并到 `POST /collect`（相对模板已存在就合并，不存在就改名），旧 ID 保留为别名，发出 `MergedInto`。
- 本项目改为外部：方向相反，做法相同。

**地址可以预先登记。** 提供两个接口，都需要登录，并检查项目权限：

| 接口 | 说明 |
|---|---|
| `GET /v1/projects/{id}/service-addresses` | 列出项目的服务地址：结论、自动还是人工、判定理由、调用次数 |
| `PUT /v1/projects/{id}/service-addresses` | 请求体 `{ address, verdict: own \| external }`，写入人工结论。地址还没出现过也可以写，之后的流量从第一条起就按这个结论处理 |

这两个接口由 observe 在合同层提供一个 `ServiceAddresses` trait，apps 负责 HTTP 和权限检查。界面和 MCP 上的操作，放到阶段 3、4 再接。

### 4.3 路径模板

第一版按这个顺序识别：

1. **声明优先**。项目里如果有声明的模板，而且它能匹配这条路径的**尾部**（例如声明 `/customer/{id}`，实际路径 `/api/customer/1001`）：
   - 直接用声明的模板和参数名。
   - 把多出来的开头部分（`/api`）记为"环境 × 服务地址"的 base path。
   - 以后这个地址上的路径都先去掉 base path，再计算模板。
2. **逐段规则**。第一版会把下面几类路径段识别为参数：
   - 纯数字
   - UUID
   - 16 位以上的十六进制
   - 日期
   - 长度 20 以上、字母和数字混合的 token

   参数按出现顺序命名为 `{id}`、`{id2}`……
3. 其他路径段原样保留。

声明可能晚于流量到达。那时，已经用完整路径建档的接口（`/api/customer/{id}`）需要归并到声明的接口（`/customer/{id}`）：

- 统计和样本整体迁过去。
- 旧 ID 保留为别名。
- 发出 `MergedInto` 事件。

2f 的"跨环境前缀自动合并"也会复用这套归并机制。

按取值多样性识别参数（例如 `/user/alice`、`/user/bob` → `/user/{name}`）会改变已有接口的身份，放到 2f，看重放数据再决定。

### 4.4 声明

- 表 `declarations(endpoint_id, environment_id?, platform, source_url, structure, updated_at)`。
- 同一来源（`platform` + `source_url`）对同一个接口的新声明，覆盖旧声明。
- 声明给出的 `required` 和类型，是标签的最高依据。
- **待裁决**只有一种来源：
  - 声明说必填，但 `full` body 的实际调用里出现了缺失，而且缺失的调用不少于 5 次；或者
  - 声明的类型与实际类型不同。

  待裁决项在读取时计算，第一版只在 `report` 里列出，没有处理界面。

### 4.5 字段标签：读取时计算，不落库

- 标签规则是最需要调的部分。所以只存指纹，每个指纹带形状、调用次数、首次和最后出现时间。
- 读取时由 core 的纯函数算出标签，改规则不需要迁移数据。

对"接口 × 环境 × 位置 × 字段路径"，把能证明缺失的指纹分成两组："出现"和"缺失"，各自有时间区间。

| 标签 | 规则（默认阈值） |
|---|---|
| 观察中 | 总调用少于 5 次 |
| 必有 | 从未缺失 |
| 可选 | 出现和缺失的时间区间有重叠 |
| 新增 | 某时刻之前全部缺失，之后全部出现，之后至少 10 次调用 |
| 疑似移除 | 某时刻之前全部出现，之后全部缺失，之后至少 20 次调用，并且跨度至少 3 天 |
| 环境差异 | 两个环境都不在"观察中"，一个始终出现，一个始终缺失 |
| 多态 | 同一字段有多种类型，时间区间重叠；不重叠时，按"新增/移除"判为接口更新 |

局限：时间区间按指纹粒度计算，一个指纹从第一次出现到最后一次出现，中间都算作"出现"。第一版接受这个精度，重放时再看够不够用。

### 4.6 outbox 与样本

- `outbox(seq bigserial, event jsonb, created_at)`，由 observe 实现 `ChangeFeed<EndpointEvent>`。阶段 2 还没有订阅方，由测试和 `report` 读取。
- 样本：每个指纹一个，存完整的 `CanonicalObservation`（原始数据不脱敏）。
  - 第一版不设上限，重放时观察样本的数量和体积。
  - 样本不进日志，也不进 fixtures。

---

## 5. 插件改造（2d）

内容按 0002 的 F1–F10。执行顺序如下：

1. **F1 合同**：把 `scripts/build-ingestion-contract.mjs` 改成 `build-collect-contract.mjs`。它从后端 `contracts/collect/v1` 复制 schema 和 fixtures，校验 `manifest.json` 的哈希，生成 AJV 校验器；然后删除 v1/v2/v3。
2. **F3 转换器**，再做 **F2** `collect.ts`。
3. **F4 组批和回执**，同时做 **F5**（上传与登录解耦）。
4. **F7** 不做过滤，**F8** 暂停其他 kind 和截图。
5. **F6 站点登记**：对接第 3 节的两个接口，本地规则变成服务端数据的缓存。
6. **F9** 迁移旧的本地数据。
7. **F10** 界面文案。

前提和约束：

- 插件仓库有 52 处未提交的修改，请你先提交一次作为基线。
- 插件是另一个项目，改动前需要你明确授权。按插件的 AGENTS.md：
  - 我只运行 `npm run build`，不跑测试，也不做浏览器 QA。
  - 我不提交，也不 push。
  - 我不启用子代理。
- 联调需要本地后端已经完成 2b。

---

## 6. 验收（2e）

**自动测试**（都在后端仓库，随 2b、2c 一起交付）：

- 合同 fixtures 全部走一遍 intake（见 2.2）。
- intake 集成测试：
  - 重发返回原回执。
  - 同一 ID 内容不同，返回 409。
  - 限流返回 429 和 `Retry-After`。
  - 未知项目和未知环境返回 404。
  - 按环境名自动创建环境。
  - 只有声明的批次可以省略环境。
  - sink 失败返回 503；重发后，observe 里只计一次。
- observe 单元测试，用构造的观测序列覆盖：
  - 模板规则、声明尾部匹配和归并。
  - 空列表与有值列表合并，不产生标签。
  - `truncated` 不证明缺失。
  - 表 4.5 里每个标签至少一个正例和一个反例。
  - 外部地址判定；预先登记的地址从第一条流量起生效；结论改变后，接口按预期归并。

**历史备份重放**：

- `backups/` 里的旧库快照有 400 条 `capture_events`、287 个 blob。旧系统从中整理出了 39 个接口。
- 写一个一次性脚本，做四件事：
  1. 把快照恢复到临时 Postgres。
  2. 读出旧的采集记录和 blob。
  3. 转成 collect v1 批次。
  4. 发给本地后端。
- 脚本放在 `experiments/legacy-replay/`，输出写到已经 gitignore 的 `experiments/results/`。验收完成后删除脚本。
- 旧表结构我还没有细看，写脚本前会先读一遍，有对不上的地方再向你汇报。
- 用 `admin observe report` 对照三项，由你抽查几个熟悉的接口：
  - 接口数量，是否接近 39。
  - 模板是否正确。
  - 标签是否合理。

**真实插件**：你在真实项目里加载插件，正常使用一段时间，再看 `report`。

---

## 7. 确认记录

已确认（2026-09-24）：

- 第 2–6 问按草稿执行：
  - 标签阈值先用默认值，重放后再调。
  - 2f 放到重放之后再决定。
  - 站点查询接口不登录，也返回项目名和环境名。
  - 你提交基线后，我修改插件仓库。
  - 重放脚本放在 `experiments/legacy-replay/`，验收后删除。
- 第 1 问改为"不单独存放"：外部接口使用带地址的绝对模板，和项目自己的接口放在同一套数据、同一份文档里；地址可以通过接口预先登记（见 4.2）。

- 原则：每个项目从始至终只有一份文档。外部接口不单独存放，不另成文档，也不在检索和排序上降级。

- 4.2 的"绝对模板 + 预先登记"已确认。

---

## 2a 交付记录（2026-09-24）

| 交付物 | 位置 | 验收 |
|---|---|---|
| `EndpointId` | `crates/common/src/ids.rs` | 与其他 ID 同一个宏 |
| 变更流 | `crates/contracts/src/feed.rs`：`Cursor`、`Change<E>`、`ChangeFeed<E>`，约定"读过的游标之后不能再冒出更早的事件" | `InMemoryFeed` 通过 `change_feed_conformance`：顺序、重读稳定、`limit`、从游标续读、末尾为空 |
| 接口合同 | `crates/contracts/src/endpoint.rs`：`EndpointEvent`（`Created` / `StructureChanged` / `MergedInto`）、`EndpointReader`（`list`、`get`，别名解析到合并后的接口）、`ServiceAddresses`（`list`、`decide`，可预先登记，`None` 交回自动判断）、`ServiceAddress`（只接受规范化的 origin）、字段事实模型（位置、`FieldPath`、类型、按环境的标签、环境差异、声明、冲突） | 单元测试；`InMemoryAddresses` 通过 `service_addresses_conformance`；`InMemoryEndpoints` 解析别名 |

说明：

- 改名不单独设事件，表现为新接口的 `Created` 加旧接口的 `MergedInto`。
- `EndpointReader` 没有一致性测试：它的行为取决于 observe 的算法，由 2c 用观测序列直接验证。

---

## 2b 交付记录（2026-09-28）

| 交付物 | 位置 | 验收 |
|---|---|---|
| intake 合同 | `modules/intake/contracts`：回执、拒绝原因、`BatchLedger`（首次写入为准、按时间清理） | `InMemoryLedger` 与 `PostgresLedger` 都通过 `batch_ledger_conformance` |
| intake 核心 | `modules/intake/core`：解析与校验（`wire.rs`）、规范化哈希（`hash.rs`）、按平台令牌桶（`limiter.rs`）、`Intake::submit` | `tests/fixtures.rs`：合同里每个 fixture 按预期接收或拒绝；`tests/submit.rs`：重发同回执（含重新排版）、内容不同 409、限流 429、未知项目/环境 404、按名称建环境、只有声明可省略环境、sink 或账本失败 503 且重发只计一次、坏记录只拒绝自己、7 天清理 |
| intake 存储 | `modules/intake/adapter`：`intake.batches` | 迁移可重复执行；一致性测试 |
| 站点查询 | `SiteRegistry::lookup` 返回 `SiteMatch`（带项目名、环境名） | access 的一致性测试补了名称断言 |
| HTTP | `POST /v1/collect/batches`、`GET /v1/sites/lookup`、`PUT /v1/sites`（`apps/backend/src/http/`） | `tests/integration/access_flow.rs` 在真实数据库上走完：查询、401/403/404、登记、上传、重发、409、404、400、413、429 + `Retry-After` |
| 装配 | `wiring::Databases`（每个模块一个连接池）、`build_api`；admin `migrate` 迁移两个模块；worker 每小时清理账本 | 生命周期测试 |

与方案不同的地方：

- **多了 `nexofolio-intake-contracts`**（2.1 说不单独建）。架构测试规定 adapter 只能依赖合同层，不能依赖自己的 core，账本端口和回执类型因此放在一个合同 crate 里。它只在 intake 的 core、adapter 和 apps 之间使用，不对其他模块开放。
- 纯逻辑 crate 允许的依赖加了 `base64`、`sha2`（解码 body、计算内容哈希）和 `tracing`（只用日志门面，输出仍由 apps 决定）。
- 2c 之前，apps 用一个只记数量的 sink（`UntilObserve`）顶替 observe：批次会进账本、回执照常返回，但观测不落库。2c 接上 observe 后删除。
- `seen_batches` 属于 observe，随 2c 一起做；现在 worker 只清理 intake 账本。
- 新配置：`NEXOFOLIO_COLLECT_RATE_PER_MINUTE`（默认 120）、`NEXOFOLIO_COLLECT_BURST`（默认 30）。
- 站点和上传接口只在配置了禅道登录时启用，因为项目来自 access。

---

## 2c 交付记录（2026-09-28）

| 交付物 | 位置 | 验收 |
|---|---|---|
| observe 合同 | `modules/observe/contracts`：存储端口 `ObserveStore` / `ObserveTx`（一次投递一个事务）和它存的行 | `InMemoryObserveStore` 与 `PostgresObserve` 都通过 `observe_store_conformance`（含回滚、别名、流量迁移、声明覆盖、outbox 顺序） |
| observe 核心 | `modules/observe/core`：地址归属（`address.rs`）、路径模板（`template.rs`）、结构与指纹（`structure.rs`、`shape.rs`）、声明（`declaration.rs`）、标签（`facts.rs`）、接口身份与归并（`identity.rs`）、投递（`ingest.rs`）；`Observe` 同时是 `ObservationSink`、`EndpointReader`、`ServiceAddresses` | 各文件单元测试；`tests/observe.rs` 用观测序列走完：两套一致性测试、重投只计一次、存储故障可重试、`Created` / `StructureChanged`、晚到声明吸收流量并学到 base path、先声明后流量、观察中/必有/可选、`truncated` 与空列表不证明缺失、环境差异、非 JSON 地址判外部、预先登记从第一条生效、改结论后流量迁移并保留别名 |
| observe 存储 | `modules/observe/adapter`：`observe` schema（`seen_batches`、`service_addresses`、`base_paths`、`endpoints`、`fingerprints`、`declarations`、`outbox`） | 迁移可重复执行；一致性测试 |
| HTTP | `GET` / `PUT /v1/projects/{id}/service-addresses`（`apps/backend/src/http/service_addresses.rs`） | `tests/integration/access_flow.rs` 在真实数据库上：上传后每个批次只计一次（重发也一样）、401/403、自动结论、改成外部后模板变成绝对形式、非法地址 4xx、交回自动后回到相对模板、迁移不丢调用 |
| 装配 | `Observe<PostgresObserve>` 替换 `UntilObserve`；`Databases` 多一个 observe 连接池；admin `migrate` 迁移三个模块；worker 每小时同时清理 intake 账本和 `seen_batches` | 生命周期测试 |
| 验收工具 | `admin observe report --project <id>`（`apps/backend/src/wiring/report.rs`）：服务地址、接口、各环境调用、base path、字段标签、待裁决 | 集成测试里跑一次 |

与方案不同的地方：

- **多了 `nexofolio-observe-contracts`**，原因同 intake：adapter 只能依赖合同层。只在 observe 的 core、adapter 和 apps 之间使用。
- 纯逻辑 crate 允许的依赖加了 `psl`（公共后缀表编译在 crate 里，查询不做 IO）。
- `service_addresses` 分开存人工结论和自动结论（`manual_verdict`、`auto_verdict` + `auto_reason`），不是 4.2 写的 `verdict + decided_by`。这样撤销人工结论时，自动结论还在，不用重算。
- 自动结论只在地址第一次出现时判一次，之后不变；要改就用人工结论。"3 个以上项目"按"本项目之外至少 2 个项目调用过"判断。
- `PUT` 的 `verdict` 可以传 `null`，表示撤销人工结论、交回自动判断（合同里的 `decide(None)`）。`GET` 返回 `{ "addresses": [...] }`。
- 指纹的键是"接口 + 环境 + 服务地址 + 哈希"，比 4.1 多了服务地址：同一接口在不同地址上的流量要能单独迁移（改地址结论、学到新 base path 时）。
- 4.1 第 6 步的"合并结构"不落库，和标签一样在读取时由指纹算出。
- 模板规则的补充：
  - 声明的模板如果本身已经以 base path 开头，直接当完整路径用。
  - 声明的模板至少要有一段固定文字，才允许用尾部匹配推出 base path，避免 `/{id}` 这类模板匹配一切。
  - 第一版不解析声明里的 `$ref`。
- 地址结论改回来时（外部 → 本项目 → 外部），会建一个新接口，之前的接口都作为别名留着，旧 ID 仍然能解析。
- 同一项目的写入用 Postgres advisory lock 串行（按项目加锁，顺序固定）；outbox 写入另有一把锁，保证游标按提交顺序递增。
- `report` 里的环境显示 ID，不显示名称：名称在 access，admin 命令不登录。

## 2e 交付记录（2026-09-28）

重放脚本：`experiments/legacy-replay/replay.py`（只用 Python 标准库）。输出在 `experiments/results/legacy-replay/`：`summary.json`（计数）、`old-interfaces.txt`（旧系统 39 个接口）、`report.txt`（`admin observe report`）。脚本不打印请求、响应和请求头。

| 项目 | 结果 |
|---|---|
| 输入 | 旧快照 400 条事件：`http_exchange` 153、`page_context` 144、`interaction` 82、`ui_snapshot` 21；HTTP 全部属于项目 1058 的「UAT 环境」 |
| 上传 | 153 条全部接收，0 拒绝，4 个批次；原样重跑一次，回执相同，`report` 一字不差 |
| 接口数量 | 39，与旧系统相同；37 个路径一致（参数名从 `{param1}` 变为 `{id}`） |
| 模板错误 | 2 个：`commodity/commodityPageForConsultOrderV2`、`commodityCategory/getCategoryTreeWithinUserProductLinePermissionsV2` 被当成参数，变成 `commodity/{id}`、`commodityCategory/{id}`。原因是 `template.rs` 的 token 规则（≥20 字符、字母数字混合）把以 `V2` 结尾的驼峰单词算成了 ID。旧系统两者都是固定路径 |
| 没识别的参数 | `listByFormKey/at2510292900001-19` 保持固定路径（18 字符，不满足 token 规则），和旧系统一样 |
| 服务地址 | `presalescloud.gree.com`（145 次）、`dun.gree.com`（8 次，滑块验证码）都自动判为本项目 |
| 标签 | 903 个字段"观察中"，40 个"总是出现"，没有可选/新增/移除/多态：只有 5 个接口调用 ≥5 次，其余数据量不够。`null\|object` 这类可空字段判为"总是出现"，不算多态 |
| 存储 | 47 个指纹（每个一份样本），约 345 kB；observe 全部表不到 1 MB |

与方案不同的地方：

- **没有把快照恢复到临时 Postgres。** 快照是 pg_dump 17 的自定义格式，而且是写到管道的（没有数据偏移），环境里没有 `pg_restore`。脚本直接解析 dump 文件，按顺序扫描数据块，读出 `capture_events`、`environments`、`interface_documents` 三张表；blob 直接从 `blobs.tar.gz` 读。
- 只重放 `http_exchange`。其余 247 条是 collect v1 还不支持的 kind。
- 不带 `site`：旧数据没有记录站点范围，不编造。环境按名称发送，由服务端创建。
- 新后端的项目 ID 是随机的，所以脚本要传 `--project-id`。本次验收在隔离的临时库里直接插入项目行，没有动本地库。
- 旧请求头只有页面可见部分，映射为 `partial`；旧的 `unreadable` 正文（1 条文件上传）映射为 `unreadable`，旧内容不上传。

待你抽查后决定：

- 上面 2 个模板错误要不要现在修（建议：token 规则排除"字母单词 + 结尾数字"，如 `…V2`）。
- 数据量太小，标签阈值无法评估，建议保持默认值，等真实插件用一段时间再看。
- `dun.gree.com` 要不要改判为外部。
- 抽查完成后删除 `experiments/legacy-replay/`。
