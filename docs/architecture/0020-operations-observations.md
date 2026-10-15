# 运行状态只读观察（性能计划第 03 步）

2026-10-13。只新增管理中心 `/management/operations`；普通页面、全局浮动入口及整理执行决策保持原有结构。没有停止、抢占、配置调整或清理操作。

## 读取边界

- HTTP 的三条 GET 路由 `/v1/operations/summary`、`/tasks`、`/tasks/{task}/diagnostics` 全部复用 `management::administrator`，在参数提取前核验启用且账号不区分大小写的 A80646；未登录、普通、禁用、伪造及 MCP 凭证均不能读取。响应 `no-store`。
- knowledge 的 `OperationsReader` 提供元数据分页、后台状态计数及精确任务所属项目。adapter 只读所属 schema；聚合/HTTP 层无 SQL，没有调度器或任务写端口。
- 任务按 `(created_at,id)` 倒序游标分页，默认 20、最多 50；可按项目、状态筛选。每页用 `limit + 1` 确认下一页，不对 GET 执行全表 count 或 OFFSET 扫描。
- 周期列表不读取候选 batches、原始 progress、base/view 或 execution_input。标量 JSONB 提取同样会解压整列，故原批次计数已移除，兼容字段 saved_batch_records 固定为 null；具体批次仍通过原 schema 2 诊断按需读取。
- 新迁移 202610130002_operations_checkpoint.sql 在候选中增加最多 1 KiB 的 operations_checkpoint。所属 schema 的 BEFORE INSERT/UPDATE OF progress 触发器在原事务内提取六个固定、有长度/类型边界的字段；不查询其他表，不改变状态/租约/重试规则，不读取 batches。直接写摘要也会从 progress 重算，避免漂移。
- 迁移不扫描大文档回填，旧记录摘要为 null，正常进度写入后才采集；单独心跳更新不改写摘要。原 progress/批次/冻结输入及候选视图均不重构，范围不包含第 05 步完整快照投影。
- 当前阶段和心跳只来自 task=running 且检查点 run_id 精确相同的候选；旧阶段单独保留为检查点。无候选、旧进度、不明心跳均不构造假进度；单任务瞬时模型/数据库等待未采集。保存进度时间来自 progress.updated_at，运行心跳来自候选记录更新，两者不混用。
- 总计每 15 秒在后台按状态聚合，范围是数据库全局，与筛选列表分开标示；不是模型在途。读事务只读，statement_timeout=1.5 秒，后台总等待上限 2 秒。超过预算保留旧值并记录错误。
- 名称复用 access 的项目权限读取，每页按不同项目去重，上限等于页大小；没有跨 schema join。
- 诊断复用 audit 既有 `task_diagnostic`、项目权限和 schema 2 白名单。管理面板直接调用原诊断 API，operations 的别名也走同一个导出函数；没有第二套 package 或放宽事件关联。

## 进程观测与生命周期

- 只在 serving API 的 `build_api_with_recovery` 启动一个采样循环，和既有恢复循环共同使用 shutdown token；预检/普通 router 构造不启动采样。没有第二套执行循环。
- 资源、模型、连接池和接收计量每 5 秒采样，missed tick 跳过；资源 IO 在单个顺序等待的 blocking 工作中进行，GET 不运行 shell 或采样。
- 每个快照含 source、scope、成功采样时间、最近尝试时间、过期阈值及安全错误码。失败不更新时间或把空值改成零。资源/模型/池/接收 20 秒、任务总计 45 秒过期；客户端每秒判断显示状态。
- 资源历史最多 60 点，页面显示最近 12 点。模型和接收耗时各最多 256 次；不保存逐请求正文、提示词、高基数标签或无限时序数据。
- 每个 API 实例有进程启动 UUID、PID 和启动时间。模型、连接池、接收量及资源仅属于该实例；没有跨副本汇总或跨副本额度控制。
- `ModelTelemetry` 从原共享 LlmClient/Budget 读取当前/配置额度、占用许可、许可等待与冷却，HTTP future 单独计数。RAII 在取消/退出时归还观测计数；不改 semaphore 获取、调整、退避或错误分类。
- 延时分为等待许可与请求处理，含失败请求；冷却拒绝无 HTTP 耗时。计数自本实例启动累计，耗时为最近有界窗口。错误标签复用现有 CompletionError 分类。
- 六个池通过 common `PoolTelemetry` 由各 adapter 暴露 size、idle、配置上限，包括独立 access_activity。使用量为非原子 size-idle 估算；sqlx 未暴露的等待和获取超时明确 null，不推断为 0。
- 接收计量在原 collect HTTP 边界增加少量计数，含重传/拒绝。记录成功读取的字节数、完成处理批次、接受批次、429/503、最近处理耗时及相邻采样窗口速率；不含请求体读取前网络时间或未完成请求。Fetcher 本地待上传始终未知。

