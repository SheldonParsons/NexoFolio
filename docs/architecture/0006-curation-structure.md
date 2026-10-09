# 0006 整理代码的职责边界

状态：2026-10-04 第一轮结构重构完成。生成规则、HTTP 合同、知识表结构和已有数据保持兼容。本文描述已实现的结构；业务知识质量的改进另行实施。

## 1. 模块仍按原来的五个所有者划分

保留 `access / intake / observe / knowledge / curate` 及各自的 `contracts / core / adapter`。模块之间只通过共享合同协作，backend 组合实现。各模块的数据库 schema 仍由自己的 adapter 独占。

- intake：接受及校验采集记录。
- observe：归纳结构、环境、真实样本等事实。
- curate：从事实产生说明、证据和目录命令，决定哪些内容需要更新。
- knowledge：校验并保存命令、修改历史和回滚。
- access：身份与项目访问权限。

HTTP 负责鉴权、解读参数、映射响应；后台执行器负责调用公开端口和控制并发。两者都不能直接查询数据库，也不能替代 curate 的解释规则。

## 2. Curate 内部按依赖方向拆开

```text
modules/curate/core/src/
  evidence/
    samples.rs          原始结构样本 → 有上限的标量及样本引用
  rules/
    dictionary.rs       字典选择参数、同一数组行的 value/label
    same_value.rs       同环境、同名同值的候选关联
    mod.rs              规则上下文及固定执行顺序
  planning/
    inputs.rs           组合自身观测与规则线索，生成 FieldInput
    fields.rs           输入摘要、人工保护、需要更新的字段
    endpoints.rs        接口说明是否需要更新
    workflow.rs         阶段依赖及执行顺序
  interpretation/
    describe.rs         接口名称与用途
    fields.rs           字段模型调用、结果校验、写入建议
    organize.rs         目录模型调用及命令建议
    prompt.rs           接口及目录提示词
    reply.rs            模型回复的 JSON 提取
  lib.rs                保持原来的公开入口
```

依赖方向：观测提取 → 规则 → 更新计划 → 模型解释与命令建议。planning 组合规则的产出，evidence 和 rules 均不调用模型、持久化端口或 HTTP。

`Describe::needs_update` 的公开方法保持兼容，内部委托 planning。`field_inputs / pending_fields / field_input_hash / sample_fields / workflow` 的公开签名保持兼容。

新增使用现有证据类型的规则时，首先在 `rules/` 增加纯函数并注册，再用经过脱敏的样本验证输入、来源和候选状态。新证据概念需要明确演进共享合同，不能通过无限扩展一个无类型 JSON 绕过边界。

### 兼容性

`FIELD_RECIPE = field-knowledge-v1` 保持不变。规则顺序、字段输入的序列化形状、提示词及输出含义未改变。`tests/fixtures/field-knowledge-v1.json` 的期望输入与摘要从重构前实现生成，兼容性测试逐一比较字典线索、同名同值线索、请求/响应分离及输入摘要。

未来改变解释方式或提示词时，必须有明确的规则版本更新和失效策略；结构迁移本身不应触发全部字段重算。

## 3. 执行器独立于 HTTP

```text
apps/backend/src/
  http/curate.rs               鉴权、路由、响应映射
  runtime/curation/
    mod.rs                    应用入口、任务监督、执行依赖有序的阶段
    jobs.rs                   项目任务共用与进度状态
    error.rs                  不依赖 HTTP 的应用错误
    snapshot.rs               通过 EndpointReader 读取事实与样本
    descriptions.rs           名称/用途读取、并发与分批写入
    fields.rs                 字段读取、并发与分批写入
    directories.rs            目录读取及写入
```

执行顺序来自 curate 的 `workflow()`。执行器仍使用原来的描述并发 2、字段接口并发 4、每个接口最多 12 个结构样本、模型字段批次 48 的行为。

名称和字段依然通过 knowledge 轮次保存。部分接口失败时已写入的接口结果保留；字段阶段继续处理其他接口；重试复用未失效结果。整份项目任务的 worker panic 会变为失败进度，避免永久处于 running。

运行日志包含 `project / run / stage / endpoint / round` 的请求 span。模型请求失败会在对应接口 span 中记录警告；debug 日志可按字段位置路径和输入摘要定位待写入知识。日志不打印原始请求、完整提示词、凭证或字段值。

