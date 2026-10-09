# 交接：接口目录页 + 整理（curate）两条流水线

写作时间：2026-09-30
交接对象：接手的下一个 agent
核对基准：后端 `NexoFolio` @ `1f9790a`（main，已与 origin 同步）；前端 `NexoFolio-Front` @ `6624ef1`（main，已与 origin 同步），**工作区有未提交改动**（见 §5）

> 本文只描述事实和待办。接手前请自己重新核对代码，本文中的行号和结论会过期。

---

## 1. NexoFolio 的精神（先读这一节，它决定所有取舍）

NexoFolio 是一个面向人和 AI agent 的**接口知识库**。数据来自真实流量（Chrome 插件采集），不是手写声明。

它的存在理由只有一条，用户原话是「关键中的关键」：

> **录入（intake + observe）能归纳出接口身份、字段结构、调用规律和真实样本，但产出的东西里没有一个字是人话。整理（curate）是把它变成文档的唯一环节，也是 NexoFolio 区别于普通 Swagger 的唯一理由。**

Swagger 只有开发者声明的类型。NexoFolio 有真实流量里**实际出现过**的值，以及接口之间**实际发生**的调用关系。后者是 Swagger 结构上给不出来的。所以：

- 标题 / 描述 / 目录是**一等公民**；类型和调用次数是次要信息。
- 整理不是可选的润色，是产品的核心。不做整理，这个产品没有存在必要。
- 检索（MCP）**不允许用 LLM**；LLM 只用在**构建文档树**的时候。这条边界不要越。
- 每个项目**永远只有一份文档**。不允许为任何接口子集另开一份存储 / 文档 / 层级。外部服务接口在同一份文档里，同等可搜，只是带一个「外部服务」标注。
- 每个地址默认属于本项目，只有人工标记才是外部。不要写自动判定规则（2026-09-28 已删除过一版）。
- 「待分类」是**虚拟目录**（用 observe 的接口列表减去已放置的算出来，不落库），接口一进 observe 就能浏览，不必等整理。
- 整理是**两条流水线**，顺序由数据依赖决定：
  1. **内容补全**（逐接口、增量：只看单个接口 + 真实示例）必须先跑；
  2. **目录整理**（全项目全量：只看方法 + 路径 + 一句话用途，约 20 token/接口，不看字段）后跑。
  绝不只整理待分类的接口。

---

## 2. 用户的要求与工作方式（照做，不要重新商量）

### 2.1 协作方式

- **逐个环节讨论清楚再实现**，不要一次铺开整套系统。架构和交互结构**用文字确认**。
- 报告已完成的工作用**大白话**：做了什么、解决了什么问题、怎么用、为什么这么设计。不要堆术语。
- 模块之间**严格代码隔离**，边界必须可验证（不是只有目录名字上的划分）。
- **不写没用的代码**。用户重视清理和解耦，宁可少写。本地数据库可以随时丢弃。
- 不做自动持续推进：按阶段汇报，等确认。
- 不用 GitButler，不建 worktree。用户自己执行 commit / push，**直接推 main，不建阶段分支**；我给出命令，他执行。
- 后端、前端、插件是**三个独立项目**。改后端不等于可以改所有客户端，跨项目接口变化必须对齐。

### 2.2 前端审美（要求极高，反复强调「要有极高的交互」）

- **绝不给我看线框图或低保真示意**。架构用文字确认；视觉只在高保真、带动效、用真实数据的实现上确认。
- 拒绝「AI 感」：暖米白纸色、玻璃卡片、发光核心、ALL-CAPS 等宽 eyebrow、01/02/03 编号、每个卡片同一个圆角同一层灰阴影——都被明确骂过。
- 动效优先，**不要静态**。一个编排好的时刻胜过满页零散 fade-in。但要克制，别到处动。
- 必须避开：缩进树 + ▸ 折叠三角的 Swagger 观感、长路径换行、没有留白的 1px 灰线切分、含义不明的标签。
- 长文本**单行截断，尾部渐隐**，hover 时跑马灯滚动。
- 内容必须来自**真实 API 内容**，不用装饰性图形凑数。
- 为了前端质量，**愿意改后端逻辑**。前端觉得别扭就回去改后端。

### 2.3 Rust 惯例（后端硬规则）

