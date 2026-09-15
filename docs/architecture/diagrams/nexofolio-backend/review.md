# 本轮架构图交付记录

日期：2026-09-15。范围：后端本地工作区的逻辑架构。

```text
diagram_type: architecture
output: /Users/sheldon/Documents/GithubProject/NexoFolio/docs/architecture/diagrams/nexofolio-backend/backend.html
specification_sha256: 0c466f9c275c61c933bc9a805f61d19fdc42848395b9dfd4f06a80b1bfbf85ca
artifact_sha256: 671ea08e38c30b84f99513b9313922b8d67e8bbc110189d7c5d8327c614aa97f
validation: 9/9 showcase, 0 errors, 0 warnings
browser_evidence: passed
visual_review: passed
correction_rounds: 1
```

## 检查范围

- Archify `doctor`：通过；已安装在用户技能目录，可在后续对话使用。
- 更新脚本实际执行通过；重新生成的 HTML 哈希与浏览器检查的 HTML 一致。
- 自动化 Chrome：1440×900、1600×1000、1920×1080、2048×1320 均无页面横向或纵向溢出。
- 明暗主题：查看了 1440×900 和 2048×1320 的四张实际截图，主线清楚，未见连线交叉、标签遮挡、节点内容裁切或底部说明卡溢出；大屏内容分布完整。
- 视觉复核是图像阅读结果；本轮没有逐项操作搜索、聚焦、导出等交互，不声明这些功能经过完整验收。自动化回执的 visualReview 保持 pending，以独立保留工具证据；本文件记录额外图像复核。
- 本轮仅添加图、图源、生成脚本和说明；未修改业务代码，未运行后端业务测试，未提交或部署。

## 修正

第一版校验发现三条竖直关系的标签靠近节点。只调整对应标签偏移后，全部 9 项检查通过。

## 后续更新

本记录仅对应上述哈希。JSON 或 HTML 改动后重新生成、运行浏览器检查，并刷新本记录。生成成功不代表当前业务功能已通过运行验收；来源依据见 sources.json 和 README.md。
