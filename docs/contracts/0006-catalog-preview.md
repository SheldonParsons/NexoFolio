# 手动目录候选实验

本轮阶段：已有接口文档 → 固定项目快照 → 生成候选目录 → 结构检查与查看。HTTP 接收、当前接口定义、待确认差异均不因生成候选而改变。自动触发、局部重构和模型分类质量评分仍未实现；正式目录/发布/回退见0008-official-catalog.md。

## 组件边界

- contracts：快照、候选、检查结果和任务的共同类型。生成 Schema 位于 contracts/catalog-preview。
- knowledge：候选任务与目录存储端口；接口文档通过稳定 ID 被引用。
- rebuild：可替换的 DirectoryGenerator 和 DirectoryReviewer，及独立的确定性结构检查器。
- application：创建、领取、生成、检查、保存的流程编排。
- infrastructure：PostgreSQL 实现、chat-completions-compatible 模型适配器。
- backend：admin CLI 负责创建/执行；新增带项目权限检查的只读HTTP列表/详情，候选生成仍通过CLI；正式目录发布/回退有单独的受权限控制HTTP入口，不自动执行。

## 输入与输出

每次快照包含项目内当前已建档的接口，每个接口包含所有环境的当前修订 ID 和观测定义。事务固定快照，后续新增接口不会混入已创建的任务。环境是接口定义的来源，不要求按环境生成业务目录。输入不包含历史版本或待确认 proposed_definition，也不额外复制原始流量正文。

首轮限200个接口、1MiB快照，超限明确失败，不悄悄抽样或截断。后续分批/采样策略应单独设计。每个接口必须被安排一次：目录 UUID 或显式 null（待分类），附简短依据。nodes/assignments 的关系独立于实际接口文档。

校验覆盖遗漏、未知接口、重复归属、无效父节点、环、同级同名、无效名称/说明、未知目标目录；目录过深与空叶子给警告。统计目录深度、每目录直接接口数、已分类/待分类接口数及比例。结构合法不等于语义合理，仍需看真实目录是否便于查找。

## 配置与命令

在原 DATABASE_URL 配置基础上，执行模型生成的命令需配置：

```bash
export NEXOFOLIO_CATALOG_MODEL_BASE_URL=https://your-provider.example/v1
export NEXOFOLIO_CATALOG_MODEL=your-model-name
export NEXOFOLIO_CATALOG_MODEL_API_KEY_FILE=/absolute/path/to/key-file
```

key-file 仅保存 API Key，不包含 Markdown、变量名或账号密码。密钥只在执行模型任务时加载。示例不含真实凭据。普通 create/show 不调用模型，模型未配置也可使用。

```bash
nexofolio-admin catalog-create --project-id <项目UUID>
nexofolio-admin catalog-run --task-id <任务UUID>
nexofolio-admin catalog-show --task-id <任务UUID>
# 一次创建并执行；stderr 先给 task ID，失败后仍能查询/重试
nexofolio-admin catalog-preview --project-id <项目UUID>
```

也可使用 `cargo run --locked -p nexofolio-backend --bin nexofolio-admin -- <命令>`。迁移仍须先独立执行 `nexofolio-admin migrate`。

Docker 中执行时应通过一次性只读 bind mount 将 key-file 挂载到上述容器路径，并显式向 `docker compose run --rm` 传入三个模型环境变量；不把密钥写入镜像、仓库或命令参数。API/worker 不读取密钥文件或自动调用模型。

## 状态与恢复

pending → running → ready / rejected / failed。ready/rejected 保留候选和检查结果，再次生成应新建任务；failed 可以手动再次 run。同一任务只能有一个有效执行者，五分钟租约、递增代次禁止旧执行覆盖新执行。模型请求最多120秒，禁止重定向；输出过大、非JSON、截断或不符合合同都会失败。Ctrl-C/进程被杀可能留下 running，租约到期后手动 run 可恢复。没有自动重试守护进程。

只有 ready 的候选写入关系化节点/归属表；rejected 的原提案与错误报告保留在任务里，方便比较。完成保存为同一事务。失败只记录固定错误码，HTTP 状态可进入日志；不打印 provider 响应正文、原始流量、密钥。

