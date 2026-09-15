# 真实源码精简修正

上一轮实际增加了1,276行Rust源码，却以“优化完成”交付，没有兑现代码体量下降的预期。这一轮固定以上次提交`81bd90a6d79717955d9c410b36ee7f9bf07863d8`为基线，直接删除冗余实现，不能用生成物排除、搬文件或压缩排版抵扣。

## 数量

| 口径 | 基线 | 本轮 | 净变化 |
|---|---:|---:|---:|
| 生产模块Rust文件（含未改动的内嵌测试） | 15,435 | 14,920 | -515 |
| 独立测试Rust文件 | 5,961 | 5,967 | +6 |
| 所有Rust源码 | 21,396 | 20,887 | **-509** |

Git逐行差异为Rust新增309行、删除818行，净减509行。

逐一核对过所有内嵌测试正文，与基线一致，因此上述-515来自生产实现。独立测试只将旧草图生成器替换为现用DirectoryGenerator接口，增加6行，原测试目的和断言仍保留。这次没有删除生成文件，也没有将SQL/业务逻辑挪到其他语言来减少Rust统计。

这些数字只与81bd90a比较。上一轮相对更早审计增加1,276行，本轮抵消509行，合计仍多767行；不声称已经全部抵消上一轮增长。

## 真正删除了什么

1. **未接入实际链路的内部草图。** 删除intake旧CaptureInput/InterfaceIdentity/Fingerprinter/Deduplicator（实际采集使用batch与版本化HTTP合同）；删除knowledge旧InterfaceRecord/ChangeProposal等草图及旧读写端口（实际使用观测定义、DocumentReader和统一发布端口）；删除application旧通用JobRequest/Checkpoint/KnowledgeTransaction（实际任务使用维护租约/检查点和观测处理端口）。同时删除只为这些草图存在的Unconfigured实现。外部v1/v2/v3回执没有删除或改写。
2. **旧重构模型的平行抽象。** 删除没有接入生产的RebuildRequest/CandidatePlan/CatalogBuilder/Evaluator；OrganizationServices及替换测试直接使用已经运行的DirectoryGenerator/CatalogSnapshot。保留触发器接口，不启用自动触发。
3. **冗余路由状态。** BackendServices的五个Option字段每次装配都赋Some，围绕它们再做clone/map/unwrap没有业务意义。改为在装配时直接合并Router，删除这个状态容器和重复路由拼装。
4. **重复响应和参数样板。** 四处完全相同的no-store闭包共用一个中间件；统一相同分页类型，带额外筛选条件的查询仍明确声明。52处相同InvalidInput构造使用Error::invalid，错误类别和文本不变。
5. **两套当前结构处理。** 旧版和v3上传共用ingestion_heads的有界读取、结构覆盖和更新。结构头在原有锁内升级，不扫描全量历史；协议各自的可靠回执、内容hash和背压保持原逻辑。v3也可复用严格结构hash快速命中。
6. **重复原文读取。** 大原文进入分片回读时，复用批次阶段已读取的原文，不再从存储重复取一次；不同回读路径共用完成记录写入。

没有删测试来凑数字；没有移除正式目录、语义维护、枚举/关系、回退、旧协议或MCP默认拒绝能力。三个已不再直接使用schemars的crate也删除了该直接依赖。

## 验证

- fmt、Clippy、五套合同一致性检查和合同治理负例通过。
- 70项常规测试、7项隔离PostgreSQL测试通过，测试数量没有减少。
- 原有模块独立性测试仍保留，现验证真实使用的生成器端口。
- 外部contracts目录与已应用migrations目录无变动。
- 独立Linux arm64容器检查通过：迁移、HTTP/MCP鉴权边界、数据库故障恢复、共享卷及停止重启；新增实际API/worker镜像与选择镜像一致性断言，修正原脚本报告镜像ID固定写foundation的问题。不重启现用服务，不修改前端/插件，不运行真实模型或真实Chrome录制。

机器可读数字及检查结果见`docs/verification/2026-09-15-source-reduction.json`。计数包含新增和删除文件，所有源码经过cargo fmt，不使用删空行/压缩格式作为精简手段。
