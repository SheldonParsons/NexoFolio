# 批量采集与项目环境 API

状态：后端已实现接收、当前观测结构去重和可靠下游交接。插件由独立任务实现，不以本文件宣称插件运行实例已经升级。

## 契约唯一来源

[机器契约包](../../contracts/ingestion/manifest.json)包括批次/条目、回执、环境管理、能力声明的 JSON Schema、生成的 TypeScript 类型、行为规则与合成样例。
源文件在 contracts/ingestion，生成命令为 `python3 scripts/ingestion_contract.py`；CI 使用 `--check` 检查漂移，对已有基线通过 `--base-ref` 检查未升版本的契约改动。
后端实际接收时使用同一 Schema 校验。仅生成的类型不能替代运行时校验。插件固定复制同一 manifest 和文件摘要，并预编译 Schema 校验器。
当前新上传契约为 v2，移除 service_key；服务端同时接收 v1 原始队列和 v2，新字段以根目录契约为准，legacy/v1 保存旧版。旧队列不能改内容和记录 ID 后冒充新请求。

## 环境

环境属于项目，稳定 UUID 与显示名称分开。名称支持中文，1～64 个 Unicode 字符，不含控制字符或首尾空白，区分大小写，不自行转换或猜测。
同项目名称唯一，不同项目可以同名。domain 是来源，不是环境身份。

| 接口 | 内容 |
| --- | --- |
| GET /v1/projects/{project_id}/environments?page=1&limit=20 | 返回 items:[{id,name}]、page、limit、total；limit 上限 100 |
| POST /v1/projects/{project_id}/environments | JSON {name}，200 返回 {id,name}，同名创建复用已有对象 |
| PATCH /v1/projects/{project_id}/environments/{environment_id} | JSON {name}，200 返回改名后的 {id,name} |

均使用内部 Bearer Token，并要求当前用户有该项目访问权限。没有环境删除接口；普通环境管理不需要裁决。
名称冲突返回 409；项目无权限 403；无效/跨项目环境 ID 返回 404；不合要求的名称返回 400。
改名保留旧名称别名，旧上传队列仍能解析到相同环境；旧名不能被另一个环境占用。
项目列表不泄露无权限项目的环境列表。

## 上传

POST /v1/ingestion/batches，Content-Type: application/json，Authorization: Bearer <内部Token>。

```json
{
  "schema_version": "2",
  "batch_id": "10000000-0000-4000-8000-000000000001",
  "project_id": "20000000-0000-4000-8000-000000000001",
  "environment": {"name": "开发环境"},
  "source": {
    "type": "chrome_extension",
    "instance_id": "30000000-0000-4000-8000-000000000001"
  },
  "records": []
}
```

上例只示意信封，实际 records 必须为 1～50 条。完整可校验样例见[HTTP 批次样例](../../contracts/ingestion/fixtures/http-batch.json)。
环境二选一：`{id:环境UUID}` 或 `{name:环境名称}`。按名称上传时不存在则自动创建；按 ID 不存在时拒绝，不猜测目标。
所有记录无效、在环境解析前已被拒绝时不创建环境，回执 environment=null。
采集绑定流程：先确定范围（domain＋path），再选择项目；选中项目后立即加载该项目的环境列表，选择已有环境或手填新名称后保存绑定。同一范围在同一项目中确定一个环境。切换项目需清空旧环境选择，不能由旧请求回填。没有额外的“接口服务标识”。
多种外部发送系统共用此协议。每条通过 kind/payload_version 分派，当前只支持 http_exchange/1；其他类型明确逐条拒绝。

发送端首次入队时分配 record_id，来源实例 instance_id 持久化，组批时分配 batch_id，建议使用 UUID v4。
批次 ID 用于关联；重试去重以已认证用户＋来源实例＋记录 ID 为依据。换批次不换记录 ID。同 ID 改环境、来源、项目或正文会冲突。
环境名称/ID在后端解析成相同稳定身份，重命名不会使原记录重试失效。

## 批量处理流程

