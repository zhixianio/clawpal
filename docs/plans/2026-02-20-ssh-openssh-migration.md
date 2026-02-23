# SSH Library Migration: russh → openssh + openssh-sftp-client

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the low-level `russh` SSH implementation with `openssh` + `openssh-sftp-client` to inherit system SSH authentication, eliminating all auth compatibility issues.

**Architecture:** The current `SshConnectionPool` stores `russh` handles and manually implements auth, config parsing, exec, and SFTP. We replace the internals with `openssh::Session` (which delegates to the system `ssh` binary via ControlMaster multiplexing) and `openssh_sftp_client::Sftp` for file operations. The public API of `SshConnectionPool` stays identical — all 45+ commands in `commands.rs` continue working without changes.

**Tech Stack:** `openssh` (wraps system ssh), `openssh-sftp-client` (pure Rust SFTP v3 over openssh session), `tokio`

---

## Background

### Current Problems
- `russh` requires manual implementation of every auth method (password, publickey, keyboard-interactive, agent)
- Custom SSH config parser only supports `Host`, `HostName`, `User`, `Port`, `IdentityFile` — no wildcards, ProxyJump, ProxyCommand
- Passphrase-protected keys silently fail (loaded with `None` passphrase)
- keyboard-interactive auth not implemented (many Linux servers use this)
- Users report "SSH authentication failed" when terminal SSH works fine
- Username input auto-capitalizes on macOS (e.g. `root` → `Root`), causing `Invalid user Root` errors on Linux

### Why openssh Fixes This
- Delegates to system `ssh` binary — if `ssh user@host` works in terminal, it works in ClawPal
- All auth methods auto-supported (agent, keyboard-interactive, GSSAPI, etc.)
- Full `~/.ssh/config` support (ProxyJump, wildcards, Include, etc.)
- No custom auth code to maintain

### Public API (unchanged)
```rust
impl SshConnectionPool {
    pub fn new() -> Self
    pub async fn connect(&self, config: &SshHostConfig) -> Result<(), String>
    pub async fn disconnect(&self, id: &str) -> Result<(), String>
    pub async fn is_connected(&self, id: &str) -> bool
    pub async fn get_home_dir(&self, id: &str) -> Result<String, String>
    pub async fn resolve_path(&self, id: &str, path: &str) -> Result<String, String>
    pub async fn exec(&self, id: &str, command: &str) -> Result<SshExecResult, String>
    pub async fn exec_login(&self, id: &str, command: &str) -> Result<SshExecResult, String>
    pub async fn sftp_read(&self, id: &str, path: &str) -> Result<String, String>
    pub async fn sftp_write(&self, id: &str, path: &str, content: &str) -> Result<(), String>
    pub async fn sftp_list(&self, id: &str, path: &str) -> Result<Vec<SftpEntry>, String>
    pub async fn sftp_remove(&self, id: &str, path: &str) -> Result<(), String>
}
```

---

### Task 1: Update Cargo.toml Dependencies

**Files:**
- Modify: `src-tauri/Cargo.toml`

**Step 1: Replace SSH dependencies**

Replace:
```toml
russh = "0.46"
russh-keys = "0.46"
russh-sftp = "2.0"
async-trait = "0.1"
```

With:
```toml
openssh = { version = "0.10", features = ["native-mux"] }
openssh-sftp-client = { version = "0.15", features = ["openssh"] }
```

Keep `tokio`, `shellexpand`, and all other dependencies unchanged.

Note: The `native-mux` feature uses a native Rust ControlMaster implementation instead of spawning `ssh -O` processes. The `openssh` feature on sftp-client enables direct `Sftp::from_session()` integration.

**Step 2: Run cargo check to verify resolution**

