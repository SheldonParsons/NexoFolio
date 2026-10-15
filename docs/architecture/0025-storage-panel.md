# 第十步：存储与清理管理面板

2026-10-14。入口 `/management/storage`，复用管理中心布局、Reka 项目选择器和确认框。仅启用的 A80646（忽略大小写）可见和操作；HTTP 复用 administrator、项目授权与写入活动守卫。

- `GET /v1/projects/{project}/storage`：只读进程缓存，不采样、不预览、不登记清理。
- `POST .../storage/refresh`：请求后台统计，复用运行状态的 5 秒时钟；每项目请求间隔至少 15 秒，最多缓存 32 个项目。
- `POST .../storage/maintenance`：`action=preview|enable|execute`，可带 `after` 游标；固定每批 10 个候选，没有修改保留策略或任意删除参数。
- `NEXOFOLIO_RETENTION_ENABLED=1` 是既有服务端允许执行的开关；API 启动时读取。未开启仍可读取和预览。
- `enable` 只用于未启用项目，由既有维护引擎登记首次时间；7 天内不精简或回收。已启用项目拒绝重复 enable；未启用项目拒绝 execute。
- 执行重新校验当前账号，并进入第九步项目锁与引用检查事务。旧预览不提供删除授权。单实例维护入口互斥；跨实例仍由项目事务锁保证安全。
- 维护外层 5 秒截止；提交失败或未获确认返回 unknown，界面说明“提交结果未知”，禁止自动重放或根据失败进入下一批。再次明确预览/执行同批可安全重入。
- 执行后缓存标记过期并排队重采样；采样的代次校验防止执行前的旧快照覆盖新状态。失败保留旧值及原采样时间。

统计通过 `common::storage::StorageReader` 调用 observe/knowledge 自有适配器；HTTP 不含 SQL，不跨模块 JOIN。每类在数据库内对全部应计记录执行 count/sum，仅返回一行计数与列字节总和，没有行数门槛或部分和。每条 SQL 350ms 截止，每模块 2 秒外层截止，保护状态 1 秒截止；REPEATABLE READ 只读事务提供模块一致快照；应用不读取候选完整正文。采样查询 work_mem 为 1MiB，禁用并行查询工作进程；无排序/分组物化。任何查询失败时丢弃本模块新结果，保留旧值及原采样时间，其他模块独立更新。后台每次处理一个待采样项目，不建设自动清理调度。

| 界面口径 | 含义与限制 |
| --- | --- |
| 项目压缩后列大小 | `pg_column_size` 汇总已列出的内容列；不含行头、索引、空闲页，不等同磁盘回收量 |
| 逻辑内容大小 | 当前未展开正文，明确显示未统计；不使用压缩大小冒充逻辑大小 |
| 共享表及 TOAST / 索引 | `pg_table_size` / `pg_indexes_size`，全库共享，不归单项目；仅覆盖列出的相关表，不宣称数据库总容量 |
| 运行快照 | 执行头与全部冻结块；冻结块包含混合用途证据，不在其他类别重复计算 |
| 观测值与证据 | observe 代表值、旧归档文本、knowledge 证据头；不含已归入快照的冻结证据块 |
| 原始资料 / 整理历史 / 正式知识 | 样本与声明 / 候选 base、view、batches 与回滚资料 / 字段与接口说明；辅助目录、关联和回执未纳入 |
| 到期处理量 | 仅预览/执行报告中的对象数与跳过原因；未提供逻辑回收字节预测，不承诺文件立即缩小 |

停止但候选尚未结束的任务单独列出最多 20 个，并标明是否还有更多。所有可恢复工作继续受第九步保护。此页面不放弃任务；明确放弃仍走原整理流程。原始资料、人工知识、废弃状态、仍引用证据不被本维护删除。

客户端可见时每 5 秒读缓存、每 30 秒请求采样，隐藏时暂停读取。身份、项目或权限变化同步清空状态并取消旧请求；返回时再核对请求所有者和项目。服务器缓存是管理员共享的项目统计，所有读取仍重新校验会话及项目访问权，不缓存身份凭证。60 秒过期；未知、失败、过期与真正的零分开显示。

## 验收与复跑

`python3 scripts/test-storage-panel.py`：自建随机端口 PostgreSQL/API，固定合成旧日期，真实构建前端和隔离 Chrome。使用本地假身份服务，不读取 .env，不联系业务系统或付费模型。退出清理自身数据库、API、浏览器 profile 和临时前端产物。需先构建 `cargo build -p nexofolio-backend --bin nexofolio-api --bin nexofolio-admin`。

定向架构检查：`cargo test -p nexofolio-backend --test architecture workspace_dependencies_are_explicit_and_directional -- --exact`；`sql_ownership::modules_only_touch_their_own_schema`；`curation_structure::curation_and_storage_files_remain_reviewable`。前端执行 `vue-tsc --noEmit -p tsconfig.app.json`。证据见 `docs/audits/2026-10-14-storage-panel/`。

第九步 RetentionReport 仅移动到知识模块合同供入口使用；维护函数、SQL、引用证明均未改变，复用指纹一致的原有深层引用、并发和边界验收。本步没有在真实开发库迁移、启用或清理，没有重启用户服务、发布或推进第十一/十二步。
