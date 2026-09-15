# 后端累计改动逐文件责任审计

> 这是优化前的冻结审计记录，行号/哈希用于历史定位。优化后的职责、修复项及验证结果见 [优化交付说明](./2026-09-15-optimization.md)。

这是一份必要性与交付范围审计，不是为现有体量寻找统一借口。能解释某文件用途，不等于证明当前实现方式最简或业务已验收。

统计基线：`a4c27c6e4d25eb404892b39c4529f53be65f27a9`（登录/项目同步基础提交）；工作区清单固定为本次审计开始时的 **238 个文件**。包括已跟踪差异和全部未跟踪但未被ignore的文件，排除本报告及随附CSV。未暂存不等于未实现；未提交也意味着无法用Git精确切出每个对话回合的作者/时间。

## 结论先行

- 当前不是4万行新增业务逻辑，而是 **46,870行文本新增、31行文本删除、5个二进制文件** 的累计交付差异。初步按字节换行统计会误计PNG，最终已将二进制剔除。
- Rust生产模块文件的差异为 **12,024行新增**，其中仍含模块内单元测试；独立测试另有4,479行。不能将12,024全部称作运行时业务代码。
- `backend.html`单独14,993行，是Archify生成的离线架构图（内嵌图工具运行库）。它不被Rust服务加载。可以解释它为何大，不能据此论证它必须放在主代码评审里。
- 协议包10,954行包含生成Schema/TS、人工Schema、行为文档和兼容样例；不能一概称为自动生成。它们直接回应跨模块合同对齐要求，但应分别审查源合同和自动生成物。
- 当前Git只有基础提交，环境、采集去重、接口建档、路径模板、旧候选、正式发布、证据与统一维护的后续变更累积在同一工作区；无法从Git严谨算出“最后一轮恰好新增多少”。本表按功能归属解释，不伪造回合归属。
- 下载API来自另一个已授权任务；架构图是独立文档产物。这两类在当前差异中存在，不能拿来充当知识维护计划的必要体量。前端/插件代码不在本仓库统计中，不能把这份清单说成三仓总账。
- 我对交付节奏承担责任：没有及时形成可审阅的阶段切片；在浏览器真实链路还未通时，以后端与模拟试验通过传达了过强的完成感。Chrome documentId错误证明测试数据和真实边界存在缺口。

## 体量账本

| 类别 | 文件数 | 新增文本行 | 删除文本行 | 解释 |
|---|---:|---:|---:|---|
| Rust生产模块（含模块内测试） | 65 | 12,024 | 9 | 按文件位置分类；并非扣除所有内嵌测试后的净生产行数 |
| 独立测试文件 | 11 | 4,479 | 10 | 见逐文件说明 |
| 协议Schema、TS、样例与说明 | 73 | 10,954 | 0 | 生成物与权威Schema混合，逐文件区分 |
| 文档及架构图产物 | 48 | 17,980 | 0 | 含4张PNG；PNG只计文件不计行 |
| 数据库迁移与迁移说明 | 11 | 312 | 0 | 见逐文件说明 |
| 生成/检查/联调工具 | 13 | 468 | 0 | 见逐文件说明 |
| 构建、配置与部署 | 15 | 234 | 11 | 见逐文件说明 |
| 依赖锁文件 | 1 | 419 | 1 | 见逐文件说明 |
| 二进制测试样例 | 1 | 0 | 0 | 68字节PNG，不计行 |

总和可复核：`12,024 + 4,479 + 10,954 + 17,980 + 312 + 468 + 234 + 419 = 46,870`。统计的是新增行而非当前文件总长，修改文件的原有行不再次算作新增。

## 为什么业务本身需要这些能力

1. 高频接收需要稳定身份/指纹、项目环境鉴权、逐记录回执与事务幂等；否则丢数据/重复建接口/串项目。对应intake、admission/capture_store及接收合同。
2. 同结构仍有新取值/标签/关系，因此必须独立证据流水与后台关联；否则结构去重直接丢掉用户想要的知识。对应evidence、证据表/worker、文件存储。
3. 观测结构、语义说明、目录归属不同步演化，需分别保留事实和版本；否则改一句说明也像修改了原始接口。对应knowledge、语义合同、发布版本。
4. 用户要求LLM完整看全部接口再决定局部或全量，因此需要固定快照、分片、完整覆盖和原文回读；这比一次提示词调用复杂得多。必要的是这些不变量，不是当前950/1475行大文件的组织方式。
5. 持续录制同时存在旧候选发布和回退，需并发代次、幂等、租约/检查点和快照后新增接口保护；否则旧任务覆盖新状态。
6. 旧队列和旧目录API不能突然失效，故保留v1/v2/旧目录专用接口；这是有终点的兼容成本，不应演变成两套并行产品。

## 不能替现状辩护的部分

| 问题 | 源码证据 | 判断/后续处理 |
|---|---|---|
| 超大生成图进入主差异 | backend.html 14,993行及4张PNG/多个检查回执 | 图源和更新方法可以留；HTML/截图作为附件或独立文档交付。此轮只标记，不擅自删除已有工作。 |
| 单文件职责过多 | maintenance_engine.rs 950行；rebuild/maintenance.rs 1475行（其中356行测试）；evidence_processing.rs 585行 | 依赖方向通过不代表模块内部解耦。按预算/分片/执行阶段、字段枚举/动作校验、纯关联规则/SQL拆解，不新增crate/框架。 |
| 认证容器承载全部业务状态 | http/access.rs、wiring/access.rs | 环境/采集/知识各自装配，认证仅提供会话与项目权限。当前命名和耦合不合理。 |
| 旧新流程重复 | admission与capture_store；catalog_model与maintenance_model；official_catalog与knowledge_publication | 旧协议/操作语义保留，底层权限/回执/版本不变量复用；停止给旧流程加新能力。 |
| 共享合同承载算法 | contracts/path_identity.rs、observed_comparison.rs | 为避免核心互相依赖而放到共享包，解决了依赖但扩大了合同职责。纯规则应有明确归属，先不要再加第11个包。 |
| SQL和JSON多处压成很长一行 | evidence_processing、capture_store、maintenance_store与两份证据迁移 | 行数并不代表简单；格式和事务步骤说明要改善。已应用迁移不能改历史来美化差异。 |
| 速度代价未闭环 | 既有对照：旧链路平均10.21ms/批，新链路27.98ms/批 | 新增持久化有原因，但约2.74倍代价是事实；单并发debug基准不是性能验收。需约定真实批量/并发/延迟目标后验证。 |
| 模型成本与耗时 | max_calls默认256；真实合成试验3接口24字段经历35次调用（包括失败恢复） | 不能以“最终成功”证明默认流程成本合理；加入实际阶段用量/总耗时展示，再以固定样本测有效调用和质量。此轮不再追加模型试验。 |
| 无法证明真实产品顺畅 | Chrome32hex documentId被误作UUID；前端以技术记录为中心 | 修复缺陷只是必要条件。后端自动验收我承担，前端主流程重做；真实Chrome结果单列，不逼用户读日志或JSON代替验收。 |

## 验收与实施边界

本次重新在独立PostgreSQL容器执行fmt、五套合同检查、合同负例、clippy、workspace测试和数据库集成测试；不操作真实项目、不调用LLM、不部署/重启18080。结果见本报告末尾。本次未重跑浏览器或全部容器镜像交付；此前报告的成功不自动当作本次验证。

前端任务系主会话误派：用户只是讨论先做前端，并未授权修改，且设计尚未完成。收到纠正后任务已停止；根据该任务明确回报的两个文件、三处补丁，已精确撤销这三处误改，保留其他内容。前端等待用户完成设计与明确授权。Header下载属于另一个已存在任务。

此轮只做审计与验收，不按行数盲删代码，不提交/push，不改已应用迁移，不清数据。整理和退役应按上述责任拆小批次，每批保持旧队列/权限/发布回退不变量。

## 第二轮：实际调用链与必要性复核

本轮仅只读审计源码、更新本报告；不修改生产代码，不调整前端，不重跑模型，不部署服务。原238文件逐一重新计算SHA-256，**全部与首次清单一致**。之后另一个下载任务新增两个部署验证文件，见本节末尾；它们不是知识维护代码增量。

### 12,024行Rust模块具体落在什么地方

这些仍是“生产模块文件内新增行”，包含DTO和内嵌测试，不冒充净算法代码。

| 所在位置 | 新增行 | 必要职责与不能滥用的理由 |
|---|---:|---|
| apps/backend HTTP、装配、CLI | 2,142 | 把协议接入运行服务并注入能力；其中下载主文件333行+解析176行属独立需求。认证装配不应成为全部业务的容器。 |
| access | 29 | 环境管理访问端口；不是又实现一遍登录系统。 |
| application | 1,319 | 接收/处理编排、发布端口、维护执行；950行集中在维护引擎，是最应收缩的部分之一。 |
| contracts | 1,286 | 共享ID、DTO、版本命令及少量规则；不是全部运行算法，也不应成为无限扩张的共享工具包。 |
| evidence | 455 | 纯值提取和证据/文件端口；这是本轮唯一新增业务crate。 |
| infrastructure | 3,971 | 数据库事务、模型HTTP、文件卷、任务状态；存在关联业务规则和SQL混合的问题。 |
| intake | 490 | 批次准备、身份/结构指纹与去重条件；不调用模型。 |
| knowledge | 485 | 观测定义和读取端口；与后续语义版本分开。 |
| rebuild | 1,847 | 字段清单、覆盖和候选校验；1475行在维护规则文件中（含356行测试）。 |

合计12,024。没有新增Redis、独立消息中间件或另一个微服务平台。现有PostgreSQL承担持久化队列、版本及元信息，文件卷承担大原文/图片。Rust不会必然要求这个行数；格式化、强类型和分包也不能为错误职责归属辩护。

### 需求到机制的因果关系

