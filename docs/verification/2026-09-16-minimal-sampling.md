# 最小采样与公共采集合同验证（2026-09-16）

本轮只收敛插件采样、公共合同及后端控件关联；不调整前端，不启用LLM，不修改真实项目数据，不发布版本。此前工作区已有的统一重构改动继续保留，不能把整个工作区diff都算成本轮改动。

## 实现

- Capture包1.3.0：wire仍为3、payload仍为1，Schema和生成类型未变。公共行为、Chrome采样规则和两个合成例子纳入manifest哈希，双方同步同一份合同。
- 插件：关闭新增截图及其专用权限申请，移除整页扫描与input/mutation/scroll/resize独立采样；保留页面元信息、关键交互、按钮/提交时的局部表单快照。一次操作的请求复用快照，不逐请求扫描DOM。
- HTTP在开始时固定原始时刻、页面和操作标识；响应晚到不改归属。事件派发结束后不再沿用旧操作ID。异步请求可能缺少操作关联，仍保存HTTP并说明能力缺口。
- 后端：同时关联interaction目标和ui_snapshot控件，兼容两种到达顺序。直接读取事件原文，不依赖最多5组代表样例仍保留该事件。先排除同值控件/重复控件ID，再检查请求字段的唯一候选。控件歧义检查使用线性索引，不做控件两两比较。
- 数字/字符串转换记入候选，保留原类型、HTTP/UI两份来源和inferred性质；不能把界面值相等当作因果或完整枚举。旧控件绑定不能推导另一个接口或定义修订中的字段省略。
- 文件控件无法读取值时为unknown；程序触发的快照也保留PROGRAMMATIC_EVENT。历史图片、旧队列、v1/v2重试及20条展示与上传队列分离保持兼容。

## 验证证据

后端完整检查日志：`experiments/results/minimal-sampling-final/`。fmt、clippy、五套合同和合同治理检查通过；workspace 75项通过、8项依赖外部条件的测试默认忽略；随后独立PostgreSQL中7项数据库集成测试通过。

最终控件歧义实现与最终插件样本的定点复验日志：`experiments/results/minimal-sampling-verified/`，包含合同、fmt、clippy、evidence单元测试和capture_evidence真实HTTP/数据库测试。临时PostgreSQL由测试创建并清理，未连接业务数据库。

后端采样矩阵覆盖：UI先到、HTTP先到、多控件同值、多字段同值、不同page/frame/view/browser/操作、无操作、时间倒序、超窗、不同producer/环境；另外验证跨接口不能伪造omitted、第三方无context的HTTP接收和重试不加支持次数。

插件任务报告：15项最小采样模拟检查、28项已有队列/生命周期回归通过，类型检查和MV3构建通过。模拟报告：

`/Users/sheldon/Documents/AsyncTest/ast-testing-core-data/nexofolio-fetcher-20260916/minimal-sampling-result.json`

跨仓库联调样本由插件实际hook、manager、converter在合成DOM/Chrome环境中生成：

`/Users/sheldon/Documents/AsyncTest/ast-testing-core-data/nexofolio-fetcher-20260916/minimal-operation-batch.json`

最终样本SHA-256：`280d4e22ef62d8a983fc86a976ec8f87fbd59ef0232865f56fb78e13b343fe5e`。整批22条只替换隔离项目ID，不改记录ID、context、时刻、值或limitations。通过真实Rust HTTP入口上传，worker处理后经HTTP读取证据/每条观测，验证status=1与“待审核”的候选映射、显式string_to_number、poll/Promise/timer样本无UI字段误关联及原回执重试。

复验此样本可在隔离测试数据库设置`TEST_DATABASE_URL`和`NEXOFOLIO_PLUGIN_SAMPLE_BATCH`后运行：

```bash
cargo test -p nexofolio-backend --test capture_evidence --locked -- --ignored --nocapture
```

未设置插件样本路径时，该集成测试仍执行仓库内公共样例与采样矩阵；不声称执行跨仓库样本验证。

## 尚未验收的边界

这是代码/模拟采样与真实后端协议联调，不是已安装Chrome扩展的真实业务页面录制。真实浏览器的注入时机、事件派发/框架异步行为、定制组件与跨frame表现仍需用户验收。本轮没有调用浏览器。

本地插件开发构建已更新；共享运行中的后端没有切换到本轮源代码，未提交或push。真实验收前需先更新本地后端服务，再重载插件（不清空历史队列）。前端保持不变。

实际验收建议只用一个有项目权限的订单页：选择状态→查询→打开详情，随后关闭侧栏、切页、断网恢复并检查原请求归属及上传。确认HTTP仍正常、无新增截图、局部表单值和来源可追溯、未关联项明确保留。通过后固定本轮生产者合同，后续优先优化后端证据解释；不能把“固定”理解为以后不修缺陷。
