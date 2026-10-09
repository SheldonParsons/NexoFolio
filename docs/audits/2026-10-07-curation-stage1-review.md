# 首轮整理优化：主会话独立审批

2026-10-07（Asia/Shanghai）。审批对象：`01a1156a-580d-73c0-a46b-2eba89ff72c9`。结论：**结构和基础功能有明确收益，但核心关系正确性、确认层级和输出脱敏暂不通过。整改前不迁移/重启真实服务或进行真实项目自动整理按钮测试。**

本次为只读代码审查、原测试复跑和独立合成反例；没有修改首轮生产实现、没有发送返工消息、没有运行真实项目整理或业务写入。前端三项已另交给新会话，后台保持本审批边界。

## 1. 已实现并可复核的收益

从实施前快照比较，而不是把整个 Git dirty diff 算成本轮：187 个基线文件的 SHA-256 全部有效。清单中两个旧集成测试未包含在基线，不能独立恢复其实施前工作区版本；其余列出的后台修改与基线均确实不同。

- 参数证据扫描与模型摘要分开。独立复跑同资料测试，非标配置扫描 39→63，第 20 个选项仍有原索引来源。模型观测值仍有上限。
- 字段增加独立中文显示名、单位、角色、补充与未知项。独立 TCP MCP 读回 `itemCode / 咨询子单号`，以及 `estimatedAmount / 预计金额 / 万 / 币种未确认`。这是固定模型和脱敏资料中的确定性标签/单位效果，不代表真实模型文案已全量重生成。
- 关系增加方向、条件、环境、来源和确认状态。脱敏资料生成并在隔离库保存 34 条关系；它们不是 34 个必须执行的步骤，也不能仅凭这条数量证明全部正确。
- MCP 真实提供 search_interfaces、read_interface、trace_dependencies。重新通过 TCP + rmcp + 临时 PostgreSQL 测试了 initialize/tools/list/tools/call、分页、已知输入复用、循环限制、项目授权撤销和凭证撤销。
- 凭证独立 hash-only 存在 access；MCP 不读取原始样本，也不现场调用模型。知识仍由 knowledge 的项目锁、事务、轮次及历史保存。人工记录与回滚的正向用例通过。

实际潜在收益：agent 可以读取参数的可能来源、来源接口自身需要的输入、当前资料缺口及明确的单位/中文名。尚不能宣称 agent 已从登录完成真实咨询创建，或者现有项目页面已得到新整理内容。

## 2. 独立复核结果

- 四包 `cargo test --lib --tests`：82 项通过，10 项架构检查包含在内。
- 真实隔离 PostgreSQL：knowledge SQL/回滚 3 项，后台迁移 1 项，真实 MCP 1 项，access 生命周期 1 项，共 6 项通过；临时库已删除。
- clippy 四包 `--all-targets -- -D warnings` 和 fmt 通过。
- 独立反例：通过临时 Rust crate 调用当前实际 `navigation_links`、`field_inputs`、`trace_upstream`、`Describe::fields` 和当前脱敏函数，复现下列 7 类问题。

反例输入仅为构造的假 ID、假数据和测试敏感值，未使用真实凭证。代码与非敏感结果在 [反例目录](stage1-review/)。临时执行方式为：

```sh
CARGO_TARGET_DIR=/Users/sheldon/Documents/GithubProject/NexoFolio/target cargo run --offline --manifest-path /tmp/nfi-stage1-review-20261007/Cargo.toml --quiet
```

验证日志及重新导出的协议 JSON 位于 `/tmp/nfi-stage1-review-*.log`、`/tmp/nfi-stage1-review-mcp-evidence.json`。前端正在另一会话修改，本审批不把其变化算进首轮后台，也不对正在编辑的前端重新声明全套通过。

## 3. 阻断问题与具体验收

### R1 / P1：其他接口的关系串入当前字段

位置：`modules/curate/core/src/planning/inputs.rs:95`。

取关系时只匹配 to_field 和 to_location，没有核对 `link.to == facts.summary.id`。由于本次算出的 links 含项目内其他目标，另一个接口的同路径字段关系会进入当前接口的字段证据和模型输入。

反例：验证码 uniqueId→校验接口 request.id 的关系，被挂到不相关接口的 request.id；两者值分别为 captcha-A 与 unrelated-B。`other_endpoint_evidence_attached=true`。

验收：关系挂载使用完整目标 FieldRef，负例不得有验证码来源，正确校验字段仍有来源。

### R2 / P1：公共 gateway 前缀被当成实体范围

位置：`modules/curate/core/src/rules/navigation.rs:101`。

路径 id 匹配的 family 集合包含 gateway，只要任何大于 5 字的片段在源路径中存在就通过。所有 gateway 接口因此可能被当作同业务实体，绕过多字段核对。

反例：商品列表 id=123 与技术咨询详情 path.id=123，同页不同实体，得到 ParameterSource。`gateway_false_parameter_source=true`。

验收：公共服务/协议前缀不作为实体证据。实体与业务范围必须有真实支持；同实体列表详情保留，不相关实体拒绝。

### R3 / P1：分页数值碰巧等于 ID，被误判为必须先知道 ID

位置：`modules/curate/core/src/rules/navigation.rs:137`。

source_input 只找源请求里任意同值字段，没有检查语义角色。`size=20` 恰好等于响应 id=20，会把可用于发现配置 ID 的列表误写成需要已有 ID 的 EntityReference。

反例：列表 id/name 与 configs.configId/configName 两项一致，查询仅有分页 size=20；ID 边仍被误标为 EntityReference，source_input 指向 size。`pagination_size_false_id_requirement=true`。

