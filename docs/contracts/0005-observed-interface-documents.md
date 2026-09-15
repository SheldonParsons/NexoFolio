# 观测建档、当前定义及查询 API

状态：后端已实现，裁决/重构尚未实现。权威返回结构见 contracts/documents/responses.schema.json，生成的 TypeScript 类型与 manifest 随版本检查，具体行为见 contracts/documents/behavior.md。

## 当前处理流程

后台处理 ingestion_inbox：领取 pending → 提取观测结构 → 匹配项目中的接口身份 → 初次为该环境建档/无变化附加样例/有差异保存候选 → 同一事务标记 completed。
接口身份为项目＋方法＋路径；每个环境有自己的当前观测定义，接口 ID 可以跨环境复用。
新文档 state=observed、classification=unclassified，暂时统一“待分类”。不生成真正的目录树，不调用分类器、模型或重构器。

当前定义来自首次成功处理的观测，不是经过裁决的完整规范。提取query参数名、头名称、请求/响应JSON字段及其类型、媒体类型和HTTP状态；保留原始记录引用。
缺失正文、截断、无法解析、null、空数组和非JSON内容显式记录限制。提取上限10000节点/64层，超限标明unknown/limitations；原始样例仍完整保留采集到的内容。
不推断必填字段、中文业务解释、枚举、默认值或字段删除规则，不对原始请求头/正文脱敏。

相同接口后来结构变化时，保存proposed_definition、base_revision和观测引用，pending差异按基线＋提取结果摘要合并。
当前定义不被新差异覆盖；所有处理过的观测仍有独立来源记录。不同环境各自建初始定义，不互相覆盖。
原始数据仍在inbox，不复制多份大正文；interface_observations保存其关联。

## 查询接口

所有接口使用内部Bearer Token，每次验证本地项目访问权限；不会请求禅道或续期Token。

| 接口 | 主要参数 | 返回内容 |
| --- | --- | --- |
| GET /v1/projects/{project_id}/interfaces | environment_id必填，page默认1、limit默认20、query可选 | 当前环境的待分类接口卡片与total；query为method/path的大小写不敏感子串 |
| GET /v1/projects/{project_id}/interfaces/{interface_id} | environment_id必填 | 当前观测定义、限制说明、原始观测ID及待处理差异数量 |
| GET /v1/projects/{project_id}/interfaces/{interface_id}/observations | environment_id必填，page/limit | 处理过的观测摘要、outcome、difference_id及基线修订ID |
| GET /v1/projects/{project_id}/observations/{ingestion_id} | 无 | 接收/处理状态、原始观测、处理结果和可能的差异候选 |

page范围1..100000、limit范围1..100。列表不加载大正文，查看原始样例时按需请求最后一个接口。
没有过滤器的接口列表等于该环境的“待分类”入口；不是自动生成的业务目录。
读差异：从观测列表取ingestion_id，读观测详情中的proposed_definition，与compared_revision_id对应的当前基线比较。
这里没有裁决按钮或accept接口；outcome=difference_recorded不表示差异已经应用。

鉴权错误：401无效/过期凭证，403明确无项目权限，503尚无权限快照/服务暂不可用，404项目/环境/接口关联不存在。403/503不代表必须退出登录。

## 后台可靠性

- 同一项目/环境/接口按received_at、id顺序处理，不能让后一条抢先决定初始定义。
- 不同接口/环境可以由多个worker并发领取；FOR UPDATE SKIP LOCKED避免重复领取。
- 领取租约120秒，执行代次每次递增；90秒处理截止时间，过期任务自动恢复，旧代次不能提交结果。
- 文档、基线、差异、观测关联和completed状态在同一个事务提交，任一步失败整笔回滚。
- 失败按2/4/8/16秒退避，最多5次后failed。failed项会阻止同接口环境后续观测抢先处理，其他接口仍可继续。
- 错误只保存固定错误码，不把原始正文或凭据写进错误日志。
- 开启业务配置（NEXOFOLIO_ZENTAO_BASE_URL存在）的worker消费队列；骨架未配置模式仍待机。此条件只作为当前部署开关，处理时不访问禅道。

运维命令：

```bash
nexofolio-admin process-one
nexofolio-admin retry-observation <ingestion_id>
```

process-one处理最多一条可领取记录；retry-observation仅对failed项重置重试次数并重新排队，不改变原始观测。
本轮没有对外手动重试接口。worker重启恢复依靠数据库租约，不依赖内存队列。

## 迁移与范围

新增migration005：interface_documents、interface_observed_revisions、interface_environment_current、interface_observed_differences、interface_observations，以及inbox租约/执行代次/重试字段。
不修改已存在的001～004迁移，不清空用户、Token、项目、环境或接收记录。
接收回执accepted继续表示可靠接收；后台完成状态从观测查询接口取得，不改变原回执。
后续裁决负责正式版本切换，历史版本不继续维护；当前仅建立初次观测基线并记录差异，没有实现历史版本编辑。

空数组比较规则（文档合同1.1.0）：后续空数组不会抹去已知元素结构，其他信息被覆盖时关联为 unchanged；先空后有数据仍记录待确认的新信息。接收去重和文档处理共享 contracts 的方向比较函数。已经产生的待确认差异不会自动删除或裁决。
