# 后端审计问题优化交付

> 本记录描述81bd90a阶段，当时Rust源码净增加1,276行。随后用户要求真实精简，结果和撤销的冗余封装见[源码精简修正](./2026-09-15-source-reduction.md)。

本轮由用户明确授权优化并push。范围只限NexoFolio后端：不改前端、不改插件、不调用真实LLM、不重启现用API/worker、不清数据库。之前的逐文件审计及CSV保持为优化前快照，不将历史行号冒充当前代码位置。

## 已完成的调整

| 审计问题 | 本轮实际改变 | 保持的行为 |
|---|---|---|
| 发布策略只有trait，数据库里写死实现 | 新增应用层KnowledgePublicationService；策略由装配层注入；数据库不再选择人工策略 | 默认只允许显式人工发布；事务内项目权限、候选和版本检查仍执行 |
| 三份事务项目权限判断 | 收敛至infrastructure/project_access.rs，三个存储模块复用 | 保留用户共享锁、enabled/grants_synced和同实例项目权限检查 |
| AccessHttp承担全部业务容器 | AccessHttp仅保留登录/会话状态；BackendServices承载传输组合，业务装配移到wiring/services.rs | 对外URL、鉴权方式、采集开关不变 |
| 维护引擎和规则文件职责混杂 | 引擎分为调用/预算、执行、上下文、回读；规则分为字段、阅读片段、引用要求、动作物化、证据上下文和测试 | 公共Rust接口与外部Schema不变；仍核对完整覆盖 |
| 每片固定6个阅读单元 | 依实际序列化输入大小打包，同一接口能放下时一起审阅 | 超大接口仍分片；不抽样、不漏字段 |
| 预算耗完才发现无法完成审阅 | 读取保存的检查点后，预估未完成片段+一次规划的最低调用数；不足在provider调用前失败 | 下限检查不承诺后续回读必然足够；仍保留总调用硬上限 |
| 小原文逐条调用模型总结 | 将预算内多个原文引用合成一个回读调用；成功后写批次检查点，再写每项完成记录 | 原文和反例仍实际进入模型请求；超大原文/图片保留单独读取路径；失败不记已读 |
| 缺少provider用量 | ModelInvocation保存业务结果与可选usage；精确请求仍先落库，调用结果保存output/usage | 供应商不返回usage时为null，不伪造0；不改变HTTP合同 |
| 旧/新模型HTTP重复 | 复用ChatTransport的端点校验、认证、无跳转、超时和体积限制 | 两种prompt/输出合同仍分别校验，旧目录接口不退役 |
| 证据关联SQL与判断混合 | 把关系适用性/歧义及推断性质集中到evidence纯模块；持久化拆为样例、关系和UI模块 | 有界SQL候选检索继续依赖索引；跨环境、自关联、技术字段仍不能生成关系 |
| 巨大接收/快照存储文件 | 收讫事务独立在capture_store/admission.rs；快照构建在maintenance_store/snapshot.rs | 事务归属、原始回执及任务租约不变 |
| 后台清理可能长时间等待 | 每轮最多500个事件、100个孤立blob；全局锁不可得则跳过，项目锁1秒、SQL5秒、worker清理30秒上限并可取消；失败有日志 | 24小时保留期、工作样例和候选/发布pin保护不变；避免已pin事件反复占用清理批次 |
| SQL压成极长单行 | 将长SQL的关键子句分行，SQL字符串值内容保持不变 | 未重写已应用迁移；依靠数据库集成回归校验行为 |
| 生成架构图混入代码提交 | .gitignore排除可再生HTML/视觉截图/回执；保留图源、生成脚本和说明；生成合同标注linguist-generated | 本地已生成文件不删除，合同生成/版本检查不关闭 |
| 构建上下文混入本地数据 | .dockerignore排除捕获文件、备份、证据结果、文档和本地秘密目录 | tests/fixtures仍可用于容器构建；运行数据只在持久卷 |

## 新文件职责

这些拆分发生在已有crate内，没有增加微服务或新crate。拆文件不是单独的成果；策略可替换、规则只维护一处、调用成本更低才是目的。

