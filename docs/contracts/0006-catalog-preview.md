# 历史目录候选兼容接口

旧目录生成机制已退役。生产代码不再提供catalog-create、catalog-run或catalog-preview命令，也不再暴露CatalogPreviewService、DirectoryGenerator或可创建/执行旧任务的存储端口。

新任务统一使用`POST /v1/projects/{project_id}/maintenance-runs`，见[现行重构说明](../architecture/0003-current-reconstruction.md)。

保留以下历史能力，不生成新候选：

- `GET /v1/projects/{project_id}/catalog-previews`：按权限读取已有历史候选列表。
- `GET /v1/projects/{project_id}/catalog-previews/{task_id}`：读取历史快照、结果及其是否当前生效。
- `nexofolio-admin catalog-show --task-id <UUID>`：管理员只读查看历史记录。
- 目录专用发布/回退：对已有候选/版本执行激活，保留当前语义资料，与统一知识发布共用事务机制。

迁移202609150006将历史pending/running任务标记为failed及LEGACY_GENERATION_RETIRED，防止它们永久显示运行中；不改已完成候选或已发布版本。历史任务不能恢复生成。历史数据库表和协议包保留，以保证已存数据可读；旧prompt文件仅属于历史协议材料，生产模型不加载它。不存在200接口/1MiB的旧生成上限路径，当前限制由维护快照与模型预算配置决定。
