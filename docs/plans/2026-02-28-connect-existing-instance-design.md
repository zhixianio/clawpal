# 连接已有实例 UX 改进设计

> 日期：2026-02-28
> 分支：refactor/gui-cli-agent-layers
> 状态：待实现

## 目标

改进 Start page 和 InstallHub 的"连接已有实例"流程：
1. 本地实例（Docker、WSL）自动发现并展示在 Start page
2. 远程 SSH 实例通过专用连接流程（替代当前 Docker-only 的连接表单）

## 设计原则

1. **本地自动、远程手动**：本地实例自动扫描发现，SSH 需要用户配置
2. **不阻塞用户**：扫描异步执行，超时跳过，不影响页面加载
3. **复用现有组件**：SSH 表单复用 `SshFormWidget`，注册复用 `connect_docker` / 注册表 API

## 1. 本地实例自动发现

### 1.1 后端：`discover_local_instances` Tauri command

新增 `src-tauri/src/commands/discover.rs`：

```rust
pub struct DiscoveredInstance {
    pub id: String,              // e.g. "docker:my-project"
    pub instance_type: String,   // "docker" | "wsl2"
    pub label: String,           // 自动推断的标签
    pub home_path: String,       // openclaw home 路径
    pub source: String,          // "container" | "data_dir" | "wsl"
    pub container_name: Option<String>,
    pub already_registered: bool,
}
```

扫描逻辑（按优先级）：

1. **Docker 容器扫描**：
   - `docker ps --format '{{json .}}'` → 解析 JSON
   - 匹配标准：容器名含 `openclaw` 或 `clawpal`，或 labels 中有 `com.clawpal.home`
   - 提取 home 路径：从 label 或 mount 推断
   - 超时 3 秒

2. **数据目录扫描**：
   - 扫描 `~/.clawpal/` 下的子目录
   - 匹配标准：目录中存在 `openclaw.json` 或 `docker-compose.yml`
   - 生成 instance ID：`docker:{dir-name}`

3. **WSL 扫描**（仅 Windows）：
   - `wsl --list --quiet` → 列出已安装 distro
   - 对每个 distro 检查 `wsl -d {name} -- test -f ~/.clawpal/openclaw.json`
   - 超时 2 秒

4. **与 registry diff**：对每个发现的实例，查 registry 是否已注册 → 设置 `already_registered`

### 1.2 前端：Start page 集成

- 页面加载时调用 `discover_local_instances()`
- 已注册实例保持当前卡片样式
- **未注册但被发现的实例** → 虚线卡片 + "连接" 按钮：
  - 虚线边框（区别于已注册的实线卡片）
  - 显示发现来源（"Docker 容器" / "数据目录"）
  - 点击"连接" → 调用 `connectDockerInstance(home_path, label)` → 注册 → 卡片变为正常样式
- 扫描中显示轻量 loading 状态（不阻塞已有卡片的展示）

### 1.3 InstanceCard 组件扩展

给 `InstanceCard` 新增 `discovered` prop：

```typescript
interface InstanceCardProps {
  // ... existing props
  discovered?: boolean;        // 虚线边框样式
  discoveredSource?: string;   // "Docker 容器" / "数据目录"
  onConnect?: () => void;      // 连接按钮回调
}
```

## 2. SSH 远程实例连接

### 2.1 InstallHub "连接远程实例" 标签

将当前 `connect` mode 从 Docker 表单改为 SSH 连接流程：

- 标签文案改为"连接远程实例"
- 内容：
  1. 如果有已配置但未注册为实例的 SSH host → 列在顶部供快速选择
  2. SSH 配置表单（复用 `SshFormWidget`，但独立使用）
  3. 提交后：连接 SSH → 探测远程 openclaw → 注册为 `remote_ssh` 实例

### 2.2 连接流程

```
用户填写 SSH 配置
    ↓
api.upsertSshHost(host) → 保存 host 配置
    ↓
api.sshConnect(hostId) → 建立 SSH 连接
    ↓
api.remoteGetInstanceStatus(hostId) → 检测远程 openclaw
    ↓
成功 → 注册实例，关闭对话框
失败 → 显示错误，建议通过 Install 安装
```

### 2.3 修复 `onEditSsh`

当前 `App.tsx` 中 `onEditSsh={() => {}}` 是空函数。需要：
- 新增 SSH 编辑对话框（复用 SshFormWidget）
- 编辑后调用 `api.upsertSshHost(host)` 保存

## 3. 不做什么

1. **不做定时扫描** — 只在 Start page 加载时和手动刷新时扫描
2. **不改 Install 新建流程** — local/docker/ssh 标签的安装流程保持不变
3. **不做 WSL auto-connect** — 仅 Windows 平台且当前不是重点，只做检测
4. **不做远程实例自动发现** — SSH 必须用户配置，无法自动发现

## 4. 改动范围

| 层 | 文件 | 改动 |
|---|------|------|
| Tauri | `src-tauri/src/commands/discover.rs` (新增) | discover_local_instances 命令 |
| Tauri | `src-tauri/src/commands/mod.rs` | 导出 discover 模块 |
| Tauri | `src-tauri/src/lib.rs` | 注册新命令 |
| Frontend | `src/lib/api.ts` | 新增 `discoverLocalInstances` API |
| Frontend | `src/lib/types.ts` | `DiscoveredInstance` 类型 |
| Frontend | `src/components/InstanceCard.tsx` | 新增 discovered 样式 |
| Frontend | `src/components/InstallHub.tsx` | connect mode 改为 SSH 流程 |
| Frontend | `src/pages/StartPage.tsx` | 集成自动发现 + 展示虚线卡片 |
| Frontend | `src/App.tsx` | 接入 onEditSsh + 发现实例连接逻辑 |
| i18n | `src/locales/en.json`, `zh.json` | 新增翻译 key |
