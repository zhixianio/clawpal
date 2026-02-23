# SSH Migration: russh → openssh (No SFTP) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace `russh` with `openssh` crate, and replace all SFTP operations with SSH exec-based equivalents (cat, base64, stat, rm). No SFTP dependency.

**Architecture:** The current `SshConnectionPool` stores `russh` handles and manually implements auth, config parsing, exec, and SFTP. We replace everything with `openssh::Session` (which delegates to the system `ssh` binary via ControlMaster multiplexing). File operations use shell commands over exec instead of SFTP protocol. The public API stays identical — all 45+ commands in `commands.rs` continue working without changes.

**Tech Stack:** `openssh` 0.11 (wraps system ssh, `native-mux` feature), `base64` 0.22 (for safe file writes), `tokio`

---

### Task 1: Update Cargo.toml Dependencies

**Files:**
- Modify: `src-tauri/Cargo.toml`

**Step 1: Replace SSH dependencies in Cargo.toml**

Replace these 4 lines:
```toml
russh = "0.46"
russh-keys = "0.46"
russh-sftp = "2.0"
async-trait = "0.1"
```

With these 2 lines:
```toml
openssh = { version = "0.11", features = ["native-mux"] }
base64 = "0.22"
```

Keep all other dependencies (`tokio`, `shellexpand`, `serde`, `dirs`, etc.) unchanged.

Note: `native-mux` uses a native Rust ControlMaster implementation instead of spawning `ssh -O` processes. `base64` is needed for safe file content transfer in `sftp_write`.

**Step 2: Run cargo check to verify dependency resolution**

Run: `cd src-tauri && cargo check 2>&1 | head -20`
Expected: Dependencies resolve. Compilation will fail because `ssh.rs` still uses old imports — that's expected and fine.

**Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "chore: replace russh with openssh + base64 deps"
```

---

### Task 2: Rewrite ssh.rs — Connection Pool Core

**Files:**
- Modify: `src-tauri/src/ssh.rs` (complete rewrite)

**Step 1: Replace the entire ssh.rs with the new implementation**

Write the complete new `ssh.rs`. The file replaces all 739 lines. Here is the exact content:

```rust
use std::collections::HashMap;

use base64::Engine;
use openssh::{KnownHosts, Session, SessionBuilder};
use serde::{Deserialize, Serialize};
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
        if config.auth_method == "password" {
            return Err(
                "Password authentication is not supported with openssh. \
                 Please use SSH Config or Private Key mode instead. \
                 If your key is in ssh-agent, select SSH Config mode."
                    .into(),
            );
        }

        // Build destination string
        let dest = if config.username.is_empty() {
            config.host.clone()
        } else {
            format!("{}@{}", config.username, config.host)
        };

        let mut builder = SessionBuilder::default();
        builder.known_hosts_check(KnownHosts::Accept);

        if config.port != 22 {
            builder.port(config.port);
        }

        // ServerAliveInterval to detect dead connections
        builder.server_alive_interval(std::time::Duration::from_secs(30));

        // Connect timeout
        builder.connect_timeout(std::time::Duration::from_secs(15));

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
        pool.insert(config.id.clone(), SshConnection { session, home_dir });
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
            .raw_command("echo $HOME")
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
        let conn = pool
            .get(id)
            .ok_or_else(|| format!("No connection for id: {id}"))?;
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
        let conn = pool
            .get(id)
            .ok_or_else(|| format!("No connection for id: {id}"))?;

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

    // -- SFTP-equivalent operations via exec ------------------------------

    /// Read a remote file via `cat`.
    pub async fn sftp_read(&self, id: &str, path: &str) -> Result<String, String> {
        let resolved = self.resolve_path(id, path).await?;
        let cmd = format!("cat '{}'", resolved.replace('\'', "'\\''"));
        let result = self.exec(id, &cmd).await?;
        if result.exit_code != 0 {
            return Err(format!(
                "Failed to read {resolved}: {}",
                result.stderr.trim()
            ));
        }
        Ok(result.stdout)
    }

    /// Write content to a remote file via base64 encoding.
    pub async fn sftp_write(&self, id: &str, path: &str, content: &str) -> Result<(), String> {
        let resolved = self.resolve_path(id, path).await?;
        let escaped_path = resolved.replace('\'', "'\\''");
        let b64 = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
        let cmd = format!(
            "mkdir -p \"$(dirname '{}')\" && printf '%s' '{}' | base64 -d > '{}'",
            escaped_path, b64, escaped_path
        );
        let result = self.exec(id, &cmd).await?;
        if result.exit_code != 0 {
            return Err(format!(
                "Failed to write {resolved}: {}",
                result.stderr.trim()
            ));
        }
        Ok(())
    }

    /// List a remote directory via `stat`.
    pub async fn sftp_list(&self, id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        let resolved = self.resolve_path(id, path).await?;
        let escaped_path = resolved.replace('\'', "'\\''");
        // Use stat to get name, type, and size in a parseable format.
        // --format is GNU stat (Linux). Output: full_path\tfile_type\tsize
        let cmd = format!(
            "stat --format='%n\t%F\t%s' '{}'/* 2>/dev/null || true",
            escaped_path
        );
        let result = self.exec(id, &cmd).await?;

        let mut entries = Vec::new();
        for line in result.stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 3 {
                continue;
            }
            let full_path = parts[0];
            let file_type = parts[1];
            let size: u64 = parts[2].parse().unwrap_or(0);

            // Extract basename from full path
            let name = full_path
                .rsplit('/')
                .next()
                .unwrap_or(full_path)
                .to_string();

            if name == "." || name == ".." || name.is_empty() {
                continue;
            }

            entries.push(SftpEntry {
                name,
                is_dir: file_type == "directory",
                size,
            });
        }
        Ok(entries)
    }

    /// Delete a remote file via `rm`.
    pub async fn sftp_remove(&self, id: &str, path: &str) -> Result<(), String> {
        let resolved = self.resolve_path(id, path).await?;
        let cmd = format!("rm '{}'", resolved.replace('\'', "'\\''"));
        let result = self.exec(id, &cmd).await?;
        if result.exit_code != 0 {
            return Err(format!(
                "Failed to remove {resolved}: {}",
                result.stderr.trim()
            ));
        }
        Ok(())
    }
}