- **不使用 `Utc::now`**，时间走 `Clock` trait（可测）。
- ID 一律走 `identifier!` 宏，不手搓 string。
- **不引入 schemars**。
- 新模块必须同时登记到 `tests/architecture/dependencies.rs` 的 `MODULES` 常量，否则架构测试会以「unregistered module directory」拦住。
- 模块划分：`modules/{access,curate,intake,knowledge,observe}/{contracts,core,adapter}`。模块之间**只依赖** `nexofolio-common` / `nexofolio-contracts`，只有 `nexofolio-backend` 负责装配。由 `tests/architecture/dependencies.rs` 与 `sql_ownership.rs` 强制。

---

## 3. 两个仓库与运行环境

后端：`/Users/sheldon/Documents/GithubProject/NexoFolio`（Rust）
前端：`/Users/sheldon/Documents/GithubProject/NexoFolio-Front`（Vue 3 + Vite + pnpm + vitest）

沙箱里的 bash **看不到 Mac 路径**，映射关系是：

| Mac | 沙箱 bash |
|---|---|
| `/Users/sheldon/Documents/GithubProject/NexoFolio` | `/sessions/brave-trusting-goldberg/mnt/NexoFolio` |
| `/Users/sheldon/Documents/GithubProject/NexoFolio-Front` | `/sessions/brave-trusting-goldberg/mnt/GithubProject/NexoFolio-Front` |

本地运行：DB 在 Docker（compose 项目 `nexofolio-local`，宿主 `127.0.0.1:15432`），后端用 `cargo run` 跑在 Mac 上 `127.0.0.1:18080`，启动脚本是仓库根目录的 **`start-nexofolio.sh`**（未提交，是用户的实际启动路径）。前端 dev server `5173`，`/api` 反代到 `127.0.0.1:18080`（并去掉 `/api` 前缀）。

沙箱限制：单次 bash 约 120–180 秒；`cargo` 在这个沙箱里 3 分钟上限；分区只剩约 1.6 G；**没有浏览器/截图能力**，视觉验证只能靠用户截图。Mac 的 `node_modules` 带 darwin 二进制，沙箱里跑 vite 需要在 `/tmp` 拷贝里装 Linux 原生包再跑（见 memory: plugin-build-in-sandbox）。

---

## 4. 已完成的（后端，已提交）

后端 curate 模块**已完整实现并提交**（`1f9790a`），架构测试干净：

- `modules/curate/contracts` — 唯一 IO 缝：`trait Completions { async fn complete(&self, prompt: &str) -> Result<String, CompletionError> }`，以及 `Arc<dyn Completions>` 的 blanket impl。
- `modules/curate/core` — 两个 pass，**都是纯函数**，返回 `knowledge` 的 `Command`，不碰存储：
  - `describe.rs`：`Describe::endpoint(&facts) -> Option<Command::PutEndpointNote>`。从回复里切出 JSON（`json_slice` 容忍 ```json 围栏和前后废话），name/purpose 都空则返回 `None`（不写空说明）。`author()` → `Author::Curate { pass: "describe", model }`。
  - `organize.rs`：`Organize::project(&summaries) -> Vec<Command>`，产出 `PutFolder` + `Place`，按模型列出的顺序定 position。
  - `prompt.rs`：中文 prompt，要求裸 JSON。
- `modules/curate/adapter` — `LlmClient`，OpenAI 兼容 `POST {base_url}/chat/completions`，`temperature: 0.2`。
- `apps/backend/src/http/curate.rs` — 两个路由，都挂 `session_auth` + `no_store`：
  - `POST /v1/projects/{project_id}/curate/describe/{endpoint_id}`
  - `POST /v1/projects/{project_id}/curate/organize`
  - 写入走 knowledge 的轮次：`begin` → `apply` → `finish(Done)`；失败则 `finish(Failed)`，所以每次整理都是可回滚的一轮。
- 测试：`modules/curate/core/tests/passes.rs`（237 行，用假 `Completions`，不碰网络）。

**设计意图（为什么要这么做）**：pass 不直接写库，只吐命令，所以「加一个新阶段」= 写一个 pass 并声明依赖，不用碰存储和回滚逻辑。模型藏在 `Completions` 后面，所以测试永远不需要网络。

## 5. 已完成的（前端）与**未提交的现状**

前端已提交到 `6624ef1`：目录树页面整体成型——一个树，文件夹是节点、接口是叶子；方法色块、hover 时的格子填充、跑马灯截断、行进入动画、`待分类` 虚拟目录、`自动整理目录` 黑条按钮、接口行上的放大镜（describe）触发。

**当前工作区有 3 个文件未提交**（这就是上一次会话停下的位置）：

```
 M src/features/knowledge/CatalogDetail.vue
 M src/features/knowledge/catalog.css
 M src/pages/KnowledgeCatalogPage.vue
