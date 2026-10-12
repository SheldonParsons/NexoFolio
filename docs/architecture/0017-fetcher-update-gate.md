# Fetcher OSS 版本准入

实现入口是实际 collect v1 `POST /v1/collect/batches`，在 intake.submit 之前做准入；不是旧版 ingestion/capture API。当前生产router无独立Fetcher图像资源PUT路由，用户头像等无关上传不纳入。本改动不动账号、项目权限、MCP或非Fetcher采集接口。

- 判断请求体platform=`nexofolio-fetcher`或存在`X-NexoFolio-Fetcher-Version`，按头中的实际运行版本校验；不能用历史body.platform_version推断当前客户端版本，不能改原批次哈希/ID。
- OSS固定 `https://asynctest.oss-cn-shenzhen.aliyuncs.com/nexofolio_fetcher/latest.json`，不采用网页下载目录环境变量，不重定向、不代理二进制。8秒HTTP超时（整个刷新9秒）、64KiB、Date偏差120秒/Age60秒边界；并发合并，成功缓存5分钟、失败10秒，不因缓存错误放行。
- revision/minimum/latest/fingerprint写入intake.fetcher_release_policy，原子单调更新；跨worker/实例/服务重启禁止策略倒退。记录清空脚本不能清这个策略表。API不会自动迁移；先用现有admin migrate流程应用202610120001_fetcher_release_policy.sql，再部署API。
- 清单原schemaVersion1各字段不变，新增policyVersion1、policyRevision、minimumVersion、releaseMode、releaseNotes。首次无策略字段最低0.0.0；缺字段/回退的新清单拒绝。普通发布不提高门槛且不降低旧门槛，强制发布提升至最新版本。
- 最低为0时允许没有运行版本头的旧正常Fetcher，避免普通发布隐式强制；最低>0时缺头不能证明合规，返回426 FETCHER_UPDATE_REQUIRED。版本过低/非法同样426。OSS/持久化/策略错误返回503 FETCHER_UPDATE_POLICY_UNAVAILABLE；均为写入前整批拒收，保留客户端队列和会话。
- 已入站获准的请求可能完成，不能声称回滚在途写入；门禁阻止后续不合规请求。已接收队列的既有正常worker处理不受此次门禁误杀。
- platform和头均为正常客户端的自报标识；在保留开放非FetcherAPI的前提下，不能声称阻止恶意改名伪装。未包含门禁代码的旧扩展本机捕获无法远程停止，但服务可拒收。

部署操作与公开强制策略发布分别执行，编译通过不代表完成了迁移、部署或真实 Chrome 验收。强制策略发布应包含已实现门禁的可下载插件；服务端拒收还需配套 API 完成迁移和部署。插件只从 OSS 检查更新，不增加到 NexoFolio 的更新查询流量。原网页下载接口仍保持现有响应，不被此门禁影响。
