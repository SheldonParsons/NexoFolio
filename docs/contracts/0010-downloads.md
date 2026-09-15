# Header 下载版本接口（v1）

## 请求与权限

`GET /v1/downloads?desktop_channel=internal|public`，省略渠道时默认 `internal`（Gree 内部版）。公开只读，无登录、项目或数据库查询依赖。前端通过现有同源 `/api` 代理调用，例如 `/api/v1/downloads?desktop_channel=internal`；代理需将 `/api` 前缀移除。

仅接受 `desktop_channel`；非法值、重复参数及未知参数（包括 `url`、`refresh`）返回 HTTP 400：

```json
{"error":"invalid_download_query"}
```

无手动刷新绕过参数。正常聚合响应始终 HTTP 200，即使某些或全部上游失败。

## 响应

```json
{
  "schema_version": 1,
  "desktop_channel": "internal",
  "fetcher": {
    "status": "ready",
    "release": {
      "version": "0.1.0",
      "url": "https://asynctest.oss-cn-shenzhen.aliyuncs.com/nexofolio_fetcher/NexoFolio-Fetcher-0.1.0-dev.zip",
      "filename": "NexoFolio-Fetcher-0.1.0-dev.zip",
      "size": 176441,
      "sha256": "47cb3660480b873b8b1c1cfe3546fc5dec667c76722564ad62ecfb1295cddcf3",
      "minimum_chrome_version": "125",
      "channel": "development",
      "installation": "load-unpacked"
    }
  },
  "desktop": [
    {"platform": "windows", "status": "unpublished", "release": null},
    {"platform": "mac-arm64", "status": "unpublished", "release": null},
    {"platform": "mac-x64", "status": "error", "release": null}
  ]
}
```

此示例仅说明结构，状态与版本以请求结果为准。

- `desktop` 固定顺序：`windows`、`mac-arm64`、`mac-x64`，分别是 Windows x64、macOS Apple Silicon、macOS Intel。
- 桌面端 `release` 对象字段：`version: string`、`url: string`、`filename: string`、`size: number`（字节，正安全整数）。
- Fetcher 额外字段：`sha256: string`（64 位小写十六进制）、`minimum_chrome_version: string`、`channel: "development"`、`installation: "load-unpacked"`。前端需明确提示开发版、解压后手动加载。
- `ready` 表示上游清单已成功获取并校验；`release` 必定非空。不会下载或校验整个安装包；`sha256`/大小来自清单。
- `unpublished` 表示上游清单返回 404，或有效桌面清单的 `files` 没有所需后缀安装包；`release` 为 `null`。
- `error` 表示超时、网络失败、非 200/404 状态、重定向、超大/无效清单、不安全文件名或无效目录配置；`release` 为 `null`。403 不会误报为未发布。
- 各上游独立；一个平台失败不影响其他平台。服务不在响应或日志输出上游错误正文、请求凭据或配置 URL 值。

## 上游与下载地址

| 环境变量 | 默认发行目录 |
| --- | --- |
| `NEXOFOLIO_FETCHER_RELEASE_DIR` | `https://asynctest.oss-cn-shenzhen.aliyuncs.com/nexofolio_fetcher/` |
| `NEXOFOLIO_DESKTOP_INTERNAL_RELEASE_DIR` | `https://asynctest.oss-cn-shenzhen.aliyuncs.com/core/updates/internal/` |
| `NEXOFOLIO_DESKTOP_PUBLIC_RELEASE_DIR` | `https://asynctest.oss-cn-shenzhen.aliyuncs.com/core/updates/public/` |

空值使用默认值。只接受 HTTPS 目录，禁止用户名、密码、查询串与片段。缺少末尾 `/` 自动补齐。目录在路由构造时读取，更改需重新启动 API。无效目录只令对应发行源返回 `error`，不影响其他业务路由；日志仅记录变量名。

Fetcher 获取 `latest.json`，要求 schemaVersion=1、product=nexofolio-fetcher、channel=development、installation=load-unpacked。严格验证文件名等于 `NexoFolio-Fetcher-{version}-dev.zip`，忽略清单中可能过期的 `artifact.url`，使用配置目录和文件名生成 URL。

每个桌面渠道分别读取：

| 平台 | 清单路径 | 选择安装包 |
| --- | --- | --- |
| windows | `win/x64/latest.yml` | `files` 中第一个 `.exe` |
| mac-arm64 | `mac/arm64/latest-mac.yml` | `files` 中第一个 `.dmg` |
| mac-x64 | `mac/x64/latest-mac.yml` | `files` 中第一个 `.dmg` |

不使用 YAML 顶层 `path`，不选择 Mac 自动更新 ZIP。即使两个渠道文件名相同，下载 URL 仍使用各自渠道和架构目录。版本每次缓存过期动态读取，不固定为示例版本。

安装包必须以单一文件名表示；拒绝完整 URL、路径、反斜杠、百分号编码、查询串、片段及控制字符，避免越出配置目录。空格由 URL 编码处理。浏览器通过普通 HTTPS 下载链接直下 OSS；后端仅读取清单，不转发二进制文件。

## 资源边界与缓存

- 每份清单最多 64 KiB；检查 Content-Length，并对分块响应逐块累计限制。
- 每个请求连接超时 2 秒，总获取超时 5 秒（含读完正文）。不跟随重定向，不自动重试失败状态。
- YAML 使用受预算限制的 `serde-saphyr` 解析：最多一个文档、16 层、2048 个节点、4096 个事件；拒绝重复键、锚点、别名、合并键和未知显式标签。JSON 使用有递归限制的标准解析。
- 固定七个缓存槽：Fetcher 一份，两个渠道各三平台。不按用户输入新增缓存键。
- 每份结果（包括未发布和失败）在完成后缓存 300 秒；过期后下一请求触发获取。没有过期旧版本回退。
- 相同上游请求去重；不同上游并行获取。浏览器断开不取消已开始的共享获取任务。两个渠道共用 Fetcher 缓存。
- 缓存为单进程内存缓存；多副本各自最多每五分钟请求七份清单。API 响应设置 `Cache-Control: no-store`，避免浏览器缓存叠加后端五分钟 TTL。

## 验证与部署

```sh
cargo test -p nexofolio-backend --lib http::downloads --locked
cargo test -p nexofolio-backend --test architecture --locked
```

无需数据库的隔离联调服务（仅监听本机，不启动共享 API）：

```sh
cargo run -p nexofolio-backend --example downloads_preview --locked -- 18981
curl 'http://127.0.0.1:18981/v1/downloads?desktop_channel=internal'
curl 'http://127.0.0.1:18981/v1/downloads?desktop_channel=public'
```

部署示例已在 `deploy/.env.example` 和 `deploy/compose.yaml` 提供三个公开目录变量。新代码需由服务负责人协调构建和加载；仅修改源码不会更新已有进程。此功能无需数据库迁移。