| 已确定要求 | 必须存在的机制 | 如果省掉会发生什么 | 不要求我们采用的实现方式 |
|---|---|---|---|
| 高频批量采集，重复直接忽略 | 指纹、项目/环境隔离、稳定记录ID、持久回执 | 重试反复建档、串环境、返回成功却没有保存 | 不要求v2/v3把公共事务逻辑再写两份 |
| 同结构仍积累枚举与关系 | 结构分支和证据分支分别推进，独立证据ID/计数 | 同结构被拦后，新状态值和ID来源消失 | 不要求把关联判断全塞进PostgreSQL适配器 |
| 接口有权威位置，目录可重构 | 接口本体独立ID、正式固定待分类、候选映射 | 目录一改接口身份也变；未分类接口无处可查 | 不要求旧候选与新维护长期保留两套模型执行器 |
| 录制中重构，完整看到全部信息 | 不可变快照、字段清单、分片、覆盖核对、受控回读 | 新旧资料混用，漏接口却宣称全量审阅 | 不要求每6个单元固定拆片、每个回读再调用一次模型总结 |
| 描述/关系/枚举可改且可回退 | 语义层与原始定义分开，统一版本、来源引用 | 模型说明覆盖原始事实，无法解释为什么改 | 不要求用宽松JSON承载所有可定义的适用条件 |
| 发布和重试并发可靠 | 事务、期望代次、幂等回执、租约执行归属 | 重复点击重复写；旧worker覆盖新候选；回退丢新接口 | 不要求不同模块复制同一权限SQL |
| 模块合同能对齐 | Schema/类型/行为说明/版本清单/负例检查 | 一端加字段或改含义，另一端仍按旧规则运行 | 不要求把所有生成物与手写逻辑放在同一审查层次 |

所以，合理理由是这些行为需要一组互相配合的状态与边界；**不是“实现计划很长，所以每一行自然合理”**。新增状态和校验的必要性，与当前代码是否足够简洁，是两个必须分别回答的问题。

### 本轮确认的具体问题与取舍

1. **发布策略的独立性目前不完整。** [knowledge_publication.rs:97](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/knowledge_publication.rs:97)直接调用`ManualKnowledgePublication.authorize_mode(true)`；虽然[application/maintenance.rs:102](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/application/src/maintenance.rs:102)定义了trait，但装配层没有注入可替换的发布策略。现有项目鉴权仍在后面执行，这不是已证明的越权漏洞；问题是“已有trait”不等于“策略已可替换”。改进应把策略选择放到应用/装配层，事务适配器只保证原子性。

2. **相同事务权限判断重复三次。** [capture_store.rs:29](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/capture_store.rs:29)、[official_catalog.rs:40](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/official_catalog.rs:40)、[maintenance_store.rs:63](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/maintenance_store.rs:63)都检查用户enabled、grants_synced、同禅道实例的项目及访问记录并持有用户共享锁。事务内再鉴权有正确性理由；重复实现没有长期价值。应保留事务内检查，提炼同一个小型辅助函数，不另造权限框架。

3. **当前“全量阅读”默认规模受硬编码分片限制。** [rebuild/maintenance.rs:324](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/rebuild/src/maintenance.rs:324)及336行限制每片最多6个阅读单元，除了字节预算之外又加固定数量上限；[maintenance_engine.rs:16](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/application/src/maintenance_engine.rs:16)默认总调用256次。静态推导：1,537个阅读单元至少需要257次审阅调用，还未开始规划/回读就超过预算。单元不严格等于字段，接口本身和超大字段分片也占单元。预算不足明确失败符合合同，但先耗尽数百次调用再失败的体验不合理。应在启动前预估最低调用量、说明规模边界，并用实际输入大小调整分片；不能把6和256描述为已经验证的成熟策略。

4. **回读的实现增加了尚未证明必要的模型次数。** [maintenance_engine.rs:610](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/application/src/maintenance_engine.rs:610)对非图片回读的每个片段再调用一次模型生成summary，之后规划器又拿到原始field/fact。必须保留的是“修改前原文确实进入成功的模型调用并有覆盖记录”；不一定需要每个引用独立总结。可将预算内的目标与两端证据批量供下一次规划调用，再记录具体读入清单。大原文/图片仍需分片，不应以减调用为由跳过原文。

5. **模型成本统计不够。** [maintenance_model.rs:177](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/maintenance_model.rs:177)检查provider的finish_reason后，只把message.content解析为业务结果返回，provider的usage没有进入统一返回对象。现在能统计调用次数，数据库也有调用起止时间，但不能据此准确报告输入/输出token成本。字节上界是防溢出措施，不是真实用量。不能声称当前预算模型已完成成本治理。

6. **业务与数据库的边界还有混合。** [evidence_processing.rs:292](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/evidence_processing.rs:292)把同页/跨SPA交互桥接、120秒顺序、候选上限写进长SQL，随后在同一函数处理类型转换、歧义与支持事实。用索引筛选候选是正确方向；但可独立验证的关联判断应更多进入evidence纯模块。该文件585行中有50行超过180字符，物理行数低估了阅读难度。

7. **后台清理还有关闭边界缺口。** [lifecycle.rs:60](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/wiring/lifecycle.rs:60)附近直接await清理，之后才进入支持取消的select；[evidence_retention.rs:6](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/evidence_retention.rs:6)又在一个事务中等待全局及各项目advisory lock。如果清理等待锁，关闭信号不会立即打断这段await，且错误被忽略。这是源码路径确认的风险，**本轮未做故障注入复现**；此前容器正常退出测试不覆盖所有锁等待。收缩时应补清理批次/锁超时/取消和失败日志，不需要新任务框架。

8. **并非所有相似代码都应直接合并。** intake的`shape`是保守的“能否证明重复”，碰到无法确定类型可拒绝短路；knowledge的`shape`要保留null/不足证据并生成可查看定义。二者有相似遍历但目的不同，共享方向性比较已存在。可共享媒体/解码等纯函数及一致性用例，不能为了少几十行把两种语义合成一个含大量模式参数的大函数。

### 可以缩减什么，不能拿什么凑减行数

- 可将架构图交付目录中**16,259行文本和4张PNG**作为独立文档产物审查；其中图源、来源清单和更新脚本有维护价值，不能简单说全部都应删除。HTML 14,993行最适合放附件或生成产物区域。这是评审减负，不是业务逻辑优化。
- 生成合同仍应可版本化分发。capture/maintenance/catalog-preview的Schema共8,480行由Rust DTO生成；ingestion/documents的Schema共1,910行属于权威Schema/旧版冻结材料。减少JSON排版行数没有实质收益；源合同与生成物自动校验、分别审查才有收益。
- 旧协议回执、旧目录回退和新接口保留机制不能直接删。先统计消费者/旧队列，定义退役条件；先复用底层不变量，再取消不再使用的入口。
- 应先处理上面已确认的策略注入、权限重复、过大文件职责和模型调用策略；拆文件可能令总行数不降反升，但若规则修改只影响一个地方，才算真正减轻维护负担。
- 不承诺“可以删掉30%生产代码”：没有逐行为等价删除证明，报一个削减百分比仍是在用行数替代工程判断。

### 首次清单之后的增量

本轮重新核对得到：原238文件哈希未变；新增以下2个下载部署证据文件，共115行。首次46,870行分母保持固定，排除本审计文档本身后，包含这两项的累计文本新增为46,985行、240个文件。它们不应计入持续知识维护的生产代码。

| 文件 | 新增行 | 作用 | 必要性判断 |
|---|---:|---|---|
| [docs/verification/2026-09-15-downloads-service-load.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-downloads-service-load.md) | 13 | 人读版现用下载API加载结果，区分源码已改与运行服务已更新 | 下载任务的交付证据，可与该任务独立归档，不是后端运行依赖 |
| [docs/verification/2026-09-15-downloads-service-load.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-downloads-service-load.json) | 102 | 机器可读镜像/容器和渠道响应核验结果 | 与MD对应的证据，不能当成知识维护业务新增 |

## 逐文件说明（238/238）

“保留”只表示该职责有需求依据；“调整/收缩”表示实现位置或范围不够好；“兼容”必须有退役条件；“文档附件”不是生产必需。每行都给出具体作用、为何采用及判断，不用同一段“模块化需要”敷衍。

