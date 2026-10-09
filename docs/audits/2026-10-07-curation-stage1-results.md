# 首轮整理优化实施结果

2026-10-07（Asia/Shanghai）。供主会话复核本批代码和证据，不代替最后的真实项目按钮测试或业务验收。

Agent 现在能从保存的知识找目标、读输入输出及分层说明、追溯缺失参数来源，并看见来源接口自身还缺什么。参数取得、实体参考、配置、认证使用与强制前置具有不同身份；资料不足返回缺口，不拼接一条看似完整的登录/创建链。

## 基线与边界

实施前保存 187 个相关源码/合同/测试文件的副本和 SHA-256，位于 /tmp/nexofolio-stage1-baseline/manifest.json。本次准确文件清单见 [文件差异](2026-10-07-stage1-file-changes.json)，按实施前工作区比较，不按 Git HEAD 计算收益。

此前已有模块拆分、runtime、SQL 分文件、字段 context 迁移、前端字段树及紧凑对象数组；它们不计本批收益。没有创建分支/worktree、提交、推送、启动代理或向其他会话发消息。

所有模型验证使用固定回复；写入、历史、凭证及 MCP 验证使用临时隔离 PostgreSQL，临时库已删除。真实项目仅只读核对。未点击“自动整理文案/文档”，未请求真实项目 /curate，未创建或提交业务单，未停止/重启 18080/5173。

结束时真实项目仍为 **47 接口、1,738 字段说明、0 条 links**，最新整理轮次开始时间仍为 **2026-09-30 06:04:47.936794 UTC**。代码具备新能力不表示真实知识已重新整理。

## 实现

1. 字段证据保留位置、同条记录行、指针、样本、环境、页面、顺序、平台及 body/url/header 完整性。规则扫描与模型摘要分开；每路径数组扫描上限 12→4,096，模型观测值仍最多 12 个。
2. 列配置 code 与原始键 SHA-256 配对，限同页面，或同业务接口范围内至少两个实体标识共同匹配到配置页面。保留标签及对应实体引用，不全局硬编码 itemCode。标题含“（万）”时保存单位，币种未知。
3. 独立保存中文显示名、单位、角色、补充说明和未知项，保留原始键。模型输入包含请求/响应位置、父级、兄弟字段、实际值、来源和旧内容。信封 code 与商品 code 分开，收紧无依据的 Sjb→设计部展开；未知/重复回复 key 拒绝写入。
4. 接口说明按位置分别选择主要字段，避免请求字段挤掉输出；加入证据、认证使用和旧说明，支持动作/对象/必要范围、用途、详细输入输出及 ID 角色/限制。
5. 方向性关系有独立类型：参数来源、实体参考、认证使用、已确认前置、条件前置、推荐检查、字典。保存位置、环境、范围、条件、确认状态、联合字段和来源。观测对应不升级为强制前置；不同环境具有独立边身份。
6. 字段/接口 recipe 升为 v2；新证据进入摘要触发生成知识更新，未变结果复用，人工说明/关系保护。写入沿用 knowledge 项目锁、事务、轮次、历史和回滚。重复模板对应合并，避免每次重写。
7. 追溯复用已有输入和调用者声明的当前有效业务认证，检查来源 API 自身输入。未知 ID 不从详情“发现”；已有目标项目 ID 可以映射到详情查询 ID，补充其他信息。循环和展开上限明确返回。
8. MCP 新增 search_interfaces、read_interface、trace_dependencies，仅通过公开端口读保存知识，不调用模型或读取原始样本。支持方法、环境、业务路径范围及字段/关系独立分页。
9. access 增加独立 hash-only MCP 凭证；每次调用检查用户状态、过期/撤销及当前项目授权。admin 发放凭证写新建 0600 文件，只输出凭证 ID。MCP 额外脱敏旧知识里的敏感字段和 Bearer 文本。
10. 前端仅同步合同并显示原始键旁的中文名、直接来源、单位/角色/补充/未知项和来源坐标。字段树与 array<object> 紧凑投影保留；目录生成及目录界面未改。

## 改前/改后与反例

固定资料为已存记录的 **32 条脱敏投影、23 个接口**：身份值换成稳定匿名引用，凭证/个人信息删除或替换，数组最多保留 24 项，第 20 项保留原索引。不是完整项目副本。固定模型负责表达；直接标签、单位、方向及范围来自资料。

