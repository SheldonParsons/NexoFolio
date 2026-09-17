# 当前入口与策略

新重构统一为维护HTTP入口→MaintenanceEngine；旧目录生成已退役。完整现行说明见[统一重构](../architecture/0003-current-reconstruction.md)，下文的早期演进记录不得解释为仍有旧生成路径。维护行为包更新为1.6.0，四策略限制明确，JSON字段结构未改变。

# 持续采集与接口知识维护

## 三条独立链路

1. 接收链路验证内部Token、项目权限、信封与记录，按项目/环境识别结构，原文落持久卷、元信息与回执事务提交后收讫。
2. 机械证据worker提取值、UI标签、字典、参数关系和反例；不调用LLM。相同结构的新观测继续处理，原记录重试不重复累计。
3. 用户创建维护任务时固定快照；worker分段审阅所有字段、核对覆盖、建立全局索引、回读实际来源后形成候选。用户确认才切换正式目录和语义。

## 合同与接口

capture协议3的权威包为`contracts/capture`，旧ingestion1/2保持接收和原回执兼容。HTTP记录的payload_version仍为1，新增类型是page_context、interaction、ui_snapshot、image_reference。所有增强记录有独立page/frame/view/interaction和时序；浏览器实例不同于持久producer。协议不要求用户手动开始或结束录制。

采集包1.4.0沿用相同Schema，将[公共传输规则](../../contracts/capture/behavior.md)与[Chrome最小采样规则](../../contracts/capture/chrome-sampling-profile.md)分开。第三方HTTP来源可省略context，不要求伪造浏览器身份；interaction_id仅在有依据时提供。插件停止新增截图、整页扫描和滚动/DOM变化触发的采样，查询/提交时同步保存一次相关表单控件状态。请求共用这份快照，后端关联interaction目标和ui_snapshot控件；多控件同值或多字段匹配不强行绑定，字符串/数字转换明确记录，结果始终是带来源的推断。异步请求可能无法关联到操作，但HTTP观测仍上传。采集面向日常无感使用，用户可随时从A切换到B；相邻事件、同页或同一上传批次不等于同一业务流程。业务控件采样扩展为有界局部组件容器，不能扩大成整页扫描或复用另一模块的控件状态。历史图片与旧队列继续兼容。

| 接口 | 用途 |
| --- | --- |
| GET `/v1/ingestion/capabilities?schema_version=3` | 协商增强接收；不带query仍返回旧合同 |
| POST `/v1/ingestion/batches` | v3的accepted是可靠收讫；structure与后台证据状态独立 |
| PUT/GET `/v1/projects/{project}/assets/{asset}` | PNG/JPEG/WebP原字节，最多8MiB，同ID不同内容409 |
| GET `/v1/projects/{project}/capture-observations/{observation}` | 结构状态、证据状态及仍可用的原文 |
| GET `/v1/projects/{project}/evidence` | 分页证据；可按environment_id过滤 |
| GET `/v1/projects/{project}/evidence/{fact}` | 单条事实及工作样例ID |
| POST/GET `/v1/projects/{project}/maintenance-runs` | 用request_id创建，或分页读取任务 |
| GET `/v1/projects/{project}/maintenance-runs/{run}` | 进度、完整覆盖、错误与候选 |
| GET `.../{run}/snapshot` | 本轮不可变的接口、字段、目录、语义及证据清单 |
| GET `.../{run}/checkpoints` | 分页真实审阅、摘要、回读记录 |
| POST `.../{run}/publish` | request_id和expected_generation，整体发布 |
| GET `/v1/projects/{project}/knowledge/versions` | 发布历史及当前共享generation |
| POST `/v1/projects/{project}/knowledge/restore` | 指定version_id（null为初始），整体回退 |
| GET `/v1/projects/{project}/interfaces/{interface}/knowledge?environment_id=...` | 描述、来源关系、枚举及是否待复核 |

所有读取和写入验证当前项目权限。原文、图片、快照不使用公开URL；响应no-store。旧目录发布/回退仍可用，且保留当前语义，不意外回退描述。v1/v2返回原结构处理语义；v3才返回独立observation_id。旧队列不得升级schema、改ID或改原payload。

## 证据边界

同一项目、环境、用户、producer、浏览器、页面和frame内做短期值关联，按采集时间判断120秒先后关系。跨SPA视图只在存在同页实际交互桥接时保留弱线索，不把“最近交互ID相同”当成因果。来源冲突、多个来源、搜索范围受限保持不确定。0/1、空值和技术性凭据不会仅凭值相等形成关系；原始凭据仍按原样存储。

值类型严格区分；数字和字符串转换显式记录。枚举分已观察值、标签映射、字典查询范围与完整约束。前端空选项不直接产生枚举值；只有已有控件/字段关联及完整请求中实际省略，才产生omitted行为。枚举说明中的值和标签必须有引用证据，完整约束必须有完整范围证据。

