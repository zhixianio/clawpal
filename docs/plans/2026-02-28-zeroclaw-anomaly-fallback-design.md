# Zeroclaw 异常兜底扩展设计

> 日期：2026-02-28
> 分支：refactor/gui-cli-agent-layers
> 状态：待实现

## 目标

扩展 zeroclaw 的异常兜底能力，从当前仅覆盖业务逻辑异常（openclaw 命令执行失败），扩展到覆盖软件本身使用中的各类异常状态：Auth 配置异常、实例配置数据不一致、连接状态过期等。

## 设计原则

1. **扩展现有管线**：复用 `agent_fallback.rs` + `guidance.ts` 管线，不新建抽象
2. **混合修复模式**：简单异常自动修复，复杂异常引导用户通过 Doctor 交由 zeroclaw 处理
3. **反应式 + 预防式**：操作失败时触发 + 关键操作前做轻量 precheck（<200ms）
4. **CLI 执行通道**：所有修复动作走 `clawpal`/`openclaw` CLI 通道，与 zeroclaw Doctor 会话共用执行路径

## 1. 新增错误分类码

在 `RuntimeErrorCode` 基础上扩展：

| 错误码 | 场景 | Precheck 可检测 |
|--------|------|:---:|
| `AuthExpired` | API key 无效/过期，provider 返回 401/403 | ❌（运行时） |
| `AuthMisconfigured` | Profile 引用不存在的 key、provider 字段缺失 | ✅ |
| `RegistryCorrupt` | registry.json 不可解析或字段缺失 | ✅ |
| `InstanceOrphaned` | 实例指向的 Docker container / 远程路径已不存在 | ✅（轻量探测） |
| `TransportStale` | UI 显示已连接但实际断开（SSH session 失效、Docker daemon 停止） | ✅ |

`classify_engine_error()` 新增 pattern matching：

- `"401"` / `"unauthorized"` / `"invalid.*api.*key"` → `AUTH_EXPIRED`
- `"403"` / `"forbidden"` / `"quota exceeded"` → `AUTH_EXPIRED`
- `"registry"` + `"parse"` / `"invalid json"` → `REGISTRY_CORRUPT`
- `"container.*not found"` / `"no such container"` → `INSTANCE_ORPHANED`

## 2. Precheck 检查层

### 2.1 检查函数

放在 `clawpal-core/src/precheck.rs`，纯 Rust 同步/轻量异步：

```rust
pub struct PrecheckIssue {
    pub code: String,           // 错误码
    pub severity: String,       // "error" | "warn"
    pub message: String,        // 用户可读描述
    pub auto_fixable: bool,     // 是否可自动修复
    pub fix_action: Option<GuidanceAction>,  // 修复动作
}

precheck_auth(profiles) → Vec<PrecheckIssue>
  - 每个 profile 有 provider + model 字段
  - 引用的 API key 在本地存储中存在

precheck_registry(registry_path) → Vec<PrecheckIssue>
  - JSON 可解析
  - 每个实例的 home 路径存在（local/docker）

precheck_transport(instance, pool) → Vec<PrecheckIssue>
  - SSH: pool 中 session 仍然 alive（ping）
  - Docker: daemon 可达（docker info 快速超时）

precheck_instance_state(instance) → Vec<PrecheckIssue>
  - 实例 config 文件存在且可读
  - 基本字段完整性
```

### 2.2 触发时机

| 时机 | 检查项 |
|------|--------|
| 切换实例前 | `precheck_transport` + `precheck_instance_state` |
| 进入 Agent 会话前 | `precheck_auth` |
| 应用启动时（一次性） | `precheck_registry` |
| 操作失败后 | 现有 guidance 管线（已有） |

目标耗时 <200ms，超时即跳过（不阻塞用户）。

## 3. 修复策略

### 3.1 修复动作分级

| 异常 | 修复策略 | 具体动作 |
|------|---------|---------|
| `TransportStale` (SSH 断开) | **自动修复** | `clawpal ssh connect --host xxx` 重连 |
| `TransportStale` (Docker daemon 停) | **引导** | 提示启动 Docker Desktop，提供检测按钮 |
| `InstanceOrphaned` (container 已删) | **引导** | 提示重新安装或移除实例 |
| `InstanceOrphaned` (远程路径不存在) | **Doctor 交接** | 跳转 Doctor，传递上下文 |
| `RegistryCorrupt` | **自动修复** | 尝试 JSON 修复（复用 Doctor 的 `json.syntax` fix），失败则引导 |
| `AuthMisconfigured` | **引导** | 指出问题 profile，引导到 Profile 页面 |
| `AuthExpired` (运行时 401/403) | **引导** | guidance 给出 "重新配置 API key" 的 action |

