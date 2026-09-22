# NexoFolio 后端

当前处于重新开发的基础阶段。2026-09-22已移除旧采集、接口知识、证据、目录重构及发布流程，保留登录、项目权限、环境管理和运行基础。新模块逐个讨论、实现与验收。

[模块边界与重新开发路线](docs/architecture/0001-restart.md)

## 项目结构

- `apps/backend`：API、空闲worker、管理CLI，HTTP/MCP传输与装配。
- `crates/common`：最小技术公共类型。
- `modules/access/contracts`：身份、项目及环境的公共合同。
- `modules/access/core`：登录与同步流程。
- `modules/access/adapter`：PostgreSQL、禅道及加密适配，拥有自己的迁移。
- `tests/architecture`：阻止禁止依赖，包含故意违规的失败用例。

接收、知识、重构模块尚未创建。旧实现和文档可从Git提交`f2d4ae3`查阅，不在新代码里保留第二套运行路径。

## 验证

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
# 仅使用独立的测试库：
TEST_DATABASE_URL=postgres://user:password@localhost/test_db cargo test -p nexofolio-backend --test postgres --test access_flow --locked -- --ignored
```

## 运行

Rust工具链与依赖由rust-toolchain.toml、Cargo.lock固定。复制`.env.example`后填入本地配置；程序从进程环境读取配置，不自动载入.env。

```bash
cargo run -p nexofolio-backend --bin nexofolio-admin -- check-config
cargo run -p nexofolio-backend --bin nexofolio-admin -- migrate
cargo run -p nexofolio-backend --bin nexofolio-api
cargo run -p nexofolio-backend --bin nexofolio-worker
```

需要DATABASE_URL。禅道登录还需要同时设置NEXOFOLIO_ZENTAO_BASE_URL和NEXOFOLIO_SESSION_KEY。普通登录仍验证禅道后创建/复用内部会话，成功登录同步项目；超级密码只登录已有用户，不增加权限。环境管理沿用项目权限。

有效入口：`/health/live`、`/health/ready`、`/v1/auth/login`、`/v1/auth/me`、项目及环境管理、下载接口，以及默认拒绝访问且无业务工具的`/mcp`。旧采集/接口文档/重构/发布路由已经从源码移除，返回404。

## Linux容器

```bash
cp deploy/.env.example deploy/.env
# 填写配置后，使用全新的隔离Compose项目检查该版本：
docker compose -p nexofolio-restart --env-file deploy/.env -f deploy/compose.yaml build api
docker compose -p nexofolio-restart --env-file deploy/.env -f deploy/compose.yaml run --rm migrate
docker compose -p nexofolio-restart --env-file deploy/.env -f deploy/compose.yaml up -d api worker
```

本轮没有替换已有运行容器，也没有修改现有数据库和录制数据。不要对原项目执行`down -v`。新建库只建访问模块的表；迁移规则见[migrations说明](migrations/README.md)。
