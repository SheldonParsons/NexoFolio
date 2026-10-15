# 第九步：历史保留与引用安全清理

2026-10-14。维护属于 knowledge；普通 API 启动和 GET 不启动清理，没有后台调度器。仅提供明确项目的单批管理命令。

| 存储 | 所有权与保留边界 |
| --- | --- |
| candidates.execution_input v2 | 旧版续跑材料；明确 applied/discarded 满 7 天后释放。独立的旧 field_value_archives 仍按引用保留。 |
| execution_input v3 | 混合用途：执行清单也定位正式观测值。释放时将原始头部完整移到 evidence_input，原 manifest_hash 不变。 |
| frozen_input_chunks | facts、inputs、links 和未发布尝试是执行材料；已发布 manifest 的 values/document 块是证据。前者可随结束 7 天释放，后者走引用与宽限。 |
| candidates.base/view/batches | 整理历史及其结构化引用；最近 30 天与每项目最近 5 个结束候选的并集保留，未结束候选始终保留。 |
| rounds/revisions | applied_rounds 是不可拆分的应用回滚分组；仍保留的 before.context.observations 保护旧归档。独立人工修改的回滚资料继续保留。 |
| field_notes.context.observations | 正式归档入口；按 archive_id 保留整个归档身份，不以字段文字猜测。 |
| observe/intake、正式文档、人工知识、document_lifecycle | 本维护不删除、不重写这些资料。废弃不构成清理许可。 |

终态只认可 applied/discarded。running/ready/partial/interrupted、关联 queued/running/waiting/needs_attention 任务，以及仍有未结束子候选的父候选均保护；停止不等于放弃。结束时间使用终态 updated_at，维护不改写它。30 天边界包含在完整历史内；满 7 天可释放执行资料。最近 5 轮按结束时间、UUID 稳定排序。

历史精简先执行：保留 ID、项目/范围、状态、时间、审核人、操作事件、原命令数量、批次数与生命周期版本，并标明详情/对比/回滚到期。base/view/batches 被释放，但完整候选读取先返回 HistoryExpired，不解析空对象冒充文档。现有 HTTP 返回 410/HISTORY_EXPIRED；历史列表通过 progress.history_available/history_message 表达降级。既有前端保留错误信息展示；本次修复仅同步诊断类型，没有页面改动。

回滚分组只有在全部 round 身份都存在、归属同项目、已结束且到期、作者明确为 curate、没有其他完整候选保留该分组时才精简。未知、缺失、跨项目或人工分组跳过。round 摘要仍保留；revisions 读取在项目锁后检查过期，不能成功执行空回滚。

然后检查正式字段、完整候选的 base/view/commands、旧 v2 prepared.inputs、所有未释放的字段输入块（包括尚未发布的尝试）、保留 revisions 的结构化归档引用。完整历史还额外保护自己的归档。确认无引用后只记录 archive_unreferenced_since；连续再满 7 天才删除旧归档文本和剩余证据块。候选身份与墓碑保留。

同一项目 advisory transaction lock 覆盖策略重读、精简、引用扫描和删除。启用项目的字段/候选/回滚/任务写入触发器复用同一锁，新引用或候选状态变化取消原宽限；已回收身份拒绝迟到引用。冻结字段块写入复用同样校验，原文档块保持不透明。未启用项目不扫描候选引用 JSON。应用、恢复、人工写入与回滚继续使用原事务。

每次限定 1–20 个候选（默认 10），使用 --after UUID 继续；下一轮需从头重查。只读取本批元数据与不超过 4 MiB 的 v3 头，v2 只取版本，不装载待释放正文。头超界或本候选格式异常时标识具体候选并跳过，其他候选继续；不再用全项目记录/字节总量阻断材料释放。

引用证明仍在同一项目锁内，但置于材料精简之后的事务保存点。SQL 只返回与本批有关的归档 ID（最多 20 个），每条引用查询限时 500 ms；冻结输入按键分页、每页 64 个元数据，逐块读取，单块最多 16 MiB、累计最多 128 MiB/20,000 条。未发布块也必须检查。未知格式、超界或查询超时只回滚引用保存点，清空本批相关宽限并延后归档回收；已安全完成的精简可以提交，不保留部分引用证明供下次删除。大证据正文按原已发布清单身份保留，不为释放执行材料再次装载正文；实际证据读取仍验证原哈希。

普通 SQL 仍限时 1.5 秒，循环约 3 秒停止继续工作，CLI 外层 5 秒截止。预览仍执行同一事务后回滚；提交前取消回滚，提交确认丢失为未知，可安全重入。reference_records 计返回的匹配归档身份及实际检查输入块，reference_bytes 计实际读取输入块字节，均不是全库扫描行数或磁盘释放量。deferred_objects 给出超界/异常对象身份。

诊断候选、任务与运行状态入口共用白名单导出：history.available/ reason_code、retained_commands/retained_batches 和 unavailable 表达过期能力；batches.available=false，included=0，truncated=null。旧精简记录未保存批次数时 recorded/retained_batches=null，不能解释为零批次；原本为空且仍完整的历史为 available=true、recorded=0、truncated=false。仅导出类型化计数/布尔值与固定代码，不复制 history_message 等自由文本，Agent 提示同步说明此区别。

## 后续部署与启用

本次仅在隔离 PostgreSQL 迁移和验收。实际数据库、开发服务和保留设置均未操作。用户后续明确部署时，先按现有管理流程执行扩展式迁移 202610140001_history_retention.sql，再部署配套 API/管理二进制；迁移没有回填或启用项目。精简发生后不能退回不认识 HistoryExpired/evidence_input 的旧读取二进制。

管理命令使用专用连接变量，绕过普通 Config 和模型配置；未提供 --execute 时只预览。以下是用户后续明确启用时的操作模板，本次未在实际库执行：

```sh
export NEXOFOLIO_RETENTION_DATABASE_URL='<明确指定的 PostgreSQL URL>'
nexofolio-admin retain-history --project '<项目 UUID>' --limit 10
NEXOFOLIO_RETENTION_ENABLED=1 nexofolio-admin retain-history --project '<项目 UUID>' --limit 10 --execute
# 完成一页后按报告 next 继续；以后新一轮不带 --after，从头重查。
```

第一次 --execute 仅登记该项目 enabled_at，并启动额外 7 天保护；首次启用后的 7 天内不会精简或删除，包括原有历史。即便迁移已很久，首次启用也重新给予这 7 天。没有自动后续执行；实际调度和第十步管理面板仍未建设。

复跑：`bash scripts/test-history-retention.sh`、`python3 scripts/test-history-retention-api.py`。前者自建 PG，后者复用第四/六步的隔离合成夹具并自建 PG/API，退出清理其自身进程与临时目录；均不读取工作区 .env。证据与源码增量见 `docs/audits/2026-10-14-history-retention/`。
