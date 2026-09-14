# 禅道认证、项目同步与权限调研

日期：2026-09-14。状态：已完成源码/官方文档调研，并使用用户提供的一个账号完成真实认证、用户信息、项目分页与可见范围验证。动作权限、跨账号隔离及撤销传播尚未验证。实测见第 9 节。

## 1. 来源和证据边界

用户指定参考 AsyncTest 前端测试报告生成模块。
本轮检查 `/Users/sheldon/Documents/GithubProject/AsyncTest-front-dev`，HEAD 为 `fa31d3a`，检查时工作区干净。
仅阅读该仓库源码，未修改 AsyncTest，未读取浏览器存储中的账号密码或已有 Token。
代码说明调用意图，官方文档说明标准契约；二者均不替代目标实例响应验证。

关键入口：

| 文件 | 位置与作用 |
| --- | --- |
| `src/views/generator/report/panels/ReportConfigPanel.vue:20` | 输入禅道账号密码，测试连接，产品/项目/执行下拉 |
| `src/views/generator/report/state.ts:34` | 禅道 REST API v1 基地址 |
| `src/views/generator/report/state.ts:223` | Token 请求头、响应 Token 提取、列表转选项 |
| `src/views/generator/report/state.ts:1241` | 账号密码获取并缓存 Token |
| `src/views/generator/report/state.ts:1312` | 按产品和 doing 状态请求项目列表 |
| `src/views/generator/report/state.ts:761` | AsyncTest 自身登录状态检查，非禅道项目授权 |
| `electron/preload.js:174`、`electron/generator/ipcMain.node.js:107` | 报告生成调用 Electron Main 的采集任务 |
| `electron/generator/zendaoRun.node.js:219` | Main 侧再次获取禅道 Token，用于缺陷统计 |

## 2. 现有调用契约

目标地址来自源码：`https://ztpm.gree.com:8888/api.php/v1`。
这是 API 路径版本，不能由此推导禅道软件发行版本或版本类型。

| 能力 | 现有请求 | 当前处理 |
| --- | --- | --- |
| 获取 Token | `POST /tokens`，JSON `{account,password}` | 提取非空 Token，Renderer 缓存在 state |
| 产品列表 | `GET /products` | 转为 id/name 下拉选项 |
| 项目列表 | `GET /projects?product={productId}&status=doing` | 一次请求，未遍历分页 |
| 执行列表 | `GET /projects/{projectId}/executions` | 转为执行下拉 |
| 测试单 | `GET /testtasks?product={productId}` | 报告选择使用 |
| 执行版本 | `GET /executions/{executionId}/builds` | 报告选择使用 |

除获取 Token 外，请求头为 `Token: <zentao_token>`、`Content-Type: application/json`。
注意禅道 Token 头与 NexoFolio MCP 的 `Authorization: Bearer <mcp_token>` 是不同凭证和协议入口。
Renderer 请求配置 15 秒 timeout；Main 的 requestJson 使用 fetch，当前未设置显式截止时间。

Renderer 尝试 token、data.token、sessionID 等多个位置；Main 又支持 Token 大写字段。
这些是兼容性尝试，不能证明目标实例实际返回所有形态。NexoFolio 应根据实测锁定解析器和测试样例，不将任意 sessionID 误当成可用 API Token。

## 3. 登录：可参考，但需补身份读取

现有报告模块只关心拿到 Token 后能否拉报告数据，没有请求当前用户身份。
官方 v1 文档提供 `GET /user`，Token 头认证，返回 `profile`，其中包含 id、account、realname、avatar 等字段。
建议链路：

```text
POST /tokens
  → 校验 HTTP 状态和 Token 结构
  → GET /user
  → 校验 profile.id / account 等身份字段
  → 映射本地用户
  → 签发本平台会话
```

