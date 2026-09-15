# 下载接口加载到现用服务

用户确认主后端工作已停止，主后端任务也确认不再占用共享服务验证，随后执行本次 API 更新。

- 使用当前源码和锁定依赖构建 `nexofolio-backend:downloads-20260915`。
- 18982 临时容器用既有 Compose 配置验证 readiness、内部版和公网版下载接口，均通过。
- 将旧镜像保存为 `nexofolio-backend:before-downloads-20260915-104927`，更新 foundation 标签后，仅重建 API 容器。
- DATABASE_URL 与会话密钥保持原值；数据库迁移列表与源码一致，无新增迁移操作。
- Worker 和数据库容器 ID 未变化，命名数据卷保留；临时检查容器已停止并自动删除。
- 现用后端 `http://127.0.0.1:18080/v1/downloads` 和前端同源代理 `http://localhost:5173/api/v1/downloads`，internal/public 两渠道均 HTTP 200、全部 ready。
- Fetcher 0.1.0，AsyncTest 两渠道三平台均 3.3.10。

详细镜像/容器标识和响应摘要见同名 JSON。本记录证明下载接口已加载到现用 API，不代表全部业务流程重新验收。