## 平台口径

- 新增精确固定 `sysinfo=0.37.2`、只启用 system 特性，安全读取当前 API 的 RSS 和 CPU；没有升级既有依赖。新增锁文件依赖主要为各操作系统适配。
- RSS 是进程常驻内存。CPU 采用核心用量：1.0 = 一个逻辑核心满载；首次样本没有差分基准，必须未知。
- Linux cgroup 从当前进程 membership/mountinfo 解析可见层级，读取 v2 memory.max 或 v1 memory.limit_in_bytes，并取可见祖先最小限制；不把不可读/格式错误当零。成功的 unlimited 与未知由成功采样时间和错误状态区分。
- cgroup 数值只代表可见层级，无法证明命名空间外祖先限制或宿主总可用内存。macOS cgroup 不适用。
- sysinfo 的 macOS 宿主可用内存估算可能饱和为零且没有逐次失败信号，本版不把它当权威可用量，所有平台都明确未采集。该值不参与任何调度或资源准入。
- macOS 已实际运行进程验收；Linux 解析夹具已通过。本轮 Docker 隔离容器未成功启动，不能把夹具称作 Linux API/容器端到端验收；部署 Linux 时需要复核实际 cgroup 和 RSS/CPU。

## 前端观察

- 复用 ManagementLayout、Reka/App 选择器、页签、抽屉及 DiagnosticDetail。摘要与列表各有请求身份、在途合并、成功提交和局部错误；任一失败或长时间挂起均不阻断另一部分。Promise.allSettled 仅为手动刷新提供完成句柄，不作为数据提交门槛。
- 只有一个 5 秒轮询时钟；按通道合并在途请求。筛选/分页只取消旧列表读取，摘要不依赖筛选；旧游标/筛选响应不能覆盖新数据。权限撤销、隐藏、离线或卸载统一取消相关读取，权限撤销清空两部分及诊断。
- 可见时每 5 秒读取；隐藏/离线取消读取和轮询，visibilitychange/online 立即唤醒；失败保留带时间的旧快照，权限失效清除私人数据。
- 修正既有会话生命周期在已知离线时仍做聚焦/定时校验的问题：仅跳过离线网络请求。凭证变更/过期仍先清除身份，在线后恢复原服务端校验；服务端失败仍遵循原 fail-closed 规则。

## 升级与复跑

- 先备份并停止旧 API，运行现有 `nexofolio-admin migrate`，包含 `202610130001_operations_task_reads.sql` 的四个分页索引及 `202610130002_operations_checkpoint.sql` 的小摘要列/触发器，再启动新 API/前端；没有修改已应用迁移或任务状态结构。本轮只在隔离 PG 迁移，未操作用户服务。
- `scripts/test-operations.sh`：隔离 PG、实际 API 和固定本地模型/身份上游，覆盖权限、任务/检查点、真实模型并发/429、采集账本、缓存失败；所有外部业务/付费模型请求为零。
- `scripts/test-operations-browser-fixture.py` 输出私有目录，随后运行两份 `test-operations-browser*.cjs`；写入目录中的 finish 清理。隐藏页测试使用单独原生 Chrome + CDP noDefaults，避免 Playwright 默认强制聚焦掩盖真实可见性。
- `scripts/test-operations-overhead.py <browser目录>` 在副本 PG 做有界 A/B，使用预先构建的源码基线二进制；准备后在输出目录写入 start 开始测量。浏览器夹具应在复制完成后退出以降低测量噪声。此脚本是测试工具，不是生产监控开关。
- 详细账本、平台限制、开销、指纹与代码量见 `docs/audits/2026-10-13-operations/`。不据此声称生产容量已标定，也不重跑或冒用历史两小时/八小时证据。

## 独立复核后的第三步修复

初版“标量输出等于轻量读取”和 summary/tasks 成功捆绑更新两项判断不成立。最终验证以 [修复报告](../audits/2026-10-13-operations-fixes/report.md) 为准；初版小数据测量和截图保留为历史证据。新增大型批次与大型 progress、默认/最大分页的实际 API/PG/EXPLAIN 对照，以及首次/保留数据下两个方向的局部失败 Chrome 验证。
