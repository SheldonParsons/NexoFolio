# 本地服务与存量派生资料更新

2026-09-17。用户明确要求更新后执行；已部署本地API/worker并更新存量证据。未提交或推送Git，没有调用LLM、发布目录或修改前端布局。

## 当前可见结果

- API、worker运行`nexofolio-backend:observation-20260917`，镜像ID为`sha256:dfe599f7a1b8a9460ca2e276ff3318e9d559c039fdedad7999816873d03b3541`。readiness正常，两个服务运行正常；deploy/.env已保存镜像名。
- 400条采集按新规则重处理完成，待处理0；当前有效证据4,290条。
- 用户指定UAT环境的`5173/api/v1/projects/9568a877-eb48-42ee-acbc-f9217f052386/evidence?...`实际返回total=4,287、当前页100条。另一个环境3条，因此环境数与项目总数不同。
- 旧5,256条证据标记`superseded=true`，默认列表与新快照不再混入；按旧ID仍可回读。物理事实行共9,546，并非删除旧事实后把磁盘占用宣称减少。
- 经前端5173/api代理回读39份接口详情，5份使用明确标记的紧凑视图。菜单响应结构2,472字节，43字段列表响应结构2,302字节；原始观测仍可按origin_ingestion_id读取。
- 正式库maintenance_calls仍为0，本次更新未发起模型重构。

## 更新做法与引用保护

新增管理命令`nexofolio-admin refresh-evidence --project-id <UUID>`。它在既有项目证据锁内检查完整原文与旧版状态，将旧事实标为历史并保留ID/支持数/样例；仅重建临时关联索引，重新排队来源采集。旧去重键移入历史命名空间，避免新提取覆盖旧事实。重复运行只继续未完成任务，不把已完成采集再累计一次；隔离测试第二次processed=0。

默认事实列表和新快照排除superseded数据，单事实读取保持历史可追溯。原有快照、发布引用不改写。紧凑结构是knowledge共享函数生成的读取视图，definition.schema_view=http-structure-4标记其性质；不修改原定义、定义哈希、修订ID或原文。新冻结的接口资料也使用该视图；历史已冻结快照不追溯改写。比较规则与读取视图有相同来源，模型能看到投影标记。

离线更新前后核对：全部39份已存定义及hash、400条原文引用、采集回执数量、全部旧事实ID与支持次数均保持一致。原始文件卷和固定录制备份未清理。临时停止API/worker后更新，在确认一致性后恢复服务。

## 前端合同兼容

前端使用Ajv严格验证，旧合同会拒绝新增schema_view和observed-http-3。已复制后端documents/2.2.0合同包，并仅更新documents/contract.ts、types.ts、interfaceReport.ts三个文件的版本引用，保留原有工作区改动。实际39份在线详情通过前端所用Ajv合同验证。

本次同时修正公共Schema原先遗漏observed-http-3的枚举值；不是仅修改manifest版本号。采集证据行为包更新为1.7.0，说明历史标记和默认查询语义；上传格式/插件字段不变。

## 验证

- backend workspace102项通过、8项独立环境测试默认忽略；显式processing与capture_evidence隔离集成测试通过。
- 以本次更新前完整数据库/文件卷恢复隔离实例，验证刷新、重试、来源不变、历史事实读取、默认页过滤、39接口详情和新快照4,290条有效事实。隔离快照测试没有启动worker，模型调用0。
- 实际本地服务重处理结果processed=400、remaining=0；随后通过5173/api回读证据、历史ID、接口详情和原始观测。
- fmt、Clippy及合同生成一致性检查通过。前端150项测试通过。
- 前端完整typecheck仍有此前已存在的product-docs/markdown.ts第35、36行string|number问题；同步前后诊断一致。本次没有声称完整前端生产构建通过，当前Vite开发服务和代理回读正常。
- 首轮隔离HTTP验证曾因测试模型配置少了key文件失败，补齐了仅用于冻结快照的测试配置；之后环境过滤断言误用了项目总数，改为环境内计数。两次均为隔离验证脚本问题，正式数据尚未变更，最终重跑通过。

## 备份与回退边界

更新前备份：backups/before-observation-update-20260917；真正停写切换前备份：backups/before-observation-rollout-20260917。原有recording-baseline-20260916与recording-followup-20260916-passive哈希仍匹配。

旧镜像stage3-20260916保留。若回退，不能只换旧镜像（旧查询不识别superseded）；必须停写并结合匹配的数据库/文件卷备份恢复。更新后若已有新录制，应先备份并处理增量，不能直接覆盖丢失新采集。

## 范围与未完成主线

本次完成本地更新和存量读取切换，不代表阶段4重构已完成。上一轮规划导航挤占原文和真实候选未完成的问题仍需要单独推进；没有自动开始另一轮模型调用。

运行证据保存在被忽略的experiments/results/observation-update/：isolated-result.json、live-data-update.json、live-verification.json、集成/合同日志与备份日志。包含实际回包的private文件不提交。
