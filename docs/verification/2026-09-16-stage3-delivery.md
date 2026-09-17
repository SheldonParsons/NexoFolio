# 阶段3：固定重构输入交付

2026-09-16。后端实现、隔离验收、前端最小合同同步和本地部署均已完成。用户随后授权同步前端合同；仅更新生成合同、引用和兼容测试，未改布局/交互，插件未修改。未调用真实模型，未生成或发布业务候选，未commit/push；阶段4未开始。

## 修复后的行为

| 问题 | 实施结果 |
| --- | --- |
| 结构判断/观测定义没有进入快照 | inputs.observations固定处理时的ObservationAssessment和定义材料，不重写历史结论 |
| 观测字段在事实中存在却没有被审阅 | EvidenceFieldRef统一正式与观测字段，使用同一字段构建、索引、分段和引用解析；观测字段保持只读 |
| 索引把未解析引用标为false | 共用字段解析器识别ObservationFieldRef；原失败回归已移除ignore并通过 |
| 处理进度/缺口混为一谈 | inputs.gaps分开表达结构/证据pending、failed、旧事实待重评、原文缺失、提取不足、未评估和未解析引用 |
| 回读缺采集时间/依赖实时元信息 | 来源描述固定时间、项目/环境、actor/producer/record、原文哈希和coverage；回读使用固定描述并验证内容哈希 |
| 多次SQL读取可能得到不同资料截面 | 接口、观测材料、事实、来源成员和处理计数在单条SQL语句中按同一MVCC截面读取；已有锁保护版本切换和保留期清理 |

字段结构重建复用现有extract_observed和path规则。对于事实需要、但历史提取器没有描述完整的观测字段，从保留原文生成单独的reconstructed_definition；原ObservationAssessment及原定义保持不变。重建身份必须匹配入队身份；不可用的原文或结构保留缺口，不生成假的revision_id。

正式字段ID保持稳定。观测来源字段的语义写入仍被拒绝：发布目标继续使用正式FieldRef，不通过写描述自动采用新结构。

## 真实录制复测

使用固定副本`9ab07494-76ba-4ca6-b2c2-c90d23ebc774`，388条观测、153条HTTP、39个接口。全部在一次性数据库完成，原始两份录制备份保留。

| 项目 | 改造前 | 改造后 |
| --- | ---: | ---: |
| 正式字段 | 1640 | 1640，ID保持 |
| 声明的审阅字段 | 1640 | 2536，包含只读观测字段及祖先结构 |
| 94个不同观测来源引用进入索引 | 0 | 94 |
| 结构观测材料 | 无独立集合 | 90 |
| 固定来源描述/对应pin | 无独立描述集合 | 357/357 |
| 回读采集时间 | 缺失 | 存在 |
| 仅结构pending的派生探针计数 | 0 | 1 |

437份观测来源值事实的94个不同引用全部可定位，未解析事实字段数0，观测字段无定义数0。新增896个审阅字段不代表接口增加或结构已采用，而是把实际观测结构及其父节点纳入只读审阅。

在64KiB分段测试预算下产生128段、2538个单元，覆盖全部2536个字段；跨段字段必须读完所有单元。合成审阅用于验证漏单元/重复单元拒绝，不宣称模型已读。

现有资料的不足仍如实呈现：89条历史结构观测没有存储的新分类，40个来源存在提取不完整。审阅清单覆盖与采集完整性分别表达，不把未知改成确定。历史快照检查能显示5256份legacy/待重评事实；新规则重建副本的legacy事实为0。

## 测试与边界

- fmt、workspace clippy、五套合同生成及合同治理通过。
- workspace：90通过、0失败、8忽略。
- 另执行隔离数据库集成：ingestion 2、processing 1、capture_evidence 1，共4项通过。
- 新增并发测试：原子写入同时更新定义/事实，6次快照读取均没有混入不同提交的标记；这是隔离一致性探针，不是生产写接口行为模拟。
- 固定描述回读：测试副本后续修改实时context/采集时间，原快照回读仍返回原描述和内容；缺少原文明确失败。
- 已存快照不随后续事实变化，相同request_id幂等，跨项目读取拒绝；大小超限仍拒绝。
- 观测字段不能通过伪造正式revision写入；正式字段ID保持一致；旧快照仍可解码。
- 真实模型调用0，没有创建主项目重构任务或知识发布。

已有fake-model集成回归仍通过，不等于真实模型重构效果验收。阶段4才进行限定范围的真实模型验证。

