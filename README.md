# NexoFolio 后端

面向 agent 的接口知识服务。已实现模块化 Rust 工程骨架，以及禅道登录、内部 Token 复用、登录后项目同步和项目访问控制。

**当前业务：**普通禅道登录或已有用户超级密码登录；3 个自然月的内部 Token 原样复用；登录后分页同步项目；无权限项目列表可见、详情拒绝。

**采集与环境：**项目环境列表、创建和改名；多类型信封的批量接收（首轮 HTTP）、当前环境结构去重和可靠 pending 交接。

**接口知识维护：**正式目录及固定待分类、目录候选/发布/回退；capture v3持续证据入口、结构与证据分流、关系/枚举线索；显式触发的全量分段审阅、组合知识候选和目录/语义统一发布回退。增强采集通过配置分阶段启用，真实Chrome录制需要用户验收。

**后续业务：**专用 MCP Token 签发、独立同步接口、接口结构裁决和物理合并、七个 MCP 业务工具、迁移导出。
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
| `crates/evidence` | 参数值、页面线索、枚举与关系的机械提取及证据存储接口；不调用模型 |
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
业务配置启用时worker消费pending观测并建档；未配置的骨架模式仍待机。SIGINT/SIGTERM正常关闭进程，未完成任务按数据库租约恢复。

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
授权成功时的 MCP 请求上下文通过扩展传递，工具列表为空。未配置登录组件时登录/项目路由返回 404；配置后提供 /v1/auth/login、/v1/auth/me、/v1/projects 和 /v1/projects/{id}。同配置还启用 /v1/ingestion/capabilities、/v1/ingestion/batches 和项目环境管理。

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

常规 workspace 测试不依赖真实禅道、模型或 PostgreSQL；HTTP MCP 测试使用随机本机端口和仅限测试的身份。需要真实数据库的集成测试默认忽略，并在 CI 中单独执行。
真实数据库测试默认标记 ignored，必须显式提供**隔离的** `TEST_DATABASE_URL` 后执行：

```bash
cargo test -p nexofolio-backend --test postgres --test access_flow --test ingestion --test processing --test catalog_preview --test capture_evidence --locked -- --ignored
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

- 提交 `Cargo.lock`、SQL 迁移、配置模板、合成测试和不含真实凭据的验证记录。业务原始观测不脱敏。
- 不提交 `.env`、私钥、运行数据、数据库备份、日志或 `target/` 编译产物。
- 已发布的 SQL 迁移保持不可变，后续变更新增迁移文件。
- 后端代码位于本仓库；网站前端和 Chrome 插件在各自仓库维护。

采集协议与环境管理见 [批量采集与环境 API](docs/contracts/0004-ingestion-and-environments.md)。权威契约校验：`python3 scripts/ingestion_contract.py --check`。

本轮执行记录：[采集与环境验证](docs/verification/2026-09-14-ingestion-environments.md)。

采集去重范围为项目＋环境，插件先选domain/path范围、再选项目和环境；v2上传无接口服务标识，旧v1队列仍可重试。

已实现观测建档与项目/环境下的接口列表、详情和原始样例查询：[观测文档 API](docs/contracts/0005-observed-interface-documents.md)。新文档统一待分类，结构变化仅记差异不覆盖。契约检查：`python3 scripts/document_contract.py --check`。


构建网络受限时可用 `python3 scripts/build_offline_image.py`：依赖按Cargo.lock准备在忽略的target目录，Linux编译阶段使用离线源。可传 --rust-image、--runtime-image 指定可用镜像源。
本轮处理结果见 [观测建档验证](docs/verification/2026-09-14-observed-documents.md)。

### 手动目录候选实验

已提供创建项目接口快照、调用模型生成候选、结构检查与查看结果的管理命令。当前接口与目录状态不自动改变。配置、任务重试和模块边界见 [目录候选说明](docs/contracts/0006-catalog-preview.md)。模型未配置时返回明确错误；测试替身不进入生产流程。

候选目录现已提供项目权限控制下的只读列表/详情接口，供前端预览；生成仍通过管理命令，预览不代表发布。

### 正式目录

每个项目已有正式目录及固定“待分类”，新建档接口立即可查。项目有权限的登录用户可显式发布候选、回退已发布版本或回到初始目录，操作受generation与request_id保护；不会自动发布。见 [正式目录说明](docs/contracts/0008-official-catalog.md)。


## 持续证据与统一知识维护

详见 [持续采集和维护 API](docs/contracts/0009-continuous-knowledge-maintenance.md)。采集包在 `contracts/capture`，维护包在 `contracts/maintenance`；Schema、生成类型、行为和manifest一起校验。后端、网站前端、插件各自修改并同步合同，不跨仓库混改。

`NEXOFOLIO_CAPTURE_ENABLED=true`启用v3及文件卷证据；默认关闭，旧v1/v2可继续使用。必须先迁移，再初始化共享卷、启动API和worker，最后升级插件。API与worker共同使用`capture_data`，备份需同时保留PostgreSQL和该卷。

```bash
docker compose --env-file deploy/.env -f deploy/compose.yaml run --rm migrate
docker compose --env-file deploy/.env -f deploy/compose.yaml run --rm storage-init
docker compose --env-file deploy/.env -f deploy/compose.yaml up -d api worker
```

需要模型时额外加载`deploy/compose.catalog-model.yaml`及私有模型环境文件，示例见该文件。worker只执行由用户POST创建的重构任务。密钥只通过只读文件注入，不进入数据库里的模型请求记录。若本机模型文件使用不同UID/GID，API、worker与`storage-init`必须使用同一模型UID/GID配置，初始化后再启动。

默认模型上下文65536、每任务最多256次调用、每次最多240秒超时、失败最多重试2次；任务保留检查点，执行租约5分钟并持续续租。视觉默认关闭，启用后仍须成功读图才计为已读。默认快照容量64MiB，超过容量明确拒绝，不改为只看少数接口。所有语义推断继续显示其性质；只有有项目权限的用户可发布。

重构入口为`POST /v1/projects/{id}/maintenance-runs`，用户不指定全量或局部；查询任务及候选后，通过`POST /maintenance-runs/{run}/publish`一次切换目录和语义。旧目录接口只改变目录，保留当前语义。构建期间新采集继续进入正式接口，快照外的新接口在发布/回退后仍属于固定待分类。

默认每个项目最多积压50000条未完成证据，超限返回`429 CAPTURE_BUSY`及`Retry-After`；原记录重试仍可取原回执。结构重复不跳过证据。临时原文默认24小时后可回收；工作样例、候选和版本引用另行固定。每项事实最多5组工作样例，频次仅代表观测次数，不能当作独立因果证据。

新增检查：

```bash
python3 scripts/capture_contract.py --check
python3 scripts/maintenance_contract.py --check
# 下列基准只使用隔离TEST_DATABASE_URL和合成请求，不代表后台整体处理吞吐量。
NEXOFOLIO_CAPTURE_BENCHMARK=/tmp/capture-benchmark.json cargo test -p nexofolio-backend --test capture_evidence --locked -- --ignored
```

## 审计与维护

- [累计改动逐文件审计](docs/audits/2026-09-15-backend-change-accountability.md)
- [审计问题优化与验证](docs/audits/2026-09-15-optimization.md)

架构图HTML和视觉检查附件可以本地重新生成，不进入业务代码提交；合同生成物仍随版本发布并由CI检查一致性。

- [真实源码精简修正与行数](docs/audits/2026-09-15-source-reduction.md)