impl Default for SshConnectionPool {
    fn default() -> Self {
        Self::new()
    }
}
```

Key changes from old implementation:
- **739 → ~230 lines** (67% reduction)
- `connect()`: Uses `SessionBuilder` — no manual auth code, no config parsing, no agent handling
- `exec()`: Single `raw_command().output()` call replaces channel open/exec/wait loop
- `sftp_read/write/list/remove`: Shell commands via `exec()` instead of SFTP protocol
- Removed: `SshHandler`, `authenticate_with_agent`, `parse_ssh_config`, `ensure_alive`, `force_reconnect`, `reconnect_locks`, `open_sftp`
- Shell quoting: single-quote paths with `'\\''` escape for embedded quotes

**Step 2: Run cargo check**

Run: `cd src-tauri && cargo check 2>&1`
Expected: Compiles cleanly (or with minor API tweaks needed — see Task 3).

**Step 3: Commit**

```bash
git add src-tauri/src/ssh.rs
git commit -m "refactor: rewrite SSH layer using openssh, replace SFTP with exec

Replaces russh (manual auth) with openssh (system ssh binary).
File operations now use cat/base64/stat/rm instead of SFTP protocol.
All auth methods automatically supported via user's ssh config."
```

---

### Task 3: Fix Compilation Errors

**Files:**
- Modify: `src-tauri/src/ssh.rs` (fix any API mismatches)

**Step 1: Run cargo check and fix each error iteratively**

Run: `cd src-tauri && cargo check 2>&1`

Known potential issues from openssh 0.11 API:
- `raw_command()` takes `AsRef<OsStr>` — string literals work fine
- `output()` returns `Result<std::process::Output, openssh::Error>`
- `output.status.code()` returns `Option<i32>` — cast to `u32` with `as u32`
- `Session::close(self)` takes ownership — must `pool.remove(id)` first (already done in `disconnect`)
- `SessionBuilder::connect()` is async and returns `Result<Session, Error>`

Fix any compile errors found.

**Step 2: Run cargo build to verify full compilation**

Run: `cd src-tauri && cargo build 2>&1 | tail -20`
Expected: Compiles successfully.

**Step 3: Commit (only if changes were needed)**