Run: `cd src-tauri && cargo check 2>&1 | head -50`
Expected: Dependencies resolve (compilation will fail because ssh.rs still uses old imports — that's fine)

**Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "chore: replace russh with openssh + openssh-sftp-client deps"
```

---

### Task 2: Rewrite SshConnectionPool Internals

**Files:**
- Modify: `src-tauri/src/ssh.rs` (complete rewrite of internals, same public API)

**Step 1: Replace the entire `ssh.rs` with the new implementation**

The new implementation:
- Removes all `russh`, `russh-keys`, `russh-sftp`, `async-trait` imports
- Replaces `SshConnection` with `openssh::Session` + cached home_dir
- `connect()` uses `openssh::SessionBuilder` — no manual auth code
- `exec()` uses `session.command().output()`
- SFTP methods use `openssh_sftp_client::Sftp`
- Removes `SshHandler`, `authenticate_with_agent`, `parse_ssh_config`, `parse_ssh_config_identity` — all unnecessary with openssh
- `SshHostConfig` struct unchanged (frontend compatibility)
- `auth_method` field becomes informational only — openssh handles auth automatically via system ssh
- When `auth_method == "key"`, we pass `-i key_path` to SessionBuilder
- When `auth_method == "password"`, we note in error that openssh requires key/agent-based auth

New `ssh.rs` contents:

```rust
use std::collections::HashMap;
use std::path::PathBuf;

use openssh::{KnownHosts, Session, SessionBuilder, Stdio};
use openssh_sftp_client::Sftp;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Data types (unchanged — frontend compatibility)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHostConfig {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// "key" | "ssh_config" | "password"
    pub auth_method: String,
    pub key_path: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

// ---------------------------------------------------------------------------
// Connection wrapper
// ---------------------------------------------------------------------------

struct SshConnection {
    session: Session,
    home_dir: String,
}

// ---------------------------------------------------------------------------
// Connection pool
// ---------------------------------------------------------------------------

pub struct SshConnectionPool {
    connections: Mutex<HashMap<String, SshConnection>>,
}

impl SshConnectionPool {
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
        }
    }

    // -- connect ----------------------------------------------------------

    pub async fn connect(&self, config: &SshHostConfig) -> Result<(), String> {
        // Build destination string
        let dest = if config.username.is_empty() {
            config.host.clone()
        } else {
            format!("{}@{}", config.username, config.host)
        };

        let mut builder = SessionBuilder::default();
        builder.known_hosts_check(KnownHosts::Accept); // match previous behavior (accept all)

        if config.port != 22 {
            builder.port(config.port);
        }

        // If user specified an explicit key, pass it via -i
        if config.auth_method == "key" {
            if let Some(ref key_path) = config.key_path {
                let expanded = shellexpand::tilde(key_path).to_string();
                builder.keyfile(expanded);
            }
        }

        let session = builder
            .connect(&dest)
            .await
            .map_err(|e| format!("SSH connection failed: {e}"))?;

        // Verify the connection is working
        session
            .check()
            .await
            .map_err(|e| format!("SSH connection check failed: {e}"))?;

        // Resolve remote $HOME
        let home_dir = Self::resolve_home_via_session(&session)
            .await
            .unwrap_or_else(|_| "/root".to_string());

        let mut pool = self.connections.lock().await;
        pool.insert(
            config.id.clone(),
            SshConnection { session, home_dir },
        );
        Ok(())
    }

    // -- disconnect -------------------------------------------------------

    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        let mut pool = self.connections.lock().await;
        if let Some(conn) = pool.remove(id) {
            conn.session
                .close()
                .await
                .map_err(|e| format!("SSH disconnect failed: {e}"))?;
        }
        Ok(())
    }

    // -- is_connected -----------------------------------------------------

    pub async fn is_connected(&self, id: &str) -> bool {
        let pool = self.connections.lock().await;
        match pool.get(id) {
            Some(conn) => conn.session.check().await.is_ok(),
            None => false,
        }
    }

    // -- resolve_home (static helper) -------------------------------------

    async fn resolve_home_via_session(session: &Session) -> Result<String, String> {
        let output = session
            .command("echo")
            .raw_arg("$HOME")
            .output()
            .await
            .map_err(|e| format!("Failed to resolve $HOME: {e}"))?;
        let home = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if home.is_empty() {
            Err("Could not resolve remote $HOME".into())
        } else {
            Ok(home)
        }
    }

    pub async fn get_home_dir(&self, id: &str) -> Result<String, String> {
        let pool = self.connections.lock().await;
        let conn = pool.get(id).ok_or_else(|| format!("No connection for id: {id}"))?;
        Ok(conn.home_dir.clone())
    }

    // -- resolve_path -----------------------------------------------------

    pub async fn resolve_path(&self, id: &str, path: &str) -> Result<String, String> {
        if path.starts_with("~/") || path == "~" {
            let home = self.get_home_dir(id).await?;
            Ok(path.replacen('~', &home, 1))
        } else {
            Ok(path.to_string())
        }
    }

    // -- exec -------------------------------------------------------------

    pub async fn exec(&self, id: &str, command: &str) -> Result<SshExecResult, String> {
        let pool = self.connections.lock().await;
        let conn = pool.get(id).ok_or_else(|| format!("No connection for id: {id}"))?;

        let output = conn
            .session
            .raw_command(command)
            .output()
            .await
            .map_err(|e| format!("Failed to exec command: {e}"))?;

        Ok(SshExecResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(1) as u32,
        })
    }

    /// Execute a command with login shell setup (sources profile for PATH).
    pub async fn exec_login(&self, id: &str, command: &str) -> Result<SshExecResult, String> {
        let target_bin = command.split_whitespace().next().unwrap_or("");
        let wrapped = format!(
            concat!(
                ". \"$HOME/.profile\" 2>/dev/null; ",
                ". \"$HOME/.bashrc\" 2>/dev/null; ",
                ". \"$HOME/.zshrc\" 2>/dev/null; ",
                "export NVM_DIR=\"${{NVM_DIR:-$HOME/.nvm}}\"; ",
                "[ -s \"$NVM_DIR/nvm.sh\" ] && . \"$NVM_DIR/nvm.sh\" 2>/dev/null; ",
                "[ -s \"$HOME/.fnm/fnm\" ] && eval \"$($HOME/.fnm/fnm env)\" 2>/dev/null; ",
                "if ! command -v {target_bin} >/dev/null 2>&1; then ",
                  "for d in \"$HOME\"/.nvm/versions/node/*/bin; do ",
                    "[ -x \"$d/{target_bin}\" ] && export PATH=\"$d:$PATH\" && break; ",
                  "done; ",
                "fi; ",
                "{command}"
            ),
            target_bin = target_bin,
            command = command
        );
        self.exec(id, &wrapped).await
    }

    // -- SFTP helpers (private) -------------------------------------------

    async fn open_sftp(&self, id: &str) -> Result<Sftp, String> {
        let pool = self.connections.lock().await;
        let conn = pool.get(id).ok_or_else(|| format!("No connection for id: {id}"))?;

        let sftp = Sftp::from_session(
            conn.session
                .subsystem("sftp")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .await
                .map_err(|e| format!("Failed to open SFTP subsystem: {e}"))?,
            openssh_sftp_client::SftpOptions::default(),
        )
        .await
        .map_err(|e| format!("Failed to initialize SFTP session: {e}"))?;

        // Drop pool lock after SFTP is established
        drop(pool);

        Ok(sftp)
    }

    // -- sftp_read --------------------------------------------------------

    pub async fn sftp_read(&self, id: &str, path: &str) -> Result<String, String> {
        let resolved = self.resolve_path(id, path).await?;
        let sftp = self.open_sftp(id).await?;
        let mut file = sftp
            .open(&resolved)
            .await
            .map_err(|e| format!("SFTP read failed for {resolved}: {e}"))?;

        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .await
            .map_err(|e| format!("SFTP read failed for {resolved}: {e}"))?;

        sftp.close().await.map_err(|e| format!("SFTP close failed: {e}"))?;
        String::from_utf8(data).map_err(|e| format!("File is not valid UTF-8: {e}"))
    }

    // -- sftp_write -------------------------------------------------------

    pub async fn sftp_write(&self, id: &str, path: &str, content: &str) -> Result<(), String> {
        let resolved = self.resolve_path(id, path).await?;
        let sftp = self.open_sftp(id).await?;

        use tokio::io::AsyncWriteExt;
        let mut file = sftp
            .create(&resolved)
            .await
            .map_err(|e| format!("SFTP create failed for {resolved}: {e}"))?;
        file.write_all(content.as_bytes())
            .await
            .map_err(|e| format!("SFTP write failed for {resolved}: {e}"))?;
        file.shutdown()
            .await
            .map_err(|e| format!("SFTP shutdown failed for {resolved}: {e}"))?;

        sftp.close().await.map_err(|e| format!("SFTP close failed: {e}"))?;
        Ok(())
    }

    // -- sftp_list --------------------------------------------------------

    pub async fn sftp_list(&self, id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        let resolved = self.resolve_path(id, path).await?;
        let sftp = self.open_sftp(id).await?;

        let mut read_dir = sftp
            .read_dir(&resolved)
            .await
            .map_err(|e| format!("SFTP read_dir failed for {resolved}: {e}"))?;

        let mut entries = Vec::new();
        while let Some(entry) = read_dir.next().await {
            let entry = entry.map_err(|e| format!("SFTP dir entry error: {e}"))?;
            let filename = entry.filename().to_string_lossy().to_string();
            if filename == "." || filename == ".." {
                continue;
            }
            let file_type = entry.file_type().ok_or("No file type")?;
            entries.push(SftpEntry {
                name: filename,
                is_dir: file_type.is_dir(),
                size: entry.metadata().len().unwrap_or(0),
            });
        }

        sftp.close().await.map_err(|e| format!("SFTP close failed: {e}"))?;
        Ok(entries)
    }

    // -- sftp_remove ------------------------------------------------------

    pub async fn sftp_remove(&self, id: &str, path: &str) -> Result<(), String> {
        let resolved = self.resolve_path(id, path).await?;
        let sftp = self.open_sftp(id).await?;
        sftp.remove_file(&resolved)
            .await
            .map_err(|e| format!("SFTP remove failed for {resolved}: {e}"))?;
        sftp.close().await.map_err(|e| format!("SFTP close failed: {e}"))?;
        Ok(())
    }
}

