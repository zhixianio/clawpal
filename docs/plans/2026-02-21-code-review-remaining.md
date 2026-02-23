# Code Review — Remaining Items

Date: 2026-02-21
Updated: 2026-02-23
Status: Tracked tech debt from full-project code review.
Commit with fixes applied: `4421750`

---

## CRITICAL — Must fix before public release

### ~~C1. SSH Host Key Verification Disabled (MITM)~~ RESOLVED 2026-02-20

Migrated from `russh` to `openssh` crate. Now uses `KnownHosts::Accept` which delegates to the system's OpenSSH `~/.ssh/known_hosts` verification. The old `check_server_key` that always returned `Ok(true)` no longer exists.

---

### C3. `curl | bash` Upgrade Pattern — No Integrity Verification

**File:** `src-tauri/src/commands.rs` (`run_openclaw_upgrade`, `remote_run_openclaw_upgrade`)

The upgrade command pipes `https://openclaw.ai/install.sh` directly into bash with no checksum or GPG verification. DNS hijack or server compromise leads to RCE.

**Fix options:**
1. Download the script first, verify a SHA-256 checksum published at a separate URL, then execute
2. Use a signed binary release mechanism instead of a shell script
3. At minimum, pin to HTTPS and document the risk

**Effort:** Requires upstream (openclaw.ai) to publish checksums or signed releases.

---

### C4. `std::env::set_var` Unsafe in Multi-Threaded Rust (NEW 2026-02-23)

**File:** `src-tauri/src/commands.rs` (`resolve_openclaw_bin`)

Since Rust 1.83, `std::env::set_var` is unsafe in multi-threaded contexts. The `OnceLock` initializer mutates global `PATH` while tokio threads may be reading it concurrently.

**Fix:** Pass the modified PATH only to child processes via `Command::env("PATH", ...)` instead of globally mutating. The resolved binary path is already cached via `OnceLock` — the global PATH mutation is unnecessary.

**Effort:** Low. Change one line to use `.env()` on Command instead of `set_var`.

---

## IMPORTANT — Should fix

### I2. Blocking I/O in Synchronous Tauri Commands (PARTIALLY RESOLVED)

**File:** `src-tauri/src/commands.rs` — multiple functions

Core status functions remain synchronous with blocking I/O:
- `get_system_status()` — reads config, spawns subprocesses, checks update cache
- `get_status_light()` — TCP connect with 200ms timeout
- `check_openclaw_update()` — subprocess + HTTP request

Many other commands have been successfully converted to async (list_channels, list_bindings, analyze_sessions, etc.).

**Fix:** Convert remaining sync commands to `async` using `spawn_blocking`.

---

### I17. Oversized Components — App.tsx (~560 lines), Doctor.tsx (~850 lines)

**App.tsx** manages routing, toasts, SSH state, config dirty/apply/discard, update checks, analytics, cron polling, navigation, chat panel, dialogs, and toast stack (14+ useState calls).

**Doctor.tsx** has 15+ state variables with deeply nested JSX.

**Suggested extractions:**
- `App.tsx`: `useToast()` hook, `useConfigDirty()` hook, `<Sidebar>` component, `<ToastStack>` component
- `Doctor.tsx`: `<SessionAnalysisPanel>`, `<SessionRow>`, `<BackupsSection>`

---

### I16. AutocompleteField Missing ARIA Roles

**File:** `src/pages/Settings.tsx`

The custom autocomplete renders a dropdown using plain `<div>` elements with no ARIA roles, no `aria-expanded`, no keyboard navigation.

**Fix:** Add proper ARIA attributes or replace with a library component (Radix Combobox, cmdk).

---

### I18. Double Mutex Gap in SSH `connect()` (NEW 2026-02-23)

**File:** `src-tauri/src/ssh.rs` (`connect` method)

Mutex is acquired to remove old session, released, then re-acquired to insert new session. Between release and re-acquire, a concurrent `connect()` for the same id could race in and insert a session that gets silently overwritten (leaked).

**Fix:** Merge the two critical sections into one — remove old + insert new under a single lock.

**Effort:** Low. Move `pool.insert(...)` into the existing lock scope.

---

### I19. `set_global_model` May Clobber Fallbacks (NEW 2026-02-23)

**File:** `src-tauri/src/commands.rs` (`set_global_model`)

When model is stored as an object `{ primary: "...", fallbacks: [...] }`, the function correctly updates only `primary`. But the fallback code path uses `set_nested_value` which replaces the entire `agents.defaults.model` with a plain string, destroying any fallbacks array.