字段引用包含接口、环境、定义修订、位置和路径；数组字段用通配路径，原始样例保留实际数组位置。接口结构不因描述更新改变。新证据矛盾时保留冲突，模型下一次维护提出修正或撤回。历史结论不能作为新的独立证据。接口结构裁决和物理合并不在本期。

## 保存、压力和运行配置

图片和大原文按项目/内容SHA256放服务端持久卷，数据库管理引用和权限。默认临时原文24小时后可回收；首次、近期和反例保留最多5组工作样例，被快照引用的样例通过单独pin保留。回收不清除回执、正式定义或仍引用的来源。原文过期时payload_available=false/raw_record=null，不能伪装成完整样例。

接收保留8MiB批次、50条记录、单记录4MiB以及并发保护；资产最多4个并发读写。项目未完成证据超过50000条时返回429 CAPTURE_BUSY/Retry-After，已接收记录的重试仍返回原回执。数据库、卷或队列故障不返回假accepted。

`NEXOFOLIO_CAPTURE_ENABLED`默认false，`NEXOFOLIO_BLOB_ROOT`在Compose固定到共享卷。维护默认上下文65536、每任务256次调用、输入使用保守UTF-8字节预算及40%余量、单次最多240秒、最多2次重试。`NEXOFOLIO_MAINTENANCE_CONTEXT_TOKENS`、`...MAX_CALLS`和`...SNAPSHOT_BYTES`可配置；快照默认64MiB。超限不改为抽样，任务明确失败并保留已有检查点。

模型只读取本轮资料。字段/关系两端、证据原文和受影响目录分支必须回读。模型读取图片需要显式启用视觉且读取成功；当前文本配置默认不把图片当作已读。分片阅读并不保证推断正确，人工候选仍显示observed/inferred/needs_review。

发布先检查幂等，再校验共享generation、定义修订、引用与系统目录保护；目录和语义事务切换。回退不删除新观测，新接口保留固定待分类。运行中的录制不修改已经生成的快照。旧执行租约失效后无法提交；进程重启后同配置可以接续已完成片段，配置改变时明确失败，避免混用两套模型结果。

## 交付顺序与验收

先备份数据库和capture_data，部署迁移/后端及模型文件、初始化卷，验证能力声明，再升级插件、前端。保留原会话密钥、旧协议、项目数据，不自动发布真实项目候选。真实Chrome操作由用户验收；合成HTTP及页面模拟结果不能冒充扩展实录结果。


## 审计后的内部实现优化

外部Schema与回执语义保持不变。审阅按实际准备请求的字节预算组片，每片最多48个单元；共享接口/环境上下文和祖先引用可无损展开，全部字段仍须逐项审阅。worker在首次模型调用前计入未读片段、受摘要大小约束的多层汇总上界、16次规划/回读余量及有界重试余量，不足则以MODEL_BUDGET_INSUFFICIENT_FOR_REVIEW结束，并保留execution-budget检查点。运行中持续检查剩余额度，不能承诺任意模型回读都会在预留次数内结束。

同一预算内的多个原文引用批量回读；成功调用后保存一个readback批次检查点，并分别保存各引用readback_complete记录。read_count仍计实际成功回读片段/批次，不冒充字段数量。超大原文继续分片，图片继续单独调用；摘要不代替原文。

模型调用审计表的response现在保存output和可选usage，缺失usage表示供应商未报告。此为内部审计结构，不修改客户端合同；不把保守字节上界当真实token费用。

人工发布策略由应用服务注入，数据库适配器只执行权限和版本校验后的事务。旧目录专用发布/回退和旧v1/v2回执继续保持兼容，冻结旧功能扩展；模型HTTP传输、事务项目鉴权等公共不变量由单一实现复用。


阶段4执行器仅将每段有界发现摘要交给全局汇总；全部事实和逐字段审阅留在机械索引/检查点中，按稳定引用回读，不再重复调用模型汇总每条原始事实。审阅摘要的JSON字符串最多512字节；字段意见不因摘要省略而删除。较大的导航回读分页保存JSON指针条目，不伪称原文已经读完。内部共享输入格式不改变上传、候选、发布的外部Schema。引擎/提示版本变化后，已有未完成任务按既有规则报MODEL_CONFIGURATION_CHANGED，需要新建任务；历史候选和发布版本不重写。


阶段4审阅标准：关系候选/反例的代表事实须在对应单元意见中显式引用，字段引用不能替代事实回应；这不等于语义理解已经被机器证明。协议头仅有观察样例不能产生枚举候选，明确的同字段声明/字典/控件证据仍可支持。原始观察数据不删除、不脱敏。审阅最多每批4路，每段自行保存检查点、批次后聚合coverage；失败保留已成功片段，不发布部分结果。外部Schema未改变，模型/引擎指纹区分新规则。

规划读取按实际请求预算分批投递，待送达材料不计为已读；已读原件仍可按稳定引用再次读取。事实读取返回完整事实与明确标为未提供内容的样例链接，原始抓包按source另行读取，不能把样例链接当成已读原文。超长字符串分段保留UTF-8位置和总长度，不静默截断。该变化属于内部投递方式，公共上传/候选Schema不变。
