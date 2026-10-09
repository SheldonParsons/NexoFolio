# NexoFolio 后端 OSS 发布与离线更新

本机入口：`./upload-server.sh`。服务器入口：`bash update-backend.sh`。两个入口及测试均为 Bash，不依赖 Python 或 jq。
当前仅负责后端；前端继续使用独立仓库的发布流程，NGINX 站点配置另行接入。

## 环境与边界

- 本机：Bash 3.2+、Git、能够构建目标 Linux 架构的 Docker、gzip、SHA-256 工具，以及已配置的 ossutil。
- 服务器：Bash、curl、tar、Docker、Compose v2.27+，以及 awk、sort、cmp、od、stat 等系统工具。SHA-256 使用 `sha256sum`，本机 macOS 也可使用自带的 `shasum`。服务器不运行 Cargo、npm、镜像构建或镜像在线拉取。
- 阿里云已确认 `linux/amd64`、Compose v2.27.0、`postgres:17.4`，使用新的专用空库。
- 内网已确认可访问 OSS、禅道、百通模型；架构、Compose 和 PostgreSQL 标签需要在首次发布前核对。
- 构建机可以联网下载编译依赖。服务器部署文件仅从指定 HTTPS OSS 地址获取，不跟随重定向，不使用环境代理。
- `pull_policy: never` 和启动命令的 `--pull never --no-build` 禁止镜像仓库下载。运行期仍需要访问禅道、模型和下载功能的 OSS 地址；这不是防火墙规则，也不代表已完成断外网实测。
- Compose v2.27.0 的 `run` 不支持 `--pull` 参数。单次配置检查和迁移使用服务定义中的 `pull_policy: never`，并在此前核实本地镜像；`up` 继续显式传入 `--pull never --no-build`。部署配置没有 `build` 段。

## 1. 本机构建、上传

在后端仓库执行：

```bash
./upload-server.sh --build-only       # 仅构建，供发布前检查
./upload-server.sh                   # 构建当前工作区并上传 OSS
```

脚本包含当前工作区中符合构建白名单的已跟踪及未提交源文件，不执行 commit、push、reset 或 worktree 操作。
在构建前复制固定源码快照，并对复制前后及快照文件逐项校验；检测到并发编辑就停止。
如果其他会话仍在修复，应等目标版本确定后再运行。清单同时记录源文件哈希、构建输入哈希和 Git HEAD，不能把 HEAD 单独当成部署版本。
真实 `.env`、密钥文件、数据库、日志和本机依赖目录不进入构建上下文；源代码不会作为单独的压缩包上传。

后续确认已构建版本后，可以只上传该版本，不再读取当前工作区：

```bash
./upload-server.sh --publish-release <上一步输出的版本号>
```

默认构建 `linux/amd64`。若内网为 ARM64，使用 `--platform linux/arm64`，并在构建机准备跨架构构建能力。
同架构两地可复用同一发布版本；不同架构有各自的 latest 清单。
`--rebuild` 可强制重新执行 Docker 构建；基础镜像需要升级时，先通过 `RUST_IMAGE` / `RUNTIME_IMAGE` 指定经过确认的版本。

默认沿用现有前端部署的 OSS 配置：

```text
OSS_BUCKET=asynctest
OSS_ENDPOINT=oss-cn-shenzhen.aliyuncs.com
OSS_PATH=linux_files/nexofolio/backend
```

可通过环境变量覆盖。优先使用 `OSS_CONFIG_FILE` 指定的本机配置，其次为已有的
`~/.config/nexofolio/ossutil.config`，最后使用 ossutil 默认配置。脚本不收集或打印 AccessKey。
上传不会调整 Bucket/Object ACL；目标网络必须已经能用 curl 读取对应对象。
OSS 是发布物存储，请为发布前缀使用合适的读写权限。清单与文件都从该可信地址获取，SHA-256 用于完整性校验，不是独立签名。

产物保存在 Git 忽略的 `target/offline-releases/`。脚本根据内容哈希复用镜像与配置文件，
OSS 已有的不可变对象不重复上传，服务器已缓存且校验通过的文件不重复下载。
Rust 源码变化时发布完整的新镜像，不把零散源文件覆盖进正在运行的容器。

OSS 对象布局：

```text
blobs/<SHA-256>.tar.gz                     Docker 镜像归档
blobs/<SHA-256>.txt                        配置、迁移清单、源文件哈希、说明和脚本
releases/<版本号>/manifest.tsv             不可变发布清单
releases/<版本号>/update-backend.sh         此版本的独立服务器脚本
latest-linux-amd64.tsv                     所有文件上传完成后最后更新
latest-linux-arm64.tsv                     仅发布过此架构时存在
```

