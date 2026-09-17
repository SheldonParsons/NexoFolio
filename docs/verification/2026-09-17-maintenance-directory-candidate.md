# 本轮候选目录（未发布）

这是真实模型产出的目录候选，已通过结构与引用校验；字段语义目标尚未完成。

- 策略：insert。
- 新增11个目录，安排39个接口。
- 接口/参数描述、参数关系、枚举注释：均为0。
- 原始接口、环境、定义、样例没有被改写。

## 目录概览

| 目录 | 接口数 | 模型说明 |
| --- | ---: | --- |
| 认证 | 2 | OAuth 令牌获取与登出接口 |
| 组织与用户 | 4 | 组织架构、用户信息、菜单与业务部门相关接口 |
| 滑块验证 | 3 | 滑块验证码启用、生成与校验接口 |
| 技术咨询 | 8 | 技术咨询生命周期相关接口，含派单规则、SKU码申请、咨询单处理 |
| 商机项目 | 2 | 商机项目信息查询与分页接口 |
| 商品与类目 | 7 | 商品类目树、商品分页与购物车数量接口 |
| 通知公告 | 3 | 消息与公告未读数相关接口 |
| 配额 | 2 | 销售配额类目列表与配额明细分页接口 |
| 工作流 | 3 | Activiti审批模板与任务列表接口 |
| 工具 | 4 | 字典、地址、表格配置与文件上传工具接口 |
| 系统入口 | 1 | 网关根路径接口，可能为前端入口或健康检查 |

## 接口归属

### 认证

OAuth 令牌获取与登出接口

- `DELETE /gateway/gree-business-order-auth/oauth/logout`
- `POST /gateway/gree-business-order-auth/oauth/token`

### 组织与用户

组织架构、用户信息、菜单与业务部门相关接口

- `GET /gateway/gree-business-order-admin/admin-api/v1/org/getCompanyOrgTreeList`
- `GET /gateway/gree-business-order-admin/admin-api/v1/sysMenu/userMenu`
- `GET /gateway/gree-business-order-admin/admin-api/v1/users/getUserInfoForConsult/{param1}`
- `GET /gateway/gree-crm-activiti/admin-api/v1/actBusinessSector/getBusinessSectorByTenantId/{param1}`

### 滑块验证

滑块验证码启用、生成与校验接口

- `GET /gateway/gree-business-order-admin/admin-api/v1/sliderCaptcha/sliderCaptchaEnable`
- `POST /slider/check`
- `POST /slider/gen`

### 技术咨询

技术咨询生命周期相关接口，含派单规则、SKU码申请、咨询单处理

- `GET /gateway/gree-business-order-consult/consult-api/v1/manager/consultTechnology/detail/{param1}`
- `GET /gateway/gree-business-order-consult/consult-api/v1/manager/consultTechnology/viewTechnologicalConsult/{param1}`
- `POST /gateway/gree-business-order-consult/consult-api/v1/client/consultDispatchRule/checkDispatchRuler`
- `POST /gateway/gree-business-order-consult/consult-api/v1/client/consultTechnology/saveAndSubmitConsultTechnology`
- `POST /gateway/gree-business-order-consult/consult-api/v1/manager/consultSkuCodeApplication/pageList`
- `POST /gateway/gree-business-order-consult/consult-api/v1/manager/consultTechnology/page`
- `POST /gateway/gree-business-order-consult/consult-api/v1/manager/consultTechnology/receiveOrder`
- `PUT /gateway/gree-business-order-consult/consult-api/v1/manager/consultTechnology/completeTask`

### 商机项目

商机项目信息查询与分页接口

- `POST /gateway/gree-business-order-oppty/oppty-api/v1/pc/opptyProjectInfo/getProjectInfoByParam`
- `POST /gateway/gree-business-order-oppty/oppty-api/v1/pc/opptyProjectInfo/pageByUserId`

### 商品与类目

商品类目树、商品分页与购物车数量接口

- `GET /gateway/gree-cos-commodity/admin-api/v1/commodityCategory/getCategoryTree`
- `GET /gateway/gree-cos-commodity/admin-api/v1/commodityCategory/getCategoryTreeByGreeEnableCommodity`
- `GET /gateway/gree-cos-commodity/admin-api/v1/commodityCategory/getCategoryTreeWithinUserProductLinePermissions`
- `GET /gateway/gree-cos-commodity/admin-api/v1/commodityCategory/getCategoryTreeWithinUserProductLinePermissionsV2`
- `GET /gateway/gree-cos-commodity/admin-api/v1/commodityCategory/getDeliveryCategoryTree`
- `GET /gateway/gree-cos-commodity/admin-api/v1/commodity/commodityPageForConsultOrderV2`
- `GET /gateway/gree-cos-commodity/admin-api/v1/shoppingCart/getUserShoppingCartNumber`

### 通知公告

消息与公告未读数相关接口

- `GET /gateway/gree-cos-notice/admin-api/v1/messagePushRecordDetail/getUnreadMessageCount`
- `GET /gateway/gree-cos-notice/admin-api/v2/announcement/client/getUnReadCount`
- `GET /gateway/gree-cos-notice/admin-api/v2/announcement/manager/getUnReadCount`

### 配额

销售配额类目列表与配额明细分页接口

- `GET /gateway/gree-cos-quota/admin-api/v1/manager/salesQuotaProductionCategory/list`
- `POST /gateway/gree-cos-quota/admin-api/v1/client/salesQuotaDetail/page`

### 工作流

Activiti审批模板与任务列表接口

- `GET /gateway/gree-crm-activiti/activiti-api/v1/actApprovalTemplateDetail/listByFormKey/at2510292900001-19`
- `GET /gateway/gree-crm-activiti/activiti-api/v1/activitiFlow/listTasksByInstance`
- `GET /gateway/gree-crm-activiti/activiti-api/v1/activitiFlow/listUserTodoTasks`

### 工具

字典、地址、表格配置与文件上传工具接口

- `GET /gateway/gree-business-order-client-tool/client-tool-api/v1/user/table-config-info/find/{param1}`
- `GET /gateway/gree-business-order-tool/admin-api/v1/toolAddress/get3LevelAddress`
- `GET /gateway/gree-business-order-tool/tool-api/v1/toolDictItem/select`
- `POST /gateway/gree-business-order-tool/tool-api/v1/file/uploadFiles`

### 系统入口

网关根路径接口，可能为前端入口或健康检查

- `GET /`

## 验收意见

目录组织具备初步检索价值；认证与滑块验证是否归到同一安全目录、单个根路径接口是否应保留待分类，仍可人工讨论。没有为了通过验收而改写模型方案。

此候选尚未完成字段知识维护：模型没有直接回读fact、field或source，也没有输出语义修改。不能把目录候选ready当成完整新文档已经交付。