impl Default for SshConnectionPool {
    fn default() -> Self {
        Self::new()
    }
}
```

Key changes:
- `connect()`: Uses `SessionBuilder` with `KnownHosts::Accept` (matches current behavior). When `auth_method == "key"`, passes key via `.keyfile()`. No manual auth code.
- `exec()`: Uses `session.raw_command(cmd).output()` — single call replaces channel open/exec/wait loop.
- `exec_login()`: Same shell wrapper as before, delegates to `exec()`.
- `open_sftp()`: Uses `Sftp::from_session()` via the subsystem API.
- `sftp_read/write/list/remove`: Adapted to `openssh-sftp-client` API (very similar to russh-sftp).
- Removed: `SshHandler`, `authenticate_with_agent`, `parse_ssh_config`, `parse_ssh_config_identity`, all `russh` imports.

**Step 2: Run cargo check**

Run: `cd src-tauri && cargo check 2>&1`
Expected: Compiles cleanly. If there are API differences in openssh-sftp-client (method names, types), fix them iteratively.

**Step 3: Commit**

```bash
git add src-tauri/src/ssh.rs
git commit -m "refactor: rewrite SSH layer using openssh + openssh-sftp-client

Replaces russh (manual auth) with openssh (system ssh binary).
All auth methods now automatically supported via user's ssh config."
```

---

### Task 3: Handle API Differences and Compile Errors

**Files:**
- Modify: `src-tauri/src/ssh.rs` (fix any API mismatches found during cargo check)

This task handles the inevitable API differences between the plan and actual crate APIs. The openssh-sftp-client API may differ from what's documented — method names like `read_dir`, `create`, `open` may have slightly different signatures or return types.

**Step 1: Run cargo check and fix each error**

Run: `cd src-tauri && cargo check 2>&1`

Common fixes expected:
- `Sftp::from_session()` may need different arguments (the subsystem child process, not the session directly)
- `read_dir` may return a different iterator type
- `entry.filename()` vs `entry.file_name()`
- `entry.metadata().len()` vs `entry.metadata().size`
- `session.raw_command()` might not exist — may need `session.shell(command)` or `session.command("sh").arg("-c").arg(command)`
- `builder.keyfile()` might be `builder.identity()` or need raw args

**Step 2: Iterate until cargo check passes**

**Step 3: Run cargo build to verify full compilation**

Run: `cd src-tauri && cargo build 2>&1 | tail -20`
Expected: Compiles successfully.

**Step 4: Commit**

```bash
git add src-tauri/src/ssh.rs
git commit -m "fix: resolve openssh API differences for compilation"
```

---

### Task 4: Update Frontend SSH UI

**Files:**
- Modify: `src/components/InstanceTabBar.tsx`

Two changes:

#### A. Fix Username auto-capitalization bug

User reported `Invalid user Root` in SSH logs — macOS auto-capitalizes the first letter in Input fields. The Username input (line 211-216) needs `autoCapitalize="off"` and `autoCorrect="off"` to prevent this. Also add `spellCheck={false}` for good measure.

Apply to Username input at line 212:
```tsx
<Input
  id="ssh-username"
  value={form.username}
  onChange={(e) => setForm((f) => ({ ...f, username: e.target.value }))}
  placeholder="(optional, defaults to current user)"
  autoCapitalize="off"
  autoCorrect="off"
  spellCheck={false}
