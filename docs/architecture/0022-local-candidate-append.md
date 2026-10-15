# 0022 — 候选局部追加保存

2026-10-14。本步保留候选 schema、公开合同、完整 view 和既有任务调度；没有实施完整候选增量投影。

`knowledge-contracts::CandidateAppend` 是事务内写入上下文，不是缺少字段的 Candidate。它包含候选身份、状态、owner、base 接口身份集合、完整 view，以及匹配 round 的身份、作者、状态、位置和命令数量。零个或多个同 ID round 均拒绝追加。

core 的 `append_proposal` 在原项目锁内读取上下文，复用原 running、owner、任务 run/租约、commands/apply::one、人工保护、项目身份、生命周期与名称规则。view 直接移入 SnapshotTx，成功后移回；没有第二套业务校验。完整 Candidate 追加调用在同一事务内重新读取完整候选后提交，worker 仅返回 view，CandidateSession 只在提交成功后替换会话视图。

PostgreSQL 只返回 base.endpoints 和 round 元数据；适配器按原 RoundId serde 定位目标 round（包括 UUID 的非规范写法）；不传输历史 commands、base.fields/notes 或冻结正文。view 和新增命令通过 sqlx Json 直接使用原 serde 合同，不经过额外的 Value 中间树。全候选读取、审核、应用和基线核验仍使用原完整读取合同；冻结 v2/v3 格式不变。

ID 和作者文本只作不透明的事务校验标记；业务作者仍由原 Author serde 解码，兼容省略可选字段及未知属性，不重新定义 JSON 语义。

一条 UPDATE 同时保存完整 view 并向原 round 追加新增 commands。WHERE 再核对项目、候选运行状态、round ID、原始作者 JSON 文本、运行状态、数组位置、已有命令数量和 ID 唯一性。项目锁与事务覆盖读取、owner/租约检查和 UPDATE；失败包括提交阶段失败均回滚。重复调用仍按原合同追加重复命令，不引入新的幂等键或去重语义。

SQL 标量提取仍可能解压完整 base/batches；JSONB 追加仍可能重写完整 batches/TOAST 并产生 WAL。完整 view 仍然读写。本步减少应用传输、解码和历史往返，不保证物理增量写入或与历史长度无关的成本。

存储端口的默认实现服务现有内存存储，保留同样的 round 身份/位置检查；PostgreSQL 重载为原子 SQL。未引入缓存、队列、独立生产模块、迁移或生产控制参数。

验证入口：`scripts/test-curation-queue.sh` 新增真实 PG 追加/并发/提交失败测试；原重试、运行状态、冻结存储、v2/v3 进程恢复和浏览器观测值测试继续复用。性能使用原 candidate-save 驱动，正式前后 release 只有共同 runtime 重复计时包装，没有 core/adapter 分项插桩。

基准、逐轮 SQL/WAL/RSS、资料不变性、源码与二进制指纹、复跑方法及限制见 `docs/audits/2026-10-14-candidate-save-local/`。没有进入第六步，部署与全量并发容量验收不属于本次交付。