## 合同、兼容性与部署

维护合同升为2.0.0，Schema/生成类型/行为/manifest同步。新增observation和source读取种类，扩展KnowledgeField.reference；上传、登录、目录列表和接口文档合同不变。capture合同的公共字段Schema生成逻辑被复用，其线形格式没有改变。

旧快照缺少inputs时保持可读，但不能作为新完整任务继续运行或发布：返回SNAPSHOT_INPUTS_UNAVAILABLE_RECREATE，须显式创建新任务。旧已发布版本及目录专用兼容操作保留。历史资料没有被自动重算或覆盖。

同步前的前端使用maintenance/1.5.0校验快照。相同的合成观测字段快照在旧validator中因缺revision_id失败，在2.0中通过。现已切换至2.0。这个例子只验证格式兼容，不冒充有效业务快照的全链路验收。

已部署`nexofolio-backend:stage3-20260916`，镜像ID为`sha256:079b031882f12ac328ee2236171c390b9470718c9e2ea391e2a3eef35b28160f`。本地Compose镜像配置已同步。部署备份位于`backups/pre-stage3-deploy-20260916T130022Z/`。本次无新数据库迁移。

## 代码收敛与体量

没有新crate、服务或额外生成流水线。主要收敛为：

- 一个字段构建器处理正式/观测定义，一个解析器处理事实引用。
- 一个固定来源清单驱动索引、回读和pin，避免不同模块分别猜测引用。
- 单次数据库材料读取替代快照中多段独立读取；复用阶段1材料查询与现有提取器。
- 互斥EvidenceFieldRef的Schema转换由capture和maintenance生成器共用，避免生成类型漂移。

相对阶段3实施起点工作区，Rust业务净增334行。统计范围新增1285、删除221，净增1064行，包含测试、生成合同和行为说明；专用tests.rs按测试统计，不算业务实现。新报告、根README及路线说明不在该统计口径中。未承诺总行数下降，也未将此前未提交工作算入本轮。

逐文件统计位于`experiments/results/stage3-implementation/stats.json`。以下是各文件修改原因。

