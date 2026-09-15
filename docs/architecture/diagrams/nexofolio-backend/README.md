# NexoFolio 后端架构图

这是基于 **2026-09-15 本地后端工作区**整理的逻辑架构图，包含尚未提交的代码。它描述源码中的结构和配置条件，不代表当前运行服务或线上部署已通过验收。范围仅限 NexoFolio 后端。

## 文件与查看

- `backend.html`：独立交互图，浏览器打开即可；支持缩放、搜索、主题切换和导出。
- `backend.architecture.json`：持续维护的源文件。组件、关系、文案和布局都在这里修改。
- `update.mjs`：校验、生成、预览和浏览器检查入口，要求 Node.js 18+ 与已安装的 Archify。
- `sources.json`：本轮源码依据及文件哈希，包含工作区状态；不是已提交版本的来源证明。
- `delivery.receipt.json`：图源和 HTML 的 SHA-256、字节数及 9 项检查结果。
- `backend.visual-check.json`、`backend.visual-check.html`、PNG：自动化浏览器检查与截图。
- `review.md`：本轮图像复核范围与交付结果。

不需要启动 Rust 服务、数据库或模型服务就能修改和查看这张图。

## 持续修改

以后可以直接告诉 Codex：

> 使用 Archify 更新 docs/architecture/diagrams/nexofolio-backend/backend.architecture.json。先核实当前后端代码，再展开采集处理链路，保留已有组件 ID；重新生成 HTML，并检查浏览器显示。同步修改本目录的代码依据和复核记录。

也可以自己编辑 JSON：

1. `components`：组件名称、说明、位置 `pos`、尺寸 `size`。
2. `connections`：方向 `from` / `to`、连线文案 `label`。保持稳定的组件与关系 ID，便于版本比较。
3. `cards`：模块边界、运行条件与补充说明。
4. 保留 `meta.quality_profile: showcase`；根据真实错误调整布局，不删除有业务意义的关系来通过检查。

在仓库根目录执行：

```bash
node docs/architecture/diagrams/nexofolio-backend/update.mjs
node docs/architecture/diagrams/nexofolio-backend/update.mjs visual-check
```

第一条先校验，再生成 HTML；失败会返回非零状态，保留上一份成功 HTML。第二条重新生成四种桌面尺寸的自动化浏览器证据。若只检查 JSON，使用 `validate`。

希望边改边看时，手动开启预览：

```bash
node docs/architecture/diagrams/nexofolio-backend/update.mjs preview
```

预览监视 JSON；保存并通过校验后刷新，错误时保留上一版，按 Ctrl-C 结束。结束后再执行生成和浏览器检查，以更新正式回执。

**源代码变化不会自动改写图中的业务关系。** 需要重新分析代码并修改 JSON；预览只自动渲染 JSON 的变化。Archify 位于用户技能目录，其他机器可用 `ARCHIFY_HOME` 指定安装位置。

版本历史保留图源 JSON、脚本和说明。HTML、视觉检查回执及截图是可重建的本地产物，已通过 .gitignore 排除，不进入主代码提交；执行 update.mjs 后仍可本地查看。比较两版时，可用 Archify 的 `compare architecture <base.json> <head.json> <delta.html> --json`。本次未执行 Git 提交。

## 如何理解这张图

主线是 **HTTP API → 应用编排 → 注入的基础设施实现 → PostgreSQL**。箭头表示概括的逻辑调用关系，不是逐个函数的调用图，也不是 Cargo 包依赖图。

- `apps/backend` 统一装配，API、worker、admin 为不同运行入口，共用业务包。
- `application` 依赖业务端口；**没有对 infrastructure 的 Cargo 依赖**。图中该箭头表示运行时调用注入的实现。
- 当前部分只读 HTTP 路由和 evidence worker 直接使用已装配的存储接口或实现；主线不表示每个请求都必须经过一个应用服务。
- `access / intake / knowledge / evidence / triggers / rebuild` 合并为领域节点，以保持概览可读。它们依赖共享 contracts，不代表领域组件相互串行调用。
- `evidence` 做机械提取，不调用模型；`triggers` 定义判断契约，不表示已有自动维护调度；`rebuild` 定义候选及评估，不拥有发布权。
- PostgreSQL 同时保存业务状态与持久化任务，worker 通过数据库租约领取任务。代码没有引入独立消息队列或 Redis。
- 模型服务在配置后用于目录候选和维护；统一知识维护任务由用户显式创建，发布/回退另行执行。
- 文件卷由 API 和 worker 共享；v3 原文和资产走文件存储，索引与状态进入 PostgreSQL。v3 采集默认关闭。
- `/mcp` 已挂载传输层，但生产装配注入 `Unconfigured` 验证器；当前拒绝凭证，没有已注册的知识工具。

## 本轮代码依据

| 图中内容 | 已检查的后端路径 |
| --- | --- |
| 入口与依赖装配 | `apps/backend/src/bin/api.rs`、`worker.rs`、`admin.rs`；`apps/backend/src/wiring/access.rs`、`catalog.rs` |
| HTTP 功能与 MCP 状态 | `apps/backend/src/http/mod.rs`、`maintenance.rs`、`ingestion.rs`、`capture.rs`；`apps/backend/src/mcp/mod.rs`；`crates/infrastructure/src/unconfigured.rs` |
| 后台轮询与配置条件 | `apps/backend/src/wiring/lifecycle.rs`、`apps/backend/src/bin/worker.rs` |
| 应用与领域边界 | `crates/application/src/lib.rs`；`crates/evidence/src/lib.rs`、`crates/triggers/src/lib.rs`、`crates/rebuild/src/lib.rs` |
| 编译依赖方向 | `Cargo.toml`、`crates/application/Cargo.toml`、`crates/infrastructure/Cargo.toml`、`tests/architecture/dependencies.rs` |
| pending、事务与租约 | `crates/infrastructure/src/admission.rs`、`crates/infrastructure/src/documents.rs` |
| PostgreSQL、证据共享卷 | `deploy/compose.yaml`、`apps/backend/src/wiring/access.rs`、`apps/backend/src/wiring/catalog.rs` |

`sources.json` 保存以上文件本轮哈希。后续修改图时，应重新核实受影响代码并更新依据，不要把旧哈希当作当前源码验证结果。由于本轮包含未提交代码，未使用 Archify 的 Git revision 来源跳转，以免错误指向旧提交。

本图描述的是生成时的逻辑架构。后端实现文件已按审计建议拆分；以当前源码与优化记录为准，图源未自动推断新文件关系。