单接口和项目入口共享同一执行器；鉴权仍在 transport 层执行，未来 MCP 入口也必须先校验身份与项目权限再调用应用入口。

## 4. SQL 按实体分文件，共用同一个事务

```text
modules/knowledge/adapter/src/
  postgres.rs                 pool、探活、迁移和事务入口
  postgres/
    transaction.rs            KnowledgeTx 的薄委托、commit、项目锁
    codec.rs                  adapter 内部存储编码
    rounds.rs                 轮次
    folders.rs                目录
    placements.rs             接口归属
    notes.rs                  接口说明、字段说明
    links.rs                  关联
    revisions.rs              修改历史及恢复历史值
```

实体文件在同一个 `PostgresTx` 上执行 SQL，没有各自建立连接或提交事务。knowledge core 原来的 `begin → 锁项目 → 校验 → 记录历史 → 写入 → commit` 保持完整。批量命令及对应历史仍全成或全败；restore 使用同一事务恢复实体。

真实 PostgreSQL 测试覆盖全部语句、历史恢复及跨目录/归属/字段/历史的未提交写入。测试使用临时隔离数据库，结束后删除。

## 5. 前端页面只组合控制器

前端仓库 `src/features/documents/`：

- `useDocumentWorkspace.ts`：页面入口、切换项目/环境时的协调，保持页面公开合同。
- `useDocumentProject.ts`：项目、环境、索引、目录分组与搜索。
- `useDocumentSelection.ts`：接口详情、tab、原始样本读取。
- `useDocumentCuration.ts`：启动/恢复任务、进度轮询、整理后的增量刷新。
- `documentRequests.ts`：统一请求归属与取消；回复必须仍属于相同项目和相同请求槽才可以修改状态。
- `FieldStructure.vue`：字段树、收展、简短说明和观测值。
- `FieldEvidencePanel.vue`：展开后的证据展示和来源跳转，不读取 API 或推断业务含义。

这次拆分保留现有页面布局。请求槽分别是 project、detail、sample、curate；槽之间独立，同槽的新请求取消旧请求，项目切换和卸载取消所有请求。

## 6. 防止再次堆叠的检查

保留 Cargo 依赖、纯 core 和 SQL schema 所有权的架构检查，补充：

- HTTP curate 不执行模型、命令写入、字段计划或后台调度。
- evidence/rules 不调用模型或知识/观测读取端口。
- 整理 core、执行器及知识 adapter 的生产文件最多 400 行；常规目标为 200–300 行。超过上限先复查职责，不能直接提高阈值。
- 前端文档 facade 不实现请求/轮询；证据组件不读取 API；文档源文件最多 400 行。
- 行数检查只是触发职责复查，不代表文件越短越好，也不能以压缩代码规避检查。

验证命令：

```sh
cargo test -p nexofolio-curate -p nexofolio-knowledge -p nexofolio-knowledge-adapter -p nexofolio-backend --lib --tests
cargo clippy -p nexofolio-curate -p nexofolio-knowledge-adapter -p nexofolio-backend --all-targets -- -D warnings
cargo fmt --all --check
```

前端：`pnpm test`、`pnpm build`，以及本次改动文件的 Prettier 检查。数据库测试需在隔离库配置 `TEST_DATABASE_URL` 后执行 knowledge adapter 的 ignored postgres 测试。

## 7. 接下来单独改进知识质量

这一轮不将已有错误说明包装成正确结果。后续优先接入已调查到的可核查线索：

1. 页面列配置的中文标签与业务字段对应关系，例如咨询子单号、所属公司、金额单位。
2. 审批操作模板的 value/name 与条件配置；观测到的映射不等于完整业务枚举。
3. 字段位置、父级和兄弟字段一起进入解释输入，区分响应码和业务编码。
4. 异名同值及同条记录的联合证据，例如验证码传递、报价物料和商品资料。
5. 收紧通用字段的关联噪声，并让新证据触发旧说明更新。

同结构的新标量值持续保留、页面上下文积累、完整层次化 claim/确认/冲突模型、任务持久化、两层目录和 MCP 知识工具仍需要后续实现。本次没有新增数据库迁移，也没有调用真实模型改写项目数据。
