# 后端工程骨架验证记录

日期：2026-09-14。范围为用户确认的“仅工程骨架”。

## 已实现

- 9 个 workspace package：8 个业务/支撑 library 和 1 个应用包。
- API、worker、admin 三个二进制入口；配置、日志、关闭信号、健康检查。
- rmcp Streamable HTTP 挂载、专用凭证验证 port 和请求身份上下文；默认拒绝所有 Token，未注册业务工具。
- SQLx PostgreSQL 惰性连接、受限健康探测、独立空迁移框架。
- 认证、项目源、权限、采集、知识、触发、重构、事务与任务执行契约。
- 未配置适配器明确失败；隔离测试身份不进入运行时默认装配。
- 依赖白名单检查、错误与生命周期测试、Linux 容器交付和验收脚本。

## 已执行的检查

| 检查 | 结果 |
| --- | --- |
| `cargo check --workspace` | 通过 |
| `cargo fmt --all --check` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | 17 个测试通过；1 个真实数据库测试默认 ignored |
| 隔离 `TEST_DATABASE_URL` 下显式运行 ignored 测试 | 1 个通过：连接、健康探测、重复空迁移 |
| Linux 多阶段镜像构建 | 通过，linux/arm64 |
| `tests/integration/compose_smoke.py` | 通过，测试容器和卷已清理 |

本轮执行了与 CI 相同的 Rust 检查；GitHub Actions 文件已提供，未声称远端 CI 已运行。

## 关键证据

- 未配置禅道、应急认证、用户目录、权限和 Token 验证均返回 NotConfigured，未返回伪造身份或空权限成功结果。
- HTTP MCP 网络测试使用仅测试可用的身份，完成 2025-11-25 协议初始化和空工具列表读取。
- 默认生产装配对 MCP GET/POST/DELETE 拒绝访问；未提供、错误和非 Bearer 凭证均被拒绝。
- 已验证的主体和服务端请求 ID 可以传递到后续处理层；日志与错误不泄露测试凭据。
- 未实现的登录、项目、采集路由返回 404。
- 触发实现可替换，单纯检查触发条件不会调用重构器。
- 架构测试包含 intake→rebuild、triggers→SQLx、application→infrastructure 等禁止依赖反例。
- API 与 worker 子进程收到 SIGTERM 后正常退出。

## Linux 容器验证

使用临时凭据、随机 Compose 项目名与动态主机端口，独立创建 PostgreSQL 17、API 和 worker。
验证项：

1. 真 PostgreSQL 连接及重复迁移。
2. 容器管理 CLI 的配置检查与迁移命令。
3. public schema 仅出现 SQLx 的 `_sqlx_migrations` 表，没有业务表。
4. 数据库运行时，liveness/readiness 均为 200；未授权 MCP 为 401。
5. 停止数据库后，liveness 保持 200，readiness 为 503；恢复数据库后 readiness 返回 200。
6. API/worker 停止时退出码为 0，再启动后恢复就绪。

验证镜像：`nexofolio-backend:foundation`，镜像 ID：

```text
sha256:acaf7d91bfaf22d6a54b341e27be54c7da2ef1aa7a1a72a17ea0abb667b28679
```

实际二进制依赖 Linux 动态加载器、libc、libm 和 libgcc_s；不是完全静态产物。
当前验收平台为 ARM64，未实际验证 AMD64 镜像。

本机 Docker Hub 镜像代理拉取停滞，验证改用公开 ECR 镜像源，未修改全局 Docker 配置：

```bash
docker build \
  --build-arg RUST_IMAGE=public.ecr.aws/docker/library/rust:1.94.0-bookworm \
  --build-arg RUNTIME_IMAGE=public.ecr.aws/docker/library/debian:bookworm-slim \
  -f deploy/Dockerfile -t nexofolio-backend:foundation .

POSTGRES_IMAGE=public.ecr.aws/docker/library/postgres:17-bookworm \
  python3 tests/integration/compose_smoke.py
```

## 未实现的业务

禅道认证与权限适配、超级密码登录逻辑、项目同步、MCP Token 签发、七个业务工具、采集指纹/去重算法、目录策略、持久任务消费、迁移与导出。
没有采集吞吐量、模型效果、召回改善或已部署线上服务的结论。