本地外部身份以禅道实例和 profile.id 为依据，账号作为业务标识保存，不能只信任客户端输入的 user_id。
只保留业务需要的基础字段，不要求保存生日、住址等完整 profile。
失败响应、空 Token、HTML 登录页、401/403、网络故障分别处理。一次请求失败不能解释为禁用用户或清空其权限。

现有报告模块会把密码写入 localStorage 的草稿（state.ts:731），NexoFolio 不沿用该保存方式；普通密码仅用于当次认证。
超级密码的本地已有用户分支与此流程独立；它不产生一个可供后续查询禅道的个人 Token。

来源：[获取我的个人信息](https://www.zentao.net/book/api/665.html)。本轮已阅读页面，未验证目标实例。

## 4. 项目同步：不能直接复用报告下拉的结果

报告模块的产品联动和 doing 筛选服务于报告生成，不等于完整项目同步。
normalizeOptions 只保留 value/label，舍弃了分页总数、项目状态和父级等字段；无法用其结果判断全部同步完成。

官方项目列表给出 page、limit 参数，响应含 page、total、limit、projects；默认 limit=20。
建议从不携带产品筛选和 doing 限制的项目列表开始实测，并验证服务端是否仍有默认状态限制。
根据返回分页信息拉完当前身份有权看到的范围，去重 external_project_id，记录完整性和查询身份。

最少映射字段：外部项目 ID、name、code、status、parent、model，内部继续生成稳定 project_id。
parent 在官方资料中指项目集，不能未经核实就作为另一个本地项目 ID。产品、项目、执行是不同对象，本平台的项目层级以禅道 project 为主。
产品关联可以后续补充；不能先假设每个项目只属于一个产品，或只有选择产品才能同步项目。

同步分开两件事：

- 项目目录同步：用明确授权的同步身份获取其覆盖的项目，更新项目元数据。
- 用户访问范围：根据该用户的有效权限判断，不能复用同步账号的全部项目作为普通用户可见集合。

没有全局同步账号时，可以按登录用户当前可见范围同步，但不能称为全公司项目全集。
某页失败、分页未完成、身份权限收窄或某账号看不到项目，都不能直接删除本地项目。

来源：[获取项目列表](https://www.zentao.net/book/api/699.html)。本轮已阅读页面，目标实例对筛选参数的实际支持仍需验证。

## 5. 项目权限：现有模块尚未覆盖

代码中 ApiCheckPermission 调用 AsyncTest `/token/check`，用于 AsyncTest 登录状态。
没有在报告模块或其 Main 采集器找到禅道当前用户权限、项目团队、白名单或独立权限分组查询。
因此只能确认“使用某个用户的 Token 请求项目列表”，不能确认项目列表的过滤规则或任何写权限。

官方权限说明区分：项目访问控制、视野限制、系统权限分组、项目独立权限，以及继承/重新定义模式。
负责人、团队、干系人和白名单均可能影响访问；仅看 PM、acl 或团队列表不能重建完整有效权限。

对 NexoFolio 的结论：

1. 先验证普通用户 Token 返回的项目集合是否真的受该用户可见范围限制，包括公开、私有和视野限制情况。
2. 项目可见不等于可以写接口文档、裁决冲突、发布目录或签发写 Token。这些是 NexoFolio 的动作，必须明确对应禅道哪个权限依据。
3. 当前源码和已查文档尚未给出目标实例可直接调用的“当前用户有效项目动作权限”完整接口，不能猜测并实现 `/permissions` 等地址。
4. 若标准接口只能取得可见性，需要取得目标版本的有效权限来源，或再决定有限映射规则；本轮不默认授予所有动作。
5. 若最终需要禅道扩展接口，最好由禅道自身计算有效权限，本平台消费归一化结果；这是候选方案，尚未确认需要或可实施。

来源：[项目的权限维护和访问控制](https://www.zentao.net/book/zentaopms/555.html)。本轮已阅读官方说明，目标版本的具体规则仍需核实。

## 6. 权限持续更新与凭证生命周期

以下是调研阶段提出的持续刷新方案，已被用户最新决策覆盖：内部登录 Token 有效期 3 个月，未过期时登录原样复用、不续期；业务请求仅检查内部 Token 和本地授权。项目/权限仅在明确同步时更新，不因普通请求访问禅道，也不承诺禅道撤权实时生效。裁决授权暂不讨论。

只在登录时拿一次可见项目，无法保证长期 MCP Token 在禅道权限撤销后及时失效。
需验证个人禅道 Token 有效期、是否可续期，以及用同步身份查询指定用户有效权限的能力。
个人 Token 如需保存，只能以独立的受保护集成凭据保存，不能返回给 MCP 客户端，也不保存普通密码用于后台无限重登。
如果没有个人 Token 的长期更新办法，也没有服务身份的权限查询，必须明确权限快照有效期和失效后的重新认证流程，不能声称实时跟随禅道。
超级密码登录不绕过原用户项目授权。没有可用、可信权限快照时，不能用应急登录自动扩大权限。

当前 ProjectPermissionSource::grants_for(user_id) 能作为业务端口，但后续适配器必须能解析外部用户映射并提供权限依据/有效期。
ProjectSourcePage 可能需要增加同步身份与覆盖范围信息；具体字段待真实响应核实后调整。

## 7. 下一轮最小响应验证

使用用户明确提供的测试账号，所有业务请求只读；唯一 POST 是获取认证 Token，不创建或修改禅道项目/用户/权限。
先验证一组普通账号，建立身份和项目响应样本；如需证明隔离，再使用两个可见范围不同的账号，以及用户明确指定的允许/不允许访问项目。

| 顺序 | 请求/检查 | 需要记录的证据 |
| --- | --- | --- |
| 1 | `POST /tokens` | HTTP 状态、响应字段路径、Token 是否非空；不保存秘密 |
| 2 | `GET /user` | profile 形态、稳定用户 ID、账号/状态字段；只保留必要脱敏样例 |
| 3 | `GET /projects?page=1&limit=2`，接续页 | 实际分页字段、分页是否生效、是否重复/漏页 |
| 4 | 不同筛选与报告筛选做对比 | 是否默认只返回 doing，product/status 参数是否有效 |
| 5 | 用户指定的项目详情 GET | 可见/不可见项目的行为、是否能获得权限相关字段；端点需按目标版本文档确认 |
| 6 | 两账号范围对比及现有权限界面只读核对 | 公开/私有/视野限制、角色行为差异；不通过试写验证权限 |

凭证可以由用户写入仓库外的本地临时文件后提供路径；只用于目标禅道，工具输出和调研文档不包含账号密码或完整 Token。
不需要为了响应调研在 AsyncTest 仓库新增测试框架。本轮仅在 NexoFolio 保存研究文档；后续测试目标与产物位置另行明确。

## 8. 首轮文档调研结论（实测前）

- 登录：已找到现有 POST /tokens 调用，官方 /user 契约可用于补齐身份；等待目标响应验证。
- 项目同步：已找到 GET /projects，明确需要分页、筛选和身份范围处理；等待目标响应验证。
- 项目权限：已确认报告模块没有完整实现，缺有效权限接口与动作映射证据。
- NexoFolio 业务代码：本轮未修改，没有把文档调研当作接入完成。

## 9. 用户提供凭证后的真实只读验证

证据：[脱敏请求摘要与响应字段类型](evidence/zentao-readonly-probe-20260914.json)。
共执行 19 次请求：2 次获取 Token，其余为只读 GET。未改用户、项目、权限或业务数据；凭证只用于源码中指定的目标实例。
密码从用户指定文件读取，Token 只保存在请求进程内存中；未保存完整 Token、原始用户资料或项目白名单人员信息。

### 登录与用户信息

| 检查 | 实际结果 |
| --- | --- |
| POST /tokens | HTTP 201，正文为顶层 token 字符串 |
| GET /user + Token 请求头 | HTTP 200，返回 profile 对象；不需要额外 Cookie |
| 身份 | profile.id 为整数；profile.account 与输入账号忽略大小写后相同，逐字比较不同 |
| 管理员标记 | profile.admin=false，deleted="0" |
| 未提供 Token 的 GET /user | HTTP 401 |

账号差异仅是本次样本中的大小写规范化，不据此假设所有实例都可任意忽略大小写。
本地映射使用 (实例, profile.id)，保存禅道返回的 canonical account；不要因为输入账号大小写不同创建第二个用户。
请求成功使用默认 HTTPS 证书校验；本轮没有关闭 TLS 验证，也没有依赖浏览器已有 Cookie。

### 项目同步和状态

| 请求 | 实际结果 |
| --- | --- |
| /projects?page=1&limit=100 | total=10，返回 10 项 |
| 相同请求加 status=all | 与默认请求 ID 集合一致，total=10 |
| status=doing | total=2，返回 2 项 |
| status=all&page=1..5&limit=2 | 每页 2 项，累计 10 个不同 ID，无重复，total 始终为 10 |
| 未提供 Token 的 GET /projects | HTTP 401 |

当前账号范围内：closed=6、doing=2、wait=2；因此报告模块固定 doing 的逻辑确实不能覆盖本次全部项目。
本轮同步没有要求先选择产品。产品列表和带 product/doing 的报告请求均成功，但这不单独证明 product 筛选生效，仍需多产品集合对照。
这里的 10 个项目是该账号当前可见范围，不是公司或禅道实例的项目总数。

### 权限方面的新发现

实际 /user 响应比此前官方示例多出 profile.view，其中包含 programs、projects、products、sprints 等字符串字段。
profile.view.projects 是逗号分隔的数字 ID 字符串；解析得到 10 个 ID，与五页 /projects 读取结果完全一致。

这是当前用户有效项目可见范围的强线索，可以作为后续项目可见性适配和完整性对照的候选依据。
仅验证了一个普通账号和同一时间窗口，尚不能证明它在权限调整后实时更新，也不能证明其他账号的范围隔离。
不能假设空字符串表示全部可见；管理员和空范围行为需要单独验证。

这 10 个项目的 acl：private=5、program=1、open=4；auth 均为 extend。
profile 和项目列表没有出现可直接使用的 priv/right/permission 动作集合。可见性不能推导出 NexoFolio 的 Write、Adjudicate、Publish 或 IssueMcpToken 权限。
如果希望“能看到项目就能在本平台写接口”，那是需要用户确认的权限映射政策，不是本次响应已经证明的禅道权限。

### 解析和性能注意点

- 项目列表中的 openedBy 可为 null 或对象，lastEditedBy 为对象；不能一概按账号字符串解析。
- PM 在列表样本中为字符串，但所读项目详情中为 null；列表与详情不应共用未经验证的强制非空字段模型。
- whitelist 是数组，某个项目返回 1264 个用户对象；同步基础项目字段时无需展开存储这些用户信息。
- 白名单含用户不等于仅白名单用户可访问；还存在公开、项目集范围等机制，不能自行只靠该字段计算权限。
- profile 含 resetToken 和大量联系方式等无关字段；本平台只提取需要的身份与范围字段，不转存完整 profile。
- 本轮耗时仅是串行功能探测，不能当作并发性能基准或 SLA。

### 当前可以确定和仍需补齐的内容

可以据此编写目标实例的登录和身份解析契约样例，以及项目分页同步的测试样例；本轮尚未实现业务适配器。
项目读取权限可以围绕 profile.view.projects 与项目列表做下一步验证，但需要第二个范围不同的账号或明确的允许/禁止项目作为对照。
动作权限映射、禅道 Token 有效期/续期、权限变更的刷新行为、管理员/空范围行为仍未确定。
长期 MCP Token 的有效项目范围必须来自可持续刷新的权限来源，不能永久使用本次登录快照。
