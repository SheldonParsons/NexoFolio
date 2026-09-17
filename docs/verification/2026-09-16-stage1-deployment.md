# 阶段1本地部署验收

2026-09-16。用户授权本次同步前端合同及引用后，已更新本地API和worker。阶段1完成至本地部署；未进入阶段2，未提交或push。

## 前端最小同步

仓库：`/Users/sheldon/Documents/GithubProject/NexoFolio-Front`。

- 原前端使用documents/1.4.0，AJV严格限制`extractor_version=observed-http-1`；实际复现了它拒绝新版定义的问题。
- 将后端`contracts/documents`完整同步至前端`src/contracts/documents/2.0.0`，逐文件字节核对一致。
- 仅修改`src/features/documents/contract.ts`、`types.ts`的合同引用；将`tests/catalog-source.test.ts`中的现有定义读取测试参数化，同时验证新旧extractor版本。
- 没有修改布局、样式或新增分类展示功能。插件不需调整。

验证：14个测试文件、149项测试通过；本次修改文件格式检查通过；Vite生产打包通过。当前5173开发服务实际提供的合同引用已是2.0.0。

**完整`pnpm build`仍未通过类型检查**：`src/features/product-docs/markdown.ts`第35、36行有`string | number`传入string参数的问题。已在合同同步之前捕获相同报错，同步后诊断完全一致，不归因于本次修改，也未扩大范围修复。Vite打包通过不能替代完整类型检查通过。

## 后端部署

- 新镜像：`nexofolio-backend:stage1-20260916`。
- 镜像ID：`sha256:366ac3f52b449e7ca7437f5b5c808eafe44e992e776635c10b8b87d5cbb11e88`。
- 停止写入后备份当前数据库和文件卷：`backups/pre-stage1-deploy-20260916T074216Z/`。独立保存，未覆盖原始两份固定录制快照。
- 独立迁移命令执行`202609160001_observation_assessments.sql`，增加两个可空列，不回填历史判断。
- 在原Compose项目、原环境配置及原文件卷上替换API和worker。`deploy/.env`的BACKEND_IMAGE同步为新镜像，避免后续普通启动退回旧镜像。
- 保留旧镜像`nexofolio-backend:recording-baseline-20260916`。本次迁移仅增加可空列，如需退回旧程序可重新选择旧镜像；不得因此自动删除新接收数据或恢复覆盖数据库。

## 运行验收

| 检查 | 结果 |
| --- | --- |
| API /health/live、/health/ready | 200；Docker健康状态healthy |
| worker | running，启动后无ERROR日志、重启次数0 |
| 已登录读取接口详情 | 200，使用前端2.0合同验证通过 |
| 单条assessment读取 | 200，schema通过 |
| 分页assessment读取 | 200，schema通过 |
| 无凭证读取assessment | 401 |
| 历史未评估记录 | assessment仍为null，未伪造为重复或重算 |
| 数据量 | 所有业务表部署前后计数一致；39个接口、400条capture观测、1用户、2项目 |
| 迁移记录 | 12→13，仅新增本次迁移 |
| 两份固定录制备份 | database.dump与blobs.tar.gz的SHA256保持原值 |

部署验证只读取现有业务资料，没有为测试向主项目插入合成接口或触发模型。新结构判断的写入与后台处理已在阶段1隔离回放中验证；本次补足镜像运行、迁移和真实鉴权读取链路。

本地部署日志、HTTP校验、前后数据计数位于`experiments/results/stage1-deployment/`。凭据只在本地验证进程内使用，不写日志或文档。

## 当前用户可见效果

刷新前端即可使用兼容新版定义的读取逻辑；之后收到的新观测由新版后端处理。历史记录仍保留原有定义与未评估状态，前端没有新增逐字段分类展示页面。本阶段不涉及重构候选生成或发布。