```

改动内容：

1. **`KnowledgeCatalogPage.vue`** — 右侧详情栏从「有选中才出现」改成**永久第二列**。之前 `has-detail` class 和 `v-if`/`<Transition name="catalog-detail">` 让树在没选中时占满整宽，用户明确抗议过（「我想要的是左右结构，为什么你给我弄成了这样」）。现在 `<CatalogDetail>` 无条件渲染。**这一项用户已截图确认修好了。**

2. **`CatalogDetail.vue`** — 整体重写成「带空闲态的常驻栏」：顶部 `.cd-bar`（`接口详情` + method chip + 关闭按钮），`<Transition name="cd-swap" mode="out-in">` 在 `.cd-idle`（水印 + `左边点一个接口` + `这里显示它的说明、观测到的字段和调用情况`）和 `.cd-body` 之间切换。

3. **`catalog.css`** — 只有一处改动（就是下面 §6.1 的 bug 修复），**但新模板用的 class 一个都还没写样式**（见 §6.3）。

---

## 6. 停在这里的三件事

### 6.1 【已修，未验证】点击接口行没有反应

用户原话：「点击接口没有反应」。

已定位并已改，**但没验证过**。根因是 CSS 特异性冲突，不是 JS：

- `.ct-row > *:not(.ct-hover-fill)` 特异性 (0,2,0)，设了 `position: relative`；
- `.ct-open`（那个撑满整行的透明按钮）特异性只有 (0,1,0)，需要 `position: absolute; inset: 0`。

高特异性那条赢了，`in` 伸展按钮塌成零尺寸，整行没有命中区。文件夹行不受影响，因为它本身就是 `<button>`。

改法（已写入 `catalog.css:289`）：

```css
/* The opener is deliberately the backdrop control, so it keeps its own absolute
   positioning; everything else stacks above it. */
.ct-row > *:not(.ct-hover-fill):not(.ct-open) {
  position: relative;
  z-index: 1;
}
```

**验证方式**：起前端 + 后端，点任意接口行，右侧栏应出详情。（沙箱里看不到画面，需要用户截图或用别的方式确认。）

### 6.2 【已诊断，未修】点放大镜返回「请求失败（500）」

用户原话：「点击放大镜出现请求失败 (500)」。

**这是配置问题，不是代码问题。**证据链：

- `apps/backend/src/wiring/api.rs` 里，三个值都有兜底：
  ```rust
  let completions: Arc<dyn Completions> = Arc::new(LlmClient::new(
      config.catalog_model_base_url.clone().unwrap_or_else(|| "http://localhost:11434/v1".into()),
      config.catalog_model.clone().unwrap_or_else(|| "deepseek-v4-pro".into()),
      config.catalog_model_api_key.clone().unwrap_or_default(),
  ));
  ```
- `deploy/compose.yaml` 的 `&backend_env` 块里**没有任何 `NEXOFOLIO_CATALOG_MODEL_*`**；`deploy/.env` 里也没有。
- **用户实际跑的 `start-nexofolio.sh` 里也一个都没有。**
- 所以进程拿到的是兜底值 `http://localhost:11434/v1` + 空 key。本机没有 ollama 在 11434，请求失败。
- `CompletionError` → `impl From<CompletionError> for CurateHttpError` → `Failed(msg)` → **500 `CURATION_FAILED`**，body 里带原始错误文本。
- 前端 `src/api/client.ts:84` 对非 ok 一律显示 `请求失败（${status}）`，把 body 丢掉了，所以用户只看到一个干巴巴的 500。

另有一处**对不上的配置**：`deploy/.env.catalog-model`（未跟踪，被 `.gitignore` 的 `.env.*` 忽略）里写的是：

```
NEXOFOLIO_CATALOG_MODEL_BASE_URL=https://baitong-aiw.gree.com/openapi/cllm
NEXOFOLIO_CATALOG_MODEL=deepseek-v4-pro
NEXOFOLIO_CATALOG_MODEL_KEY_HOST_FILE=/Users/sheldon/.config/nexofolio/catalog-model-key
NEXOFOLIO_CATALOG_MODEL_UID=501
NEXOFOLIO_CATALOG_MODEL_GID=20
```

