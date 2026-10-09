# 文档导航三项统一实施交接

用户已授权在一个新会话统一实现：目录点击后的右侧内容、目录/接口 hover 预览、每个目录/接口的独立路由。先完整阅读 `/Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/0009-document-navigation.md`，按其中交互及验收直接实施，不再停留在规划。

主实现工作区为 `/Users/sheldon/Documents/GithubProject/NexoFolio-Front`。保存的 Codex 项目为后端 NexoFolio，因此会话初始 cwd 可能在 `/Users/sheldon/Documents/GithubProject/NexoFolio`，请显式切到前端做实现。

## 三项完整交付

1. 保留 `/projects/:projectId/interfaces`，新增 `/projects/:projectId/interfaces/folders/:folderId` 和 `/projects/:projectId/interfaces/endpoints/:endpointId`，分享携带环境 query。URL 决定选择/右侧内容，刷新、新页、前进后退、登录返回均恢复；展开祖先和定位侧栏。用真正 RouterLink 支持复制地址及 Ctrl/Cmd 点击，折叠箭头独立。不绕过授权。
2. 点目录名称显示右侧紧凑概览：介绍、子目录或接口摘要、现有知识中的场景/关系及缺口；点接口显示原详情。支持父子目录数据，搜索和环境过滤不隐藏有效的目标。原真实目录仍只有一层，使用两层 fixture 检验；不要宣称后端生成已变两层。
3. 共享 hover 预览卡：目录/接口摘要，打开详情与复制链接。延迟约350ms、离开约180ms、入160ms/出120ms和小位移作为初值，实际观察微调；鼠标进卡片保持、快速扫动无旧内容、边缘避让、Portal不被裁切、滚动/路由切换关闭、键盘/Escape/触屏及 reduced-motion。复用 reka-ui 和现有主题；不抢焦点、不改变 URL/右侧选择、不请求完整样本/凭证、不现场调用模型。

## 结构和工作边界

- 先记录当前文件基线，已有大量未提交变化，不覆盖、不创建 branch/worktree、不 commit/push、不启动代理。
- 独立路由处理、目录投影/概览、共享预览交互，保持 useDocumentWorkspace 作为薄组合层，不把页面/控制器堆成巨型文件；遵守已有 400 行文档职责检查。
- 修正同项目深链接切换引起整页进入动画/侧栏滚动重置的问题，复用已加载索引。保留正常鉴权与跨项目处理。
- 正确处理目标环境没有观测、ID无效、资源失效/别名、权限不足、快速切换及迟到响应，不默默选另一个同名资源。
- 两层目录后端生成不在这三项范围，backend curate/knowledge/access/MCP 正由主会话审批，不修改其代码、迁移或真实知识数据。若缺读取能力，记录需求并用已有摘要/关系与明确空状态实现兼容，不自行修改后台。
- 刚完成的首轮后台同步了前端 `src/contracts/knowledge/types.ts`、FieldStructure.vue 和 FieldEvidencePanel.vue；保留新增中文名、分层说明及来源。主会话审批以这批实现前基线比对，UI任务另留独立基线，避免收益混算。
- 不移动/删除后台结果报告或基线。不要修改用户其他非本任务文件。

## 验证和运行

前后端目前由主会话管理，5173/18080；不要停止/重启进程，不能关闭认证来做浏览器演示。Node24 路径：`/Users/sheldon/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin`。

使用适当的 Vue/Vitest 路由、异步状态、投影及交互测试，跑类型检查/构建，检查本批文件格式。用 browser/组件 harness 观察实际布局、hover 出入、保留卡片、快捷键和深链接行为；可以用临时脱敏 fixture，完成后清理，并区分模拟与真实登录/数据验证。不要要求用户提供平台密码，也不要打印既有凭证/启动配置或完整原始样本。

用户要求最后统一按“自动整理文案/文档”按钮验收；本批不得点击、启动等价真实项目 /curate，或做业务系统创建/提交。没有真实工作台登录时，不冒充已验证真实项目权限流程，使用隔离测试并记录边界。

完成后在前端仓库 `docs/reviews/2026-10-07-document-navigation-results.md` 保存可复核报告：准确文件改动、实现前对照、独立路由示例和恢复行为、预览排版/动画/键盘证据、两层 fixture 数据来源、测试/构建、截图及仍未解决的边界。最终用人话解释收益，不把界面模拟当真实后端目录整理已升级；不向别的会话发消息。主会话会读取报告进行审批，随后整体按钮测试。