### 3.2 自动修复安全边界

只自动执行以下条件的修复：
1. 修复动作是 `read` 类型（不修改用户数据）
2. 或是已有的重连逻辑（SSH reconnect）
3. 修复失败不会让状态更差

任何涉及删除、写入配置、重新安装的操作 → 生成 guidance 引导用户确认。

## 4. GuidanceAction 结构化

将 guidance actions 从纯文字升级为可执行的结构化动作：

```rust
pub struct GuidanceAction {
    pub label: String,              // "重连 SSH" / "让小龙虾修复"
    pub action_type: String,        // "inline_fix" | "doctor_handoff"
    // inline_fix: 通过 clawpal CLI 执行
    pub tool: Option<String>,       // "clawpal" | "openclaw"
    pub args: Option<String>,       // CLI args, e.g. "ssh connect --host xxx"
    pub invoke_type: Option<String>,// "read" | "write"
    // doctor_handoff: 传给 Doctor 的上下文
    pub context: Option<String>,    // 异常上下文消息
}
```

执行流程：
1. 用户点击 `inline_fix` 按钮
2. 前端构造 `ToolIntent { tool, args, invoke_type }`
3. 走现有 tool intent 执行管线（read 自动执行，write 需确认）
4. 执行结果显示在卡片上

`doctor_handoff` 执行流程：
1. 用户点击「让小龙虾修复」按钮
2. 跳转 Doctor 页面
3. 异常上下文自动作为第一条消息发送给 zeroclaw Doctor 会话

## 5. 前端集成

### 5.1 Guidance 卡片升级

新增 `GuidanceCard` 组件，替代现有纯文本提示：

- **按钮类型**：
  - `inline_fix`：在当前页面执行修复，显示 loading → 成功/失败
  - `doctor_handoff`：跳转 Doctor 页面并预填异常上下文

### 5.2 Doctor 上下文传递

跳转 Doctor 时传递：
```typescript
navigate(`/doctor?context=${encodeURIComponent(JSON.stringify({
  errorCode: "INSTANCE_ORPHANED",
  error: "container xxx not found",
  instanceId: "xxx",
  transport: "docker_local",
  suggestedAction: "检查 Docker container 状态并决定是否重新安装"
}))}`)
```

Doctor 页面接收 context 后，自动作为第一条消息发送给 zeroclaw Doctor 会话。

### 5.3 Precheck 结果展示

- **非阻塞性**（warn）：显示警告 toast，不阻止用户操作
- **阻塞性**（error，如 registry 损坏）：显示 modal，必须先修复

## 6. 不做什么

1. **不做定时巡检**：不增加后台轮询，只在操作触发时检查
2. **不做 Auth 主动校验**：不在 precheck 中请求 provider API 验证 key（太慢），运行时 401/403 由 guidance 管线处理
3. **不新增 zeroclaw 域**：复用 Doctor 域 adapter 处理所有复杂修复
4. **不做 Precheck 结果缓存**：每次重新检查（<200ms，缓存引入一致性问题）
5. **inline_fix 只通过 CLI 通道执行**：不引入新的修复逻辑通道

## 7. 改动范围

| 层 | 文件 | 改动 |
|---|------|------|
| Core | `clawpal-core/src/precheck.rs` (新增) | precheck 函数 |
| Core | `clawpal-core/src/lib.rs` | 导出 precheck 模块 |
| Tauri | `src-tauri/src/runtime/types.rs` | 新增错误码 |
| Tauri | `src-tauri/src/doctor.rs` | 扩展 `classify_engine_error` |
| Tauri | `src-tauri/src/agent_fallback.rs` | 扩展 `rules_fallback`、`GuidanceAction` 结构 |
| Tauri | `src-tauri/src/commands/` | 新增 precheck Tauri command |
| Frontend | `src/lib/guidance.ts` | 扩展 error pattern matching |
| Frontend | `src/lib/types.ts` | `GuidanceAction` 类型 |
| Frontend | `src/components/GuidanceCard.tsx` (新增) | 可交互 guidance 卡片组件 |
| Frontend | `src/App.tsx` | 替换纯文本 guidance 为 GuidanceCard |
| Frontend | `src/pages/Doctor.tsx` | 接收外部 context 并自动开始会话 |
| Prompt | `prompts/error-guidance/operation-fallback.md` | 更新输出 schema 支持 GuidanceAction |
