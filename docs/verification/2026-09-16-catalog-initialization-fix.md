# 当前目录404修复

2026-09-16。已修复本地两个项目的目录读取404，并修正重置脚本与增量构建依赖。此项关闭阶段3的目录初始化阻断，不代表其余快照输入问题已经修复。

## 原因与历史核对

`GET /v1/projects/{project_id}/catalog`路由与前端合同一致，代理可用。直接原因是已有2个projects，但project_catalogs为0；官方目录读取查不到状态返回NotFound/404。

旧recording清理路径保留projects，却TRUNCATE project_catalogs，没有重新初始化。数据库触发器只在INSERT projects时运行，因此保留的项目不会再次触发它。隔离测试已复现该缺陷。

本地备份链路支持这一来源：`20260916T021931Z-fa26887a`有2个项目状态；之后固定录制基线及阶段1/2部署前备份均为0。未取得完整历史操作日志，因此不作超出这一证据的执行历史断言。

修复前核实catalog_versions、knowledge_releases、两类发布回执、maintenance_runs、catalog_preview_tasks均为0。不存在应恢复而被此次初始化覆盖的历史关联。

## 处理方式

1. 停止写入并备份数据库及文件卷：`backups/pre-catalog-initialization-20260916T114424Z/`。
2. 运行有历史保护的幂等补齐SQL，仅为缺失状态的项目插入默认状态。没有清空任何项目数据。
3. 新增迁移`202609160003_repair_missing_project_catalogs.sql`。正常状态不变；若缺失状态同时仍有历史，则报错要求恢复权威状态，不猜测当前版本或系统目录ID。
4. recording清理在同一个事务里保存系统目录ID/代次、清理录制数据、恢复已有项目状态：目录ID保持，当前版本清空，代次递增；原本缺失的项目补齐默认状态。all清理仍清空全部项目。
5. 补充`crates/infrastructure/build.rs`跟踪migrations目录。此前本地增量产物未包含新增SQL，干净容器构建则包含；补充依赖后，本地admin和回归入口也正确嵌入新迁移。

未新增API或在读取路径中偷偷写状态，前端路径和合同没有改动。

## 验证与当前服务

- 带现有登录态，两个项目分别直连18080、通过5173/api代理访问，共4次200；均通过前端现用official.schema校验。
- 商用订单项目仍为39个接口，CRM为0；每个项目一个locked的待分类节点。无凭证仍401。
- 修复数据时只有project_catalogs从0变2，其他业务表计数不变。
- 新镜像部署迁移后，只有迁移记录从14变15。定义、接收原文和回执摘要与部署前一致；400条capture保留；两份固定录制备份SHA256不变。
- 隔离测试通过：正常状态不变、缺失状态补齐幂等、历史歧义拒绝、目录ID保留、重复recording清理、full清理、受保护用户/项目/环境数据、备份恢复及文件权限、未知表保护、服务重启。
- 阶段3隔离回放已用新迁移复测：不需要测试补行即可创建快照；观测字段和结构材料未接入审阅的问题仍存在，未调用模型。
- fmt、Python语法检查、infrastructure目标clippy和diff检查通过。

当前API/worker镜像：`nexofolio-backend:catalog-init-20260916`。
镜像ID：`sha256:b4fbf8a68dc3e4e8b0e0727dd2eeaf61aebec9ad163bea466e78cbab563232a9`。
本地Compose镜像配置已同步；迁移版本202609160003已登记。验证为实际认证HTTP及代理链路，未冒称完成浏览器视觉验收。

## 代码增删

相对本次404修复开始时的工作区，不计之前阶段3审计改动：

| 文件 | 新增/删除 | 理由 |
| --- | ---: | --- |
| `scripts/recording_data.py` | +19/−2 | 事务内恢复系统目录状态，修正清理成功条件 |
| `tests/integration/recording_data.py` | +37/−3 | 防止同一缺陷复发，验证历史保护、ID保持及幂等 |
| `migrations/202609160003_repair_missing_project_catalogs.sql` | +20/−0 | 一次性、有保护、可重复执行的状态补齐 |
| `crates/infrastructure/build.rs` | +4/−0 | 让新增迁移触发本地增量重编译 |
| `migrations/README.md` | +2/−0 | 记录迁移及清理语义 |

合计新增82、删除5，净增77行；另有本报告及阶段状态更新。Rust业务逻辑未改，没有新增服务、模块体系或前端行为。没有commit/push。

本地证据：`experiments/results/catalog-initialization-fix/`中的repair.json、http-checks.json、verification.json、reset-tests.log和stage3-after-repair.json。后端主线继续停留在阶段3，下一项是快照材料、字段来源与索引/回读合同对齐。
