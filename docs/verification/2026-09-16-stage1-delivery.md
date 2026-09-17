# 阶段1交付：接收后的结构判断

2026-09-16。**代码实现和隔离验收完成；随后经用户授权已完成本地部署，见[部署验收](2026-09-16-stage1-deployment.md)。未进入阶段2。**

以下记录首次代码交付时的范围、统计和测试证据；其中“未部署、未改前端”描述的是首次交付时点，后续最小合同同步及部署以部署验收报告为准。 本报告对应用户批准实施的阶段1，不代表整体重构或新文档交付完成。

## 实际效果

接口收到以后，入口和后台现在共用结构提取与比较规则。后台保存逐字段的补全、差异、信息不足解释；同一条观测可以同时具有几种性质。完整读取到的null不会再让整份JSON失去快速去重能力；未读取、截断、超限也不会因为两份残缺结构相等而被当作确定重复。

这次解决的是结构判断和模块交接。没有自动采用补全、改写当前定义、裁决结构变化或调用LLM。没有修改插件和前端，也没有向它们派任务。旧录制快照与回执保留，不重新录制、不请求录制中的真实业务URL。

## 测试结果

使用固定补充快照的全部153条HTTP观测，在隔离数据库中重新接收并执行后台处理：

| 指标 | 改造前 | 改造后 |
| --- | ---: | ---: |
| HTTP输入 | 153 | 153 |
| 进入结构处理 | 90 | 52 |
| 入口判定结构重复 | 63 | 101 |
| 最终接口数 | 39 | 39 |
| 入口无法形成结构指纹 | 69 | 5 |

结构重复仍进入capture证据接收流程，不等于丢弃HTTP原文。测试核对了153条capture载荷、入队原文以及重试回执；结构去重不会改写旧记录的重试结果。剩下5条无指纹来自4条HTML与1条不可读上传体，不把未支持的结构当作已理解。

后台结果是39次初次建档、2次unchanged、11次difference_recorded，复用为9份待比较材料。`difference_recorded`沿用旧生命周期名称，**不能将其全部解释为确定的结构变更**，精确性质以新assessment为准。

| 验证 | 结果 |
| --- | --- |
| `cargo fmt --all --check` | 通过 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | 84通过、0失败、8忽略 |
| 架构检查 | 3通过，含禁止依赖反例；已包含在workspace结果中，不重复计数 |
| 单独运行数据库测试 | capture_evidence 1、ingestion 2、processing 1、真实replay 1，共5通过 |
| 五套合同生成及合同治理 | 通过 |
| 备份恢复测试 | 自动备份、清理范围、重复清理、DB/blob及权限恢复、损坏快照拒绝、未知表保护、服务重启通过 |

workspace中需专用环境的忽略项不宣称全部已验收；本阶段另外执行了上表5项数据库测试。测试时长不是生产吞吐基准，真实样本来自UAT，项目/环境隔离另由合成测试覆盖。

本地证据在`experiments/results/stage1-implementation/signoff/`，备份恢复日志为同级`backup-restore.log`。这些忽略目录可能包含私有录制资料，不提交原始载荷或连接文件。

## 原8份待确认材料的逐项对照

下表对历史原文做只读重评，不回填历史记录，也不修改其当前定义。新153条完整回放的9份材料与此历史8份对照不是同一统计口径。

| 接口末段 | 新判断 | 依据 |
| --- | --- | --- |
| commodityPageForConsultOrderV2 | 补全 | 原空数组现在观察到元素结构 |
| userMenu | 差异＋不足 | 观察到不同元素变体，无法唯一对应旧变体；不武断选择一个旧元素当基准 |
| saveAndSubmitConsultTechnology | 不足 | 本次未观察到file字段，不能判定删除 |
| page | 差异 | 请求首次观察到status字段；不等于已确认接口契约变化 |
| getProjectInfoByParam | 补全 | floorArea、strategicContractCode由null获得具体类型 |
| token | 不足 | employeeNo变为null，不能证明类型被改为null |
| getDeliveryCategoryTree | 补全 | 原空数组现在有元素结构 |
| table-config-info/find/{param1} | 补全＋差异＋不足 | data获得结构；新增响应头；本次未观察到content-length |

合计3份纯补全、2份纯不足、1份纯差异、2份混合结果。userMenu重新提取时已能读取原预算之外的msg、popupType，不再因遗漏制造删除判断；仍保留多变体对应关系的不确定性。

完整结果在`experiments/results/stage1-implementation/comparison/eight-before-after.json`，包含字段路径、前后结构、理由与原始观测标识。

