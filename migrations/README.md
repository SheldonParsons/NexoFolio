# Database migrations

`202609140001_access.sql` creates the access-owned users, internal sessions,
projects, user/project access and login audit tables, plus a sync generation sequence.
The SQLx runner maintains `_sqlx_migrations`.

Run `nexofolio-admin migrate` explicitly before serving business requests.
API and worker never migrate automatically. Applied migration files are immutable;
future changes require a new migration and tests against existing data.

`202609140002_environments.sql` adds stable project environment IDs and name aliases.
`202609140003_ingestion.sql` adds current comparison heads, immutable observation receipts and the pending downstream inbox; these are not formal API version tables.

`202609140004_project_environment_scope.sql` removes the service partition from current dedup heads, preserving the latest admitted head and all raw inbox/receipt history. Legacy service labels remain provenance only.

`202609140005_observed_documents.sql` adds observed interface definitions, environment current pointers, pending differences, observation references and durable processing leases/retries. Apply before running the configured worker.

`202609140006_directional_observation_comparison.sql` 为当前采集去重头增加结构投影。增量兼容迁移，不改写历史回执、原始样例或待确认差异；运行时按需升级旧算法头。

`202609150001_catalog_previews.sql` 增加手动候选任务、候选目录节点和接口归属。快照、租约和代次用于固定输入及隔离执行；不修改当前接口或待裁决状态，不提供正式目录发布。

`202609150002_path_identity.sql` 增加项目路径策略和inbox识别快照。旧数据字段为NULL，继续按字面路径处理；不改写原始记录、头、文档或候选。

`202609150003_official_catalog.sql` 初始化所有项目的固定待分类目录状态，加入不可变目录版本/归属、当前指针和幂等回执；新项目自动初始化。系统节点身份不能修改或删除。

`202609150004_capture_evidence.sql` 增加独立采集回执、原文/图片引用、机械证据、短期值索引、工作样例组、引用固定和积压容量。旧原文列允许在保留规则满足后为空，旧回执不重写。

`202609150005_knowledge_maintenance.sql` 增加固定维护快照、带执行代次的任务/检查点、模型调用审计和统一知识版本。旧目录来源仍保留，新目录版本可关联维护任务。迁移把已有正式目录初始化为不含语义修改的基线，不删除项目或观测。

- `202609150006_retire_directory_generation.sql`：仅终止被退役生成器的pending/running任务，标记LEGACY_GENERATION_RETIRED；不删除历史快照、完成候选或发布版本。

- `202609160001_observation_assessments.sql`：为已处理观测追加不可变分类结果和实际提取器版本；历史NULL明确表示未按新规则评估，不自动回填、裁决或改写回执。新worker/API必须在迁移后启动。文档读取合同升级到2.0.0；严格旧客户端应先同步支持observed-http-1/2，上传合同不变。

`202609160002_evidence_coverage.sql` adds per-event extraction coverage and marks existing evidence facts as legacy/needs_reassessment. It retains all raw data and fact IDs; it does not reprocess recordings or adopt new interface fields. Rollback to older binaries can retain the additive column.

`202609160003_repair_missing_project_catalogs.sql` backfills missing system-directory state only for projects without retained directory/knowledge/run history. Existing state is unchanged. Ambiguous missing state with history raises an error rather than guessing a published version or system directory ID. The recording reset script now restores system-directory identities inside its reset transaction, clears active versions and advances the generation; full reset still clears all projects.