任务记录接口快照时间和 SHA256、合同版本、模型/适配器/提示词版本与完整系统提示 SHA256（含输出 Schema）、执行代次。部署后必须区分协议测试通过、真实模型成功及人工认为分类合理这三种结论。

## 合同对齐

```bash
python3 scripts/catalog_preview_contract.py          # 从Rust生成Schema和清单
python3 scripts/catalog_preview_contract.py --check  # 检查漂移
cargo test --workspace --locked
```

Rust 测试直接对比类型生成的 Schema 与提交的文件，CI 检查 manifest 和版本变更。模型适配器消费同一候选 Schema，后台替换模块无需修改接收模块。目录合同目前不传给前端/插件执行，因此此次不修改它们已有合同镜像。

保存 `catalog-show` 的 JSON 后，可执行 `python3 scripts/catalog_preview_report.py --input task.json --output review.txt` 生成人可读的目录与归属说明，不需要修改前端。

## 独立容器模型入口

可选 `deploy/compose.catalog-model.yaml` 只定义管理命令容器，不暴露端口、不启动后台生成。使用本机配置文件中的非密钥参数和私有 Key 文件只读挂载；示例变量包括 `NEXOFOLIO_CATALOG_MODEL_BASE_URL`、`NEXOFOLIO_CATALOG_MODEL`、`NEXOFOLIO_CATALOG_MODEL_KEY_HOST_FILE`，UID/GID应与Key文件可读权限匹配。`deploy/.env.catalog-model` 已在忽略规则内，勿提交实际配置。

```bash
docker compose --project-name nexofolio-local \
  --env-file deploy/.env --env-file deploy/.env.catalog-model \
  -f deploy/compose.yaml -f deploy/compose.catalog-model.yaml \
  run --rm --no-deps -T catalog catalog-run --task-id <任务UUID>
```

同一入口支持 `catalog-create`、`catalog-show` 和 `catalog-preview`。ready/rejected任务不可重复执行；比较新方案时创建新任务。

## 前端只读预览（合同包1.1.0）

- `GET /v1/projects/{project_id}/catalog-previews?page=1&limit=20`：返回items/total/page/limit，按snapshot_at与task_id倒序。可选status=pending/running/ready/rejected/failed；默认最近ready可直接使用status=ready&limit=1。
- `GET /v1/projects/{project_id}/catalog-previews/{task_id}`：返回 `{published:boolean,task:PreviewTask}（published表示当前生效）`，包括原始快照、候选和检查结果。查询不生成新候选，也不改正式目录。

使用现有内部Bearer Token。reader层再次检查用户、项目实例及权限，权限快照替换期间使用同一用户行锁。无凭证401、无项目权限403、权限未同步/数据库不可用503、任务不属于路径项目或不存在404。page范围1..100000，limit范围1..100，非法或未知列表参数400；请求不需要environment_id。响应禁止缓存。

page.schema.json、detail.schema.json 与types.generated.ts由同一Rust类型生成，前端镜像使用同一manifest。页面展示快照中的接口及环境定义，不将后来新增接口悄悄放进旧候选。rejected的结构可能无效，前端应展示问题并安全平铺；普通用户没有生成、编辑、发布入口。

## 路径识别与候选合并（合同1.2.0）

新快照增加可选recognized_path，用项目当前机械规则给路径识别提示，原始path与成员定义均保留。模型通过merge_groups提出合并组，含representative_id、member_ids、path_template、reason。所有原始ID仍须有assignment；代表必须在成员内，组间不能重叠，成员须同方法/同目录，模板必须以完整路径段覆盖成员。逻辑展示数由metrics.logical_interfaces给出，原始覆盖数保持不变。

合并组是候选展示关系，不创建正式接口别名、不删除记录、不覆盖环境定义，也不会直接写成接收端规则。旧候选仍可原样查看，新候选才能反映这次提示词和合并能力。已有字面路径文档暂不重写，重构时可将这些历史重复记录集中展示；后续新观测则按模板建立接收头和文档。
