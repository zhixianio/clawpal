# CLI-Based Config Refactoring Design

## Goal

将 ClawPal 从"直接读写 openclaw.json 的配置编辑器"重构为"openclaw CLI 的 GUI wrapper"。

## Motivation

ClawPal 目前直接读写 `openclaw.json`，这导致三个问题：

1. **错过副作用** — 如 `agents add` 创建 workspace/sessions 目录、schema 校验等
2. **Schema 耦合** — openclaw 改了 JSON 结构，ClawPal 就坏了
3. **逻辑重复** — 重新实现了 openclaw 已有的验证、默认值、路径解析

## Principles

- 写入操作全部通过 openclaw CLI，获得校验、默认值填充、副作用
- 读取操作优先用 CLI 结构化输出（`agents list --json`），其次用 `config get`，兜底直接读 JSON
- 本地和远程逻辑统一，区别仅在 transport 层（本地直接执行 vs SSH exec）
- 保留 preview/apply/discard 的 UX，通过命令队列 + `OPENCLAW_HOME` 沙盒实现

---

## Command Queue

### Data Structure

```rust
struct PendingCommand {
    id: String,              // uuid
    label: String,           // 人类可读，如 "创建 agent: myBot"
    command: Vec<String>,    // ["openclaw", "agents", "add", "myBot", "--model", "..."]
    created_at: String,
}

struct CommandQueue {
    commands: Vec<PendingCommand>,  // 有序，按添加顺序执行
}
```

### Lifecycle

- 用户在 UI 上做操作（创建 agent、改 model 等）→ 生成对应的 CLI 命令入队
- 队列非空时，UI 显示 pending changes 提示（条数 + apply/discard 按钮）
- 用户可以查看队列、删除单条命令
- Preview → 沙盒执行生成 diff
- Apply → 真实执行 + 清空队列
- Discard → 清空队列

### Persistence

不持久化，仅存在于内存（Rust 侧的 `State<CommandQueue>`）。App 关闭时队列非空 → 弹确认提示。

---

## Preview Mechanism

### Flow

1. `mkdir -p ~/.clawpal/preview/.openclaw`
2. `cp ~/.openclaw/openclaw.json → ~/.clawpal/preview/.openclaw/openclaw.json`
3. 逐个执行队列命令，带 `OPENCLAW_HOME=~/.clawpal/preview/.openclaw`
4. 如果某步执行失败 → 停止，返回错误（直接删临时目录，无需回滚）
5. diff 原始 `openclaw.json` vs `~/.clawpal/preview/.openclaw/openclaw.json`
6. 返回给前端：命令列表 + config diff
7. `rm -rf ~/.clawpal/preview/`

### Frontend Display

- 左侧：将要执行的命令列表（人类可读的 label）
- 右侧：配置 diff（复用现有的 diff 展示组件）

### Remote Preview

同样的逻辑，通过 SSH exec 执行：

- `ssh exec "mkdir -p ~/.clawpal/preview/.openclaw && cp ~/.openclaw/openclaw.json ~/.clawpal/preview/.openclaw/openclaw.json"`
- 每条命令：`ssh exec "OPENCLAW_HOME=~/.clawpal/preview/.openclaw openclaw ..."`
- diff：读回两份文件对比
- 清理：`ssh exec "rm -rf ~/.clawpal/preview/"`

### Validation

已验证 `OPENCLAW_HOME` 环境变量可以完全隔离配置读写：

```bash
# 写入隔离 ✓
OPENCLAW_HOME=~/.clawpal/preview/.openclaw openclaw config set agents.defaults.model.primary '"test"'
# → 只修改 ~/.clawpal/preview/.openclaw/openclaw.json

# 主配置不受影响 ✓
openclaw config get agents.defaults.model.primary
# → 仍然是原值

# Schema 校验生效 ✓
openclaw config set test.foo '"bar"'
# → Error: Config validation failed
```

---

## Apply & Rollback

### Apply Flow

1. 保存快照（复用现有 `add_snapshot()` 机制，存到 `~/.clawpal/history/`）
2. 逐个执行队列命令（真实环境，无 `OPENCLAW_HOME` 覆盖）
3. 全部成功 → 清空队列 → 清空缓存 → 重启 gateway（`openclaw gateway restart`）
4. 某步失败 → 用快照恢复 `openclaw.json` → 返回错误信息（"第 N 步失败：xxx"）

### Discard Flow

清空队列，完毕。没有文件操作。

### Rollback Limitations

- `openclaw.json` 可以精确恢复
- 文件系统副作用（如 `agents add` 创建的目录）不会自动清理
- 可接受：空目录不影响功能，下次创建同名 agent 会复用

### App Close

