# NexoFolio MCP 文档入口

当前 MCP 围绕 Agent 主导的渐进式检索设计，使用账号级独立凭证和 Streamable HTTP。接口名称、目录、字段、样本与来源由工具发现，用户无需手工查 UUID。

- 实现与职责：[渐进式 MCP 检索架构](docs/architecture/0011-progressive-mcp-retrieval.md)。
- 用户验收：[商用订单管理系统测试话术](docs/handoff/2026-10-09-mcp-agent-acceptance.md)。
- Agent 阅读：前端实例的 `/docs/mcp` 或 `/mcp.md`，使用当前实例实际地址。
- 工具定义以 `apps/backend/src/mcp/tools.rs` 和运行服务的 `tools/list` 为准。
- 服务整体指引在 `apps/backend/src/mcp/instructions.md`，随知识服务初始化的 `instructions` 返回，说明结构用途和收益，检索路线由 Agent 自主决定。

现有入口共 11 个。项目/环境发现后，Agent 自选目录探索、接口搜索或字段线索，不要求先下载整个项目，也不要求每次先读接口概览。

- 精确字段通过 `matched_field.open` 或字段树 `open` 直接读 `field_summary`，保留完整语义和未知点；证据正文、观测原值及旧 `field` 完整快照按需展开。只有有子字段的节点提供 `expand`。
- 已保存关系与 `source_reads` 提供明确的工具/参数入口；仅有文字提及时搜索核实，引用失效或需要比较时再选择样本。
- 来源核对分别检查正文与采集上下文。判断先后使用来源 `completed_at` 和目标 `started_at`；同页面不等于同视图，样本同值不证明必调顺序。
- 覆盖检查的 `summary` 展示问题分布，`issues` 可按 `issue_kind` 或精确 `field_key` 读取明细；按任务选择视图，区分生成处理、语义确认与采集缺口。
- 列表默认 20、最多 50；长文档默认和上限均为 16384 字符。续读使用实际 `next.arguments`，不猜偏移或混合版本。

网页 `/docs/mcp` 与无脚本 `/mcp.md` 的内容在前端仓库统一维护；已补充任务 ID 来源、文字线索找字典及定向缺口检查的可复制话术。旧 read_interface 组合读取继续兼容，完整工具定义仍需从运行服务确认，不能只按数量判断版本。

本文件不再维护旧 `nexofolio_catalog` 等工具名、项目级凭证流程或未经发布的 npm 客户端安装命令。服务是否已加载新能力需要实际初始化和工具发现确认。

## 项目文档的废弃状态

目录、接口和字段可由有项目权限的用户在网页废弃或撤销。MCP 保持 11 个只读工具，不能改变这些决定。生命周期与证据确认状态独立；废弃接口并不让引用它的其他接口自动失效。

`browse_directory`、`search_interfaces`、`read_interface` 支持 `include_retired:true` 展开历史。默认有效内容优先，返回的 `retired_history`、`retired_directories`、`expand_retired_fields`、`retired_relations` 提供对应读取入口。已知 ID/字段的精确读取仍返回历史及状态；不要将 retired 解读为从未存在。

`project_lifecycle` 包含版本、已核对版本、直接废弃数和 `pending_reconciliation`；存在变更时提示“存在废弃变更，当前整理尚未重新核对”。`target_lifecycle` 描述精确目标，条目 `lifecycle` 描述该条目。有效对象引用旧来源时另有 `retired_source_count` / `source_notice`，不能把提示传播成引用方自动废弃。`read_source_evidence` 可继续读取原因、操作人和时间。原始报文保持原内容，状态在外层返回。

下一次调用读取当前状态；分页/分块的 revision 包含生命周期版本，状态变化后从该次读取入口重新开始。Agent 根据状态、证据与历史自行裁决；平台不承诺枚举自由文本中的完整语义依赖。