清单和本地状态使用严格的两列 TSV 格式，拒绝重复键、未知键和不合规的值，绝不 `source` 或 `eval`。
配置文件逐个按哈希下载，不在主机上解压任意归档；镜像归档交给 `docker load`，身份兼容检查仅用 `tar -O` 读取指定元数据到标准输出进行校验。
Docker 的 containerd 存储与传统存储可能分别显示同一镜像的索引摘要和配置摘要。
当镜像 ID 不同，脚本要求完整归档的 SHA-256 和大小正确、保存标签匹配、配置文件内容与摘要匹配、实际架构一致，才接受该配置摘要。
运行容器仍必须使用本机已验证的镜像 ID。保留服务器 `cache/` 中的镜像归档，以便传统 Docker 后续进行身份验证。
同一 checkout 用原子创建的锁目录避免同时发布。多个构建机共同发布时，应串行操作；latest 由最后一次成功上传决定。
正常退出会释放锁。若进程被强制杀死而残留锁，必须确认没有发布进程后再手工删除对应空锁目录，脚本不会自动抢锁。
不自动清理旧镜像、发布包、源码构建缓存或服务器备份。

## 2. 首次把脚本放到服务器

上传成功会输出服务器脚本的精确 OSS 地址及 SHA-256。在服务器执行下面的命令，替换两个占位符：

```bash
mkdir -p /home/nexofolio/backend
chmod 700 /home/nexofolio/backend
cd /home/nexofolio/backend
curl -fSs --noproxy '*' --proto '=https' --max-redirs 0 --output update-backend.sh '<上传脚本输出的服务器脚本 HTTPS 地址>'
printf '%s  %s\n' '<上传脚本输出的 SHA-256>' update-backend.sh | sha256sum -c - &&
  bash update-backend.sh init
```

`init` 从 OSS 读取匹配服务器架构的清单和配置模板，创建权限为 `0600` 的 `.env`，
自动生成数据库密码和会话密钥，不打印它们，不创建容器或数据库。
已有 `.env` 时拒绝覆盖。只在服务器编辑 `.env`，填写禅道地址、百通模型地址、模型名和模型密钥；
不要把文件内容粘贴到聊天或提交到 Git。镜像版本和架构由更新脚本管理，无需每次修改配置。
`.env` 使用一行一个 `KEY=value`，支持裸值或单引号包裹的字面值；不支持双引号展开、行内注释或多行值。
重复键会报错，`$()`、反引号等内容不会当作命令执行。数据库密码建议保留自动生成值；自定义时使用 16–128 位 URL 安全字符并同步修改 DATABASE_URL。
脚本通过受控环境变量传给 Compose，不让 Compose 再次展开 `.env` 中的字面值。

包含模型控制能力的新镜像与本目录新版 Compose 配套发布后，可在服务器 `.env` 设置：

```dotenv
NEXOFOLIO_CATALOG_MODEL=deepseek-v4-flash
NEXOFOLIO_MODEL_THINKING=high
NEXOFOLIO_MODEL_JSON_OUTPUT=false
```

修改已有模型名字段；新字段仅在不存在时追加，避免重复键。思考档位支持 `provider/low/high/max/disabled`，默认 `provider` 不显式指定服务商思考参数。
JSON 输出是独立选项，默认 `false`。`NEXOFOLIO_CURATION_IDLE_SECONDS` 默认 900，控制整理任务的空闲超时。
这些字段已在共享后端环境中声明，会同时传入 API 和管理容器。已有并发设置继续沿用服务器 `.env`。
旧镜像和旧版本 Compose 不具备新增字段的完整支持，不能只改 `.env` 或只重启旧容器。

默认使用 `/home/nexofolio/backend`，可在每次运行时传入 `--deploy-dir` 或设置 `DEPLOY_DIR`。
如果 OSS 路径与默认值不同，在服务器设置 `OSS_BASE_URL=https://你的OSS域名/发布前缀`，或每次传入 `--oss-base-url`。
URL 不接受账号、密码、签名查询参数；私有对象的访问方式需要另行配置，不能把长期凭据装进发布包。

## 3. 首次部署与日常更新

```bash
bash update-backend.sh update
# 或固定某个发布版本，避免其他人更新 latest 影响此次部署：
bash update-backend.sh update --release <版本号>
bash update-backend.sh status
```

