# Remaining CLI Migration — Cleanup Direct File I/O

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove all remaining direct reads/writes of `~/.openclaw/openclaw.json` via SFTP or `fs::read_to_string`, migrating to `openclaw config get/set` CLI commands. Also clean up dead code from the prior CLI migration that's no longer called by the frontend.

**Architecture:** The first CLI migration (2026-02-21) introduced `cli_runner.rs`, `CommandQueue`, `PendingChangesBar`, and migrated most frontend operations to `queueCommand()`. However, many Rust backend functions that directly manipulate config via `sftp_read`/`sftp_write` (remote) or `read_openclaw_config` (local) were left behind. Most of these are now dead code — the frontend bypasses them. We clean up the dead code, and migrate the few still-active functions to CLI.

**Tech Stack:** Rust/Tauri backend, openclaw CLI (`config get`, `config set`, `agents list`, `channels list`)

---

## Scope Analysis

### Active functions (still called from frontend) that need migration:

| Function | File:Line | Current I/O | Used By |
|----------|-----------|-------------|---------|
| `remote_get_system_status` | commands.rs:4339 | `sftp_read` openclaw.json | Home.tsx (status polling) |
| `remote_read_raw_config` | commands.rs:4334 | `sftp_read` openclaw.json | Home.tsx, Cook.tsx |
| `remote_list_discord_guild_channels` | commands.rs:4851 | `sftp_read` openclaw.json | App.tsx |
| `list_channels_minimal` | commands.rs:706 | `read_openclaw_config` | Channels.tsx via dispatch |
| `remote_list_channels_minimal` | commands.rs:4447 | `sftp_read` openclaw.json | Channels.tsx via dispatch |

### Dead code to remove (frontend uses `queueCommand` instead):

| Function | File:Line | Why Dead |
|----------|-----------|----------|
| `remote_create_agent` | commands.rs:4521 | CreateAgentDialog uses `queueCommand` |
| `remote_delete_agent` | commands.rs:4578 | Home.tsx uses `queueCommand` |
| `remote_assign_channel_agent` | commands.rs:4614 | Channels.tsx uses `queueCommand` |
| `remote_set_global_model` | commands.rs:4676 | Home.tsx uses `queueCommand` |
| `remote_set_agent_model` | commands.rs:4709 | Home.tsx uses `queueCommand` |
| `remote_write_config_with_snapshot` | commands.rs:4465 | Only used by above dead functions |
| `assign_channel_agent` (local) | commands.rs:951 | Channels.tsx uses `queueCommand` |

### Keep as-is (acceptable direct reads):

| Function | File | Reason to Keep |
|----------|------|----------------|
| `bridge_client.rs:319` | `fs::read_to_string` | Reads auth token for gateway connection — needs to be fast, internal use |
| `node_client.rs:244` | `fs::read_to_string` | Same — auth token extraction |
| `doctor.rs:26,83` | `fs::read_to_string` | Config validation/diagnostic — doctor has its own concerns |
| `doctor_commands.rs:321,649` | `fs::read_to_string` | Context collection for debugging |
| `get_status_light` (local) | commands.rs:378 | Local reads via `read_openclaw_config` are fast, no subprocess needed |

---

## Tasks

### Task 1: Migrate `remote_get_system_status` to CLI

**Files:**
- Modify: `src-tauri/src/commands.rs:4339-4387`

**Current:** Reads entire `openclaw.json` via `sftp_read`, manually parses agents count, model, fallbacks.

**New:** Use `openclaw config get agents --json` via CLI to get agents config.

**Step 1: Replace sftp_read with CLI call**

Replace lines 4350-4370 with:

```rust
// 2. Read remote config via CLI
let config_output = crate::cli_runner::run_openclaw_remote(
    &pool, &host_id, &["config", "get", "--json"]
).await;
let (active_agents, global_default_model, fallback_models) = match config_output {
    Ok(ref output) if output.exit_code == 0 => {
        let cfg: Value = crate::cli_runner::parse_json_output(output).unwrap_or(Value::Null);
        let explicit = cfg.pointer("/agents/list")
            .and_then(Value::as_array)
            .map(|a| a.len() as u32)
            .unwrap_or(0);
        let agents = if explicit == 0 && cfg.get("agents").is_some() { 1 } else { explicit };
        let model = cfg.pointer("/agents/defaults/model")
            .and_then(|v| read_model_value(v))
            .or_else(|| cfg.pointer("/agents/default/model").and_then(|v| read_model_value(v)));
        let fallbacks = cfg.pointer("/agents/defaults/model/fallbacks")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default();
        (agents, model, fallbacks)
    }
    _ => (0, None, Vec::new()),
};
```

**Step 2: Run `cargo check`**

Run: `cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | tail -5`
Expected: Compiles cleanly.