队列非空 → 弹确认提示 "有 N 条未应用的变更，确定退出吗？"

---

## Read Operations

### Priority

| 读取场景 | 方式 |
|---------|------|
| Agent 列表 | `openclaw agents list --json` |
| Channel 列表 | `openclaw channels list --json` |
| Model 目录 | `openclaw models list --all --json`（已用 CLI，不变） |
| 单个配置值 | `openclaw config get <path> --json` |
| 整体配置展示（raw JSON） | 直接读 JSON（兜底） |
| Cron 列表 | `openclaw cron list --json` |
| Hooks 列表 | `openclaw hooks list --json` |
| Plugins 列表 | `openclaw plugins list --json` |

### Caching

Apply 成功后统一清空所有缓存，其他时候缓存一直有效。

| 数据 | 缓存策略 | 理由 |
|------|---------|------|
| Agent 列表 | Apply 后失效 | 只有 Apply 会改变 |
| Channel 列表 | Apply 后失效 | 同上 |
| Model 目录 | 10 分钟 TTL | 外部数据，不受 ClawPal 控制 |
| 单个配置值 | Apply 后失效 | 同上 |
| Cron/Hooks/Plugins 列表 | Apply 后失效 | 同上 |

### Remote Reads

同样的命令，通过 `ssh exec` 执行。比现在的 `sftp_read` + 本地解析更简洁。

---

## Write Operations

### Command Mapping

| UI 操作 | 入队的命令 |
|---------|-----------|
| 创建 agent | `openclaw agents add <name> --model <id> --non-interactive` |
| 删除 agent | `openclaw agents delete <id> --force --json` |
| 设置全局 model | `openclaw config set agents.defaults.model.primary '<id>'` |
| 设置 agent model | `openclaw config set agents.<name>.model.primary '<id>'` |
| 绑定 channel 到 agent | `openclaw agents add <name> --bind <channel>` 或 `config set` |
| 添加 channel account | `openclaw channels add --channel <type> --token <token> ...` |
| 删除 channel account | `openclaw channels remove --channel <type> --delete` |
| Recipe 应用 | 拆解为上述命令的组合序列 |

### Recipe Decomposition Example

"Telegram 客服 bot" recipe 从一整块 JSON patch 变为命令序列：

```
1. openclaw agents add support-bot --model claude-sonnet-4-20250514 --non-interactive
2. openclaw config set agents.support-bot.systemPrompt '"你是客服助手..."'
3. openclaw channels add --channel telegram --token <token>
4. openclaw agents add support-bot --bind telegram
```

---

## Backend Architecture

### New Module: `src-tauri/src/cli_runner.rs`

Responsibilities:
- 封装 openclaw CLI 调用（本地 `Command::new("openclaw")` / 远程 `ssh exec`）
- 管理命令队列（`CommandQueue` state）
- Preview 沙盒逻辑（复制配置、执行、diff、清理）
- Apply 逻辑（快照、执行、回滚、清缓存、重启 gateway）
- 读取缓存（Apply 后失效）

### Slimmed Module: `config_io.rs`

- 移除 `write_json` 写 openclaw.json 的逻辑
- 保留 `read_json` 仅用于快照读写和兜底读取

### Slimmed Module: `commands.rs`

- 移除 `set_global_model()`、`set_agent_model()`、`assign_channel_agent()` 等直接写 JSON 的函数
- 替换为入队操作

### New Tauri Commands

- `queue_command(label, command)` — 入队
- `remove_queued_command(id)` — 删除单条
- `list_queued_commands()` — 查看队列
- `preview_queued_commands()` — 沙盒执行生成 diff
- `apply_queued_commands()` — 真实执行
- `discard_queued_commands()` — 清空队列

### Unchanged

- 快照 / 历史机制（`history.rs`）
- SSH transport 层（`ssh.rs`）
- 前端页面结构

---

## Frontend Changes

### API Layer (`api.ts`)

- 现有的 `setGlobalModel()`、`setAgentModel()` 等改为调用 `queue_command()`
- 新增 `previewQueuedCommands()`、`applyQueuedCommands()`、`discardQueuedCommands()`、`listQueuedCommands()`、`removeQueuedCommand()`

### UI Changes

- 新增 **PendingChangesBar** 组件 — 队列非空时出现，显示"N 条待应用变更"+ Preview / Apply / Discard 按钮
- Preview 弹窗复用现有的 diff 展示组件，左侧加上命令列表
- 各页面的操作按钮行为从"立即生效"变为"入队 + 提示用户有待应用变更"

### UX Change

- 之前：点击 → 立即写入 → 手动重启 gateway
- 之后：点击 → 入队 → 可继续做更多操作 → 统一 Preview → Apply（自动重启 gateway）

批量操作、一次性确认应用。
