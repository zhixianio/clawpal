# Code Review Notes (Claude → Codex)

Last updated: 2026-02-27

This file contains review findings and action items from architecture audits. Codex should check this file periodically and work through the items.

## Codex Feedback

Last run: 2026-02-27

| Action | Status | Result |
|--------|--------|--------|
| Action 1: Batch E2 Sessions | PASS | 新增 `clawpal-core/src/sessions.rs`，迁移 `remote_analyze_sessions` / `remote_delete_sessions_by_ids` / `remote_list_session_files` / `remote_preview_session` 的纯解析与过滤逻辑到 core（`parse_session_analysis`、`filter_sessions_by_ids`、`parse_session_file_list`、`parse_session_preview`）；Tauri 端改为调用 core。新增 4 个 core 单测并通过。 |
| Action 2: Batch E3 Cron | PASS | 新增 `clawpal-core/src/cron.rs`，迁移 `parse_cron_jobs` / `parse_cron_runs`；`commands.rs` 本地与远端 cron 读取路径改为调用 core 解析。新增 2 个 core 单测并通过。 |
| Action 3: Batch E4 Watchdog | PASS | 新增 `clawpal-core/src/watchdog.rs`，迁移 watchdog 状态合并判断到 `parse_watchdog_status`；`remote_get_watchdog_status` 改为调用 core 解析后补充 `deployed`。新增 1 个 core 单测并通过。 |
| Action 4: Batch E5 Backup/Upgrade | PASS | 新增 `clawpal-core/src/backup.rs`，迁移 `parse_backup_list` / `parse_backup_result` / `parse_upgrade_result`；`remote_backup_before_upgrade` 与 `remote_list_backups` 改为调用 core 解析，`remote_run_openclaw_upgrade` 接入升级输出解析。新增 3 个 core 单测并通过。 |
| Action 5: Batch E6 Discord/Discovery | PASS | 新增 `clawpal-core/src/discovery.rs`，迁移 Discord guild/channel 与 bindings 解析（`parse_guild_channels`、`parse_bindings`）及绑定合并函数（`merge_channel_bindings`）。`remote_list_discord_guild_channels` 与 `remote_list_bindings` 已改为优先调用 core 解析，保留原 SSH/REST fallback。新增 3 个 core 单测并通过。 |
| Action 6: 质量验证 | PASS (remote_api ignored) | `cargo build --workspace` 通过；`npx tsc --noEmit` 通过；`cargo test --workspace --all-targets` 仅 `remote_api` 因 `192.168.65.2:22 Operation not permitted` 失败，按说明忽略。`commands.rs` 行数：`9367 -> 9077`（减少 `290` 行）。 |
| Action 7: commands.rs 拆文件 | PENDING | - |

---

## Context

三层架构重构（Phase 1-10）已完成，见 `cc-architecture-refactor-v1.md`。

本轮目标：将 `commands.rs` 中剩余 `remote_*` 函数按领域迁移到 `clawpal-core`。

当前 `commands.rs`：9,367 行，41 个 `remote_*` 函数。其中约 20 个已部分调用 core，约 21 个纯 inline SFTP+JSON。

迁移原则：只迁移有实际 JSON 解析/操作逻辑的函数。纯薄包装（Logs 4 个、Gateway 1 个、Agent Setup 1 个）保留在 Tauri 层，不值得抽。

---

## Outstanding Issues

### P1: `commands.rs` remote domain migration

按领域逐批迁移 `remote_*` 函数到 core 纯函数。模式同 Phase A/E1：

1. 抽取 JSON 操作为 `clawpal_core` 纯函数（String in/out 或 serde_json::Value）
2. Tauri 端改为调用 core
3. 加单元测试
4. 每批独立 commit

---

### P2: Doctor/Install prompt 结构重叠

~60% 内容重复。可考虑抽取 `prompts/common/tool-schema.md`。不急。

---

## Resolved Issues

| Issue | Resolution | Commit |
|-------|-----------|--------|
| Sessions domain inline parsing | 4 pure functions in `clawpal_core::sessions` | `de8fce4` |
| Cron domain inline parsing | 2 pure functions in `clawpal_core::cron` | `d47e550` |
| Watchdog domain inline parsing | `parse_watchdog_status` + `WatchdogStatus` struct in core | `bd697d9` |
| Backup/Upgrade domain parsing | 3 pure functions + 3 typed structs in `clawpal_core::backup` | `7554bd6` |
| Discord/Discovery domain parsing | 3 pure functions + 2 typed structs in `clawpal_core::discovery` | `64717b5` |