首次使用专用 Compose 项目 `nexofolio-server` 和专用数据卷 `nexofolio-server-postgres`。
如果这些名字已经被占用，脚本拒绝接管；可以在首次部署前修改 `.env` 中的项目名与数据卷名。
数据库只在 Compose 网络中可访问，API 绑定主机 `127.0.0.1:18080`。脚本按以下顺序执行：

1. 校验发布清单、文件大小、SHA-256、镜像架构、Compose 配置和本地 PostgreSQL 镜像。
2. 导入所选后端镜像；首次创建带归属标记的专用数据卷，更新时复用记录的数据卷和数据库镜像。
3. 启动数据库并校验实际镜像及数据卷；运行后端配置检查。
4. 停止 API 写入，生成 `pg_dump -Fc` 备份，并用 `pg_restore --list` 检查归档可读性。
5. 显式运行数据库迁移，再启动 API，核验实际运行镜像与 `/health/live`、`/health/ready`。
6. 仅成功后记录 current / previous 版本。相同版本且配置未变时仅核验健康，不重复迁移、备份或重启。

更新有短暂停机时间。备份可读性检查不等于完整恢复演练；当前自动验证范围不包含真实登录、模型调用、MCP 客户端或浏览器。
配置变化会触发同一版本重新部署。已部署后的项目名、数据库卷、PostgreSQL 镜像/镜像 ID、数据库账号配置不可通过普通更新替换，数据库升级需单独安排。
会话密钥持续保留，版本更新不重新生成。

日志中的工具原始输出默认不显示，避免凭据或服务错误详情外泄。配置检查或迁移失败时仅输出退出码、固定错误类别和已知配置项名称，不打印配置值。遇到失败先用 `status` 查看状态。
`backups/` 保存受限数据库备份，`configs/` 保存受限环境快照，`state.tsv` 保存成功及未完成部署记录。
不要上传或公开这些目录，也不要删除仍用于恢复的文件。

## 4. 失败处理与回滚

下载、校验、架构检查或已有资源冲突失败时不停止原 API。
备份失败不会执行迁移。迁移或健康检查失败不宣告部署成功；新 API 健康检查失败时会尝试将其停止。
API 一旦停止，失败后可能保持停止状态，脚本不会在未知 schema 上自动启动旧代码。

修复原因后，按 `status` 显示的未完成版本重新运行：

```bash
bash update-backend.sh update --release <未完成版本号>
```

同一未完成发布重试会保留原始迁移前备份，不用已迁移的数据覆盖它。
重试时可修正禅道、模型等运行配置，脚本另存环境快照并保留原始数据库备份；数据库身份及连接凭据仍不可替换。
脚本不会自动恢复数据库、删除数据卷或清理其他项目容器。

```bash
bash update-backend.sh rollback
```

回滚使用记录的上一成功版本，只有两版本迁移文件及校验值完全一致时才执行。
如果数据库迁移有变化，拒绝自动回滚，需单独评估旧代码兼容性、恢复点与新增数据处理。
前进发布也拒绝删除或改写上一成功版本包含的迁移文件。

## 5. 前端与 NGINX

本脚本不改动共享服务器的 NGINX。后端就绪后，需要给现有站点配置 `/api/` 反向代理并去掉前缀，
例如浏览器 `/api/v1/auth/me` 转到后端 `/v1/auth/me`。如果 NGINX 运行在容器或另一台机器，
不能使用其自身的 `127.0.0.1` 连接 Docker 主机，需先确认网络拓扑。
MCP 入口、Host/Origin 白名单和整理接口代理超时单独配置。
前端构建时开启 `VITE_WORKSPACE_ENABLED=true`，保持同源 `/api` 和真实数据源。
证书预先准备或复用既有证书，离线部署流程不执行在线证书申请。

## 本地验证

```bash
bash tests/deployment/offline-release.sh
bash tests/deployment/image-identity.sh
bash -n upload-server.sh deploy/update-backend.sh deploy/offline/preflight.sh
```

测试使用临时源码仓库、真实文件与 SHA-256 校验，以及模拟的 Docker/OSS 命令边界验证失败顺序；还会拦截 Python/jq 命令，确保流程没有调用它们。
它们不连接 OSS、不启动真实容器，不代表已在服务器完成部署或 PostgreSQL 恢复演练。

参考：[Docker 离线导入](https://docs.docker.com/reference/cli/docker/image/load/)、
[Compose 启动选项](https://docs.docker.com/reference/cli/docker/compose/up/)、
[PostgreSQL 17 pg_dump](https://www.postgresql.org/docs/17/app-pgdump.html)。