`config.rs` 只读 `NEXOFOLIO_CATALOG_MODEL_BASE_URL` / `NEXOFOLIO_CATALOG_MODEL` / `NEXOFOLIO_CATALOG_MODEL_API_KEY` —— **`KEY_HOST_FILE` 全仓库没有任何 Rust/脚本读它**（那是旧 Compose 时代挂只读 key 文件留下的）。而且 **`/Users/sheldon/.config/nexofolio/` 目录在本机不存在**。

**修法（建议，需向用户确认 key 从哪来）**：

- 在 `start-nexofolio.sh` 里给 `nexofolio-api` 那段加：
  ```
  NEXOFOLIO_CATALOG_MODEL_BASE_URL="https://baitong-aiw.gree.com/openapi/cllm" \
  NEXOFOLIO_CATALOG_MODEL="deepseek-v4-pro" \
  NEXOFOLIO_CATALOG_MODEL_API_KEY="$(cat /Users/sheldon/.config/nexofolio/catalog-model-key)" \
  ```
  并把 key 文件建出来（或让用户给出 key 的正确来源）。
- 同时把这三个变量补进 `deploy/compose.yaml` 的 `&backend_env`（读 `deploy/.env`），否则容器路径跑不通。
- 顺带把 `.env.example` 补上这三项说明。
- **另外值得做的**：让前端把 500 的 body 里的 `error.message` 显示出来，而不是只显示状态码。现在后端明明返回了原因，前端丢掉了——这个体验问题在演示里会很难看。

**这条是演示链路的断点，优先级最高。**

### 6.3 【未做】常驻详情栏的 CSS 收尾

`CatalogDetail.vue` 的新模板引用了这些 class，**`catalog.css` 里一个规则都没有**：

`.cd-bar`、`.cd-bar-label`、`.cd-bar-tools`、`.cd-idle`、`.cd-idle-mark`、`.cd-idle-title`、`.cd-idle-hint`、`.cd-body`，以及 `cd-swap` 的 transition 类。

同时这些是**死代码，应删**：

- `.cd-panel`（`catalog.css:657`）还是旧的浮动灰卡片（`background: var(--surface-muted)` + `border-radius: 10px` + `padding: 18px 18px 8px`）。常驻栏应该是一列：顶部 bar 与 `.catalog-tree-bar` 同样的黑色材质和 `--ct-bar-height`，下面才是内容。`padding` 要挪到内容区去。
- `.cd-head`（`catalog.css:668`）已无人使用（模板改用了 `.cd-bar`）。
- `.catalog-detail-enter-active` / `.catalog-detail-leave-active` / `.catalog-detail-enter-from` / `.catalog-detail-leave-to`（`catalog.css:957–967`）——那是「栏不存在时向右收起、`margin-right` 负值塌陷」的旧转场，栏现在是常驻的，永远不会触发。

设计约束（用户的口味，务必遵守）：

- 右侧栏顶部 bar 要和左侧树顶栏**同一套材质**：`--ct-bar-bg`（`linear-gradient(180deg, #292929 0%, #171717 48%, #080808 100%)`）、`--ct-bar-height: 40px`、`--ct-bar-shadow`、`inset 0 -1.5px 0 var(--ct-bar-edge)`、`animation: ct-bar-in`。左边黑条右边也必须黑条，否则两边不对齐会很丑。
- 空闲态的水印走 `--cd-mark-image`（由 `import.meta.env.BASE_URL` 拼出来，模板已传），透明度用 `--project-watermark-opacity`（亮色 0.025 / 暗色 0.045，见 `tokens.css:12` 与 `:43`）。
- `cd-swap` 的转场必须是**编排好的**，不是普通 fade：idle ↔ body 的切换要有方向感和位移，配合 `--ct-ease: cubic-bezier(0.22, 1, 0.36, 1)`。用户会盯着这里看。
- 不要用新的圆角 / 阴影语言，沿用它已有的 hairline（`inset 0 0 0 1px var(--ct-line)`）和 `--surface-muted` / `--bg` 两个层次。
- `.catalog-body { display:flex; gap:22px }` + `.catalog-tree-frame { flex:1; min-width:0 }` 已经是对的，两列布局本身不用改。

---

## 7. 其余待办（按优先级）