验收：判断“源接口需要同一 ID”要核对输入字段角色/位置和实体，而不是任意数值。真正按 ID 查详情的反例仍应保留为参考。

### R4 / P1：字段位置未进入关系身份，路径参数和查询参数被合并

位置：`rules/navigation.rs:190`、`planning/links.rs`、`knowledge/contracts/src/lib.rs:46` 及 navigation_context 迁移中的唯一索引。

记录包含位置，但生成去重、更新匹配、LinkIdentity、SQL 唯一身份缺少源/目标位置和响应状态。相同接口下 Query.id 和 Path.id 实际不同，却共享边身份。

反例：同一来源 id 分别映射到目标 Path.id、Query.id，期待两条实际仅得到一条。`distinct_target_locations_expected_2_actual=1`。

验收：整条链统一完整 source/target FieldRef：endpoint+location/status+path+环境。生成、命令、SQL 身份、回滚、读取与分页都应保持两条关系，旧数据有明确兼容方式。

### R5 / P1：合法省略环境会丢认证与关系提示

位置：`curate/core/src/planning/trace.rs:125` 与 MCP Query 的可选 environment_id。

环境未传时 None 只匹配 None，实际带环境的认证和关系全部不匹配。参数合法但返回缺口不完整。

反例：接口有某环境 Bearer 使用，authenticated=false；传环境得到 authentication_gap，不传则没有该提示。`auth_gap_with_explicit_env=true`、`auth_gap_when_env_omitted=false`。

验收：唯一可用环境可按明确策略选择；多环境需返回需要选择环境，或显式分环境读取。不能静默丢失现存证据。

### R6 / P1：直接标签的确认等级扩散到模型补充

位置：`interpretation/field_reply.rs:14`、`interpretation/fields.rs:72`。

有直接中文标签就给整个 FieldMeaning 标 Direct，之后将无单独证据的模型 details 放进同一个对象。名称/单位来源直接，不能证明所有详细解释也直接。

反例：名称/单位有直接标签，固定模型返回“未提供依据的业务限制”，保存结果仍为 Direct。`model_details_marked_direct=true`。

验收：中文名、单位、模型解释及业务约束各保留自己的证据身份；至少不能让推测继承 Direct。简单修正即可，不要求本批建设复杂通用声明框架。

### R7 / P1：敏感路径保护没有递归覆盖新层，关系来源也漏识别

位置：`apps/backend/src/mcp/redaction.rs:7`。

对 password 路径，只在本层遮盖 text/values 等；进入 context.meaning 后不继承敏感状态。Link 的 from_field/to_field 未作为敏感路径检测，evidence 可以残留测试敏感值。

反例：实际函数保留 password 字段嵌套 details 和 access_token 来源关系的 evidence；`nested_sensitive_meaning_retained=true`、`sensitive_link_evidence_retained=true`。

这是构造输入中复现的输出保护缺口，不代表已证明真实数据发生外泄。验收需覆盖嵌套 meaning、中文名、所有证据/关系位置，保留协议结构且不返回测试敏感值。

## 4. 结构是否健康、该耦合的是否正确耦合

总体结构方向正确。模块没有跨 schema/Cargo 依赖，规则和追溯是纯逻辑，MCP 通过公开端口读数据，access 拥有凭证和权限，knowledge 拥有存储/历史。修改文件普遍在 40–300 行，生产主知识合同拆为 notes/navigation 两个小文件，不是新增一个巨型后台服务。

正确的耦合：字段含义与其来源/版本一起保存；关系与其条件/来源一起保存；变更和历史同事务；MCP、文档共用知识；权限检查与每次工具调用结合。

需要修正的耦合：

- endpoint/location/path/环境是不可拆散的身份。目前它们分散在字段、context、唯一索引，导致 R1/R4。
- 名称直接证据与模型生成说明应分别标来源，不能因为放在同一 meaning 就共享 Direct（R6）。
- 敏感状态应递归传播，识别规则应覆盖新增结构，不能按少数旧 JSON 键临时处理（R7）。
- 实体范围与源输入角色的定义要共享清晰判据；任意路径片段/任意同值的快捷判断造成 R2/R3。

非阻断但应规划的结构事项：field_inputs 在每个接口上重新计算全项目 navigation_links，名称和字段阶段还各做一遍，重复全图扫描随数据增长会放大；可在项目证据快照上计算一次，再按目标 FieldRef 投影。MCP 内技术咨询搜索别名是业务特例，今后应移到可测试的检索/目标语义层，不继续堆在 transport 分支。纯关系写入已在字段模型前独立轮次，这点正确；但整个 workflow 仍先跑名称模型，名称阶段失败会令后面的关系阶段不执行，可再明确纯规则阶段与模型阶段的依赖。

## 5. 审批处置

先整改 R1–R7，增加本次反例和正向控制的长期回归，再复跑关系/含义/脱敏、SQL 身份与回滚、真实 MCP 默认参数及授权测试。不要用更新 fixture 期望值的方式接受误关系。

本审批不否认已有收益，也不要求重做全部结构。当前缺口恰好在产品核心“agent 沿关系找值”的可信性上，因此不能以 82 项现测试通过作为放行理由。

后台整改通过和前端导航验收之后，再由主会话进行迁移/重启及最后的真实项目自动整理按钮测试。尚未对真实模型中文质量、全部业务规则、完整登录/创建路径或生产部署作通过声明。

## 6. 修复后的处置（2026-10-07）

主会话已直接修复 R1–R7，原反例、正常控制、SQL 身份/回滚、旧索引升级和真实 TCP MCP 验收通过。原七类阻断在代码及隔离验收层面解除，详见 [修复与验收](2026-10-07-curation-stage1-fixes.md)。真实项目整体按钮验收仍留在所有任务完成后的最后一步。