---

## Next Actions (for Codex)

迁移模式同 Phase A/E1：抽 JSON 解析/操作逻辑为 `clawpal_core` 纯函数（`&str` / `serde_json::Value` in/out），Tauri 端改为调 core，加单元测试，每批独立 commit。

**不要动 SFTP/SSH 连接层**——只抽纯数据逻辑。Tauri 端仍负责 `pool.sftp_read()` / `pool.exec()` 调用，拿到原始字符串后传给 core 解析。

### Action 1: Batch E2 — Sessions 领域

目标文件：`clawpal-core/src/sessions.rs`（新建）

迁移以下 5 个函数中的 JSON/JSONL 解析逻辑：

1. **`remote_analyze_sessions`**（line ~7913，~160 行）：抽取 shell 输出 → session 统计的 JSON 解析逻辑到 core `parse_session_analysis(raw: &str) -> Result<SessionAnalysis>`
2. **`remote_delete_sessions_by_ids`**（line ~8066）：抽取 sessions JSON 操作（读取 → 按 ID 过滤 → 序列化）到 core `filter_sessions_by_ids(json: &str, ids: &[&str]) -> Result<String>`
3. **`remote_list_session_files`**（line ~8113）：抽取 shell 输出解析到 core `parse_session_file_list(raw: &str) -> Result<Vec<SessionFileInfo>>`
4. **`remote_preview_session`**（line ~8199）：抽取 JSONL 解析到 core `parse_session_preview(jsonl: &str) -> Result<SessionPreview>`
5. **`remote_clear_all_sessions`**（line ~8173）：太薄（只返回 count），跳过不迁移

每个 core 函数加至少 1 个单元测试（用 inline JSON fixture）。Commit message: `refactor: move session parsing logic to core`

### Action 2: Batch E3 — Cron 领域

目标文件：`clawpal-core/src/cron.rs`（新建）

1. **`remote_list_cron_jobs`**（line ~8891）：抽取 JSON 解析到 core `parse_cron_jobs(json: &str) -> Result<Vec<CronJob>>`
2. **`remote_get_cron_runs`**（line ~8903）：抽取 JSONL 解析到 core `parse_cron_runs(jsonl: &str) -> Result<Vec<CronRun>>`
3. **`remote_trigger_cron_job`** / **`remote_delete_cron_job`**：太薄（纯 exec），跳过

Commit message: `refactor: move cron parsing logic to core`

### Action 3: Batch E4 — Watchdog 领域

目标文件：`clawpal-core/src/watchdog.rs`（新建）

1. **`remote_get_watchdog_status`**（line ~9236，~50 行）：抽取状态判断逻辑到 core `parse_watchdog_status(config_json: &str, ps_output: &str) -> WatchdogStatus`
2. **`remote_deploy_watchdog`**：主要是 SFTP 写，逻辑薄，跳过
3. **`remote_start/stop/uninstall_watchdog`**：纯 exec，跳过

Commit message: `refactor: move watchdog status parsing to core`

### Action 4: Batch E5 — Backup/Restore + Upgrade

目标文件：`clawpal-core/src/backup.rs`（新建）

1. **`remote_list_backups`**（line ~6329，~80 行）：抽取 `du` 输出解析到 core `parse_backup_list(du_output: &str) -> Vec<BackupEntry>`
2. **`remote_backup_before_upgrade`**（line ~6280）：抽取输出解析到 core `parse_backup_result(output: &str) -> BackupResult`
3. **`remote_run_openclaw_upgrade`**（line ~8719）：抽取版本解析到 core `parse_upgrade_result(output: &str) -> UpgradeResult`
4. **`remote_restore_from_backup`**：纯 exec，跳过

Commit message: `refactor: move backup and upgrade parsing to core`

### Action 5: Batch E6 — Discord/Discovery

目标文件：`clawpal-core/src/discovery.rs`（新建）

1. **`remote_list_discord_guild_channels`**（line ~7589，~320 行）：这是最大的单函数。抽取 channel/guild/binding 的 JSON 解析和合并逻辑到 core。拆为：
   - `parse_guild_channels(raw: &str) -> Result<Vec<GuildChannel>>`
   - `merge_channel_bindings(channels: &[GuildChannel], bindings: &str) -> Vec<ChannelWithBinding>`