## 模块边界和下游合同

- contracts统一每个JSON正文100000节点、深度64的提取预算和结构覆盖规则。预算用尽标记未知，不能静默当作完整。
- intake只准备输入、判断快速覆盖，不调用模型或做全库关联扫描。比较有工作上限；详细结果最多512项，超限明确保留缺口。
- knowledge负责HTTP定义比较语义，允许补全、差异、不足并存；多变体无法唯一匹配时保留歧义，不加入任意相似度分数。
- infrastructure在原事务中保存assessment与extractor_version，不另建版本服务或复制完整定义。读取端不重新计算历史结论。
- 独立AssessmentReader端口供后续证据/快照模块使用；HTTP适配与普通DocumentReader共用一个PostgresDocuments实例。已用测试替身核对消费合同，**实际阶段2证据处理和阶段3快照装配尚未接入**。
- 未采用字段用项目、接口、环境、ingestion_id、位置和路径定位，不能假称属于当前正式revision。结构重复记录的值证据还须保留自己的capture event来源。
- 后续快照必须冻结选中的assessment、材料及ID清单，不能在审阅时反复读取移动中的分页结果。

新增读取接口：

```text
GET /v1/projects/{project_id}/observations/{ingestion_id}/assessment
GET /v1/projects/{project_id}/interfaces/{interface_id}/assessments?environment_id=...&page=...&limit=...
```

接口沿用登录、项目权限与no-store。历史行的assessment为空表示“未评估”，不是重复；初次/待比较材料复用原存储，未生成新定义的重复记录不虚构incoming_definition。

## 部署和兼容边界

`contracts/documents`升为2.0.0：ObservedDefinition支持`observed-http-2`，历史`observed-http-1`仍可读取。严格按旧schema校验的文档消费者必须先同步合同，才能部署产出新定义的后端。插件上传合同未变化。

新迁移仅为interface_observations增加两个可空列，历史记录不批量回填。结构指纹版本升为http-structure-3，旧当前头在接收时按原锁机制惰性重算，不扫描历史、不改回执。无法恢复原文时不伪造指纹。

本轮未执行主库迁移、未重建或替换主API/worker、未提交或push。主服务继续运行原录制基线镜像，因此当前前端不会自动出现本轮的新分类。

## 代码体量与合理性

统计对比的是**开始实施阶段1时的工作区快照**，不是Git HEAD；此前已有大量未提交改动，不能归入本轮。快照位于`experiments/results/stage1-implementation/before/`，逐文件统计在`change-statistics.json`。

| 分类 | 净变化 |
| --- | ---: |
| Rust业务实现（剔除文件内测试） | +618行 |
| 文件内测试 | +219行 |
| 外部测试、回放及schema生成示例 | +153行 |
| 合同schema、生成类型、manifest与样例 | +452行 |
| 迁移、合同脚本及范围内说明 | +31行 |
| 上述范围合计 | +1473行（新增1743、删除270） |

这个口径覆盖crates、apps/backend、contracts/documents、migrations、tests、scripts中的既定文本文件，不包含本交付报告、根README和阶段路线文档。**业务代码净增618行，超过原200–400行预估上限218行；本轮不是总代码瘦身。**

删掉了intake/knowledge各自的JSON提取实现和重复布尔比较。增加的业务实现主要用于可解释且有界的多变体比较、混合结论与来源合同、HTTP完整性判断、带权限的独立读取端口。这些分别解决已复现的误判、下游无法定位未采用字段、历史结论无法查询的问题；不是新造业务包、后台服务、缓存或规则DSL。前期估算低估了这些合同和不确定性处理的成本，应以实际数字纠正。

### 逐文件修改理由（增/删均相对阶段起点）