1. **§6.2 打通 catalog-model 配置** —— 演示链路的断点，先做这个。
2. **§6.3 常驻栏 CSS 收尾 + 删死规则**。
3. **§6.1 验证行点击**。
4. 前端验证：`pnpm typecheck`、`pnpm test`、`pnpm build`、`pnpm format:check`（= `pnpm check` 一次跑完）；用到的测试文件是 `tests/knowledge-curate.test.ts`、`tests/catalog-tree.test.ts`、`tests/api-client.test.ts`。
5. 后端验证：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`。真库测试要独立测试库：`TEST_DATABASE_URL=... cargo test -p nexofolio-backend --test postgres --test access_flow --locked -- --ignored`。
6. 任务 #53（前端【整理】按钮）实际已经做了：`catalog-organize` 按钮在 `KnowledgeCatalogPage.vue` 的 header 里，接了 `tree.organize()`。可以在任务表里收掉。
7. 后续 backlog 全在 `docs/architecture/0005-curate-and-backlog.md` §7，其中最便宜、收益最大的是 **§7.1 补采页面上下文**（`context.page_title` / `page_url` / `interaction_id`：observe 的 ingest 完全没有引用这三个字段，是**目前唯一白扔掉的证据**，加上之后一级目录会准得多，还能推出「哪些接口总是一起被调用」）。**代价是已采集的 47 个接口的 context 已永久丢失，必须重新采集才有数据。**

---

## 8. 提交（用户自己执行，直接推 main）

前端工作区里有 **234 个待删除**（旧 catalog / catalog-preview / maintenance 的清理）和一堆无关改动，**绝对不要 `git add .`**。只暂存相关路径：

**前端**（目录页 + 详情栏 + 行点击修复）：

```bash
cd /Users/sheldon/Documents/GithubProject/NexoFolio-Front
git add src/features/knowledge/CatalogDetail.vue \
        src/features/knowledge/catalog.css \
        src/pages/KnowledgeCatalogPage.vue
git commit -m "catalog: 详情栏常驻为右列，修复接口行点击"
git push origin main
```

**后端**（如果改了 `start-nexofolio.sh` 之外的跟踪文件）：

```bash
cd /Users/sheldon/Documents/GithubProject/NexoFolio
git add deploy/compose.yaml .env.example      # 按实际改动调整
git commit -m "curate: 打通模型配置"
git push origin main
```

注意 `start-nexofolio.sh`、`STORYBOARD.md`、`check-db.sh`、`login-test.sh`、`mcp-documentation.md`、`style-demo-v*.html`、`ref-frames/`、`ref.mp4`、`promo/` 等在后端仓库里**都是未跟踪文件**，用户一直没提交。别再自作主张替它们决定。

---

## 9. 容易踩的坑

- **前端 `KnowledgeCatalogPage.vue` 里的 `catalog-rows` 用了 `<TransitionGroup>`**，行进入靠 `--row-enter-delay` 错峰，`ct-row-in` 是 `380ms`。改行结构时别把 `:key="row.key"` 弄丢，否则整棵树会重放动画。
- **`useCatalogTree.ts` 的 `select()` 会先把 `detail` 清空再请求**，并按 `AbortController` 取消上一次。右侧栏常驻之后，「先清空」会让栏闪一下——如果视觉上不能接受，改成保留上一份并加 loading 骨架，但要小心和 `cd-swap` 的 key 冲突。
- **`describe()` 结束后会 `refreshLoadedFolders()` 并按需重新 `select()`**，所以描述写完右侧栏会刷新一次。这是有意的，别当成 bug 修掉。
- **`organize()` 会清空 `loaded`/`expanded` 再 `loadCatalog()`**，整棵树收起。刻意如此。
- **诊断 500 时不要只看前端**：`请求失败（500）` 是 `client.ts` 的通用文案，真正原因在后端 body 的 `error.message` 里。用 curl 打一次那个路由就能拿到原文。
- 后端根目录 `mcp-documentation.md`、前端 `docs/design/*` 里有旧叙述，可能与当前实现不符；以代码为准。

---

## 10. 演示背景（时间敏感）

用户 2026-09-29 说 **2026-09-30 要演示 NexoFolio**。他明确表态非演示相关的工程问题（容量压力、归档策略、并发、预算限流等）可以往后排。

演示路径就是：**目录树 → 接口详情 → 点一次【整理】看到目录和描述变好**。这条链路的完成度优先于一切；不要主动引入需要重新跑数据的改动。

（这条是有时效的：如果接手时演示已经过去，请忽略这一节的优先级，回到 §7 的顺序。）