2. **`remote_list_bindings`**（line ~7118）：抽取 JSON 数组解析到 core
3. **`remote_list_channels_minimal`**：已部分迁移，补齐即可

Commit message: `refactor: move discord discovery parsing to core`

### Action 6: 质量验证

1. `cargo build --workspace` 通过
2. `cargo test --workspace --all-targets` 通过（remote_api 忽略）
3. `npx tsc --noEmit` 通过
4. 统计 `commands.rs` 新行数，报告缩减量

### Action 7: `commands.rs` 拆分为领域模块

**在 Action 1-6 全部完成后执行。** 纯机械重组，不改任何逻辑。

将 `src-tauri/src/commands.rs` 拆为目录结构：

```
src-tauri/src/commands/
  mod.rs              // re-export 所有子模块的 pub 函数
  config.rs           // remote_read_raw_config, remote_write_raw_config, remote_apply_config_patch, remote_list_history, remote_preview_rollback, remote_rollback
  sessions.rs         // remote_analyze_sessions, remote_delete_sessions_by_ids, remote_list_session_files, remote_preview_session, remote_clear_all_sessions
  doctor.rs           // remote_run_doctor, remote_fix_issues, remote_get_system_status, remote_get_status_extra
  profiles.rs         // remote_list_model_profiles, remote_upsert_model_profile, remote_delete_model_profile, remote_resolve_api_keys, remote_test_model_profile, remote_extract_model_profiles_from_config
  watchdog.rs         // remote_get_watchdog_status, remote_deploy_watchdog, remote_start/stop/uninstall_watchdog
  cron.rs             // remote_list_cron_jobs, remote_get_cron_runs, remote_trigger_cron_job, remote_delete_cron_job
  backup.rs           // remote_backup_before_upgrade, remote_list_backups, remote_restore_from_backup, remote_run_openclaw_upgrade, remote_check_openclaw_update
  discovery.rs        // remote_list_discord_guild_channels, remote_list_bindings, remote_list_channels_minimal, remote_list_agents_overview
  logs.rs             // remote_read_app_log, remote_read_error_log, remote_read_gateway_log, remote_read_gateway_error_log
  rescue.rs           // remote_manage_rescue_bot, remote_diagnose_primary_via_rescue, remote_repair_primary_via_rescue
  gateway.rs          // remote_restart_gateway
  agent.rs            // remote_setup_agent_identity, remote_chat_via_openclaw
```

步骤：
1. 将 `commands.rs` 改为 `commands/mod.rs`
2. 按上述分组将函数剪切到对应子文件
3. 共享的 helper 函数（`remote_write_config_with_snapshot`, `remote_read_openclaw_config_text_and_json`, `remote_resolve_openclaw_config_path` 等）留在 `mod.rs` 或放入 `commands/helpers.rs`
4. 每个子文件 `use super::*` 引入共享依赖
5. `mod.rs` 中 `pub use` re-export 所有 `#[tauri::command]` 函数，确保 `main.rs` 的 `invoke_handler` 不需要改动
6. `cargo build --workspace` + `cargo test --workspace --all-targets` 通过
7. `npx tsc --noEmit` 通过

Commit message: `refactor: split commands.rs into domain modules`

**关键约束：不改任何函数签名或逻辑，只移动代码位置。**

每个 Action 完成后在 Codex Feedback 区域更新状态。

---

## Execution History

| Batch | Status | Commits | Review Notes |
|-------|--------|---------|-------------|
| Batch E2: Sessions | **Done** | `de8fce4` | 4 pure functions, 4 tests, -237 lines from commands.rs |
| Batch E3: Cron | **Done** | `d47e550` | 2 pure functions, 2 tests, -51 lines from commands.rs |
| Batch E4: Watchdog | **Done** | `bd697d9` | 1 pure function + typed struct, 1 test, -21 lines from commands.rs |
| Batch E5: Backup/Upgrade | **Done** | `7554bd6` | 3 pure functions + 3 structs, 3 tests, -17 lines from commands.rs |
| Batch E6: Discord/Discovery | **Done** | `64717b5` | 3 pure functions + 2 structs, 3 tests, -116 lines from commands.rs |