| 文件 | 新增/删除 | 作用 | 为什么这样做 | 必要性判断 |
|---|---:|---|---|---|
| [.github/workflows/check.yml](/Users/sheldon/Documents/GithubProject/NexoFolio/.github/workflows/check.yml) | +19/−1 | 把合同漂移、Rust检查、隔离数据库和容器烟测纳入CI | 防止只在本机编译成功；新增模块进入同一验收入口 | 保留：CI运行成功与本地成功仍须分别报告 |
| [Cargo.lock](/Users/sheldon/Documents/GithubProject/NexoFolio/Cargo.lock) | +419/−1 | 锁定新增JSON Schema、哈希/编码、URL、模型HTTP及下载解析的依赖树 | 新增直接依赖会产生传递依赖与校验和；419行并非419行业务 | 保留：应用仓库应提交锁文件，逐项审查直接依赖即可 |
| [Cargo.toml](/Users/sheldon/Documents/GithubProject/NexoFolio/Cargo.toml) | +1/−0 | 注册nexofolio-evidence工作区依赖 | 新增纯机械证据crate是用户计划明确要求 | 保留：仅1行，不新增微服务 |
| [README.md](/Users/sheldon/Documents/GithubProject/NexoFolio/README.md) | +63/−6 | 补充采集、目录、维护、命令、运行配置和验收边界 | 仓库交付后需能启动并知道什么已做/未做 | 保留：应聚焦入口，过程试验下沉验证文档 |
| [apps/backend/Cargo.toml](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/Cargo.toml) | +28/−0 | 增加传输校验/模型装配/测试目标及下载reqwest+YAML+semver依赖 | 实际路由和独立集成测试编译需要；下载依赖来自另一个任务 | 保留：直接依赖按功能分审，别把下载算入维护引擎 |
| [apps/backend/examples/downloads_preview.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/examples/downloads_preview.rs) | +21/−0 | 启动隔离下载路由联调服务 | 下载任务不重启共享18080也能测试真实路由 | 保留：开发/交付工具，不计入运行时业务 |
| [apps/backend/src/bin/admin.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/bin/admin.rs) | +112/−0 | 增加路径策略、旧目录候选、单条处理和失败重试命令 | 早期无前端时提供实际操作入口；路径例外和故障恢复仍需运维通道 | 收缩：旧目录生成命令停止扩展，统一维护成熟后再退役 |
| [apps/backend/src/bin/worker.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/bin/worker.rs) | +36/−1 | 装配并运行观测处理、证据处理和知识维护三个后台循环 | 把解析、关联和模型耗时移出批次接收；共用关闭信号 | 保留：调度入口应继续保持薄 |
| [apps/backend/src/http/access.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/http/access.rs) | +78/−3 | 在既有会话鉴权下增加环境列表、创建和改名，同时容纳业务HTTP状态 | 用户要求插件选环境及平台内部管理环境；沿用项目权限 | 调整：环境路由应独立，AccessHttp不应长期容纳所有业务状态 |
| [apps/backend/src/http/capture.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/http/capture.rs) | +306/−0 | 接收v3、能力声明、资产上传下载、观测与证据读取 | 同结构仍需独立证据收讫；资产与JSON分开；每次读取要鉴权 | 保留并拆小：接收、资产、证据查询三个传输职责当前合在306行 |
| [apps/backend/src/http/catalog_preview.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/http/catalog_preview.rs) | +76/−0 | 旧目录候选任务列表和详情接口 | 已有候选页面/记录需要继续可读 | 兼容保留：不作为新维护主入口 |
| [apps/backend/src/http/documents.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/http/documents.rs) | +123/−0 | 正式观测接口列表、环境定义、观测历史与原文读取 | 目录之外需要查看接口本体；结构是观测资料，不等于人工确认合同 | 保留：与后续语义层分开读取 |
| [apps/backend/src/http/downloads.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/http/downloads.rs) | +333/−0 | 公开发行元信息API、并发合并、缓存、上游限时和分项结果 | 另一下载任务用于解决浏览器直读OSS及渠道聚合；与接口知识维护无关 | 单独交付：有独立需求依据，不能拿来解释本轮核心功能体量 |
| [apps/backend/src/http/downloads/manifests.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/http/downloads/manifests.rs) | +176/−0 | 解析Fetcher JSON及桌面YAML，验证版本/文件名/安装包链接 | 下载任务需处理两种清单；只返回经过约束的发行地址 | 单独交付：当前直接放HTTP模块，可接受小型读服务但不要蔓延 |
| [apps/backend/src/http/downloads/tests.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/http/downloads/tests.rs) | +458/−0 | 下载清单解析、缓存并发、体积/超时、渠道和路由测试 | 独立下载任务的故障面与知识维护无关 | 单独交付：458行测试不属于生产代码 |
| [apps/backend/src/http/ingestion.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/http/ingestion.rs) | +289/−0 | v1/v2接收、协议选择、JSON校验、大小并发保护及v3分流 | 旧插件队列不能失效；共同HTTP边界应稳定 | 保留并关注：三代协议分支需明确退役规则，不能无限复制 |
| [apps/backend/src/http/maintenance.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/http/maintenance.rs) | +179/−0 | 显式创建维护、任务状态/快照/检查点、语义读取、发布回退 | 前端无需挑全量或局部；所有写操作走会话和项目权限 | 保留：对应用户明确的统一知识维护入口 |
| [apps/backend/src/http/mod.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/http/mod.rs) | +41/−1 | 挂载采集、文档、目录、维护及下载路由，保留健康检查/MCP | 新功能只有装入主Router才能被客户端调用；下载是另外授权的功能 | 保留：装配变更需独立审查，源码挂载不等于运行镜像已更新 |
| [apps/backend/src/http/official_catalog.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/http/official_catalog.rs) | +120/−0 | 正式目录读取及旧目录专用发布/回退 | 保留已存在API，并保证旧目录操作不回退语义 | 兼容保留：界面应降为次级，避免误用 |
| [apps/backend/src/wiring/access.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/wiring/access.rs) | +65/−3 | 创建存储实现、应用服务和HTTP依赖并注入 | 保持业务crate不直接new数据库；按capture配置启用新链路 | 调整：函数已超出access职责，应拆环境/采集/知识装配 |
| [apps/backend/src/wiring/catalog.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/wiring/catalog.rs) | +72/−0 | 分别创建旧目录模型和统一维护模型/存储/预算 | 复用现有provider，同时允许未配置能力明确报错 | 调整：旧模型路径不再新开发，最终收敛一条模型调用适配 |
| [apps/backend/src/wiring/config.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/wiring/config.rs) | +62/−0 | 增加模型、调用预算、快照大小、截图理解开关和文件目录配置 | 外部能力和资源上限不能散落硬编码；密钥用文件提供 | 保留并改进：需增加任务总耗时/实际用量观测；不能把256次上限当可接受时长 |
| [apps/backend/src/wiring/lifecycle.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/wiring/lifecycle.rs) | +66/−0 | 增加结构、证据、维护worker运行循环和关闭处理 | 后台任务持续领取、失败后退让、退出时停止，需要独立于HTTP生命周期 | 调整：三个类似循环可共享少量生命周期辅助，避免做通用调度框架 |
| [apps/backend/src/wiring/mod.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/apps/backend/src/wiring/mod.rs) | +8/−1 | 导出新的装配/生命周期入口 | bin入口只引用明确公共接口 | 保留：只是模块注册，不是独立业务功能 |
| [contracts/capture/asset.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/asset.schema.json) | +37/−0 | 独立资产上传/复用回执 | 从Rust DTO生成；为独立资产上传/复用回执提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/capture/batch.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/batch.schema.json) | +93/−0 | v3项目/环境/producer批次信封 | 从Rust DTO生成；为v3项目/环境/producer批次信封提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/capture/behavior.md](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/behavior.md) | +15/−0 | 该合同包无法仅靠Schema表达的行为规则 | 说明幂等/权限/兼容/状态解释，避免accepted被当成新接口 | 保留：人工维护的协议语义 |
| [contracts/capture/capabilities.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/capabilities.schema.json) | +123/−0 | 版本/大小/并发/资产能力声明 | 从Rust DTO生成；为版本/大小/并发/资产能力声明提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/capture/context.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/context.schema.json) | +64/−0 | 浏览器/页面/frame/view/事件顺序和时间上下文 | 从Rust DTO生成；为浏览器/页面/frame/view/事件顺序和时间上下文提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/capture/evidence-page.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/evidence-page.schema.json) | +86/−0 | 证据分页列表 | 从Rust DTO生成；为证据分页列表提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/capture/evidence.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/evidence.schema.json) | +53/−0 | 单项证据事实、样例引用和计数 | 从Rust DTO生成；为单项证据事实、样例引用和计数提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/capture/image-reference.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/image-reference.schema.json) | +51/−0 | 截图资产ID、视口与完整性 | 从Rust DTO生成；为截图资产ID、视口与完整性提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/capture/interaction.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/interaction.schema.json) | +172/−0 | 点击/选择/提交及目标控件 | 从Rust DTO生成；为点击/选择/提交及目标控件提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/capture/manifest.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/manifest.json) | +21/−0 | 该合同包的版本和文件SHA清单 | 让跨仓库同步能检测错版/漏同步/改内容不升版 | 保留：合同治理必要的机器清单 |
| [contracts/capture/observation.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/observation.schema.json) | +152/−0 | 可靠收讫后的结构/证据分别处理状态 | 从Rust DTO生成；为可靠收讫后的结构/证据分别处理状态提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/capture/page-context.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/page-context.schema.json) | +43/−0 | 页面地址/标题/面包屑和覆盖缺口 | 从Rust DTO生成；为页面地址/标题/面包屑和覆盖缺口提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/capture/receipt.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/receipt.schema.json) | +115/−0 | v3逐记录accepted/replayed/rejected回执 | 从Rust DTO生成；为v3逐记录accepted/replayed/rejected回执提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/capture/record.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/record.schema.json) | +703/−0 | 多种记录payload联合与公共元信息 | 从Rust DTO生成；为多种记录payload联合与公共元信息提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/capture/types.generated.ts](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/types.generated.ts) | +34/−0 | 该合同包的TypeScript类型 | 由Schema自动生成，插件/前端复制同版类型；静态类型不替代运行时校验 | 保留分发物：只改源合同后重生成 |
| [contracts/capture/ui-snapshot.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/capture/ui-snapshot.schema.json) | +163/−0 | 元素快照、值/标签/选项与完整性 | 从Rust DTO生成；为元素快照、值/标签/选项与完整性提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/catalog-preview/activation.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/activation.schema.json) | +30/−0 | 旧目录发布/回退幂等结果与代次 | 从Rust DTO生成；为旧目录发布/回退幂等结果与代次提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/catalog-preview/behavior.md](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/behavior.md) | +27/−0 | 该合同包无法仅靠Schema表达的行为规则 | 说明幂等/权限/兼容/状态解释，避免accepted被当成新接口 | 保留：人工维护的协议语义 |
| [contracts/catalog-preview/candidate.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/candidate.schema.json) | +114/−0 | 目录节点/接口归属/展示合并组 | 从Rust DTO生成；为目录节点/接口归属/展示合并组提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/catalog-preview/detail.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/detail.schema.json) | +501/−0 | 候选任务详情与是否已发布 | 从Rust DTO生成；为候选任务详情与是否已发布提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/catalog-preview/manifest.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/manifest.json) | +19/−0 | 该合同包的版本和文件SHA清单 | 让跨仓库同步能检测错版/漏同步/改内容不升版 | 保留：合同治理必要的机器清单 |
| [contracts/catalog-preview/official-interfaces.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/official-interfaces.schema.json) | +93/−0 | 正式目录接口分页及环境定义 | 从Rust DTO生成；为正式目录接口分页及环境定义提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/catalog-preview/official.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/official.schema.json) | +135/−0 | 正式树、固定待分类及来源版本 | 从Rust DTO生成；为正式树、固定待分类及来源版本提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/catalog-preview/page.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/page.schema.json) | +99/−0 | 旧候选任务分页 | 从Rust DTO生成；为旧候选任务分页提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/catalog-preview/prompt.txt](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/prompt.txt) | +8/−0 | 旧目录模型的外置提示词 | 限定候选树和来源，不给模型发布权限 | 兼容保留：旧模型路径停止扩展 |
| [contracts/catalog-preview/publish.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/publish.schema.json) | +25/−0 | 旧目录发布请求及期望代次 | 从Rust DTO生成；为旧目录发布请求及期望代次提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/catalog-preview/restore.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/restore.schema.json) | +30/−0 | 旧目录回退目标与幂等请求 | 从Rust DTO生成；为旧目录回退目标与幂等请求提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/catalog-preview/snapshot.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/snapshot.schema.json) | +142/−0 | 旧候选所依据的接口与环境定义快照 | 从Rust DTO生成；为旧候选所依据的接口与环境定义快照提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/catalog-preview/task.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/task.schema.json) | +485/−0 | 旧候选生命周期、候选/评估/生成器信息 | 从Rust DTO生成；为旧候选生命周期、候选/评估/生成器信息提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/catalog-preview/types.generated.ts](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/types.generated.ts) | +31/−0 | 该合同包的TypeScript类型 | 由Schema自动生成，插件/前端复制同版类型；静态类型不替代运行时校验 | 保留分发物：只改源合同后重生成 |
| [contracts/catalog-preview/versions.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/catalog-preview/versions.schema.json) | +74/−0 | 目录历史版本分页 | 从Rust DTO生成；为目录历史版本分页提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/documents/behavior.md](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/documents/behavior.md) | +25/−0 | 该合同包无法仅靠Schema表达的行为规则 | 说明幂等/权限/兼容/状态解释，避免accepted被当成新接口 | 保留：人工维护的协议语义 |
| [contracts/documents/manifest.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/documents/manifest.json) | +8/−0 | 该合同包的版本和文件SHA清单 | 让跨仓库同步能检测错版/漏同步/改内容不升版 | 保留：合同治理必要的机器清单 |
| [contracts/documents/responses.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/documents/responses.schema.json) | +587/−0 | 观测接口列表/详情/历史/原文响应集合 | Schema作为权威源维护；为观测接口列表/详情/历史/原文响应集合提供运行时类型校验，前后端不靠手写猜字段 | 保留权威Schema：不能笼统算作生成代码 |
| [contracts/documents/types.generated.ts](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/documents/types.generated.ts) | +8/−0 | 该合同包的TypeScript类型 | 由Schema自动生成，插件/前端复制同版类型；静态类型不替代运行时校验 | 保留分发物：只改源合同后重生成 |
| [contracts/ingestion/batch.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/batch.schema.json) | +321/−0 | v2完整HTTP批次与结构payload | Schema作为权威源维护；为v2完整HTTP批次与结构payload提供运行时类型校验，前后端不靠手写猜字段 | 保留权威Schema：不能笼统算作生成代码 |
| [contracts/ingestion/behavior.md](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/behavior.md) | +29/−0 | 该合同包无法仅靠Schema表达的行为规则 | 说明幂等/权限/兼容/状态解释，避免accepted被当成新接口 | 保留：人工维护的协议语义 |
| [contracts/ingestion/capabilities.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/capabilities.schema.json) | +47/−0 | v1/v2接收版本和资源限制 | Schema作为权威源维护；为v1/v2接收版本和资源限制提供运行时类型校验，前后端不靠手写猜字段 | 保留权威Schema：不能笼统算作生成代码 |
| [contracts/ingestion/envelope.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/envelope.schema.json) | +87/−0 | 先校验公共信封再逐记录返回错误的浅层结构 | Schema作为权威源维护；为先校验公共信封再逐记录返回错误的浅层结构提供运行时类型校验，前后端不靠手写猜字段；浅信封与完整batch用途不同，允许逐条拒绝而不整批失败 | 保留权威Schema：不能笼统算作生成代码 |
| [contracts/ingestion/environment-page.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/environment-page.schema.json) | +50/−0 | 项目环境分页 | Schema作为权威源维护；为项目环境分页提供运行时类型校验，前后端不靠手写猜字段 | 保留权威Schema：不能笼统算作生成代码 |
| [contracts/ingestion/environment-write.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/environment-write.schema.json) | +16/−0 | 新建/改名环境的名称请求 | Schema作为权威源维护；为新建/改名环境的名称请求提供运行时类型校验，前后端不靠手写猜字段 | 保留权威Schema：不能笼统算作生成代码 |
| [contracts/ingestion/environment.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/environment.schema.json) | +21/−0 | 项目环境ID与名称 | Schema作为权威源维护；为项目环境ID与名称提供运行时类型校验，前后端不靠手写猜字段 | 保留权威Schema：不能笼统算作生成代码 |
| [contracts/ingestion/fixtures/http-batch.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/fixtures/http-batch.json) | +73/−0 | 该协议版本的合法HTTP采集样例 | 跨仓库联调用稳定样例验证，不用真实密钥/业务载荷 | 保留：旧样例不能覆盖改写 |
| [contracts/ingestion/legacy/v1/batch.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/legacy/v1/batch.schema.json) | +328/−0 | 旧v1冻结兼容：v2完整HTTP批次与结构payload | 冻结旧队列原签名（包括旧service_key），避免升级后无法重试；不把旧行为迁移为新行为 | 兼容保留：不是第二套新功能；确认旧队列耗尽且退役后才能清理 |
| [contracts/ingestion/legacy/v1/behavior.md](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/legacy/v1/behavior.md) | +29/−0 | 旧v1冻结兼容：该合同包无法仅靠Schema表达的行为规则 | 说明幂等/权限/兼容/状态解释，避免accepted被当成新接口 | 兼容保留：不是第二套新功能；确认旧队列耗尽且退役后才能清理 |
| [contracts/ingestion/legacy/v1/capabilities.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/legacy/v1/capabilities.schema.json) | +46/−0 | 旧v1冻结兼容：v1/v2接收版本和资源限制 | 冻结旧队列原签名（包括旧service_key），避免升级后无法重试；不把旧行为迁移为新行为 | 兼容保留：不是第二套新功能；确认旧队列耗尽且退役后才能清理 |
| [contracts/ingestion/legacy/v1/envelope.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/legacy/v1/envelope.schema.json) | +94/−0 | 旧v1冻结兼容：先校验公共信封再逐记录返回错误的浅层结构 | 冻结旧队列原签名（包括旧service_key），避免升级后无法重试；不把旧行为迁移为新行为 | 兼容保留：不是第二套新功能；确认旧队列耗尽且退役后才能清理 |
| [contracts/ingestion/legacy/v1/environment-page.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/legacy/v1/environment-page.schema.json) | +50/−0 | 旧v1冻结兼容：项目环境分页 | 冻结旧队列原签名（包括旧service_key），避免升级后无法重试；不把旧行为迁移为新行为 | 兼容保留：不是第二套新功能；确认旧队列耗尽且退役后才能清理 |
| [contracts/ingestion/legacy/v1/environment-write.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/legacy/v1/environment-write.schema.json) | +16/−0 | 旧v1冻结兼容：新建/改名环境的名称请求 | 冻结旧队列原签名（包括旧service_key），避免升级后无法重试；不把旧行为迁移为新行为 | 兼容保留：不是第二套新功能；确认旧队列耗尽且退役后才能清理 |
| [contracts/ingestion/legacy/v1/environment.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/legacy/v1/environment.schema.json) | +21/−0 | 旧v1冻结兼容：项目环境ID与名称 | 冻结旧队列原签名（包括旧service_key），避免升级后无法重试；不把旧行为迁移为新行为 | 兼容保留：不是第二套新功能；确认旧队列耗尽且退役后才能清理 |
| [contracts/ingestion/legacy/v1/fixtures/http-batch.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/legacy/v1/fixtures/http-batch.json) | +74/−0 | 旧v1冻结兼容：该协议版本的合法HTTP采集样例 | 跨仓库联调用稳定样例验证，不用真实密钥/业务载荷 | 兼容保留：不是第二套新功能；确认旧队列耗尽且退役后才能清理 |
| [contracts/ingestion/legacy/v1/manifest.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/legacy/v1/manifest.json) | +21/−0 | 旧v1冻结兼容：该合同包的版本和文件SHA清单 | 让跨仓库同步能检测错版/漏同步/改内容不升版 | 兼容保留：不是第二套新功能；确认旧队列耗尽且退役后才能清理 |
| [contracts/ingestion/legacy/v1/receipt.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/legacy/v1/receipt.schema.json) | +113/−0 | 旧v1冻结兼容：v1/v2逐记录accepted/ignored/rejected收讫结果 | 冻结旧队列原签名（包括旧service_key），避免升级后无法重试；不把旧行为迁移为新行为 | 兼容保留：不是第二套新功能；确认旧队列耗尽且退役后才能清理 |
| [contracts/ingestion/legacy/v1/types.generated.ts](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/legacy/v1/types.generated.ts) | +11/−0 | 旧v1冻结兼容：该合同包的TypeScript类型 | 由Schema自动生成，插件/前端复制同版类型；静态类型不替代运行时校验 | 兼容保留：不是第二套新功能；确认旧队列耗尽且退役后才能清理 |
| [contracts/ingestion/manifest.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/manifest.json) | +32/−0 | 该合同包的版本和文件SHA清单 | 让跨仓库同步能检测错版/漏同步/改内容不升版 | 保留：合同治理必要的机器清单 |
| [contracts/ingestion/receipt.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/receipt.schema.json) | +113/−0 | v1/v2逐记录accepted/ignored/rejected收讫结果 | Schema作为权威源维护；为v1/v2逐记录accepted/ignored/rejected收讫结果提供运行时类型校验，前后端不靠手写猜字段 | 保留权威Schema：不能笼统算作生成代码 |
| [contracts/ingestion/types.generated.ts](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/ingestion/types.generated.ts) | +11/−0 | 该合同包的TypeScript类型 | 由Schema自动生成，插件/前端复制同版类型；静态类型不替代运行时校验 | 保留分发物：只改源合同后重生成 |
| [contracts/maintenance/activation.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/activation.schema.json) | +41/−0 | 统一目录/语义发布回执与版本代次 | 从Rust DTO生成；为统一目录/语义发布回执与版本代次提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/maintenance/behavior.md](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/behavior.md) | +17/−0 | 该合同包无法仅靠Schema表达的行为规则 | 说明幂等/权限/兼容/状态解释，避免accepted被当成新接口 | 保留：人工维护的协议语义 |
| [contracts/maintenance/candidate.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/candidate.schema.json) | +875/−0 | 目录与语义物化结果、变更计划和覆盖校验 | 从Rust DTO生成；为目录与语义物化结果、变更计划和覆盖校验提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/maintenance/checkpoints.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/checkpoints.schema.json) | +166/−0 | 分阶段审阅/回读检查点分页 | 从Rust DTO生成；为分阶段审阅/回读检查点分页提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/maintenance/interface-knowledge.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/interface-knowledge.schema.json) | +338/−0 | 指定环境与修订的接口语义资料 | 从Rust DTO生成；为指定环境与修订的接口语义资料提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/maintenance/manifest.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/manifest.json) | +20/−0 | 该合同包的版本和文件SHA清单 | 让跨仓库同步能检测错版/漏同步/改内容不升版 | 保留：合同治理必要的机器清单 |
| [contracts/maintenance/page.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/page.schema.json) | +155/−0 | 维护任务列表摘要分页 | 从Rust DTO生成；为维护任务列表摘要分页提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/maintenance/plan.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/plan.schema.json) | +626/−0 | 模型选择策略及各类变更动作 | 从Rust DTO生成；为模型选择策略及各类变更动作提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/maintenance/publish.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/publish.schema.json) | +20/−0 | 统一知识发布请求及期望代次 | 从Rust DTO生成；为统一知识发布请求及期望代次提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/maintenance/reply.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/reply.schema.json) | +811/−0 | 模型分段审阅/汇总/回读/计划回复联合 | 从Rust DTO生成；为模型分段审阅/汇总/回读/计划回复联合提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/maintenance/restore.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/restore.schema.json) | +30/−0 | 统一知识回退请求及显式目标 | 从Rust DTO生成；为统一知识回退请求及显式目标提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/maintenance/run.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/run.schema.json) | +960/−0 | 任务状态、进度、模型次数与候选 | 从Rust DTO生成；为任务状态、进度、模型次数与候选提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/maintenance/snapshot.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/snapshot.schema.json) | +777/−0 | 全量接口/字段/事实/目录/语义固定输入 | 从Rust DTO生成；为全量接口/字段/事实/目录/语义固定输入提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/maintenance/start.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/start.schema.json) | +15/−0 | 仅含幂等request_id的显式重构请求 | 从Rust DTO生成；为仅含幂等request_id的显式重构请求提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [contracts/maintenance/types.generated.ts](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/types.generated.ts) | +51/−0 | 该合同包的TypeScript类型 | 由Schema自动生成，插件/前端复制同版类型；静态类型不替代运行时校验 | 保留分发物：只改源合同后重生成 |
| [contracts/maintenance/versions.schema.json](/Users/sheldon/Documents/GithubProject/NexoFolio/contracts/maintenance/versions.schema.json) | +83/−0 | 统一知识版本历史 | 从Rust DTO生成；为统一知识版本历史提供运行时类型校验，前后端不靠手写猜字段 | 保留合同分发物：生成Schema有重复定义，适合自动校验/折叠审查而非逐行人工维护 |
| [crates/access/src/platform.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/access/src/platform.rs) | +29/−0 | 增加项目环境管理的访问端口和分页类型 | 插件与平台需同一项目权限约束下读写环境 | 调整：环境存储当前挂在PlatformAccess，使认证端口变宽 |
| [crates/application/Cargo.toml](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/application/Cargo.toml) | +3/−0 | application包依赖/特性声明 | 编排维护阶段使用序列化、内容摘要及evidence端口；不接入实际禅道或LLM到核心crate | 保留：依赖白名单检查可验证，但不能替代文件职责审查 |
| [crates/application/src/capture.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/application/src/capture.rs) | +10/−0 | 用一个trait组合采集收讫与证据读取存储端口 | 避免intake直接依赖evidence；基础设施可以实现同一对象 | 保留：10行是实际依赖边界，不值得再拆一个crate |
| [crates/application/src/catalog_preview.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/application/src/catalog_preview.rs) | +61/−0 | 旧目录快照生成、模型调用、结构检查的编排 | 早期可评估目录而不发布的入口 | 兼容保留：与新引擎有功能重叠，应冻结 |
| [crates/application/src/catalog_publication.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/application/src/catalog_publication.rs) | +85/−0 | 旧目录发布与回退应用服务、期望版本检查 | 保证旧候选先检查再进入事务，不绕过目录保护 | 兼容保留：和统一发布共享不变量，不继续双份实现规则 |
| [crates/application/src/ingestion.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/application/src/ingestion.rs) | +43/−0 | 串接项目鉴权、批次预处理和可靠接收存储 | 热路径只做必要判断，不触发LLM | 保留：服务较薄但有真实编排价值 |
| [crates/application/src/lib.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/application/src/lib.rs) | +19/−0 | 导出采集、处理、目录发布、维护编排模块 | 对HTTP/基础设施只暴露应用层公共入口 | 保留：不应直接塞入实现 |
| [crates/application/src/maintenance.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/application/src/maintenance.rs) | +120/−0 | 定义租约/检查点/模型调用审计/统一发布/受控回读端口 | 让长任务、发布策略和外部存储可替换 | 保留并调整：120行多个端口可按生命周期组织，勿变成通用工作流框架 |
| [crates/application/src/maintenance_engine.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/application/src/maintenance_engine.rs) | +950/−0 | 逐段审阅、汇总、全局决策、回读、预算及失败恢复 | 用户明确要求全量覆盖且不静默抽样；单次LLM调用无法可靠完成所有阶段 | 需要收缩：950行同时处理分片和编排，拆预算/快照读取/执行阶段；模型成本收益尚未充分证明 |
| [crates/application/src/processing.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/application/src/processing.rs) | +31/−0 | 领取并完成一条观测处理 | 把可靠收讫与正式接口建档分开 | 保留：31行薄编排，避免HTTP承担后台任务 |
| [crates/contracts/Cargo.toml](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/contracts/Cargo.toml) | +0/−2 | contracts包依赖/特性声明 | 让目录/维护公共合同能被Schema生成示例和跨crate调用；不接入实际禅道或LLM到核心crate | 保留：依赖白名单检查可验证，但不能替代文件职责审查 |
| [crates/contracts/examples/capture_schema.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/contracts/examples/capture_schema.rs) | +11/−0 | 导出Rust采集DTO的Schema集合 | 供capture_contract脚本使用，不是生产bin | 保留：开发/交付工具，不计入运行时业务 |
| [crates/contracts/examples/catalog_preview_schema.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/contracts/examples/catalog_preview_schema.rs) | +25/−0 | 导出Rust目录DTO的Schema集合 | 旧合同也要与Rust类型一致 | 保留：开发/交付工具，不计入运行时业务 |
| [crates/contracts/examples/maintenance_schema.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/contracts/examples/maintenance_schema.rs) | +9/−0 | 导出Rust维护DTO的Schema集合 | 约束模型输出与前端类型来源 | 保留：开发/交付工具，不计入运行时业务 |
| [crates/contracts/src/capture.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/contracts/src/capture.rs) | +260/−0 | 持续采集上下文、记录种类、字段引用、证据与收讫类型 | HTTP与未来页面/图片扩展共用稳定信封，结构与证据分开返回 | 保留：合同必要，但浏览器原生ID不能直接假设等同此UUID |
| [crates/contracts/src/catalog_preview.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/contracts/src/catalog_preview.rs) | +195/−0 | 旧快照、候选树、归属、展示合并组、任务与评估类型 | 保存历史候选及前端预览，合并展示不修改原始接口 | 兼容保留：防止与统一维护重复定义同一目录结构 |
| [crates/contracts/src/environment.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/contracts/src/environment.rs) | +39/−0 | 按ID/名称指定环境、环境实体与名称检查 | 满足插件选择已有环境或临时创建，禁止从domain猜环境 | 保留：和请求体稳定性直接相关 |
| [crates/contracts/src/ids.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/contracts/src/ids.rs) | +1/−0 | 增加EnvironmentId强类型 | 避免将环境ID与项目/接口ID混用，接口ID仍独立于目录 | 保留：与业务规则直接对应 |
| [crates/contracts/src/lib.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/contracts/src/lib.rs) | +21/−0 | 注册跨模块环境/目录/证据/维护合同及共享比较工具 | 使核心模块通过公共数据结构通信 | 调整：公共合同已夹带算法，不能继续把所有共享逻辑都堆入contracts |
| [crates/contracts/src/maintenance.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/contracts/src/maintenance.rs) | +372/−0 | 语义目标、关系/枚举/描述动作、快照、审阅、候选、发布和历史类型 | 语义修改可追溯可回退，不能只存一段LLM中文说明 | 保留并审查：动作模型较宽，应以真实使用证明每种动作；自由JSON元数据需更清楚约束 |
| [crates/contracts/src/observed_comparison.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/contracts/src/observed_comparison.rs) | +109/−0 | 空数组/null等不足证据的方向性结构覆盖比较 | 同一判断被接收和后台定义处理使用，不能各自判成删除 | 调整归属：规则必要，放contracts是为避免核心相互依赖，但它是算法不是纯合同 |
| [crates/contracts/src/official_catalog.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/contracts/src/official_catalog.rs) | +127/−0 | 正式目录、固定待分类、版本和并发发布回执类型 | 发布/回退需要明确目标与期望代次，恢复空初始版也必须显式指定 | 保留：防止缺字段被当成清空目录 |
| [crates/contracts/src/path_identity.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/contracts/src/path_identity.rs) | +162/−0 | 路径数字/UUID/32或64hex参数化及项目例外策略 | 第一次采集即识别高置信动态段，保留原路径与保守例外 | 调整归属：算法应有独立纯规则边界；普通数字误判仍需项目例外，不能宣称无误 |
| [crates/evidence/Cargo.toml](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/evidence/Cargo.toml) | +18/−0 | evidence包依赖/特性声明 | 独立纯提取crate所需contracts、serde、URL/编码工具；不接入实际禅道或LLM到核心crate | 保留：依赖白名单检查可验证，但不能替代文件职责审查 |
| [crates/evidence/src/extraction.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/evidence/src/extraction.rs) | +398/−0 | 提取字段值、字典/控件选项、类型转换与省略行为 | 纯机械处理不调用模型；保留值类型、数组通配字段与样例位置 | 保留：这是新增evidence crate的主要价值，关联规则应更多移入这里 |
| [crates/evidence/src/lib.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/evidence/src/lib.rs) | +57/−0 | 暴露纯提取接口、BlobStore及证据存储端口 | 文件卷未来可换对象存储，提取不能直接调用SQL | 保留：接口和实现依赖方向可检查 |
| [crates/infrastructure/Cargo.toml](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/Cargo.toml) | +2/−0 | infrastructure包依赖/特性声明 | 数据库之外新增文件编码/模型HTTP/证据适配；不接入实际禅道或LLM到核心crate | 保留：依赖白名单检查可验证，但不能替代文件职责审查 |
| [crates/infrastructure/src/admission.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/admission.rs) | +248/−0 | v1/v2事务接收、幂等回执、同结构快速忽略及入箱 | 旧队列重试必须稳定；先可靠落库再返回 | 兼容保留并复用：与capture_store存在头记录/幂等重复实现，抽小型共享事务辅助 |
| [crates/infrastructure/src/blob_store.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/blob_store.rs) | +184/−0 | 项目隔离内容寻址文件存取、原子写、读取校验 | 图片/原文不把大二进制塞进批次；持久卷可替换 | 保留：真实外部存储边界，内容去重与权限仍由上层配合 |
| [crates/infrastructure/src/capture_store.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/capture_store.rs) | +503/−0 | v3收讫事务、结构与证据分流、背压、原文和资产索引读取 | 重复结构的新证据不能被丢；回执表示已可靠接收 | 需要收缩：503行覆盖收讫与多种查询，应按事务写入/资产/证据读拆开 |
| [crates/infrastructure/src/catalog_model.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/catalog_model.rs) | +138/−0 | 旧单轮目录模型HTTP适配及输出解析 | 早期目录预览复用用户配置的模型 | 兼容保留：和maintenance_model共存有迁移原因，不是两套长期模型平台 |
| [crates/infrastructure/src/catalog_preview.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/catalog_preview.rs) | +280/−0 | 旧候选快照/租约/状态存储与鉴权读取 | 历史快照需要不可变和可审阅，失败不能伪造成功 | 兼容保留：新任务统一走maintenance_store |
| [crates/infrastructure/src/documents.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/documents.rs) | +247/−0 | 后台建档/修订/差异记录、领取重试与文档查询 | 将已接收观测转为环境最新定义，差异保持待确认 | 保留并分层：读查询与处理事务当前混在同一实现，可分文件但无需新crate |
| [crates/infrastructure/src/environments.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/environments.rs) | +110/−0 | 事务内按名称解析/自动建环境、并发重名处理与改名 | 平台管理与外部上传必须用同一个环境解析规则 | 保留：避免插件和后端创建两个同名环境 |
| [crates/infrastructure/src/evidence_processing.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/evidence_processing.rs) | +585/−0 | 任务领取、值索引、关系/UI关联、支持样例、反例和去重落库 | 把120秒同上下文的关联移到后台，重试不刷支持次数 | 需要收缩：585行把关联判断和SQL交织；纯规则移回evidence，数据库只提供有界候选 |
| [crates/infrastructure/src/evidence_retention.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/evidence_retention.rs) | +53/−0 | 24小时临时原文回收，工作样例/候选pin保护与孤立blob清理 | 用户要求控制持续录制占用，同时保留可追溯证据 | 保留但待规模验证：当前一次事务按项目加锁，需验证大数据下的清理批量和锁时长 |
| [crates/infrastructure/src/knowledge_publication.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/knowledge_publication.rs) | +297/−0 | 目录与语义原子发布、统一回退、旧目录操作保留语义 | 不能出现目录切换而描述没切换，也不能回退删除新观测 | 保留并去重：与official_catalog共享权限/版本/回执工具，不改变两种操作语义 |
| [crates/infrastructure/src/lib.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/lib.rs) | +35/−0 | 注册数据库、模型和文件存储适配实现 | 由装配层注入具体实现，核心模块无需依赖SDK | 保留：逐步收紧pub导出，避免把内部辅助函数暴露出去 |
| [crates/infrastructure/src/maintenance_model.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/maintenance_model.rs) | +220/−0 | 按阶段构造受约束模型输入、协议schema与HTTP调用 | 保存发给provider的精确JSON；模型只能产候选，无发布能力 | 保留并改进：字节预算是保守估算，不是真token成本；图片能力不能默认算已读 |
| [crates/infrastructure/src/maintenance_store.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/maintenance_store.rs) | +516/−0 | 固定知识快照、任务幂等/租约/检查点/调用记录/受控原文读取 | 持续录制期间任务必须有固定输入，旧执行不能覆盖新执行 | 需要收缩：516行跨快照和执行存储，分文件并明确一致性/读取清单测试 |
| [crates/infrastructure/src/official_catalog.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/official_catalog.rs) | +417/−0 | 正式目录/版本读取、旧发布回退、固定待分类和新接口兜底 | 旧快照发布或回退不能丢掉快照后新接口 | 兼容保留并复用：读取继续需要，旧发布路径应复用新底层不变量 |
| [crates/infrastructure/src/path_policy.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/path_policy.rs) | +47/−0 | 持久化每项目路径参数化开关和静态前缀例外 | 数字段并非总是ID，需可修正规则而不改历史回执 | 保留：小型适配，不扩展为通用规则引擎 |
| [crates/infrastructure/src/platform_store.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/infrastructure/src/platform_store.rs) | +91/−0 | 实现环境增查改及与权限一致的事务锁 | 同名环境自动创建必须处理并发，并在项目权限内完成 | 调整：与登录/项目同步同文件，后续应聚合为独立环境存储适配 |
| [crates/intake/Cargo.toml](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/intake/Cargo.toml) | +5/−0 | intake包依赖/特性声明 | 结构指纹、URL解析和原始载荷解码；不接入实际禅道或LLM到核心crate | 保留：依赖白名单检查可验证，但不能替代文件职责审查 |
| [crates/intake/src/batch.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/intake/src/batch.rs) | +121/−0 | v1/v2与v3批次、预处理记录、收讫回执和存储端口 | 先定义外部可用签名，再让数据库实现它 | 保留：接口面较宽需保持版本隔离，旧载荷不可重写 |
| [crates/intake/src/http_exchange.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/intake/src/http_exchange.rs) | +361/−0 | 请求/响应结构投影、指纹、值无关比较、路径策略及v3准备 | 频繁XHR要快速忽略同结构，空/截断不能伪证字段删除 | 保留：性能与正确性核心；本轮证据持久化开销应另算，不能被指纹优化掩盖 |
| [crates/intake/src/lib.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/intake/src/lib.rs) | +7/−0 | 导出批次合同、HTTP结构识别与v3准备函数 | 接收热路径与后台处理分离 | 保留：不能引入模型或触发器实现 |
| [crates/intake/src/models.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/intake/src/models.rs) | +1/−0 | 结构身份类型增加可序列化支持 | 接口身份需要稳定地用于指纹/传输准备 | 保留：小幅数据类型适配 |
| [crates/knowledge/Cargo.toml](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/knowledge/Cargo.toml) | +4/−0 | knowledge包依赖/特性声明 | 原始观测结构提取和目录DTO；不接入实际禅道或LLM到核心crate | 保留：依赖白名单检查可验证，但不能替代文件职责审查 |
| [crates/knowledge/src/catalog_preview.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/knowledge/src/catalog_preview.rs) | +39/−0 | 旧候选创建/读取/领取/完成存储端口 | 隔离目录生成与数据库，便于替换生成器 | 兼容保留：39行边界，不新增平行能力 |
| [crates/knowledge/src/lib.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/knowledge/src/lib.rs) | +6/−0 | 导出观测定义、文档读取、旧候选存储端口 | 正式接口与候选结构有明确读取边界 | 保留：仅公共入口 |
| [crates/knowledge/src/observed.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/knowledge/src/observed.rs) | +415/−0 | 从原始观测提取字段类型、完整性和环境定义，定义文档读写端口 | 保留观测的事实与不足，不能由空items删除已知数组结构 | 保留并去重：与intake结构投影相邻规则需共享一致性测试 |
| [crates/knowledge/src/ports.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/knowledge/src/ports.rs) | +25/−0 | 增加正式目录、目录内接口与历史版本读取端口 | 前端读取正式结果不能直接读取候选或访问SQL | 保留：接口职责明确 |
| [crates/rebuild/Cargo.toml](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/rebuild/Cargo.toml) | +2/−0 | rebuild包依赖/特性声明 | 候选JSON处理与字段稳定引用摘要；不接入实际禅道或LLM到核心crate | 保留：依赖白名单检查可验证，但不能替代文件职责审查 |
| [crates/rebuild/src/lib.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/rebuild/src/lib.rs) | +8/−0 | 注册目录预览与统一维护的规则/模型端口 | 重构规则可替换，模型实现仍在infrastructure | 保留：旧预览仅兼容，不继续双轨加功能 |
| [crates/rebuild/src/maintenance.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/rebuild/src/maintenance.rs) | +1475/−0 | 枚举字段、划分阅读单元、覆盖校验、语义动作验证/物化、回读需求和证据提示 | 保证模型确实覆盖全部字段、不跨项目、不改固定待分类、不把少量值当闭集 | 需要收缩：1475行含356行测试，剩余1119行仍混合多个职责；规则必要不等于单文件合理 |
| [crates/rebuild/src/maintenance_model.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/rebuild/src/maintenance_model.rs) | +10/−0 | 定义prepare/invoke模型端口 | 把请求构造与实际调用分开，使应用能在发送前审计 | 保留：10行端口符合可替换要求，无需额外抽象层 |
| [crates/rebuild/src/preview.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/crates/rebuild/src/preview.rs) | +354/−0 | 校验目录图、归属覆盖、固定规则及展示合并组 | LLM候选必须机械检查，不能把业务分类的“结构有效”说成正确 | 保留：新维护也复用目录校验，应保留一份规则 |
| [deploy/.env.example](/Users/sheldon/Documents/GithubProject/NexoFolio/deploy/.env.example) | +20/−0 | 列出采集卷、模型预算/密钥路径和下载发行源可配置项 | 部署不能依赖开发机隐式配置；示例不放真实秘密 | 保留：下载项独立需求；超时/调用数只是上限而非体验承诺 |
| [deploy/Dockerfile](/Users/sheldon/Documents/GithubProject/NexoFolio/deploy/Dockerfile) | +6/−2 | 补齐evidence crate构建以及持久卷运行所需目录 | workspace构建需包含新crate；API/worker使用相同镜像 | 保留：未额外引入服务 |
| [deploy/compose.catalog-model.yaml](/Users/sheldon/Documents/GithubProject/NexoFolio/deploy/compose.catalog-model.yaml) | +30/−0 | 单独注入模型配置和只读密钥文件挂载 | 不强迫基础服务依赖模型密钥；可显式组合启用 | 保留：密钥留在本地，不写入Git |
| [deploy/compose.yaml](/Users/sheldon/Documents/GithubProject/NexoFolio/deploy/compose.yaml) | +33/−0 | 共享blob卷、初始化所有权、采集开关及下载变量透传 | API写入资产后worker须可读，用户UID下重启仍可用 | 保留：storage-init不是业务微服务；运行实例需另行验证 |
| [docs/architecture/0002-modular-backend.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/0002-modular-backend.md) | +22/−0 | 更新证据与统一维护边界和依赖方向 | 架构文档需与当前实现一致 | 保留并纠偏：要明示infrastructure里尚存业务编排，不宣称已完全解耦 |
| [docs/architecture/diagrams/nexofolio-backend/README.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/README.md) | +77/−0 | 架构图使用和更新方法 | 可选可视化的操作说明 | 可选保留图源/生成方法：独立文档交付，不以业务必要性辩护 |
| [docs/architecture/diagrams/nexofolio-backend/backend.architecture.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/backend.architecture.json) | +256/−0 | 架构图组件/连线/布局源数据 | 维护图源，避免人工修改生成HTML | 可选保留图源/生成方法：独立文档交付，不以业务必要性辩护 |
| [docs/architecture/diagrams/nexofolio-backend/backend.html](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/backend.html) | +14993/−0 | Archify生成的独立交互架构图及内嵌渲染/导出运行库 | 离线打开可交互；14993行属于图工具输出，不被Rust服务加载 | 建议移出主代码审查：作为文档附件/发布产物归档；不是后端运行必要文件 |
| [docs/architecture/diagrams/nexofolio-backend/backend.visual-check.1440x900.dark.png](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/backend.visual-check.1440x900.dark.png) | 二进制 121121 B | 架构图在1440x900.dark的截图 | 人工核对该视口/主题；二进制不计文本新增 | 建议移出主代码审查：作为文档附件/发布产物归档；不是后端运行必要文件 |
| [docs/architecture/diagrams/nexofolio-backend/backend.visual-check.1440x900.light.png](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/backend.visual-check.1440x900.light.png) | 二进制 118789 B | 架构图在1440x900.light的截图 | 人工核对该视口/主题；二进制不计文本新增 | 建议移出主代码审查：作为文档附件/发布产物归档；不是后端运行必要文件 |
| [docs/architecture/diagrams/nexofolio-backend/backend.visual-check.2048x1320.dark.png](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/backend.visual-check.2048x1320.dark.png) | 二进制 163320 B | 架构图在2048x1320.dark的截图 | 人工核对该视口/主题；二进制不计文本新增 | 建议移出主代码审查：作为文档附件/发布产物归档；不是后端运行必要文件 |
| [docs/architecture/diagrams/nexofolio-backend/backend.visual-check.2048x1320.light.png](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/backend.visual-check.2048x1320.light.png) | 二进制 161069 B | 架构图在2048x1320.light的截图 | 人工核对该视口/主题；二进制不计文本新增 | 建议移出主代码审查：作为文档附件/发布产物归档；不是后端运行必要文件 |
| [docs/architecture/diagrams/nexofolio-backend/backend.visual-check.html](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/backend.visual-check.html) | +32/−0 | 可视化检查结果查看页 | 只用于架构图交付检查 | 建议移出主代码审查：作为文档附件/发布产物归档；不是后端运行必要文件 |
| [docs/architecture/diagrams/nexofolio-backend/backend.visual-check.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/backend.visual-check.json) | +548/−0 | 架构图多视口/主题检查数据 | 证明图的布局检查，不证明业务功能 | 建议移出主代码审查：作为文档附件/发布产物归档；不是后端运行必要文件 |
| [docs/architecture/diagrams/nexofolio-backend/delivery.receipt.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/delivery.receipt.json) | +24/−0 | 图交付文件哈希与尺寸回执 | 追踪图源/生成物的一致性 | 建议移出主代码审查：作为文档附件/发布产物归档；不是后端运行必要文件 |
| [docs/architecture/diagrams/nexofolio-backend/review.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/review.md) | +31/−0 | 图的人工复核范围 | 区分视觉质量与架构事实 | 建议移出主代码审查：作为文档附件/发布产物归档；不是后端运行必要文件 |
| [docs/architecture/diagrams/nexofolio-backend/sources.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/sources.json) | +152/−0 | 生成图时依据的源码路径与哈希 | 图对某一工作区快照负责，不代表现在运行服务 | 可选保留图源/生成方法：独立文档交付，不以业务必要性辩护 |
| [docs/architecture/diagrams/nexofolio-backend/update.mjs](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/update.mjs) | +55/−0 | 调用Archify校验/生成/检查的更新入口 | 重复生成图而不手动修巨大HTML | 可选保留图源/生成方法：独立文档交付，不以业务必要性辩护 |
| [docs/architecture/diagrams/nexofolio-backend/validation.receipt.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/validation.receipt.json) | +91/−0 | 图结构验证回执 | 仅架构图结构检查证据 | 建议移出主代码审查：作为文档附件/发布产物归档；不是后端运行必要文件 |
| [docs/contracts/0003-ingestion-contract-governance.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/contracts/0003-ingestion-contract-governance.md) | +132/−0 | 规定Schema/TS/manifest/行为样例和升版流程 | 用户明确要求改A能发现其他模块受影响 | 保留：行为文档不是自动生成代码，需与真实实现一致 |
| [docs/contracts/0004-ingestion-and-environments.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/contracts/0004-ingestion-and-environments.md) | +102/−0 | 接收签名、回执和环境增查改说明 | 插件与其他来源应使用公开稳定合同 | 保留：行为文档不是自动生成代码，需与真实实现一致 |
| [docs/contracts/0005-observed-interface-documents.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/contracts/0005-observed-interface-documents.md) | +64/−0 | 观测定义/不足信息/差异及查询语义 | 避免把观测当已确认最终接口合同 | 保留：行为文档不是自动生成代码，需与真实实现一致 |
| [docs/contracts/0006-catalog-preview.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/contracts/0006-catalog-preview.md) | +92/−0 | 旧目录候选、结构检查和模型约束 | 保留历史预览使用方式和兼容语义 | 保留：行为文档不是自动生成代码，需与真实实现一致 |
| [docs/contracts/0007-path-identification.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/contracts/0007-path-identification.md) | +20/−0 | 路径模板规则和例外 | 解释数字/hex模板化的适用范围与保留原文 | 保留：行为文档不是自动生成代码，需与真实实现一致 |
| [docs/contracts/0008-official-catalog.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/contracts/0008-official-catalog.md) | +21/−0 | 正式目录/固定待分类/发布回退规则 | 发布不应删除快照后接口 | 保留：行为文档不是自动生成代码，需与真实实现一致 |
| [docs/contracts/0009-continuous-knowledge-maintenance.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/contracts/0009-continuous-knowledge-maintenance.md) | +54/−0 | 持续证据、全量审阅、统一知识发布的当前合同 | 连接插件录制与字段知识维护的实际行为 | 保留：行为文档不是自动生成代码，需与真实实现一致 |
| [docs/contracts/0010-downloads.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/contracts/0010-downloads.md) | +101/−0 | 下载聚合API响应和失败状态 | 另外下载任务，不能混作持续录制需求 | 保留：行为文档不是自动生成代码，需与真实实现一致 |
| [docs/plans/0002-evidence-maintenance-implementation.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/plans/0002-evidence-maintenance-implementation.md) | +18/−0 | 本轮阶段检查单及真实Chrome未验收标记 | 记录已实现与还未验证的界限 | 保留但纠正完成措辞：浏览器关键链路失败时不能笼统说验收完成 |
| [docs/verification/2026-09-14-directional-array-comparison.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-14-directional-array-comparison.json) | +48/−0 | 空数组与已有元素结构的方向性比较的机器可读结果 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-14-directional-array-comparison.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-14-directional-array-comparison.md) | +19/−0 | 空数组与已有元素结构的方向性比较的人工阅读说明 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-14-document-quality-audit.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-14-document-quality-audit.json) | +73/−0 | 观测定义质量及不充分样本的机器可读结果 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-14-document-quality-audit.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-14-document-quality-audit.md) | +14/−0 | 观测定义质量及不充分样本的人工阅读说明 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-14-ingestion-environments.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-14-ingestion-environments.md) | +60/−0 | 接收与环境管理的人工阅读说明 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-14-observed-documents.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-14-observed-documents.json) | +41/−0 | 观测建档及环境文档读取的机器可读结果 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-14-observed-documents.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-14-observed-documents.md) | +44/−0 | 观测建档及环境文档读取的人工阅读说明 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-14-project-environment-scope.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-14-project-environment-scope.md) | +33/−0 | 取消服务标识后的项目/环境去重范围的人工阅读说明 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-catalog-preview-http.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-catalog-preview-http.json) | +31/−0 | 旧候选HTTP读取权限和响应的机器可读结果 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-catalog-preview-http.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-catalog-preview-http.md) | +14/−0 | 旧候选HTTP读取权限和响应的人工阅读说明 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-catalog-preview-model.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-catalog-preview-model.json) | +99/−0 | 旧候选真实模型试验的机器可读结果 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-catalog-preview-model.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-catalog-preview-model.md) | +28/−0 | 旧候选真实模型试验的人工阅读说明 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-catalog-preview.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-catalog-preview.json) | +43/−0 | 旧目录候选机械校验的机器可读结果 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-catalog-preview.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-catalog-preview.md) | +18/−0 | 旧目录候选机械校验的人工阅读说明 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-continuous-maintenance.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-continuous-maintenance.json) | +92/−0 | 持续证据与统一维护端到端试验的机器可读结果 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-continuous-maintenance.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-continuous-maintenance.md) | +65/−0 | 持续证据与统一维护端到端试验的人工阅读说明 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-downloads-http.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-downloads-http.json) | +167/−0 | 下载接口真实HTTP与发行源检查的机器可读结果 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-downloads.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-downloads.md) | +74/−0 | 独立下载任务验证与未部署状态的人工阅读说明 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-observation-replay.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-observation-replay.json) | +23/−0 | 观测重放的机器可读结果 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-observation-replay.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-observation-replay.md) | +9/−0 | 观测重放的人工阅读说明 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-official-catalog.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-official-catalog.json) | +34/−0 | 正式目录发布/回退的机器可读结果 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-official-catalog.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-official-catalog.md) | +11/−0 | 正式目录发布/回退的人工阅读说明 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-path-template-merge.json](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-path-template-merge.json) | +41/−0 | 历史候选模板展示合并的机器可读结果 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [docs/verification/2026-09-15-path-template-merge.md](/Users/sheldon/Documents/GithubProject/NexoFolio/docs/verification/2026-09-15-path-template-merge.md) | +14/−0 | 历史候选模板展示合并的人工阅读说明 | 让测试结论有范围/数据来源，而不是只口头说通过 | 建议整理：成对JSON/MD可留摘要+结果链接，历史试验可归档；均非生产运行依赖 |
| [migrations/202609140002_environments.sql](/Users/sheldon/Documents/GithubProject/NexoFolio/migrations/202609140002_environments.sql) | +17/−0 | 项目环境实体、名称唯一规则 | 外部按名称首次上传可建环境，平台可管理 | 保留已应用迁移：后续修复新建迁移；长单行DDL可读性不足，不能按38行说简单 |
| [migrations/202609140003_ingestion.sql](/Users/sheldon/Documents/GithubProject/NexoFolio/migrations/202609140003_ingestion.sql) | +39/−0 | 批次/记录回执、收件箱、结构头状态 | 可靠接收、幂等重试、同结构短路需要可持久化状态 | 保留已应用迁移：后续修复新建迁移；长单行DDL可读性不足，不能按38行说简单 |
| [migrations/202609140004_project_environment_scope.sql](/Users/sheldon/Documents/GithubProject/NexoFolio/migrations/202609140004_project_environment_scope.sql) | +16/−0 | 将接口身份范围收敛为项目+环境 | 响应用户取消service_key去重维度；旧回执仍保存 | 保留已应用迁移：后续修复新建迁移；长单行DDL可读性不足，不能按38行说简单 |
| [migrations/202609140005_observed_documents.sql](/Users/sheldon/Documents/GithubProject/NexoFolio/migrations/202609140005_observed_documents.sql) | +62/−0 | 接口本体、环境当前修订、差异及来源关联 | 结构首次入库并将变化留待确认；接口ID不依赖目录 | 保留已应用迁移：后续修复新建迁移；长单行DDL可读性不足，不能按38行说简单 |
| [migrations/202609140006_directional_observation_comparison.sql](/Users/sheldon/Documents/GithubProject/NexoFolio/migrations/202609140006_directional_observation_comparison.sql) | +3/−0 | 调整结构比较所需存储状态 | 空数组等不足证据不能使已知结构退化 | 保留已应用迁移：后续修复新建迁移；长单行DDL可读性不足，不能按38行说简单 |
| [migrations/202609150001_catalog_previews.sql](/Users/sheldon/Documents/GithubProject/NexoFolio/migrations/202609150001_catalog_previews.sql) | +37/−0 | 旧候选快照和执行状态表 | 人工评估LLM目录结果且不自动发布 | 保留已应用迁移：后续修复新建迁移；长单行DDL可读性不足，不能按38行说简单 |
| [migrations/202609150002_path_identity.sql](/Users/sheldon/Documents/GithubProject/NexoFolio/migrations/202609150002_path_identity.sql) | +3/−0 | 项目路径识别策略及相关状态 | 首次发现即模板化并允许静态前缀例外 | 保留已应用迁移：后续修复新建迁移；长单行DDL可读性不足，不能按38行说简单 |
| [migrations/202609150003_official_catalog.sql](/Users/sheldon/Documents/GithubProject/NexoFolio/migrations/202609150003_official_catalog.sql) | +44/−0 | 正式目录、固定待分类、版本与发布回执 | 发布/回退并发受控，新接口始终有正式位置 | 保留已应用迁移：后续修复新建迁移；长单行DDL可读性不足，不能按38行说简单 |
| [migrations/202609150004_capture_evidence.sql](/Users/sheldon/Documents/GithubProject/NexoFolio/migrations/202609150004_capture_evidence.sql) | +34/−0 | 资产/原文索引、事件回执、值索引、关系/样例/pin/背压表 | 结构去重不能丢新证据，持续录制需可靠关联和保留策略 | 保留已应用迁移：后续修复新建迁移；长单行DDL可读性不足，不能按38行说简单 |
| [migrations/202609150005_knowledge_maintenance.sql](/Users/sheldon/Documents/GithubProject/NexoFolio/migrations/202609150005_knowledge_maintenance.sql) | +38/−0 | 知识快照/任务/调用/检查点、统一发布版本和回执 | 完整审阅可恢复；目录与语义统一切换并兼容旧目录版本 | 保留已应用迁移：后续修复新建迁移；长单行DDL可读性不足，不能按38行说简单 |
| [migrations/README.md](/Users/sheldon/Documents/GithubProject/NexoFolio/migrations/README.md) | +19/−0 | 记录新增环境/观测/目录/证据/维护迁移用途 | 已有数据库需按顺序升级且不可启动时争抢执行 | 保留：已执行迁移不能为减少行数重写 |
| [scripts/audit_observed_documents.py](/Users/sheldon/Documents/GithubProject/NexoFolio/scripts/audit_observed_documents.py) | +101/−0 | 只读分析已观测接口完整性/差异 | 定位空数组、截断和错误结构；不改正式数据 | 保留：开发/交付工具，不计入运行时业务 |
| [scripts/build_offline_image.py](/Users/sheldon/Documents/GithubProject/NexoFolio/scripts/build_offline_image.py) | +25/−0 | 用锁定vendor依赖构建离线Linux镜像 | Docker网络访问crates失败时复用同一Dockerfile；vendor留ignored target | 保留：开发/交付工具，不计入运行时业务 |
| [scripts/capture_contract.py](/Users/sheldon/Documents/GithubProject/NexoFolio/scripts/capture_contract.py) | +43/−0 | 生成和校验v3采集Schema/TS/摘要版本 | 多个记录种类共享Rust DTO，防止手写多份漂移 | 保留：开发/交付工具，不计入运行时业务 |
| [scripts/catalog_preview_contract.py](/Users/sheldon/Documents/GithubProject/NexoFolio/scripts/catalog_preview_contract.py) | +54/−0 | 生成旧目录Schema/TS并提供类型转换辅助 | 支持历史候选合同；新的生成器复用部分代码 | 保留：开发/交付工具，不计入运行时业务 |
| [scripts/catalog_preview_report.py](/Users/sheldon/Documents/GithubProject/NexoFolio/scripts/catalog_preview_report.py) | +39/−0 | 把旧候选JSON渲染成文本报告 | 早期无前端的人工评估工具，不在生产请求链路 | 保留工具；其中旧文本报告可归档 |
| [scripts/document_contract.py](/Users/sheldon/Documents/GithubProject/NexoFolio/scripts/document_contract.py) | +26/−0 | 从文档Schema生成TS和manifest | 文档响应以Schema为权威源，区别于Rust生成的新合同 | 保留：开发/交付工具，不计入运行时业务 |
| [scripts/ingestion_contract.py](/Users/sheldon/Documents/GithubProject/NexoFolio/scripts/ingestion_contract.py) | +53/−0 | 生成接收TS与manifest、校验旧版本变更 | 用户要求模块合同改动可发现，旧回执不能被重写 | 保留：开发/交付工具，不计入运行时业务 |
| [scripts/maintenance_contract.py](/Users/sheldon/Documents/GithubProject/NexoFolio/scripts/maintenance_contract.py) | +27/−0 | 生成统一维护Schema/TS/manifest并检查版本 | LLM、后端和前端必须使用同一候选/回读协议 | 保留：开发/交付工具，不计入运行时业务 |
| [scripts/test_ingestion_contract.py](/Users/sheldon/Documents/GithubProject/NexoFolio/scripts/test_ingestion_contract.py) | +34/−0 | 测试合同改动必须升版及兼容门禁 | 不能只检查当前合同通过，还要验证违反规则确实失败 | 保留：开发/交付工具，不计入运行时业务 |
| [tests/architecture/dependencies.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/tests/architecture/dependencies.rs) | +30/−7 | 新增evidence白名单和禁止依赖失败用例 | 验证核心不能直接依赖SQLx/模型SDK，也不能绕dev依赖进入 | 保留：证明依赖边，不证明每个模块内职责纯净 |
| [tests/fixtures/pixel.png](/Users/sheldon/Documents/GithubProject/NexoFolio/tests/fixtures/pixel.png) | 二进制 68 B | 1×1合法PNG二进制测试样本 | 真实资产HTTP收讫/回读需要合法图片字节 | 保留：68字节二进制，不计算代码行 |
| [tests/integration/capture_benchmark.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/tests/integration/capture_benchmark.rs) | +113/−0 | 同批20条旧结构链路与新证据链路接收对照 | 量化持久化代价；单并发debug数据不能作生产性能合格证 | 保留：按明确失败行为拆用例；大型总测试应拆以便定位 |
| [tests/integration/capture_evidence.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/tests/integration/capture_evidence.rs) | +612/−0 | 真实HTTP+隔离PG验证多类型接收、去重、关联、权限、资产和保留 | 核心分流跨存储与worker，单元测试无法代替 | 保留：按明确失败行为拆用例；大型总测试应拆以便定位 |
| [tests/integration/capture_rollback.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/tests/integration/capture_rollback.rs) | +70/−0 | 失败写入时收讫事务与文件/数据库边界检查 | 不能部分落库却返回成功，也不能重试改变回执 | 保留：按明确失败行为拆用例；大型总测试应拆以便定位 |
| [tests/integration/catalog_preview.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/tests/integration/catalog_preview.rs) | +918/−0 | 旧目录快照、合并组、正式发布回退与新接口保留 | 保留兼容API需要跨事务回归 | 保留：按明确失败行为拆用例；大型总测试应拆以便定位 |
| [tests/integration/compose_smoke.py](/Users/sheldon/Documents/GithubProject/NexoFolio/tests/integration/compose_smoke.py) | +13/−3 | 扩展共享卷和非默认UID、健康/迁移/退出的隔离烟测 | 之前真实部署遇到卷所有权问题，需覆盖真实容器行为 | 保留：容器验收与浏览器验收分开 |
| [tests/integration/ingestion.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/tests/integration/ingestion.rs) | +763/−0 | 批次幂等、环境与项目权限、重复结构及并发写入 | 高频采集热路径正确性门禁 | 保留：按明确失败行为拆用例；大型总测试应拆以便定位 |
| [tests/integration/maintenance_fixture.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/tests/integration/maintenance_fixture.rs) | +621/−0 | 可控模型驱动全量审阅/回读/候选/发布与失败恢复 | 可重复测试无效输出、预算、租约和版本冲突；不冒充真实LLM效果 | 保留：按明确失败行为拆用例；大型总测试应拆以便定位 |
| [tests/integration/processing.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/tests/integration/processing.rs) | +662/−0 | 观测建档/修订/差异/空数组/路径策略流程 | 证明没有把不充分样本当成字段删除 | 保留：按明确失败行为拆用例；大型总测试应拆以便定位 |
| [tests/integration/replay.rs](/Users/sheldon/Documents/GithubProject/NexoFolio/tests/integration/replay.rs) | +219/−0 | 采集样本通过后端流程重放 | 检验路径模板与已有观测行为，非真实Chrome录制 | 保留：按明确失败行为拆用例；大型总测试应拆以便定位 |

## 当前核验结果

本次独立环境检查全部exit 0：fmt、ingestion-contract、capture-contract、document-contract、catalog_preview-contract、maintenance-contract、contract-governance、clippy、workspace-tests、database-tests。隔离数据库已移除，18080未重启。

| 检查 | 退出码 | 秒 |
|---|---:|---:|
| fmt | 0 | 0.43 |
| ingestion-contract | 0 | 0.04 |
| capture-contract | 0 | 0.67 |
| document-contract | 0 | 0.04 |
| catalog_preview-contract | 0 | 0.46 |
| maintenance-contract | 0 | 0.44 |
| contract-governance | 0 | 0.07 |
| clippy | 0 | 3.23 |
| workspace-tests | 0 | 34.68 |
| database-tests | 0 | 11.55 |

## 复核方法

基准差异使用`git diff HEAD --numstat`，新增文件使用`git ls-files --others --exclude-standard`逐文件计数。PNG等二进制只计字节；CSV固定保存路径、SHA-256、增删行、类别和本表判断。没有利用文件mtime推断作者。

本清单为审计起点快照，之后其他任务或本报告本身产生的改动不会回写分母；若提交前文件哈希变化，需补审新增差异。
