# 正式目录、固定待分类和版本切换

每个项目创建时即有正式目录状态，初始version_id=null、generation=0。固定“待分类”的UUID永久保存在project_catalogs；名称由系统定义，接口无改名、移动、删除能力。数据库保护其ID和归属，LLM不能生成同名节点或占用系统ID。

接口完成建档即属于正式视图：没有当前版本业务目录归属的接口，自动属于固定待分类，无需另外等待任务、模型或补写归属。发布时只复制候选的节点/归属/合并展示组，并原子切换current_version_id。未覆盖接口继续待分类。接口ID、环境当前修订、原始样例和差异不被目录操作修改。合并组仍是展示关系，每个成员独立。

所有项目有访问权的登录用户可以发布/回退（用户已明确确认，沿用环境管理权限）。发布/回退均需expected_generation与request_id；重复原请求返回原回执，修改请求但复用ID409。成功后需重新GET当前状态，因为回执可能是旧请求的重放。状态行锁与generation防止并发覆盖。项目权限在写事务内重查，撤权/跨项目操作不能绕过。

## HTTP

- GET /v1/projects/{project_id}/catalog：状态、系统节点、业务节点、直属数量、合并展示组。
- GET /v1/projects/{project_id}/catalog/interfaces：可选directory_id、expected_generation，page/limit，返回实时接口与各环境当前修订引用。无directory_id为全部。参数页码1..100000、大小1..100。
- GET /v1/projects/{project_id}/catalog/versions：已发布版本列表，page/limit。
- POST /v1/projects/{project_id}/catalog/publish：{task_id,expected_generation,request_id}。须ready、结构复检通过、同项目且所有接口仍属于该项目。
- POST /v1/projects/{project_id}/catalog/restore：{version_id,expected_generation,request_id}。version_id必须显式提供，null回初始全部待分类；UUID只能引用该项目已发布版本。

响应写回执为{request_id,generation,version_id,replayed}。同目标无变化时generation不变。版本历史保留，初始null状态无需历史行，始终可选。401无凭证，403无项目权，404未知或跨项目资源，409代次/幂等冲突，400/422非法输入，503存储或权限服务不可用。读取禁止缓存，前端不得因GET失败重新POST。

契约位于contracts/catalog-preview（1.3.1），包括官方目录/接口页/历史/发布/回退/回执Schema及生成TS。documents1.3.0的classification反映当前正式归属，候选detail.published表示“当前生效”，而非曾发布。

常规目录此轮仍只有读取/版本切换，没有编辑端点。system节点locked=true是固定保护；普通节点locked=false不代表本轮已有编辑API。旧目录页和候选分别保留访问入口。
