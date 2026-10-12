# 最新版固定下载链接

以下链接不含版本号，可长期放在群公告或帮助文档中。用户直接打开链接，就会跳转到对应的最新安装包，无需登录或先打开下载菜单。

这些入口随本次后端代码发布后生效。

## 可复制到群公告的链接

### NexoFolio Fetcher

- [下载最新版 ZIP（开发版）](https://nexofolio.net/api/v1/downloads/fetcher/latest)

下载后解压，在 Chrome 扩展管理页开启开发者模式，选择“加载已解压的扩展程序”。

### AsyncTest Desktop · 格力内网版

- [Windows x64](https://nexofolio.net/api/v1/downloads/asynctest/internal/windows/latest)
- [macOS Arm64（Apple 芯片）](https://nexofolio.net/api/v1/downloads/asynctest/internal/mac-arm64/latest)
- [macOS x64（Intel 芯片）](https://nexofolio.net/api/v1/downloads/asynctest/internal/mac-x64/latest)

### AsyncTest Desktop · 公网版

- [Windows x64](https://nexofolio.net/api/v1/downloads/asynctest/public/windows/latest)
- [macOS Arm64（Apple 芯片）](https://nexofolio.net/api/v1/downloads/asynctest/public/mac-arm64/latest)
- [macOS x64（Intel 芯片）](https://nexofolio.net/api/v1/downloads/asynctest/public/mac-x64/latest)

## 开发与部署说明

- 后端路由从 `/v1/downloads/…` 开始；公网网关添加 `/api` 前缀。本地 Vite 使用 `http://127.0.0.1:5173/api/v1/downloads/…`，直连本地后端使用 `http://127.0.0.1:18080/v1/downloads/…`。
- 支持 `GET` 和 `HEAD`。成功返回 **307 临时重定向**，`Location` 是实际安装包地址，响应带 `Cache-Control: no-store`。浏览器不会因永久重定向停留在旧版本。
- 复用已有 `/v1/downloads` 元数据接口的版本清单和缓存，每次只读取所选产品、渠道、平台的清单。清单缓存 5 分钟，因此新版发布后最迟约 5 分钟生效；无需修改群公告或重新发布 NexoFolio。
- Fetcher 使用 `latest.json`；Windows 使用 `latest.yml` 中的 EXE；macOS 使用 `latest-mac.yml` 中的 DMG，排除自动更新用的 ZIP。两个 AsyncTest 渠道分别读取各自的清单。
- NexoFolio 只解析清单并跳转，安装包由原文件服务下载。原有元数据接口和前端下载菜单保持兼容。
- 固定入口不接受查询参数或任意下载目标。渠道仅接受 `internal`、`public`；平台仅接受 `windows`、`mac-arm64`、`mac-x64`。

| 状态 | 含义 |
| --- | --- |
| 307 | 已找到安装包，跟随 `Location` 下载 |
| 400 · `invalid_download_query` | 固定入口带了不支持的查询参数 |
| 404 · `download_not_found` | 不支持的渠道或平台 |
| 404 · `download_unpublished` | 清单或对应安装包尚未发布 |
| 503 · `download_unavailable` | 清单不可用、格式异常或读取超时，稍后重试 |

错误响应也禁止缓存，不包含上游诊断内容。某个渠道或平台失败时，不会替换成另一个渠道、平台或已过期版本。

发布目录沿用 `NEXOFOLIO_FETCHER_RELEASE_DIR`、`NEXOFOLIO_DESKTOP_INTERNAL_RELEASE_DIR`、`NEXOFOLIO_DESKTOP_PUBLIC_RELEASE_DIR` 三个配置，无需新增配置。当前默认目录为既有 OSS 发布目录。
