# 无感、交错使用下的局部采样修复

用户不需要进入“录制会话”，也不需要连续完成预设业务场景。A进行中临时转去处理B，是正常使用方式；相邻事件、同一标签页、同一批次、短时间窗口或同值本身都不能证明共同意图。

## 本轮边界

- 插件支持有界的组件表单、同一弹窗内唯一可见表单的外置按钮、局部字段组和自定义下拉的实际DOM值/选项文字。
- 不读取框架私有状态，不把显示文字猜成枚举ID，不把placeholder当成selected；明确的data/ARIA/表单值缺失时保留unknown。
- 每次重新读取当前局部DOM，不缓存另一模块的控件或选项。隐藏页签、其他弹窗和并列独立面板不应混入当前快照；找不到明确归属时保留覆盖缺口。
- 同URL切页签记录实际交互线索，但不根据文本虚构业务任务或view。请求上下文在开始时冻结；晚到A响应不归给B。
- interaction_id仍只描述所观察事件派发期间的请求。不改写Promise/定时器，不延长“最近一次点击”的有效期；无依据的异步、防抖、轮询请求独立保存。
- FormData本轮保持原有元信息投影和unreadable状态，不抹掉可读的文件元信息，也不冒称获取了完整multipart/文件字节。
- 不新增截图，不清理账号、绑定、旧队列或真实录制基线，不改网站前端，不运行真实业务页面自动操作。

采集合同包更新为1.4.0，wire仍为3、payload仍为1，Schema和生成类型无变化。公开规则见`contracts/capture/behavior.md`及`chrome-sampling-profile.md`；新增`fixtures/interleaved-use.json`展示同URL中独立A/B操作的乱序响应。

## 已完成的后端联调

采用一次性PostgreSQL与真实Rust HTTP接口，不修改主库。

1. 公共同URL交错A/B样例：B先完成、A晚到；A订单状态仍为“待审核”，B库存状态为“可用”，不能交叉套用。每项仍标inferred。
2. 插件新函数实际生成的13条合成组件记录，经hook→manager→converter输出后原样接收（只改隔离项目ID），全部accepted、worker全部完成、回执重试全部replayed。
3. `/api/orders`的status=1得到“待审核”候选及显式string_to_number；`/api/b`仅得到自己的“B状态”控件标签，不能借用A。
4. `/poll`、`/promise`、`/timer`、`/debounce`没有UI字段误绑定。
5. fmt、clippy、合同生成/版本校验、证据单元检查和capture_evidence数据库回归通过。

联调样本：`/Users/sheldon/Documents/AsyncTest/ast-testing-core-data/nexofolio-fetcher-20260916/business-operation-batch.json`。
SHA-256：`81fc3887f7b8744c36cb6d6b024c27c8d5c78c3fdf5dea99b105dbd8b4ea8083`。
后端日志：`experiments/results/passive-component-capture/`；此前公共交错样例日志：`experiments/results/interleaved-capture/`。

## 验收界限

以上真实的是HTTP服务和数据库；浏览器侧输入来自组件DOM/Chrome替身，并非真实平台操作。现有用户录制基线保持不变，不能凭缺失的历史DOM复原新采样效果。正常插件构建完成后，用户重新加载一次，照常使用产生的新数据再用于检查真实业务覆盖，不要求用户按预设流程“构建场景”。

## 插件交付结果

插件任务完成20项新增业务组件/中断边界检查、15项既有最小采样与28项队列/生命周期回归，typecheck和正常MV3构建通过；3个合同fixture通过AJV，双方1.4.0文件哈希一致。插件开发构建在 `/Users/sheldon/Documents/GithubProject/AsyncTestFetcher/.output/chrome-mv3`，不是维护构建，尚未发布/提交。

用户重新加载原插件一次即可照常使用；无需清空队列、重新登录、重新绑定或重复整套录制。本轮代码与模拟/协议检查通过，但真实平台DOM覆盖仍以正常使用的新数据核实。旧179条观测测试基线不变。