| 文件 | 新增/删除 | 原因 |
| --- | ---: | --- |
| `apps/backend/examples/recording_regression/stage3.rs` | +8/−3 | 真实回放增加字段入索引、材料可读、采集时间与结构pending断言。 |
| `contracts/maintenance/behavior.md` | +17/−2 | 记录2.0输入、只读字段、旧快照及部署兼容边界。 |
| `contracts/maintenance/candidate.schema.json` | +3/−1 | 生成合同/类型/摘要同步，不是手写业务流程。 |
| `contracts/maintenance/checkpoints.schema.json` | +3/−1 | 生成合同/类型/摘要同步，不是手写业务流程。 |
| `contracts/maintenance/interface-knowledge.schema.json` | +3/−1 | 生成合同/类型/摘要同步，不是手写业务流程。 |
| `contracts/maintenance/manifest.json` | +10/−10 | 生成合同/类型/摘要同步，不是手写业务流程。 |
| `contracts/maintenance/plan.schema.json` | +3/−1 | 生成合同/类型/摘要同步，不是手写业务流程。 |
| `contracts/maintenance/reply.schema.json` | +3/−1 | 生成合同/类型/摘要同步，不是手写业务流程。 |
| `contracts/maintenance/run.schema.json` | +3/−1 | 生成合同/类型/摘要同步，不是手写业务流程。 |
| `contracts/maintenance/snapshot.schema.json` | +418/−2 | 生成合同/类型/摘要同步，不是手写业务流程。 |
| `contracts/maintenance/types.generated.ts` | +13/−3 | 生成合同/类型/摘要同步，不是手写业务流程。 |
| `crates/application/src/maintenance.rs` | +1/−1 | 来源端口改为接收冻结描述，避免回读重新查询实时元信息。 |
| `crates/application/src/maintenance_engine.rs` | +1/−1 | 更新执行设置标识，避免复用不兼容的检查点语义。 |
| `crates/application/src/maintenance_engine/context.rs` | +13/−25 | 收敛引用解析，索引补材料/来源/缺口，原失败测试启用；删除旧FieldRef专用遍历。 |
| `crates/application/src/maintenance_engine/execution.rs` | +6/−2 | 旧快照执行门禁、输入缺口提示与新读取种类；不增加模型入口。 |
| `crates/application/src/maintenance_engine/readback.rs` | +29/−10 | 依据快照成员清单回读冻结来源，拒绝越界来源。 |
| `crates/contracts/src/assessment.rs` | +32/−2 | 统一字段引用的路径派生、Hash和正式引用转换，保持正式ID。 |
| `crates/contracts/src/maintenance.rs` | +43/−1 | 定义观测材料、固定来源及输入清单，旧快照可解码。 |
| `crates/infrastructure/src/documents.rs` | +1/−1 | 仅放开crate内部材料查询复用，未新增公共业务接口。 |
| `crates/infrastructure/src/documents/assessments.rs` | +1/−1 | 复用阶段1查询，避免在快照模块另写一份材料合同。 |
| `crates/infrastructure/src/maintenance_store.rs` | +10/−7 | 从固定哈希读取并验证原文，返回冻结时间/来源元信息。 |
| `crates/infrastructure/src/maintenance_store/snapshot.rs` | +40/−42 | 替换分散读取，建立完整字段、缺口及批量pin；保留事务和大小保护。 |
| `crates/infrastructure/src/maintenance_store/snapshot_inputs.rs` | +98/−0 | 唯一数据库输入装配位置：同一截面取集合、受限复用原提取器重建历史只读结构。 |
| `crates/rebuild/src/maintenance/apply.rs` | +7/−3 | 阻止旧不完整快照作为新候选发布；观测证据不伪造正式revision basis。 |
| `crates/rebuild/src/maintenance/context.rs` | +19/−15 | 字段上下文识别来源，保留不确定性；长摘要明确标记省略。 |
| `crates/rebuild/src/maintenance/fields.rs` | +195/−68 | 一个定义遍历处理两类来源，共用解析/定位，缺资料保留未知字段。 |
| `crates/rebuild/src/maintenance/reads.rs` | +20/−3 | 增加快照内材料/来源定位，旧待重评事实不作独立验证证据。 |
| `crates/rebuild/src/maintenance/review.rs` | +1/−1 | 同一阅读单元支持两类字段引用。 |
| `crates/rebuild/src/maintenance/tests.rs` | +109/−5 | 验证只读边界、稳定正式ID、来源定位、旧快照拒绝及现有行为。 |
| `scripts/capture_contract.py` | +6/−4 | 提取共享Schema转换，capture输出保持不变。 |
| `scripts/maintenance_contract.py` | +3/−1 | 同步维护2.0合同，生成有效互斥引用类型。 |
| `tests/integration/capture_evidence.rs` | +2/−0 | 接入固定输入集成验收。 |
| `tests/integration/maintenance_fixture.rs` | +6/−2 | 合成语义写入明确选正式字段，避免随机命中只读字段。 |
| `tests/integration/snapshot_inputs.rs` | +158/−0 | 验证冻结回读、缺源失败、并发单一截面及旧快照可读。 |

## 下一步

阶段3已部署完成。之后由用户决定进入阶段4真实模型试验；本轮不提前生成知识文档。


## 前端同步与部署验收补充

- 前端新增`src/contracts/maintenance/2.0.0`，与后端合同逐文件字节一致；8个业务文件及2个测试文件同步引用。Vue文件只修改类型导入路径，未调整模板/样式。
- 新增兼容用例：旧快照仍可读、观测字段不必伪造revision、同时提供两类basis被拒绝。
- 前端150项测试通过；Vite生产打包通过；5173开发服务实际提供2.0合同引用。
- 完整类型检查仍有`product-docs/markdown.ts`第35/36行的原有`string | number`错误；同步前后诊断完全一致，没有新增错误。打包通过不等于全量类型检查通过。
- 带登录态对两个项目分别直连18080、通过5173/api读取目录和任务列表均200，并由前端当前Schema校验通过；无凭证仍401。
- 所有业务表及迁移表计数与部署前一致。39个接口、400条capture保留，定义/入队原文/回执摘要一致，两份固定录制备份SHA256不变。
- API健康、worker运行，重启计数0、启动后无ERROR日志；maintenance_runs与maintenance_calls均0。没有为主服务验证而创建会触发模型的任务。
- 新快照内容、冻结回读和并发边界在前述隔离后端测试中验证；本次主服务验收是部署、权限、合同与读接口链路，不冒称新文档生成或浏览器视觉验收。

日志、部署回执及逐前端文件变更位于`experiments/results/stage3-implementation/deployment/`。本次合同同步未新增source/observation引用的专用界面呈现功能，已有未支持引用继续明确显示为不可读取；后端模型受控回读已支持新种类。
