# OpenClaw 安装引导前置设计文档

## 日期
2026-02-24

## 背景

当前 ClawPal 主要服务于“已安装 OpenClaw 后的配置与运维”。
目标是把入口前移到“用户准备安装 OpenClaw”的阶段，覆盖尽可能完整的安装路径，并在安装成功后无缝进入现有配置功能。

## 目标与验收

### 产品目标

1. 让用户先选择安装方式（用户自选优先）。
2. 为每种方式提供 step-by-step 引导（检测、安装、初始化、验证）。
3. 安装完成后自动进入现有 Home/Recipes 配置链路。

### MVP 验收（本次确认）

1. 支持四类路径：Local / WSL2 / Docker / Remote SSH。
2. 成功标准：
- OpenClaw 可执行可调用。
- 最小配置目录存在且可读。
- 可返回基础 status。
- 自动跳转到现有配置入口（不要求 Doctor 必须通过）。
3. 每个步骤失败可重试并给出下一步建议。
4. 全流程提供可回看日志（敏感信息脱敏）。

## 方案对比

### 方案 A：独立全屏 Onboarding

优点：引导感最强，心智清晰。  
缺点：改动面大，与现有路由和状态耦合深。

### 方案 B：Home 增加 Install Hub（推荐）

优点：改动可控，兼容现有页面和能力，便于渐进迭代。  
缺点：首次启动“强制引导感”弱于全屏方案。

### 方案 C：把安装能力并入 Recipes

优点：可复用部分 recipe 机制。  
缺点：安装与配置语义混杂，长期可维护性较差。

最终选择：方案 B（Home 中新增 Install Hub）。

## 信息架构（IA）

### 入口

在 Home 顶部增加主 CTA：`Install OpenClaw`，进入 Install Hub。

### Install Hub 结构

1. 安装方式选择：
- Local
- WSL2
- Docker
- Remote SSH

2. 步骤面板：
- 按选中方式展示标准步骤
- 当前步骤状态、动作按钮、失败重试

3. 状态与日志面板：
- 进度、摘要、错误码
- 命令摘要（脱敏）

4. 完成态：
- `OpenClaw is ready`
- 按钮：进入 Home/Recipes

### 与现有功能衔接

安装成功后：
1. 刷新 instance status/config。
2. 自动返回 `home`。
3. 给出下一步建议：`Run Doctor` / `Apply first Recipe`。

## 流程与状态机

统一状态机（四种方式共用）：

1. `idle`
2. `selected_method`
3. `precheck_running`
4. `precheck_failed | precheck_passed`
5. `install_running`
6. `install_failed | install_passed`
7. `init_running`
8. `init_failed | init_passed`
9. `ready`

### 步骤定义

- `precheck`：环境与依赖检查
- `install`：执行安装动作
- `init`：执行最小初始化
- `verify`：验证可用性并产出可衔接状态

### 路径化步骤示例

- Local：OS/包管理器检查 -> 安装 -> `~/.openclaw` 初始化 -> status 验证
- WSL2：WSL 可达检查 -> WSL 内安装 -> 初始化 -> 连通验证
- Docker：Docker daemon 检查 -> 镜像/容器准备 -> 挂载初始化 -> status 验证
- Remote SSH：主机信息/连通性 -> 远程安装 -> 初始化 -> 回写到实例管理

## 数据模型与 API 设计

### 前端模型

- `InstallMethod = "local" | "wsl2" | "docker" | "remote_ssh"`
- `InstallStep = "precheck" | "install" | "init" | "verify"`
- `InstallSession`
  - `id`
  - `method`
  - `state`
  - `currentStep`
  - `logs[]`
  - `artifacts`
  - `createdAt`
  - `updatedAt`

### 后端命令（Tauri）

- `install_create_session(method, options?) -> session`
- `install_run_step(sessionId, step) -> stepResult`
- `install_get_session(sessionId) -> session`
- `install_cancel_session(sessionId)`
- `install_list_methods() -> methodCapabilities`

### stepResult 结构

- `ok`
- `summary`
- `details`
- `commands`（脱敏）
- `artifacts`
- `nextStep`
- `errorCode`

### 现有能力复用

1. 复用 `InstanceContext` 与现有 status 刷新链路。
2. `ready` 后触发一次统一刷新，自动回到 Home。
3. `remote_ssh` 安装成功可直接沉淀到现有 SSH Host 列表。

## 错误处理与风险控制

### 错误策略

- 步骤失败只阻断后续，不销毁会话。
- 错误分类：
  - `env_missing`
  - `permission_denied`
  - `network_error`
  - `command_failed`
  - `validation_failed`
- 每类错误提供明确修复建议与重试入口。

### 风险控制

1. 默认执行低风险、可逆动作。
2. 高风险动作（覆盖配置、远程提权）要求显式确认。
3. 全程命令日志脱敏，不记录明文 token/password。

## MVP 边界

### 本期包含

- 四种安装路径引导
- 通用四步状态机
- 会话级日志与可重试
- 安装完成后自动衔接现有配置流程

### 本期不包含

- 跨平台无人值守全自动安装
- k8s/多机集群等复杂拓扑
- 以 Doctor 通过作为安装完成门槛

## 实施建议（高层）

1. 先在 Home 落 Install Hub 壳层与状态机骨架。
2. 优先打通 Local + Remote SSH 的可用闭环。
3. 再补 WSL2 与 Docker 细节分支。
4. 最后完善错误码、日志脱敏、完成态跳转。

## 任务状态

- 当前任务目标：把安装场景前移并定义完整安装引导方案。  
- 预期验收项：四路径、分步引导、安装后无缝衔接配置。  
- 完成后状态：完成（待进入 implementation planning）。