- `apps/backend/src/http/services.rs`：HTTP模块组合，替代认证模块充当业务容器。
- `apps/backend/src/wiring/services.rs`：已有业务传输和应用服务的依赖注入。
- `crates/application/src/knowledge_publication.rs`：发布策略与存储端口的编排，含拒绝策略不触达存储的测试。
- `crates/application/src/maintenance_engine/context.rs`：预算内数据分组、原文定位、完整索引、回复合同校验。
- `crates/application/src/maintenance_engine/execution.rs`：快照审阅、检查点恢复、规划和候选生成顺序。
- `crates/application/src/maintenance_engine/readback.rs`：原文批量/分片/图片读取和成功检查点。
- `crates/rebuild/src/maintenance/fields.rs`：稳定字段引用与原定义指针。
- `crates/rebuild/src/maintenance/review.rs`：按体积组片和逐单元覆盖验证。
- `crates/rebuild/src/maintenance/reads.rs`：候选修改必须读取哪些原文和证据。
- `crates/rebuild/src/maintenance/apply.rs`：候选动作验证与目录/语义物化。
- `crates/rebuild/src/maintenance/context.rs`：字段已有知识和证据提示。
- `crates/rebuild/src/maintenance/tests.rs`：原有规则测试搬迁及按体积保持接口完整的回归。
- `crates/evidence/src/relations.rs`：无数据库依赖的关系适用性、歧义与候选事实构造。
- `crates/infrastructure/src/project_access.rs`：采集、目录和维护事务共用的项目鉴权实现。
- `crates/infrastructure/src/chat_transport.rs`：旧目录与新维护共用的有界模型HTTP传输。
- `crates/infrastructure/src/capture_store/admission.rs`：v3可靠收讫事务，不混入查询接口。
- `crates/infrastructure/src/maintenance_store/snapshot.rs`：不可变知识快照构建，不混入租约/检查点更新。
- `crates/infrastructure/src/evidence_processing/facts.rs`：事实去重、工作样例与冲突持久化。
- `crates/infrastructure/src/evidence_processing/links.rs`：检索参数关系候选和持久化支持记录。
- `crates/infrastructure/src/evidence_processing/ui.rs`：界面选项/字段绑定及反例存储。
- `.gitattributes`：标注自动生成合同，权威手写Schema没有被一并标成生成物。

## 仍保留的合理成本

- v1/v2收讫和旧目录专用发布/回退必须继续工作。本轮复用公共边界，不在没有消费者/旧队列退役依据时删除入口。
- intake用于“能否证明结构重复”，knowledge用于“保存观察到的结构与不足”，二者不能为了减行数强行合并。共同的路径身份与方向性覆盖规则仍维持一个源，未为移动两个纯规则再新增公共crate。
- 参数关系的时间/项目/上下文过滤仍在数据库查询中，以使用索引限制候选数量；没有把整库读到内存后再做纯函数筛选。
- 语义候选仍可表达完整计划支持的动作，未以减代码为由删除关系/枚举/描述/目录能力。
- 本次证明的是可控模型下的调用合并与完整覆盖，不宣称真实业务的模型质量和费用已经完成验收。此前单并发接收性能对照仍有实际适用限制，本轮没有声称接收吞吐提高。

## 验证

- fmt、五套合同生成/版本检查、合同治理负例、clippy全部通过。
- workspace：70通过、0失败，8项需要外部环境或显式重放的测试默认忽略；不是把忽略项计作通过。
- 隔离数据库：7通过、0失败。使用独立临时PostgreSQL容器，结束后清理，不使用真实项目数据。
- 新回归：替换发布策略会在存储前拒绝；40字段接口按预算完整组片；实际维护链路中多个小原文进入同一次成功回读；失败回读不标完成；预算不足零provider调用；usage缺失保持未知；竞争清理锁直接跳过。
- 首轮回归发现批量回读未写批次检查点，导致read_count为0，已补齐并重跑通过。旧“summary错误”用例也改为显式产生需要压缩的审阅发现，不再依赖旧六单元分片的偶然行为。
- Linux arm64容器检查通过：数据库探测与重复迁移、HTTP健康与MCP拒绝未授权、管理CLI、数据库故障恢复、共享证据卷重启保留、API/worker正常停止重启。使用独立optimization-check镜像及临时Compose项目，未重启现用服务；未将此结果当作x86_64或真实Chrome验收。

自动检查日志位于被ignore的`experiments/results/optimization/`。真实Chrome录制和前端设计不在本次验收范围。
