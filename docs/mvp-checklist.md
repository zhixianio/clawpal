# ClawPal MVP 验收清单

## 1. 安装向导

- [x] 打开 Recipes 列表
- [x] 选择一个 Recipe
- [x] 参数校验阻止非法输入
- [x] 点击 Preview 显示变更
- [x] 点击 Apply 成功写入并生成历史快照

## 2. 历史与回滚

- [x] 历史列表可见最近记录
- [x] 选中历史项可预览回滚 diff
- [x] 执行回滚后回填新快照（用于再次回滚）
- [x] 回滚后配置文件发生可见变化

## 3. Doctor

- [x] 运行 Doctor 返回至少一项问题（如有）
- [x] 对语法/字段问题展示修复建议
- [x] auto-fix 的问题可点击 fix，状态刷新
- [x] 关键问题导致状态 score 下降

## 4. 可交付性

- [x] 无需网络也能完成核心流程
- [x] 目录存在于 `~/.openclaw`，历史文件落在 `.clawpal/history`
- [x] `npm run build` 成功
- [ ] `npm run release:dry-run` 输出通过项（无需执行发布）

## 5. 模型与频道管理（v0.2）

- [x] 模型 Profile 支持列表、创建、更新、删除
- [x] 全局模型绑定可设置与清空
- [x] Agent 模型覆盖可设置与清空
- [x] Channel 模型绑定可设置与清空
- [x] Channel 节点可更新 `type/mode/allowlist/model`
- [x] Channel 节点可安全删除
- [x] Recipes 支持外部文件/URL 源加载

## 6. Memory 与 Session 管理（v0.2）

- [x] Memory 文件列表可见
- [x] Memory 单文件删除与清空可用
- [x] Session 文件列表可见（active + archive）
- [x] Session 单文件删除与按 agent/全部清空可用

## 7. OpenClaw 安装引导（v0.4）

- [x] Home 提供 Install OpenClaw 入口
- [x] 支持四种安装方式选择：Local / WSL2 / Docker / Remote SSH
- [x] 创建安装会话后可执行 precheck/install/init/verify 四步流程
- [x] 每步显示执行结果、错误态重试入口、命令摘要
- [x] 完成 `ready` 后可直接衔接 Doctor/Recipes 配置流程
- [ ] 四种方式接入真实执行器（当前为可审计命令计划与流程骨架）

## 8. Recipe Authoring Workbench（v0.5）

- [x] 内置 recipe 可 `Fork to workspace`
- [x] Workspace recipe 支持 `New / Save / Save As / Delete`
- [x] UI 可直接编辑 canonical recipe source，并通过后端做 validate / list / plan
- [x] Studio 支持 sample params 与 live plan preview
- [x] Draft 可直接进入 Cook 并执行
- [x] Runtime run 可追溯到 `source origin / source digest / workspace path`
- [x] 至少一个 workspace recipe 可在 `Source / Form` 模式之间往返且不丢关键字段
