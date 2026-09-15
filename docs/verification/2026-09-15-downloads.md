# Header 下载接口验证（2026-09-15）

## 结论

后端 `GET /v1/downloads` 已实现并接入主路由。`desktop_channel` 仅允许 `internal` / `public`，默认 `internal`。公开元数据独立于项目、登录与数据库查询；安装包保持普通 OSS 直链。详细契约见 [0010-downloads](../contracts/0010-downloads.md)。

真实清单验证在本机隔离服务 `127.0.0.1:18981` 完成；没有启动数据库、读取凭据、重启或部署共享服务。API 主路由挂载另有本地测试验证。

| 验证 | 结果 |
| --- | --- |
| 下载专项测试 | 11 / 11 通过 |
| backend 全部 lib 测试（含上述专项） | 16 / 16 通过 |
| 架构依赖策略测试 | 3 / 3 通过 |
| HTTP/MCP 集成测试 | 3 / 3 通过 |
| backend lib/tests/examples Clippy，`-D warnings` | 通过 |
| `cargo fmt --all --check` | 通过 |
| 两渠道真实 OSS 聚合 | HTTP 200；Fetcher + 六个桌面源全部 ready |
| 七个下载地址 HEAD | HTTP 200，Content-Length 均等于清单 size |
| 重复内部渠道请求 | HTTP 200，2 ms，响应不变 |
| 共享 18080 的下载路由探测 | HTTP 404，尚未加载 |

## 真实上游结果

Fetcher：`0.1.0`，最低 Chrome `125`，`development` / `load-unpacked`。下载地址使用 `nexofolio_fetcher` 下划线目录，忽略当前 JSON 内旧连字符目录 URL。

桌面六份清单：`3.3.10`。Mac 选择 DMG；两个渠道的相同文件名保留不同目录，HEAD 大小和 ETag 也不同。

| 渠道 | 平台 | 文件大小（字节） |
| --- | --- | --- |
| internal | windows | 160597550 |
| internal | mac-arm64 | 193724780 |
| internal | mac-x64 | 201287310 |
| public | windows | 160589144 |
| public | mac-arm64 | 193727951 |
| public | mac-x64 | 201296914 |

完整响应、HEAD 元数据与 HTTP 状态记录在 [downloads-http.json](2026-09-15-downloads-http.json)。此次 HEAD 验证没有下载完整安装包，不代表安装或 SHA256 实体校验已完成。

## 专项覆盖

- Fetcher 旧目录 URL 修正，开发版/手动加载信息完整保留。
- 版本、大小、散列、最低浏览器版本和文件名校验。
- Mac 自动更新 ZIP 与可安装 DMG 区分，Windows EXE 选择。
- 渠道隔离和独立失败状态；两个渠道共享 Fetcher 缓存，总共七个固定缓存槽。
- 20 个并发调用只产生一次上游获取；缓存命中、过期重新获取。
- 首个请求取消后，其余请求复用同一次获取。
- 404/无目标包、403、500、302、无效正文、超时和超大清单（含无 Content-Length 分块流）。失败/未发布也缓存。
- 拒绝 URL 代理参数、refresh 参数、未知/重复渠道参数、路径逃逸、重复 YAML 键、锚点/别名、多文档和过深结构。
- 无 access 服务时主路由仍挂载，无数据库访问。

## 修改文件

新增：

- `apps/backend/src/http/downloads.rs`
- `apps/backend/src/http/downloads/manifests.rs`
- `apps/backend/src/http/downloads/tests.rs`
- `apps/backend/examples/downloads_preview.rs`
- `docs/contracts/0010-downloads.md`
- 本验证文档及 JSON 记录

公共文件的协调修改：

- `apps/backend/src/http/mod.rs`：模块声明、公开路由合并。
- `apps/backend/Cargo.toml` / `Cargo.lock`：HTTP 客户端、SemVer、受预算限制的 YAML 解析依赖。
- `deploy/.env.example` / `deploy/compose.yaml`：三个公开发行目录变量。

修改前已向主后端任务确认无编辑冲突，保留现有大量未提交工作。未修改前端，未提交或推送。

## 服务加载与联调边界

隔离服务仅用于前端下载链路联调，不替代共享 API。前端任务已获得 `18981` 地址与契约，并负责浏览器层验收。

共享 `18080` 未重启；主后端任务明确要求当前插件录制验收期间不要加载，以免中断录制。后续由用户/主后端协调重新构建并重载 API。无需数据库迁移。不要将本次源码/隔离服务验证报告为共享服务已部署。

## 前端联调完成与清理

前端任务已完成实际浏览器联调，且本任务已读取其验证文档：
`/Users/sheldon/Documents/GithubProject/NexoFolio-Front/docs/verification/2026-09-15-downloads-backend-integration.md`。

- 临时同源代理 `5187` → 本任务实际路由 `18981` → 真实 OSS。仅页面认证/目录使用合成 fixture。
- 浏览器显示 Fetcher 0.1.0 开发版、内部/公开两个渠道各三平台 3.3.10，链接切换正确。
- 渠道切换与重新检查仅发起同源 `/api/v1/downloads?desktop_channel=...` 请求；浏览器 OSS JSON/YAML 清单请求为 0。
- 前端报告 49 项相关测试、类型检查及生产构建通过。浏览器操作由前端任务执行，证据保存在前端验证文档同级 `downloads-backend-20260915/`。
- 前端完成后，已核对 `downloads_preview 18981` 进程身份并发送 SIGINT 正常关闭，确认 `127.0.0.1:18981` 不再监听。前端临时代理与浏览器亦已由前端任务清理。

共享 18080 未重启，下载接口仍待另行协调加载。