/>
```

Also apply `autoCapitalize="off"` to Host input (line 191) and Key Path input (line 244) — these are also case-sensitive values that should never be auto-capitalized.

#### B. Update password auth method warning

Since openssh delegates auth to the system ssh binary, password auth requires `sshpass` or ssh-agent. When password is selected, update the warning text:

```tsx
<p className="text-xs text-yellow-600 dark:text-yellow-400">
  Password auth requires sshpass on the system. Recommended: use SSH Config / Agent mode instead.
</p>
```

**Step 2: Verify frontend builds**

Run: `cd /Users/zhixian/Codes/clawpal && npm run build 2>&1 | tail -10`
Expected: Builds cleanly.

**Step 3: Commit**

```bash
git add src/components/InstanceTabBar.tsx
git commit -m "fix: prevent username auto-capitalization in SSH config UI

macOS auto-capitalizes Input fields, causing 'Invalid user Root' errors.
Add autoCapitalize=off to username, host, and key path inputs."
```

---

### Task 5: Integration Testing

**Step 1: Run cargo build to verify full compilation**

Run: `cd src-tauri && cargo build 2>&1 | tail -20`
Expected: Compiles.

**Step 2: Run npm build to verify frontend**

Run: `npm run build 2>&1 | tail -10`
Expected: Compiles.

**Step 3: Manual testing checklist**

Test locally:
- [ ] App launches
- [ ] SSH Config mode connects to a known host
- [ ] Private Key mode connects with explicit key path
- [ ] `exec` runs commands on remote
- [ ] `sftp_read` reads remote files
- [ ] `sftp_write` writes remote files
- [ ] `sftp_list` lists remote directories
- [ ] `sftp_remove` deletes remote files
- [ ] Disconnect works cleanly
- [ ] Reconnect after disconnect works
- [ ] All remote pages (Home, Channels, History, Settings, Doctor) load data

---

### Task 6: Cleanup Old Dependencies

**Files:**
- Modify: `src-tauri/Cargo.toml` — remove `async-trait` if no longer used elsewhere

**Step 1: Check if async-trait is used outside ssh.rs**

Run: `grep -r "async_trait\|async-trait" src-tauri/src/ --include="*.rs" | grep -v ssh.rs`

If not used: remove `async-trait = "0.1"` from Cargo.toml.

**Step 2: Verify cargo check still passes**

Run: `cd src-tauri && cargo check 2>&1`

**Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "chore: remove unused russh dependencies"
```

---

## Files Summary

| File | Action | Description |
|------|--------|-------------|
| `src-tauri/Cargo.toml` | Modify | Replace russh deps with openssh + openssh-sftp-client |
| `src-tauri/src/ssh.rs` | Rewrite | New internals using openssh, same public API |
| `src/components/InstanceTabBar.tsx` | Modify | Fix username auto-capitalization + update password auth warning |

## What Does NOT Change

- `src-tauri/src/commands.rs` — all 45+ remote commands remain untouched
- `src-tauri/src/lib.rs` — SshConnectionPool initialization unchanged
- `src/lib/api.ts` — all API bindings unchanged
- All frontend pages — no changes needed

## Risk Mitigation

- **openssh requires system ssh binary**: macOS and Linux always have it. Windows users would need OpenSSH installed (already common on Win 10+).
- **KnownHosts::Accept**: Matches current behavior (russh accepts all host keys). Can be tightened to `KnownHosts::Add` later.
- **Password auth**: Not natively supported by openssh crate. Users should use key-based auth or ssh-agent. This is a deliberate trade-off — the problematic user's issue is precisely that key auth doesn't work with russh but will work with system ssh.