| 项目 | 改前 | 改后实测及限制 |
|---|---|---|
| 第 20 项非标配置 | 原提取器前 12 项截断，缺索引 19 | 同条配置标量 **39→63**；id/name→configId/configName 匹配定位 /19/ |
| 咨询 itemCode | “物料条目编码”，无独立显示名 | 直接标签得到 **咨询子单号**；原始键不变；无关仓库接口同名字段不能继承 |
| estimatedAmount | “预估金额”，未保存单位 | 保存 **万** 和“币种未确认”，来源可查 |
| 响应信封 code | 宽泛编码说明 | 请求/响应位置及 msg/data 兄弟结构用于业务响应码，具体码值未知 |
| 报价/Baan | 通用 code 被排除、异名对应不足 | 编码、型号、单位联合对应；需要同一值的查询作为实体参考，非未知编码发现来源 |
| Sjb | 无依据展开为设计部 | 固定回复中的错误展开被收紧，保留简称待确认 |
| 验证码 | uniqueId/id/sliderCaptchaId 异名漏报 | 当前匹配样本传递保存；另一登录样本不补出匹配，不宣称所有登录强制此链 |
| 待办/处理 | 只有同名候选 | taskId/processInstanceId 联合对应及页面顺序保存，自身查询输入继续检查 |
| 模板/操作 | 数字/名称弱匹配 | 由任务身份绑定 formKey，再保存 type/name→actionType/actionName；意见/附件是配置，未验证后台强制 |
| 商品/生产代码 | 候选不足 | productionCodeId/commodityId/commodityModel 三项联合对应 |
| 项目详情→创建 | 相同 data.id 容易误当发现来源 | id→opptyProjectId 为实体参考；查询自己需要 ID；已知目标 ID 可以复用 |
| 创建认证 | 未形成可靠来源链 | Authorization/Bearer 使用、headers complete、脱敏引用及登录来源缺口保存 |
| msg/size/sort | 原调查弱匹配 190/693（27.4%） | 本批生成关系中 **0 条**目标是 msg/size/sort/status |
| 咨询状态/类型 | 可能误套字典 | 90/124/130/165 未套审批 0–3；未知类型码继续未知 |

原提取器冻结在 modules/curate/core/tests/stage1/legacy_samples.rs，用于同输入对照；v1 样本继续可读，v2 摘要有意失效。脱敏资料产生 **4,338 个标量信号、34 条合并关系**，不是 34 个必调步骤。隔离固定模型写入 **1 条接口说明、79 条分页字段说明、34 条关系**，不是全项目 47 接口的文案重生成。

来源包含 example/environment/location/pointer/page/sequence/platform/complete/endpoint。确认状态为 candidate/observed/direct/confirmed。当前自动关系主要为 observed 及作用域内 direct；没有由调用先后生成已确认强制前置。

## 实际 MCP

[完整非敏感协议结果](2026-10-07-stage1-mcp-evidence.json) 来自 **TCP HTTP + rmcp + 真实隔离 PostgreSQL + 固定模型/脱敏资料**，没有伪造 tools/call 字符串。

- initialize 返回 nexofolio-knowledge，协议 2025-11-25；tools/list 返回三工具和有界 schema。
- search_interfaces(创建并提交技术咨询) 返回唯一保存目标“保存并提交技术咨询”；名称来自固定回复，真实模型质量仍待最后按钮验证。
- read_interface(limit=2) 返回 2 字段、field_total=79、fields_has_more=true；offset=2 下一页不重复。定向分页实际读到 **itemCode / 咨询子单号** 和 **estimatedAmount / unit=万 / 币种未确认**。
- trace_dependencies 传已有 opptyProjectId、authenticated=true，复用认证、目标 ID 及详情查询自身 ID，其他参数仍返回缺口。不把历史 ID/token 当本次输入。
- 无认证状态返回 authentication_gap；未知项目 ID 保留实体参考及缺口；合成循环返回 cycle 并终止；max_steps=2 只有两步且 truncated=true。
- 未认证 HTTP 401；无权项目 PROJECT_ACCESS_DENIED；项目内不存在/外部接口 ENDPOINT_NOT_IN_PROJECT；limit=51 为 INVALID_BOUNDS。initialize 后删除授权，下次调用即拒绝；撤销凭证后 HTTP 401。
- 查询端口 example 被设置为调用即 panic，证明 MCP 没读 raw sample。协议结果不含测试 MCP token 或业务认证文本；另有旧敏感字段/Bearer 输出脱敏测试。

边界：limit 1–50；offset≤100,000；深度 1–8；步骤 1–100；加载图≤32 接口；单工具输出≤128 KiB。超限返回错误或 truncated，可以分页/继续读取。检索是保存文字/方法/路径匹配及少量技术咨询动作别名，不是通用语义搜索。