**Step 3: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "refactor: remote_get_system_status uses CLI instead of sftp_read"
```

---

### Task 2: Migrate `remote_read_raw_config` to CLI

**Files:**
- Modify: `src-tauri/src/commands.rs:4334-4336`

**Current:** `pool.sftp_read(&host_id, "~/.openclaw/openclaw.json")`

**New:** Use `openclaw config get --json` which returns the full config.

**Step 1: Replace implementation**

```rust
#[tauri::command]
pub async fn remote_read_raw_config(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<String, String> {
    let output = crate::cli_runner::run_openclaw_remote(&pool, &host_id, &["config", "get", "--json"]).await?;
    if output.exit_code != 0 {
        // Fallback: sftp_read for cases where openclaw binary isn't available
        return pool.sftp_read(&host_id, "~/.openclaw/openclaw.json").await;
    }
    // Re-format as pretty JSON for display
    match serde_json::from_str::<Value>(&output.stdout) {
        Ok(val) => serde_json::to_string_pretty(&val).map_err(|e| e.to_string()),
        Err(_) => Ok(output.stdout),
    }
}
```

**Step 2: Run `cargo check`**

**Step 3: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "refactor: remote_read_raw_config uses CLI with sftp fallback"
```

---

### Task 3: Migrate `remote_list_channels_minimal` to CLI

**Files:**
- Modify: `src-tauri/src/commands.rs:4447-4451`

**Current:** `sftp_read` + `collect_channel_nodes(&cfg)`

**New:** Use `openclaw config get channels --json` and parse the same way.

**Step 1: Replace implementation**

```rust
#[tauri::command]
pub async fn remote_list_channels_minimal(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<Vec<ChannelNode>, String> {
    let output = crate::cli_runner::run_openclaw_remote(&pool, &host_id, &["config", "get", "channels", "--json"]).await?;
    // channels key might not exist yet
    if output.exit_code != 0 {
        let msg = format!("{} {}", output.stderr, output.stdout).to_lowercase();
        if msg.contains("not found") {
            return Ok(Vec::new());
        }
        return Err(format!("openclaw config get channels failed: {}", output.stderr));
    }
    // parse_json_output strips noise and parses JSON
    let channels_val = crate::cli_runner::parse_json_output(&output).unwrap_or(Value::Null);
    // Wrap in a top-level object with "channels" key so collect_channel_nodes works
    let cfg = serde_json::json!({ "channels": channels_val });
    Ok(collect_channel_nodes(&cfg))
}
```

**Step 2: Also migrate local `list_channels_minimal`** (commands.rs:706)

```rust
#[tauri::command]
pub fn list_channels_minimal() -> Result<Vec<ChannelNode>, String> {
    let output = crate::cli_runner::run_openclaw(&["config", "get", "channels", "--json"])
        .map_err(|e| format!("Failed to run openclaw: {e}"))?;
    if output.exit_code != 0 {
        let msg = format!("{} {}", output.stderr, output.stdout).to_lowercase();
        if msg.contains("not found") {
            return Ok(Vec::new());
        }
        // Fallback: direct read
        let paths = resolve_paths();
        let cfg = read_openclaw_config(&paths)?;
        return Ok(collect_channel_nodes(&cfg));
    }
    let channels_val = crate::cli_runner::parse_json_output(&output).unwrap_or(Value::Null);
    let cfg = serde_json::json!({ "channels": channels_val });
    Ok(collect_channel_nodes(&cfg))
}
```

**Step 3: Run `cargo check`**

**Step 4: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "refactor: list_channels_minimal uses CLI for both local and remote"
```

---

### Task 4: Migrate `remote_list_discord_guild_channels` to CLI

**Files:**
- Modify: `src-tauri/src/commands.rs:4851-4960ish`

**Current:** Reads entire config via `sftp_read` to extract Discord bot token, then calls Discord API directly.

**New:** Use `openclaw config get channels.discord --json` to get just the discord section.

**Step 1: Replace the sftp_read portion only**

The function is complex — it reads config to get bot token + guild structure, then calls Discord HTTP API. We only need to change the config read, not the Discord API logic.

Replace lines 4855-4856:
```rust
let raw = pool.sftp_read(&host_id, "~/.openclaw/openclaw.json").await?;
let cfg: Value = serde_json::from_str(&raw).map_err(|e| format!("Failed to parse remote config: {e}"))?;
```

With:
```rust
let output = crate::cli_runner::run_openclaw_remote(&pool, &host_id, &["config", "get", "channels.discord", "--json"]).await?;
let discord_section = if output.exit_code == 0 {
    crate::cli_runner::parse_json_output(&output).unwrap_or(Value::Null)
} else {
    Value::Null
};
// Wrap to match existing code expectations
let cfg = serde_json::json!({ "channels": { "discord": discord_section } });
```

The rest of the function (extracting bot token, calling Discord API) stays the same since it operates on `cfg`.

**Step 2: Run `cargo check`**

**Step 3: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "refactor: remote_list_discord_guild_channels uses CLI for config read"
```

---

### Task 5: Remove dead remote write functions

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs` (unregister commands)
- Modify: `src/lib/api.ts` (remove bindings)
- Modify: `src/lib/use-api.ts` (remove dispatch entries)

**Dead functions to remove from commands.rs:**
1. `remote_write_config_with_snapshot` (~line 4465-4485)
2. `remote_create_agent` (~line 4521-4576)
3. `remote_delete_agent` (~line 4578-4611)
4. `remote_assign_channel_agent` (~line 4614-4673)
5. `remote_set_global_model` (~line 4676-4707)
6. `remote_set_agent_model` (~line 4709-4728)
7. `assign_channel_agent` (local, ~line 951-1010)

**Step 1: Remove the functions from commands.rs**

Delete each function body. Keep a `// Removed: migrated to command queue` comment if helpful.

**Step 2: Remove from lib.rs invoke_handler**

Remove these from the `invoke_handler![]` macro:
- `remote_create_agent`
- `remote_delete_agent`
- `remote_assign_channel_agent`
- `remote_set_global_model`
- `remote_set_agent_model`
- `assign_channel_agent`

**Step 3: Remove from api.ts**

Remove the `invoke()` bindings for:
- `remoteCreateAgent`
- `remoteDeleteAgent`
- `remoteAssignChannelAgent`
- `remoteSetGlobalModel`
- `remoteSetAgentModel`
- `assignChannelAgent`

**Step 4: Remove from use-api.ts**

Remove dispatch entries that reference the removed functions.

**Step 5: Run both `cargo check` and `npm run build`**

Run: `cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | tail -5`
Run: `npm run build 2>&1 | tail -10`

Fix any compilation errors from dangling references.

**Step 6: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs src/lib/api.ts src/lib/use-api.ts
git commit -m "refactor: remove dead direct-write functions replaced by command queue"
```

---

### Task 6: Remove helper functions only used by dead code

**Files:**
- Modify: `src-tauri/src/commands.rs`

**Step 1: Check if these helpers are still used after Task 5:**

- `set_nested_value` — search for remaining callers
- `set_agent_model_value` — search for remaining callers
- `collect_agent_ids` — search for remaining callers

If any are only used by the removed dead functions, delete them too.

**Step 2: Run `cargo check`**

**Step 3: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "refactor: remove orphaned helper functions"
```

---

### Task 7: Run tests and verify

**Step 1: Run existing test suite**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test cli_runner_test 2>&1
```

Expected: All tests pass.

**Step 2: Run frontend build**

```bash
cd /Users/zhixian/Codes/clawpal && npm run build 2>&1 | tail -10
```

Expected: Builds cleanly.

**Step 3: Manual verification checklist**

Run: `npm run tauri dev`

- [ ] Local: Agent list loads
- [ ] Local: Channel list loads
- [ ] Local: Bindings list loads (even when empty)
- [ ] Local: Discord guild channels load
- [ ] Local: Raw config view works
- [ ] Remote: Connect to trendsite
- [ ] Remote: Status light shows correctly (agents, model, version)
- [ ] Remote: Channel list loads
- [ ] Remote: Discord guild channels load
- [ ] Remote: Create agent via queue → preview → apply works
- [ ] Remote: Delete agent via queue works
- [ ] Remote: Bind channel to agent via queue works
- [ ] Remote: Set model via queue works
- [ ] No errors in console about missing commands

**Step 4: Commit any fixes**

```bash
git add -A
git commit -m "fix: resolve issues found during CLI migration verification"
```

---

## Files Summary

| File | Action | Tasks |
|------|--------|-------|
| `src-tauri/src/commands.rs` | Modify | 1-6 (migrate reads, remove dead writes) |
| `src-tauri/src/lib.rs` | Modify | 5 (unregister dead commands) |
| `src/lib/api.ts` | Modify | 5 (remove dead bindings) |
| `src/lib/use-api.ts` | Modify | 5 (remove dead dispatch) |

## What Does NOT Change

- `src-tauri/src/ssh.rs` — transport layer
- `src-tauri/src/cli_runner.rs` — already correct
- `src-tauri/src/doctor.rs` — acceptable direct reads for diagnostics
- `src-tauri/src/bridge_client.rs` — acceptable direct read for auth token
- `src-tauri/src/node_client.rs` — acceptable direct read for auth token
- `src/components/PendingChangesBar.tsx` — already working
- `src/pages/Home.tsx` — already uses queueCommand
- `src/pages/Channels.tsx` — already uses queueCommand

## Risk Mitigation

- **CLI not available on remote:** `remote_read_raw_config` includes sftp fallback
- **Config paths not found:** All CLI reads handle "not found" gracefully (return empty)
- **Breaking changes:** Dead code removal is safe — confirmed no frontend callers
