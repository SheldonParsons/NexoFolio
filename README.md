# NexoFolio 后端

面向 agent 的接口知识服务。已实现模块化 Rust 工程骨架，以及禅道登录、内部 Token 复用、登录后项目同步和项目访问控制。

**当前业务：**普通禅道登录或已有用户超级密码登录；3 个自然月的内部 Token 原样复用；登录后分页同步项目；无权限项目列表可见、详情拒绝。

**后续业务：**专用 MCP Token 签发、独立同步接口、采集去重、目录重构、七个业务工具、裁决、迁移导出。
未配置的后续能力仍返回 `NotConfigured`。`/mcp` 仍拒绝全部凭证；内部登录 Token 不作为 MCP Token。

## 技术栈与准备条件

- Rust 1.94.0、Cargo workspace；首次构建由 rustup 安装仓库指定工具链。
- Tokio + Axum + Tower 提供 HTTP，rmcp 提供 Streamable HTTP。
- PostgreSQL 17 + SQLx 管理持久化和迁移。
- Serde/Schemars 定义数据契约；AES-256-GCM 加密内部 Token，Argon2id 验证可选超级密码。
- Docker Engine/Desktop 和 Compose v2 用于容器运行；Python 3 仅用于容器验收脚本。

所有命令在仓库根目录执行。现有业务和待实现业务的边界以本文及接口文档为准，设计文档不代表全部功能已经完成。

## 工程入口

| 位置 | 职责 |
| --- | --- |
| `apps/backend` | API、worker、管理 CLI；HTTP/MCP 适配及装配 |
| `crates/contracts` | 稳定 ID、版本、事件、错误和保密字段 |
| `crates/access` | 认证、已有用户、禅道项目和权限、专用 MCP Token 契约 |
| `crates/knowledge` | 接口修订、证据、待裁决、目录与条件发布契约 |
| `crates/intake` | 采集输入、校验、身份、结构指纹与去重契约 |
| `crates/triggers` | 只输出触发判断，不执行重构 |
| `crates/rebuild` | 只输出候选和评估，不发布或写数据库 |
| `crates/application` | 跨组件编排、事务、任务派发和执行归属契约 |
| `crates/infrastructure` | SQLx PostgreSQL、禅道 REST v1、内部 Token 加密存储；后续能力明确返回未配置 |

Cargo workspace 是组件管理边界，不是微服务拆分。核心业务包只依赖 contracts；应用层依赖业务契约；基础设施实现契约；入口统一装配。
依赖白名单和禁止边的反例在 `tests/architecture/dependencies.rs` 中检查。

## 本地构建与运行

工具链由 `rust-toolchain.toml` 固定为 Rust 1.94.0，依赖由 Cargo.lock 锁定。
配置从进程环境读取，程序不自动加载 `.env`。根据 `.env.example` 设置环境，真实凭据不要入库。

先准备可连接的 PostgreSQL，再创建本地配置（文件已被 Git 忽略）：

```bash
cp .env.example .env
# 编辑 .env：填写数据库连接、禅道 REST v1 地址和会话加密密钥。
# 只加载自己维护的可信配置文件；含 $ 的值使用单引号包裹。
set -a
. ./.env
set +a
```

`NEXOFOLIO_ZENTAO_BASE_URL` 示例为 `https://your-zentao.example/api.php/v1`。
使用 `openssl rand -hex 32` 生成 `NEXOFOLIO_SESSION_KEY` 并保存到本地配置；每次重启复用原密钥，不能重复生成。
不启用超级密码时，将 `NEXOFOLIO_EMERGENCY_PASSWORD_HASH` 留空。

```bash
cargo build --workspace --locked
cargo run -p nexofolio-backend --bin nexofolio-admin -- check-config
cargo run -p nexofolio-backend --bin nexofolio-admin -- migrate
cargo run -p nexofolio-backend --bin nexofolio-api
```

以上运行命令要求设置 `DATABASE_URL`。`check-config` 检查配置与 URL 格式，不验证数据库连通性。
启动另一个终端可运行 `cargo run -p nexofolio-backend --bin nexofolio-worker`。
worker 当前待机，不领取任务；SIGINT/SIGTERM 会正常关闭 API 和 worker。

| 地址 | 当前行为 |
| --- | --- |
| `GET /health/live` | 进程存活返回 200 |
| `GET /health/ready` | PostgreSQL `SELECT 1` 成功返回 200，不可用/超时返回 503 |
| `POST /v1/auth/login` | 禅道认证，创建或复用内部 Token，并同步项目 |
| `GET /v1/auth/me` | 使用内部 Bearer Token 读取本地用户 |
| `GET /v1/projects` | 分页展示已同步项目及当前用户可访问状态 |
| `GET /v1/projects/{project_id}` | 有权限返回项目；无权限返回 403 |
| `/mcp` | 挂载 rmcp Streamable HTTP；默认凭证验证失败，返回 401 |

登录/项目的完整配置和协议见 [登录与项目 API](docs/contracts/0002-login-and-projects.md)。同时设置 NEXOFOLIO_ZENTAO_BASE_URL 与 NEXOFOLIO_SESSION_KEY 并执行迁移后启用；超级密码使用可选 Argon2id 哈希配置。