## 复用与历史

同字段/接口摘要再次整理无 pending；相同关系不产生命令。新增独立证据使生成接口摘要失效；既有字段测试覆盖外部字典变化而自身结构不变。变化关系提出 PutLink，人工接口/字段不进入 pending，人工关系不覆盖。

隔离库人工修改中文名/说明后，自动 pending 仍为空；回滚恢复完整字段、中文名、分层 context 和来源。回滚生成轮次删除新字段和关系。原有数据库用例验证全部 SQL、restore、未提交写入消失；新环境身份及 context 经过真实 SQL 读写。旧历史缺 context 时默认读取；旧 Precondition 不因名字升级为已确认前置。

## 检查

| 检查 | 最终结果 |
|---|---|
| 四包 cargo test --lib --tests（curate/knowledge/knowledge-adapter/backend） | 82 通过，隔离库用例默认 ignored 后另行执行 |
| curate navigation --nocapture | 5 通过，含 39→63、作用域负例、已知输入和循环 |
| backend architecture | 10 通过：依赖、纯 core、schema 所有权、400 行 |
| curate/knowledge-adapter/access-adapter/backend clippy --all-targets -- -D warnings | 通过 |
| cargo fmt --all --check、git diff --check | 通过 |
| knowledge adapter PostgreSQL --ignored | 3 通过 |
| backend PostgreSQL --ignored | 1 通过，迁移可重复 |
| backend navigation_mcp --ignored | 1 通过，真实协议/持久化/授权/历史 |
| backend access_flow --ignored | 1 通过，隔离数据库 + 本机假上游登录/授权/会话生命周期 |
| 前端 Node24 pnpm test | 124 通过，保留字段树、对象数组紧凑投影、文档职责检查 |
| 前端 pnpm build | 类型检查及构建通过；既有大 bundle 提示仍在 |
| 三个修改前端文件 Prettier | 通过 |

检查日志在 /tmp/nfi-stage1-*.log。中途发现的模板重复更新、标签范围、已知 ID 复用及编译问题已修正；补跑 access_flow 发现旧测试仍从详情根读取 summary，已对齐此前已有 facts/knowledge 包装合同（未改生产接口）；本表只记录最终结果。

## 准确文件与迁移

[文件清单](2026-10-07-stage1-file-changes.json) 包含 **42 个后端/合同/测试修改、21 个新增文件、3 个前端修改**。已有 untracked runtime/SQL 拆分文件标为“修改”，不算本批新建。本报告、文件清单和协议 JSON 为另外三个审核产物。

新增迁移：

- modules/knowledge/adapter/migrations/202610070001_navigation_context.sql：接口/关系 JSONB context、关系枚举、按环境区分边身份。
- modules/access/adapter/migrations/202610070001_mcp_tokens.sql：独立 MCP 凭证 hash、用户、有效期、撤销。

202609300001_field_context.sql 是既有迁移。前端仅改 src/contracts/knowledge/types.ts、src/features/documents/FieldStructure.vue、src/features/documents/FieldEvidencePanel.vue。

## 限制与主会话准备

1. 主会话使用既有受保护配置执行 admin migrate，应用新增 access/knowledge 及尚未应用的既有迁移，然后重建/重启后端。当前 18080/5173 仍由主会话管理。本批未重启。
2. MCP 实际接入需选择有真实项目授权的用户，使用 nexofolio-admin issue-mcp-token --user <UUID> --output <受保护新文件> --days 7。客户端按 Secret 配置，不 cat/打印文件。撤销命令 revoke-mcp-token --id <UUID>。本批未给真实账号发放凭证。
3. 其他分派工作及证据复核后，由主会话最后统一点击真实项目“自动整理文案/文档”，验证真实模型表达、全部关键接口质量、运行耗时/失败恢复、历史及 MCP 读回。真实项目整理尚未运行。
4. 调查/交接已确认创建 Bearer 不匹配已存管理端登录样本；本批保存使用观测和缺口，未生成伪造认证来源边。未知项目 ID 发现入口、创建 opptyId 来源、状态/类型码、后台必填及业务成功约束仍需资料或受控验证。
5. trace 是参数来源、条件和缺口资料，不是业务执行 agent，也不保证唯一可执行顺序。条件前置需核对触发条件，推荐检查不强制。没有真实业务写入。
6. 两层目录/前端仍是独立后续交付；同结构新值持续采集、全项目语义推广、任务持久化、通用语义检索未包含。12 结构样本/接口、4,096 数组候选/路径等限额不表示资料完整，超限需定向读取/采样。