| 文件 | 新增/删除 | 为什么修改 |
| --- | ---: | --- |
| `apps/backend/examples/recording_regression.rs` | +17/−4 | 验收：复用真实录制回放，增加历史8份材料重评；不新建回放服务。 |
| `apps/backend/examples/recording_regression/stage1.rs` | +2/−2 | 验收：诊断跟随新算法；旧回执兼容由集成测试单独验证。 |
| `apps/backend/src/http/documents.rs` | +42/−1 | 能力交接：暴露单条及分页assessment读取，沿用认证和项目权限。 |
| `apps/backend/src/wiring/services.rs` | +5/−3 | 装配：同一存储实例提供两个独立读取端口，避免重复实例。 |
| `contracts/documents/assessment-page.schema.json` | +155/−0 | 生成合同：明确分页读取格式，供消费者自动校验。 |
| `contracts/documents/assessment.schema.json` | +123/−0 | 生成合同：明确混合分类、来源、前后结构和历史未评估。 |
| `contracts/documents/behavior.md` | +18/−4 | 合同说明：解释新分类、版本升级和未采用字段身份。 |
| `contracts/documents/fixtures/assessment-historical.json` | +10/−0 | 兼容样例：NULL assessment不能被当成duplicate。 |
| `contracts/documents/fixtures/assessment-mixed.json` | +102/−0 | 行为样例：同一观测补全、差异、不足可并存。 |
| `contracts/documents/fixtures/observed-field-ref.json` | +8/−0 | 来源样例：未采用字段由观测定位，不伪造修订。 |
| `contracts/documents/manifest.json` | +10/−4 | 合同治理：升2.0.0并固定schema和样例摘要。 |
| `contracts/documents/observed-field-ref.schema.json` | +39/−0 | 生成合同：约束观测字段的稳定定位格式。 |
| `contracts/documents/responses.schema.json` | +4/−1 | 兼容边界：允许历史与新extractor_version，明确严格旧读者需升级。 |
| `contracts/documents/types.generated.ts` | +7/−1 | 生成类型：与Rust/schema同步，减少消费端手写漂移。 |
| `crates/contracts/examples/assessment_schema.rs` | +13/−0 | 生成入口：从Rust类型生成新schema，避免维护第二套手写结构。 |
| `crates/contracts/src/assessment.rs` | +139/−0 | 新增必要合同：多分类、逐字段原因、观测身份、分页与历史空值；附样例一致性测试。 |
| `crates/contracts/src/lib.rs` | +5/−1 | 公共边界：导出共享提取和assessment类型。 |
| `crates/contracts/src/observed_comparison.rs` | +315/−60 | 修复＋收敛：一个比较器提供快速覆盖和详细发现；处理未读、混合变化及多变体歧义，并限制成本。 |
| `crates/contracts/src/observed_shape.rs` | +76/−0 | 修复＋收敛：共用提取预算、null/空数组和超限标记，替换两套shape实现。 |
| `crates/infrastructure/src/documents.rs` | +11/−8 | 修复：移除残缺定义哈希相等即unchanged的捷径，在现有事务中保存判断。 |
| `crates/infrastructure/src/documents/assessments.rs` | +78/−0 | 能力交接：实现权限、分页、来源及历史语义，查询不重算结论。 |
| `crates/infrastructure/src/ingestion_heads.rs` | +8/−7 | 兼容：旧当前头惰性升级新算法；不动历史回执。 |
| `crates/intake/src/http_exchange.rs` | +15/−45 | 修复＋收敛：改用共享shape，null不阻断完整JSON指纹；非JSON保持未知。 |
| `crates/knowledge/src/assessment.rs` | +374/−0 | 新增必要能力：HTTP级分类、混合结果、独立读取端口与边界测试；替换旧布尔路由。 |
| `crates/knowledge/src/lib.rs` | +3/−0 | 公共边界：导出assessment端口与比较入口。 |
| `crates/knowledge/src/observed.rs` | +11/−120 | 修复＋收敛：删除本地shape及重复比较逻辑，使用统一规则并标记extractor版本。 |
| `migrations/202609160001_observation_assessments.sql` | +4/−0 | 持久化：两个可空列保留分类和算法来源，历史不伪造结果。 |
| `migrations/README.md` | +2/−0 | 操作说明：记录新迁移用途与历史NULL语义。 |
| `scripts/document_contract.py` | +14/−3 | 合同治理：生成新schema/类型、核对重名定义和样例摘要。 |
| `tests/integration/ingestion.rs` | +34/−0 | 回归：旧头升级、null重复和旧accepted回执原样重试。 |
| `tests/integration/processing.rs` | +92/−1 | 回归：分类落库、权限、查询schema、历史NULL及当前修订不被覆盖。 |
| `tests/integration/replay.rs` | +7/−5 | 真实验收：报告分类；断言避免失败时输出完整私有载荷。 |

范围外的说明文件：本报告用于验收和逐文件归因；README增加入口及部署兼容提醒；阶段路线更新当前状态；原测试方案增加历史标记。均不改变业务行为。

## 下一阶段

阶段2针对evidence与evidence_processing测试同值碰撞、来源歧义、枚举适用范围和交错操作，不预设用户在构建业务场景。阶段3再把结构判断和证据接入不可变重构快照；之后才进行真实模型重构、新文档检查、发布回退。当前只交付阶段1，等待用户反馈后推进。