数据库使用惰性连接，数据库宕机不妨碍 liveness。日志不输出 URL 查询串、数据库 URL、凭证或请求正文。
授权成功时的 MCP 请求上下文通过扩展传递，工具列表为空。未配置登录组件时登录/项目路由返回 404；配置后提供 /v1/auth/login、/v1/auth/me、/v1/projects 和 /v1/projects/{id}。采集路由仍返回 404。

## 数据库与 Linux 容器

编辑 `deploy/.env.example` 的副本 `deploy/.env`，将占位密码替换并同步写入数据库 URL。

```bash
cp deploy/.env.example deploy/.env
# 编辑 deploy/.env 后再执行下面的构建和启动命令。
```

容器配置中的 `DATABASE_URL` 使用主机名 `db`；宿主机运行 Rust 时使用 `127.0.0.1` 和映射端口。
Compose 会优先使用当前 shell 已导出的同名变量，因此本地 Rust 和 Docker 配置建议在不同终端加载，避免宿主机连接地址覆盖容器地址。

```bash
docker compose --env-file deploy/.env -f deploy/compose.yaml build api
docker compose --env-file deploy/.env -f deploy/compose.yaml up -d db
docker compose --env-file deploy/.env -f deploy/compose.yaml run --rm migrate
docker compose --env-file deploy/.env -f deploy/compose.yaml up -d api worker
```

默认主机端口：API 18080、PostgreSQL 15432，均只绑定本机。外部接入时应配置 HTTPS 入口和准确的 MCP Host/Origin 列表。
API 与 worker 使用同一镜像，运行用户为非 root。数据库使用独立持久卷。
显式迁移命令创建登录与项目数据表及 `_sqlx_migrations` 跟踪表。
应用启动不会自动迁移。日常停止使用 `docker compose ... down`，保留数据库卷；不要对需要保留的数据使用 `--volumes`。

Dockerfile 支持 `RUST_IMAGE`、`RUNTIME_IMAGE` 构建参数，Compose 支持 `POSTGRES_IMAGE`，用于选择可达的镜像源或固定镜像摘要。
默认 Rust 为 1.94.0/bookworm，运行镜像为 Debian bookworm-slim。实际产物仍需核实平台和运行库，不假设是完全静态二进制。

查看状态与日志：

```bash
docker compose --env-file deploy/.env -f deploy/compose.yaml ps
docker compose --env-file deploy/.env -f deploy/compose.yaml logs --tail=100 api worker
```

升级时先备份数据库，在检查迁移兼容性后重新构建镜像、执行迁移，再启动服务；回退程序版本不等于自动回退数据库结构。

## 验证

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

常规 workspace 测试不依赖真实禅道、模型或 PostgreSQL；HTTP MCP 测试使用随机本机端口和仅限测试的身份。两个需要真实数据库的测试默认忽略，并在 CI 中单独执行。
真实数据库测试默认标记 ignored，必须显式提供**隔离的** `TEST_DATABASE_URL` 后执行：

```bash
cargo test -p nexofolio-backend --test postgres --test access_flow --locked -- --ignored
```

镜像构建完成后，可运行完整隔离容器验收：

```bash
python3 tests/integration/compose_smoke.py
```

该脚本创建随机项目名、临时凭据和数据卷，验证真实 PostgreSQL、迁移、健康检查、MCP 拒绝访问、数据库断连恢复及进程停止/重启，最后清理自己创建的容器和卷。
CI 运行 lint、单测/架构/HTTP 测试、真实 PostgreSQL 测试、镜像构建和隔离 Compose 验收。

## 后续接入约定

- 禅道 REST v1 已按实测响应实现；只同步项目可见范围，写入/裁决等动作权限仍未推测或授予。
- 超级密码仅用于登录任意已有用户，不创建用户，不增加原用户项目权限。
- MCP 使用独立签发的 Token，目前没有 MCP Token 签发能力；登录返回的内部 Token 可用于本平台的用户与项目 HTTP API。
- 项目只同步；接口 ID 不绑定项目或目录，预留后续迁移和导出。
- 前端位于独立的 NexoFolio-Front 仓库。

详细设计见 [模块化架构](docs/architecture/0002-modular-backend.md)、[MCP 契约](docs/contracts/0001-mcp-access-and-tools.md)、[路线图](docs/plans/0001-backend-foundation.md)。这些文档包含未来业务，不能把全部路线图视为本轮已实现。

历史骨架结果见 [工程骨架验证记录](docs/verification/2026-09-14-foundation.md)；当前业务验收见 [登录与项目验证记录](docs/verification/2026-09-14-login-projects.md)。

本次业务接口及验证范围见 [登录与项目 API](docs/contracts/0002-login-and-projects.md)。

## 仓库文件约定

- 提交 `Cargo.lock`、SQL 迁移、配置模板、测试代码和脱敏验证记录。
- 不提交 `.env`、私钥、运行数据、数据库备份、日志或 `target/` 编译产物。
- 已发布的 SQL 迁移保持不可变，后续变更新增迁移文件。
- 后端代码位于本仓库；网站前端和 Chrome 插件在各自仓库维护。