1. 检查内部 Token、本地项目权限、批次结构、条数和大小。
2. 按条目验证 payload Schema 和正文状态；个别不支持的类型/无效记录返回逐条拒绝，其他条目继续。
3. 计算不可变观测摘要，按记录 ID 识别重试；原样重试返回原回执。
4. HTTP 接口身份由方法与字面路径识别，在项目＋环境 ID 范围内比较当前观测结构。
5. 当前结构相同：ignored；首次、变化或不确定：原样写 ingestion_inbox，返回 accepted/FORWARDED。
6. 整批合法条目的事务提交后才确认；数据库失败整批回滚，不虚报接收成功。

当前结构包含 query 参数名称、头名称、媒体类型、请求/响应正文结构和响应状态码；普通 JSON 标量值不参与，数组检查所有元素的不同结构，不因数组长度变化产生新版本。
不猜测动态路径参数，不用全量历史指纹集合判重；A→B→A 第三次仍转交。空数组只表明数组类型，不代表元素结构消失。已知元素结构→空数组且其他结构被覆盖时 ignored，保留较完整的当前依据；空数组→首次出现元素则转交。null、不可读、截断或超出解析预算仍不肯定判重。算法为 http-structure-2，旧头按需从其当前原始观测升级，不改变历史回执。
非 JSON 正文以内容摘要比较，避免把完全不同的文本或图片内容视为同一种结构。完整算法行为见[契约行为规则](../../contracts/ingestion/behavior.md)。
原始采集头与正文不脱敏；指纹抽象只用于比较，不修改保存原文。重复观测可忽略，不意味着永久保存全部重复流量。

## 当前与历史的边界

ingestion_heads 是当前环境最新“已交接观测”的比较投影，不是正式接口版本。ingestion_inbox 是可靠的后续处理入口，状态 pending。
接收模块本身不进行建档；独立后台处理器现已实现初次观测建档和差异记录，成功后将pending/processing改为completed。自动分类、裁决、模型和重构器尚未接入，见0005文档。
未来裁决确认当前版本时需显式更新/失效这一投影；历史接口版本仅用于检索，不继续维护，不作为所有历史均可忽略的集合。

## 回执与限制

成功处理批次返回 HTTP 200：schema_version、batch_id、解析后的 environment:{id,name}、逐条 results。
每条包含 record_index、record_id、status、reason_code、retryable、ingestion_id、replayed。
accepted/ignored 均可让插件结束该记录的上传；accepted 不代表后续知识处理完成。永久拒绝保留可见失败原因；临时故障原 ID 重试。
当前每个新 record_id 都持久保存紧凑回执，包括结构重复项，以便保证改内容重试不被误接收。没有 TTL 自动淘汰，容量/保留策略是后续上线前的必要工作。

GET /v1/ingestion/capabilities 返回支持版本、类型及限制，使用内部 Bearer Token。
当前批次上限 50 条/8 MiB，单条上限 4 MiB，按 JSON UTF-8 字节计数；不支持压缩；单进程同时接收最多16批。
客户端建议20条或4MiB或等待1秒触发发送。大于上限的内容不能偷偷截断，需明确失败/暂停并保留数据。
上传队列与界面20条测试缓冲分开，不得因UI淘汰丢失未确认记录。队列耗尽应暂停新增采集并提示，不能承诺无限存储。

401暂停认证；403处理项目权限但不清Token；429按Retry-After退避；413拆批或保留超限单条失败；503重试原记录。单条具体错误见receipt schema与behavior文档。

## 数据与部署

环境表及别名表、ingestion_inbox/heads/receipts 通过迁移002/003创建；迁移004合并旧服务标识产生的当前判重分区，移除heads的service列，仅在原始观测保留legacy_service_key历史来源。原记录和幂等回执不删除。
API 与 worker 不自动迁移；执行005迁移后，启用业务配置的worker会消费后续入口。
环境、批次和原文操作仅在后端实现；插件数据转换/队列由插件任务独立实现。


## v1 兼容与 v2 升级

- 新插件只发送schema_version=2，不再出现service_key；传入该字段会被v2 Schema拒绝。
- 旧v1批次保留原字段用于核验原幂等摘要，回执回显schema_version=1，但service_key不再区分去重范围。
- 原生v2与旧v1在同项目、同环境内识别同一方法/路径与结构，相同则去重；不同服务标识不能绕过这一判断。
- 迁移004对原current heads按观测接收时间保留最新指向，不删历史inbox和receipts，不需要清空插件或重新登录。