**Fix:** Always promote to object format when setting primary model — never write a plain string to `agents.defaults.model` if fallbacks might exist.

**Effort:** Low-medium. Add an object-promotion path before the `set_nested_value` fallback.

---

## SUGGESTIONS — Nice to have

### S1. Split `commands.rs` (~5800 lines) Into Modules

The file contains models, DTOs, local commands, remote commands, helpers, cron, watchdog, backup/restore, and session analysis.

**Suggested structure:**
```
commands/
  mod.rs          — re-exports
  models.rs       — DTOs/structs
  local.rs        — local instance commands
  remote.rs       — remote/SSH commands
  cron.rs         — cron + watchdog
  backup.rs       — backup/restore
  sessions.rs     — session management
  helpers.rs      — shared utilities
```

---

### ~~S2. DRY up Local/Remote API Branching~~ RESOLVED 2026-02-22

The `useApi()` hook in `src/lib/use-api.ts` now provides a `dispatch()` pattern that auto-selects local vs remote based on context. All pages use it.

---

### S3. Duplicate `groupAgents` Function

Identical function in `Home.tsx` and `Channels.tsx`. Extract to `src/lib/agent-utils.ts`.

---

### S4. Regex Compiled on Every Call

`doctor.rs` and `commands.rs` compile regexes on each invocation. Use `std::sync::LazyLock`.

---

### S5. Config File Concurrent Access

Multiple Tauri commands read-modify-write the config file without locking. Two concurrent commands can lose each other's changes. Consider a Mutex around config operations.

---

### S6. `resolve_paths()` Side Effects

`src-tauri/src/models.rs` — the path resolution function contains filesystem migration logic. Should be a separate explicit startup step.

---

### S7. `state.ts` Naming

Only used by `Doctor.tsx`, but named `AppState`. Rename to `DoctorState` or inline into Doctor.tsx.

---

### S8. Accessibility — Nav Buttons, Status Dots

- Nav buttons in `App.tsx` missing `aria-current="page"` for active state
- Escalated cron badge and update dot (colored `<span>`) lack accessible labels
- Consider `<nav aria-label="Main navigation">` wrapper

---

### S9. Empty `.catch(() => {})` in Multiple Files

Cron.tsx data loading, App.tsx analytics — silently swallowed errors. At minimum log to console for debugging.

---

### S10. Chat.tsx Hardcoded `AGENT_ID = "main"`

If no agent named "main" exists, chat fails. After loading agents, validate that current `agentId` is in the list; if not, default to first available.

---

### S11. Duplicate `shell_escape` / `shell_quote` (NEW 2026-02-23)

`commands.rs:shell_escape` and `ssh.rs:shell_quote` are identical functions. Extract to a shared `crate::util::shell_quote`.

---

### S12. ~200 Lines Duplicated Between Unix/Windows SSH Impl (NEW 2026-02-23)

**File:** `src-tauri/src/ssh.rs`

9 methods (`get_home_dir`, `resolve_path`, `exec`, `exec_login`, `sftp_read`, `sftp_write`, `sftp_list`, `sftp_remove`, `reconnect`) are copy-pasted verbatim between `#[cfg(unix)]` and `#[cfg(not(unix))]` blocks.

**Fix:** Extract shared methods into a trait with default implementations or free functions. Platform-specific blocks only need `connect`, `disconnect`, `is_connected`.

---

### S13. Repeated Config-Mutation Preamble (NEW 2026-02-23)

**File:** `src-tauri/src/commands.rs` (7 occurrences)

```rust
let paths = resolve_paths();
let mut cfg = read_openclaw_config(&paths)?;
let current = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
```

Extract to `fn load_config_for_mutation() -> Result<(ResolvedPaths, Value, String), String>`.

---

### S14. Repeated "Not Found" Guard Pattern (NEW 2026-02-23)

**File:** `src-tauri/src/commands.rs` (4 occurrences)

The pattern checking `exit_code != 0` + `msg.contains("not found")` → return empty is duplicated across `list_bindings`, `remote_list_bindings`, `list_channels_minimal`, `remote_list_channels_minimal`.

Extract to `fn is_cli_not_found(output: &CliOutput) -> bool`.

---

### S15. Duplicate Model Profile Matching Logic (NEW 2026-02-23)

**File:** `src/pages/Home.tsx`

The profile-matching logic (normalized model string → profile ID lookup) is duplicated between `currentModelProfileId` memo and the per-agent model select. Extract to `findProfileIdByModelValue()`.
