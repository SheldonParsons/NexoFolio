# 收集合同 v1

`POST /v1/collect/batches` 的公开合同。后端仓库中的这份文件是唯一权威来源；客户端（例如 NexoFolio Fetcher）复制整个目录，并按 `manifest.json` 中的 SHA-256 校验。

| 文件 | 内容 |
|---|---|
| `batch.schema.json` | 请求体 |
| `receipt.schema.json` | HTTP 200 响应体 |
| `error.schema.json` | 整批被拒绝时（HTTP 4xx/5xx）的响应体 |
| `fixtures/<schema>/valid/*.json` | 必须通过校验的示例，数据全部是虚构的 |
| `fixtures/<schema>/invalid/*.json` | 必须校验失败的示例，文件名就是失败原因 |
| `manifest.json` | 以上所有文件的 SHA-256 |

修改任何文件后都要更新 `manifest.json`，否则后端的合同测试会失败（`cargo test -p nexofolio-backend --test contracts`，失败信息会给出新的哈希）。

## 请求

```
POST /v1/collect/batches
Content-Type: application/json
```

- 不需要 `Authorization`。调用方必须在请求体中填写 `platform`（小写字母、数字、`.`、`_`、`-`，最多 64 个字符）。它只用于标记来源、统计和限流，不代表身份。
- 上限：每批 1 到 50 条记录；整个请求体不超过 8 MiB；单条记录序列化后不超过 4 MiB。
- 第一版只有两种 `kind`：`http_exchange`（实际发生的调用）和 `http_declaration`（Swagger/OpenAPI 或手工声明的结构）。每种 kind 当前只有 `version: 1`。

## 幂等

- `batch_id` 由客户端生成一次，重试时原样复用。
- 服务端在 7 天内记住已处理的批次：同一 `batch_id` 带着相同内容重发，直接返回第一次的回执，不重复计数。
- 同一 `batch_id` 带着不同内容重发，返回 409 `BATCH_ID_REUSED`。

## 归属（`target`）

归属由客户端决定，服务端只校验，不猜测。

| 字段 | 规则 |
|---|---|
| `project_id` | 必填，项目必须存在，否则 404 `UNKNOWN_PROJECT` |
| `environment` | `{ "id": … }` 或 `{ "name": … }` 二选一。`id` 必须属于该项目，否则 404 `UNKNOWN_ENVIRONMENT`；`name` 不存在时自动创建。批次里只要有 `http_exchange` 就必须填写；只有 `http_declaration` 时可以省略，表示声明对项目的所有环境有效 |
| `site` | 可选。`{ origin, prefix }` 表示采集时所在的站点入口，只作为环境标注，不参与接口身份 |
| `source_url` | 可选。声明来源的文档地址 |

接口身份 = 项目 + 方法 + 路径模板，不含域名和环境。

## 记录

- `id`：客户端生成，回执里用它和 `index` 定位被拒绝的记录。
- `observed_at`：带时区的 RFC 3339 时间。
- `context`：只有 `http_exchange` 可以带，全部字段可选；服务端第一阶段只保存、不推断。
- `body.state`：
  - `full`：内容完整，必须带 `encoding` 和 `content`。
  - `truncated`：只有前面一部分，必须带 `encoding` 和 `content`，`bytes` 可以写原始长度。
  - `unreadable`：有内容但读不到，原因写在 `note`。
  - `none`：确实没有 body。
- 只有 `full` 的 body 能证明"某个字段不存在"；`truncated` 只能补充结构。
- `http_exchange.response` 可以省略，表示没有采集到响应。
- `http_declaration` 中的 schema 必须已经展开所有 `$ref`。

## 响应

**HTTP 200：回执**

```json
{ "batch_id": "…", "accepted": 1, "rejected": [ { "index": 1, "id": "…", "reason": "RECORD_TOO_LARGE" } ] }
```

- 没有列在 `rejected` 里的记录都已被接收，客户端可以删除本地副本。
- `rejected` 里的记录都是不可重试的错误。

| `reason` | 含义 |
|---|---|
| `INVALID_RECORD` | 记录不符合 schema |
| `UNSUPPORTED_KIND` | 服务端不认识这种 kind |
| `UNSUPPORTED_VERSION` | 服务端不支持这个版本 |
| `RECORD_TOO_LARGE` | 单条超过 4 MiB |

**HTTP 4xx/5xx：整批被拒绝**

```json
{ "error": { "code": "UNKNOWN_PROJECT", "message": "…" } }
```

| HTTP | `code` | 客户端应该 |
|---|---|---|
| 400 | `INVALID_BATCH` | 批次外层（`batch_id`、`platform`、`target`、`records` 数组）不合法。修正后用新的 `batch_id` 重发 |
| 413 | `BATCH_TOO_LARGE` | 拆成更小的批次，用新的 `batch_id` 重发 |
| 404 | `UNKNOWN_PROJECT` | 重新选择项目 |
| 404 | `UNKNOWN_ENVIRONMENT` | 重新选择环境 |
| 409 | `BATCH_ID_REUSED` | 客户端错误：换一个 `batch_id` |
| 429 | `RATE_LIMITED` | 按 `Retry-After` 等待后，用同一个 `batch_id` 重发 |
| 503 | `UNAVAILABLE` | 退避后，用同一个 `batch_id` 重发 |

可重试的情况只会以整批的 429/503 出现，不会逐条出现在回执里。