```bash
git add src-tauri/src/ssh.rs
git commit -m "fix: resolve openssh API differences for compilation"
```

---

### Task 4: Update Frontend Password Auth Warning

**Files:**
- Modify: `src/components/InstanceTabBar.tsx`

Note: Auto-capitalization fix (autoCapitalize="off") is already in place on Host, Username, and Key Path inputs. Only the password warning text needs updating.

**Step 1: Find and update the password warning text**

The password warning is at approximately line 303-305 using the translation key `instance.passwordWarning`. Find the translation file and update the warning text to mention that openssh requires key/agent auth.

Alternatively, if the warning is a simple hardcoded string, update it directly.

Run: `grep -rn "passwordWarning" src/` to find the translation.

Update the warning to something like: "Password authentication is not supported. Please use SSH Config or Private Key mode."

**Step 2: Verify frontend builds**

Run: `npm run build 2>&1 | tail -10`
Expected: Builds cleanly.

**Step 3: Commit**

```bash
git add -A
git commit -m "fix: update password auth warning for openssh migration"
```

---

### Task 5: Build Verification

**Step 1: Full cargo build**

Run: `cd src-tauri && cargo build 2>&1 | tail -20`
Expected: Compiles.

**Step 2: Frontend build**

Run: `npm run build 2>&1 | tail -10`
Expected: Compiles.

**Step 3: Manual testing checklist**

Test locally with `npm run tauri dev`:
- [ ] App launches
- [ ] SSH Config mode connects to a known host
- [ ] Private Key mode connects with explicit key path
- [ ] `exec` runs commands on remote
- [ ] File read works (remote config pages load)
- [ ] File write works (saving config changes)
- [ ] Directory listing works (snapshots page)
- [ ] Disconnect works cleanly
- [ ] Reconnect after disconnect works
- [ ] All remote pages (Home, Channels, History, Settings, Doctor) load data

---

### Task 6: Cleanup Old Dependencies

**Files:**
- Modify: `src-tauri/Cargo.toml` (if needed)

**Step 1: Check if async-trait is used outside ssh.rs**

Run: `grep -rn "async_trait\|async-trait" src-tauri/src/ --include="*.rs"`
If not used anywhere: it was already removed in Task 1. Verify it's gone.

**Step 2: Check if dirs crate is still needed**

The old `parse_ssh_config` used `dirs::home_dir()`. Since that code is removed, check if `dirs` is used elsewhere:
Run: `grep -rn "dirs::" src-tauri/src/ --include="*.rs"`
If still used elsewhere: keep it. If not: remove from Cargo.toml.

**Step 3: Verify cargo check still passes**

Run: `cd src-tauri && cargo check 2>&1`

**Step 4: Commit (only if changes were made)**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "chore: remove unused dependencies"
```

---

## Files Summary

| File | Action | Description |
|------|--------|-------------|
| `src-tauri/Cargo.toml` | Modify | Replace russh/russh-keys/russh-sftp/async-trait with openssh + base64 |
| `src-tauri/src/ssh.rs` | Rewrite | 739→~230 lines. openssh internals, exec-based file ops, no auto-reconnect |
| `src/components/InstanceTabBar.tsx` | Modify | Update password auth warning text |
| Translation file (TBD) | Modify | Update passwordWarning translation |

## What Does NOT Change

- `src-tauri/src/commands.rs` — all 45+ remote commands untouched
- `src-tauri/src/lib.rs` — SshConnectionPool initialization unchanged
- `src/lib/api.ts` — all API bindings unchanged
- All frontend pages — no changes needed

## Risk Mitigation

- **openssh requires system ssh binary**: macOS/Linux always have it. Windows needs OpenSSH (common on Win 10+).
- **KnownHosts::Accept**: Matches current behavior (russh accepted all host keys). Can tighten later.
- **Base64 for file writes**: `base64` command available on all Linux servers. Content-safe transfer for any file content.
- **GNU stat for directory listing**: Assumed Linux remote servers. If macOS remote support needed, can add `stat -f` fallback later.
- **Password auth rejected**: Deliberate trade-off. The auth issues users report are precisely because russh's manual auth doesn't work with their setup. openssh + system ssh agent solves this.
- **No auto-reconnect**: openssh ControlMaster handles connection persistence natively. If connection drops, user reconnects manually — same UX as terminal SSH.
