# 首次观测路径识别

默认机械规则 path-template-1：完整段的UUID、32/64位十六进制串、无符号ASCII纯数字，替换成按出现顺序命名的{param1}/{param2}。v1、me、search、latest、带连字符日期、负数和百分号编码段保留原样；保留大小写、斜杠和尾斜杠。不调用模型，不扫描历史。

项目权限检查时同时读取一次path_policy，在同一批次内应用；先固定identity再取得去重锁。项目+环境+方法+模板决定比较对象，结构hash/方向比较再决定ignored或转交。结构变化仍进入待确认差异，不覆盖当前定义。路径识别不会改变原始URL、原始body或不可变重试hash。

项目默认enabled=true、numeric_segments=true。管理命令：

```bash
nexofolio-admin path-policy-show --project-id <UUID>
nexofolio-admin path-policy-set --project-id <UUID> --literal-prefix /reports
nexofolio-admin path-policy-set --project-id <UUID> --keep-numbers
nexofolio-admin path-policy-set --project-id <UUID> --disable
```

set为完整替换配置；不带开关恢复默认，可重复--literal-prefix。前缀按路径段边界匹配：/reports不影响/reports2。设置只影响后续观测，不撤销已被忽略的样例，也不改写历史文档或回执。URL已截断时不参数化。

inbox的path_identity固定当次识别结果，worker使用这个结果，不重新读取更新后的项目规则。旧inbox的NULL继续走字面路径，旧文档/候选保留。首次模板请求建立新的模板head；本轮不扫描/重键旧head、不物理归并旧接口，因此保留历史查询与原始ID的含义。旧重复记录由LLM候选合并组集中显示。

观测定义path使用模板，request.parameters增加in=path的中性参数名；URI参数的观测类型为string，不猜测业务字段名。真实值只在原始URL中。intake和knowledge共享contracts中的纯路径类型/规则，模型仅在后台候选生成阶段使用。
