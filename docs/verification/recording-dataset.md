# 清空旧录制与固定测试数据集

工具：`scripts/recording_data.py`。只管理选定本地 Docker Compose 项目的 PostgreSQL 和 capture_data 卷；不读取/输出登录凭据，不操作 Chrome profile 或插件存储。

## 清空一次，重新录制

默认保留用户、会话Token、项目、项目权限、环境及名称别名；清空接口定义、观测、去重回执、证据、原文/图片、正式/候选目录、发布和重构历史。`--scope all`连用户、会话、项目、环境一起清空，但保留数据库Schema与SQLx迁移记录。

```bash
python3 scripts/recording_data.py inspect
python3 scripts/recording_data.py --keep-stopped reset --scope recording
```

脚本按照Compose标签核对唯一db/api/worker及共享文件卷，停止API/worker，确认无其他数据库/文件卷客户端，先在`backups/时间-随机ID/`保存完整数据库和文件卷，并校验备份，再事务清空明确列出的表。未知业务表会拒绝执行，不使用CASCADE扩大删除范围。文件清理后再次检查表为空，保留表计数未变。任何失败均保持服务停止，不能把部分清空当作成功。

`--keep-stopped`用于新一轮录制准备：清空后继续停止API/worker，避免旧插件队列回灌。正常不带该选项时，只重新启动本次操作前正在运行的服务；原本已停止的服务不会擅自启动。

插件旧上传队列与服务器是两个存储，不能仅靠清空服务器解决。插件任务提供[开发维护清理说明](/Users/sheldon/Documents/GithubProject/AsyncTestFetcher/docs/capture-reset.md)：临时无采集/上传的维护后台、用户重载原扩展、清items/batches/assets且保留设置/producer、恢复正常后台。该工具不自动操作Chrome。清理成功且正常插件/后端已更新后，保持活动页在未授权页面，恢复服务，再进入绑定页面开始录制。没有待上传旧队列时也应核对数量，不能用面板20条列表推断队列为空。

## 录完固定一份数据

完成录制后先离开授权页面，让上传完成；保存基线时停止新的业务操作：

```bash
python3 scripts/recording_data.py snapshot backups/recording-baseline-20260916
```

快照目标目录必须不存在，防止覆盖唯一基线。包含：

- `database.dump`：Schema、迁移记录、原始ID/回执、接口、证据、目录和任务状态。
- `blobs.tar.gz`：同一停写边界的原文文件，含原有目录权限与属主。
- `manifest.json`：文件SHA-256、数据集ID、各表数量、运行镜像ID及原文已缺失数量。

保存录制基线时，用独立`recording_dataset`引用固定所有仍有原文的观测，避免其被24小时原文回收删除。已经过期的原文无法凭空恢复；manifest的missing_raw_events不为0时，不能将该基线宣称为完整原文。临时关联索引仍按既有策略回收，不将它作为唯一原始资料。

快照保存时保留当前处理状态；pending任务不会假装已完成。若希望得到处理完毕的基线，先等上传和worker处理结束，再保存。快照包含真实资料与会话数据，目录默认700、文件600，并已在.gitignore排除，不提交仓库。解密会话所需的原环境密钥不复制进快照，隔离服务需单独配置；无需把它写入日志或文档。

## 后端测试复用

保留基线目录不动，每轮写测试先恢复到独立Compose测试实例：

```bash
python3 scripts/recording_data.py --project nexofolio-replay --keep-stopped restore backups/recording-baseline-20260916
```

目标Compose须先配置好唯一db/api/worker和其专属capture_data命名卷，db运行、API/worker可先停止。工具不会临时创建或猜测测试服务配置。恢复前校验文件哈希和tar路径，自动备份目标现状；数据库恢复为单个事务，恢复文件属主，核对各表数量。目标Schema不同则拒绝，改为恢复到空的隔离数据库，再执行新版迁移。恢复后按本轮测试需要启用API/worker；不要把用户正在录制的主实例当作反复覆盖的测试库。

需要重新录制时：归档上一份基线→暂停服务/处理旧插件队列→再次reset→录制→保存到新的快照目录。固定数据只能覆盖录制到的场景；插件采集能力修复或需要新业务场景时，才需要补录。

## 已验证范围

`python3 tests/integration/recording_data.py`使用一次性PostgreSQL及带独立标签/命名卷的占位API/worker测试容器，验证：快照固定原文、reset自动备份、保留表逐行不变、recording/all两种范围、重复清空、数据库和原文件恢复、权限属主恢复、损坏快照拒绝、未知表拒绝，以及服务停止/恢复。它不操作真实Chrome，不把占位容器测试等同于业务API上线验收。

## 2026-09-16：空卷重建权限修复

实际录制发现：清空后capture_data完全为空，重建容器触发Docker从镜像初始化卷目录，使属主变为10001，而运行用户为501，导致上传503。已按部署的storage-init恢复501:20/700，插件保留队列自行重试。reset现在保留/创建非业务标记`.initialized`，避免空卷重新初始化权限；隔离测试增加真实后端镜像挂载后的501用户写入检查。数据库健康不等于原文卷可写，重建容器后需同时验证存储初始化。

## 当前固定基线

用户于2026-09-16完成的真实录制已固定在本地忽略目录 `backups/recording-baseline-20260916/`，数据集 `7e2e9be1-070e-4a57-b799-7b5ed96cb6f4`。包含34个接口、179条观测（70 HTTP，其余109条页面/交互/局部快照），179份观测原文已固定；145个内容文件逐一通过SHA-256校验，缺失原文0。保存后服务恢复并通过健康检查。

后续后端测试优先使用该基线的隔离副本；勿将再次清空、重录当作默认准备步骤。备份成功只证明数据保存与完整性，不证明枚举、参数关系或重构结果已正确。用户后续新增录制不自动覆盖此快照。
