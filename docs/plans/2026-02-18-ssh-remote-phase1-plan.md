# SSH Remote Management — Phase 1 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable ClawPal to connect to remote VPS instances via SSH and display their openclaw status, with a tab-based UI for switching between local and remote instances.

**Architecture:** Add `russh` + `russh-sftp` for async SSH/SFTP in Rust. New `ssh.rs` module provides connection pool + primitives. Frontend gets an Instance Tab Bar above the main content area, with a React context-based API router so pages can work against either local or remote backends.

**Tech Stack:** `russh` 0.46+, `russh-sftp` 2.0+, `russh-keys` 0.46+, Tauri async commands, React Context API.

**Design doc:** `docs/plans/2026-02-18-ssh-remote-management-design.md`

---

## Task 1: Add SSH dependencies to Cargo.toml

**Files:**
- Modify: `src-tauri/Cargo.toml`

**Step 1: Add russh dependencies**

Add to `[dependencies]` in `src-tauri/Cargo.toml`:

```toml
russh = "0.46"
russh-keys = "0.46"
russh-sftp = "2.0"
async-trait = "0.1"
tokio = { version = "1", features = ["sync"] }
```

Notes:
- `russh` requires a crypto backend. It defaults to `aws-lc-rs` which should work.
- `tokio` is already a transitive dep via Tauri, but we need the `sync` feature explicitly for `Mutex`/`RwLock`.
- `async-trait` is needed by `russh`'s `Handler` trait.

**Step 2: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: compiles successfully (no code changes yet, just dependency resolution).

**Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "chore: add russh SSH/SFTP dependencies"
```

---

## Task 2: Create SSH module with connection pool and primitives

**Files:**
- Create: `src-tauri/src/ssh.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod ssh;`)

This is the core SSH infrastructure. It provides:
- `SshConnectionPool`: a global pool of SSH connections keyed by instance ID
- `connect()`: establish SSH connection using key auth or ssh-agent
- `exec()`: run a command over SSH, return stdout/stderr/exit_code
- `sftp_read()`: read a remote file as String
- `sftp_write()`: write a String to a remote file
- `sftp_list()`: list a remote directory
- `sftp_remove()`: delete a remote file
- `disconnect()`: close a connection

**Step 1: Create `src-tauri/src/ssh.rs`**

```rust
use std::collections::HashMap;
use std::sync::Arc;

use russh::client;
use russh_keys::key;
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// SSH connection pool — global singleton, keyed by instance ID.
pub struct SshConnectionPool {
    connections: Mutex<HashMap<String, SshConnection>>,
}

struct SshConnection {
    session: client::Handle<SshHandler>,
    sftp: Option<SftpSession>,
}

/// Minimal client handler for russh.
struct SshHandler;

impl client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // Accept all host keys (like StrictHostKeyChecking=no).
        // TODO Phase 3: implement known_hosts checking
        Ok(true)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshHostConfig {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String, // "key" | "ssh_config"
    pub key_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

impl SshConnectionPool {
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
        }
    }

    /// Connect to a remote host. If already connected, reuse.
    pub async fn connect(&self, config: &SshHostConfig) -> Result<(), String> {
        let mut conns = self.connections.lock().await;
        if conns.contains_key(&config.id) {
            return Ok(());
        }

        let russh_config = Arc::new(client::Config::default());
        let handler = SshHandler;
        let addr = format!("{}:{}", config.host, config.port);

        let mut session = client::connect(russh_config, &addr, handler)
            .await
            .map_err(|e| format!("SSH connect failed: {}", e))?;

        // Authenticate
        let authenticated = if config.auth_method == "key" {
            let key_path = config
                .key_path
                .as_deref()
                .ok_or_else(|| "key_path is required for key auth".to_string())?;
            let expanded = shellexpand::tilde(key_path).to_string();
            let key_pair = russh_keys::load_secret_key(&expanded, None)
                .map_err(|e| format!("Failed to load key {}: {}", key_path, e))?;
            session
                .authenticate_publickey(&config.username, Arc::new(key_pair))
                .await
                .map_err(|e| format!("Key auth failed: {}", e))?
        } else {
            // ssh_config mode: try ssh-agent
            let mut agent = russh_keys::agent::client::AgentClient::connect_env()
                .await
                .map_err(|e| format!("Cannot connect to ssh-agent: {}", e))?;
            let identities = agent
                .request_identities()
                .await
                .map_err(|e| format!("Failed to list agent identities: {}", e))?;
            let mut authed = false;
            for identity in identities {
                if let Ok(true) = session
                    .authenticate_publickey_with(&config.username, Arc::new(identity), &mut agent)
                    .await
                {
                    authed = true;
                    break;
                }
            }
            authed
        };

        if !authenticated {
            return Err("Authentication failed".into());
        }

        conns.insert(
            config.id.clone(),
            SshConnection {
                session,
                sftp: None,
            },
        );
        Ok(())
    }

    /// Disconnect a specific instance.
    pub async fn disconnect(&self, instance_id: &str) -> Result<(), String> {
        let mut conns = self.connections.lock().await;
        if let Some(conn) = conns.remove(instance_id) {
            conn.session
                .disconnect(russh::Disconnect::ByApplication, "", "en")
                .await
                .ok();
        }
        Ok(())
    }

    /// Check if an instance is connected.
    pub async fn is_connected(&self, instance_id: &str) -> bool {
        let conns = self.connections.lock().await;
        conns.contains_key(instance_id)
    }

    /// Execute a command on the remote host.
    pub async fn exec(&self, instance_id: &str, command: &str) -> Result<SshExecResult, String> {
        let mut conns = self.connections.lock().await;
        let conn = conns
            .get_mut(instance_id)
            .ok_or_else(|| "Not connected".to_string())?;

        let mut channel = conn
            .session
            .channel_open_session()
            .await
            .map_err(|e| format!("Failed to open channel: {}", e))?;

        channel
            .exec(true, command)
            .await
            .map_err(|e| format!("Failed to exec: {}", e))?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_code = 0u32;

        while let Some(msg) = channel.wait().await {
            match msg {
                russh::ChannelMsg::Data { data } => {
                    stdout.extend_from_slice(&data);
                }
                russh::ChannelMsg::ExtendedData { data, ext } => {
                    if ext == 1 {
                        stderr.extend_from_slice(&data);
                    }
                }
                russh::ChannelMsg::ExitStatus { exit_status } => {
                    exit_code = exit_status;
                }
                _ => {}
            }
        }

        Ok(SshExecResult {
            stdout: String::from_utf8_lossy(&stdout).to_string(),
            stderr: String::from_utf8_lossy(&stderr).to_string(),
            exit_code,
        })
    }

    /// Get or create an SFTP session for the given instance.
    async fn get_sftp(&self, instance_id: &str) -> Result<SftpSession, String> {
        let mut conns = self.connections.lock().await;
        let conn = conns
            .get_mut(instance_id)
            .ok_or_else(|| "Not connected".to_string())?;

        if let Some(ref sftp) = conn.sftp {
            return Ok(sftp.clone());
        }

        let channel = conn
            .session
            .channel_open_session()
            .await
            .map_err(|e| format!("Failed to open SFTP channel: {}", e))?;

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| format!("Failed to request SFTP subsystem: {}", e))?;

        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| format!("Failed to init SFTP: {}", e))?;

        conn.sftp = Some(sftp.clone());
        Ok(sftp)
    }

    /// Read a remote file as a String.
    pub async fn sftp_read(&self, instance_id: &str, path: &str) -> Result<String, String> {
        let sftp = self.get_sftp(instance_id).await?;
        let mut file = sftp
            .open(path)
            .await
            .map_err(|e| format!("SFTP open failed: {}", e))?;

        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .await
            .map_err(|e| format!("SFTP read failed: {}", e))?;

        String::from_utf8(buf).map_err(|e| format!("UTF-8 decode error: {}", e))
    }

    /// Write a String to a remote file.
    pub async fn sftp_write(
        &self,
        instance_id: &str,
        path: &str,
        content: &str,
    ) -> Result<(), String> {
        let sftp = self.get_sftp(instance_id).await?;
        let mut file = sftp
            .create(path)
            .await
            .map_err(|e| format!("SFTP create failed: {}", e))?;

        use tokio::io::AsyncWriteExt;
        file.write_all(content.as_bytes())
            .await
            .map_err(|e| format!("SFTP write failed: {}", e))?;
        file.flush()
            .await
            .map_err(|e| format!("SFTP flush failed: {}", e))?;

        Ok(())
    }

    /// List a remote directory.
    pub async fn sftp_list(&self, instance_id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        let sftp = self.get_sftp(instance_id).await?;
        let entries = sftp
            .read_dir(path)
            .await
            .map_err(|e| format!("SFTP readdir failed: {}", e))?;

        Ok(entries
            .into_iter()
            .filter(|e| {
                let name = e.file_name();
                name != "." && name != ".."
            })
            .map(|e| SftpEntry {
                name: e.file_name(),
                is_dir: e.file_type().is_dir(),
                size: e.metadata().size.unwrap_or(0),
            })
            .collect())
    }

    /// Remove a remote file.
    pub async fn sftp_remove(&self, instance_id: &str, path: &str) -> Result<(), String> {
        let sftp = self.get_sftp(instance_id).await?;
        sftp.remove_file(path)
            .await
            .map_err(|e| format!("SFTP remove failed: {}", e))?;
        Ok(())
    }
}
```

**Step 2: Add `pub mod ssh;` to lib.rs**

In `src-tauri/src/lib.rs`, add after the existing module declarations:

```rust
pub mod ssh;
```

**Step 3: Add `shellexpand` dependency for `~` expansion in key paths**

In `src-tauri/Cargo.toml`, add:

```toml
shellexpand = "3.1"
```

**Step 4: Verify it compiles**

Run: `cd src-tauri && cargo check`

Note: The russh API may differ slightly from the code above. This step may require adjusting method signatures based on compiler feedback. The intent and structure are correct — adjust types/methods as needed to match the actual russh API.

**Step 5: Commit**

```bash
git add src-tauri/src/ssh.rs src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: add SSH connection pool with exec and SFTP primitives"
```

---

## Task 3: Remote instance config CRUD (Rust commands)

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

CRUD for remote instance configurations, persisted to `.clawpal/remote-instances.json`.

**Step 1: Add commands to `commands.rs`**

Add at the end of `commands.rs`:

```rust
use crate::ssh::SshHostConfig;

#[tauri::command]
pub fn list_ssh_hosts() -> Result<Vec<SshHostConfig>, String> {
    let paths = resolve_paths();
    let hosts_path = paths.clawpal_dir.join("remote-instances.json");
    if !hosts_path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&hosts_path).map_err(|e| e.to_string())?;
    let hosts: Vec<SshHostConfig> = serde_json::from_str(&text).unwrap_or_default();
    Ok(hosts)
}

#[tauri::command]
pub fn upsert_ssh_host(host: SshHostConfig) -> Result<SshHostConfig, String> {
    let paths = resolve_paths();
    let hosts_path = paths.clawpal_dir.join("remote-instances.json");

    let mut hosts: Vec<SshHostConfig> = if hosts_path.exists() {
        let text = fs::read_to_string(&hosts_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).unwrap_or_default()
    } else {
        Vec::new()
    };

    // Upsert: replace existing or append
    if let Some(pos) = hosts.iter().position(|h| h.id == host.id) {
        hosts[pos] = host.clone();
    } else {
        hosts.push(host.clone());
    }

    let json = serde_json::to_string_pretty(&hosts).map_err(|e| e.to_string())?;
    fs::write(&hosts_path, json).map_err(|e| e.to_string())?;
    Ok(host)
}

#[tauri::command]
pub fn delete_ssh_host(host_id: String) -> Result<bool, String> {
    let paths = resolve_paths();
    let hosts_path = paths.clawpal_dir.join("remote-instances.json");
    if !hosts_path.exists() {
        return Ok(false);
    }

    let text = fs::read_to_string(&hosts_path).map_err(|e| e.to_string())?;
    let mut hosts: Vec<SshHostConfig> = serde_json::from_str(&text).unwrap_or_default();
    let before = hosts.len();
    hosts.retain(|h| h.id != host_id);
    if hosts.len() == before {
        return Ok(false);
    }

    let json = serde_json::to_string_pretty(&hosts).map_err(|e| e.to_string())?;
    fs::write(&hosts_path, json).map_err(|e| e.to_string())?;
    Ok(true)
}
```

**Step 2: Register commands in `lib.rs`**

Add `list_ssh_hosts, upsert_ssh_host, delete_ssh_host` to the import block and `generate_handler!`.

**Step 3: Verify**

Run: `cd src-tauri && cargo check`

**Step 4: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: add remote instance config CRUD commands"
```

---

## Task 4: SSH connect/disconnect/status Tauri commands

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

Wire up the SSH pool to Tauri commands. The pool is stored as Tauri managed state.

**Step 1: Initialize pool as Tauri managed state**

In `src-tauri/src/lib.rs`, modify the `run()` function:

```rust
use crate::ssh::SshConnectionPool;

pub fn run() {
    tauri::Builder::default()
        .manage(SshConnectionPool::new())
        .invoke_handler(tauri::generate_handler![
            // ... existing commands ...
            ssh_connect,
            ssh_disconnect,
            ssh_status,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run app");
}
```

**Step 2: Add SSH commands to `commands.rs`**

```rust
use crate::ssh::SshConnectionPool;
use tauri::State;

#[tauri::command]
pub async fn ssh_connect(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<bool, String> {
    // Load host config from file
    let hosts = list_ssh_hosts()?;
    let config = hosts
        .into_iter()
        .find(|h| h.id == host_id)
        .ok_or_else(|| format!("Host {} not found", host_id))?;

    pool.connect(&config).await?;
    Ok(true)
}

#[tauri::command]
pub async fn ssh_disconnect(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<bool, String> {
    pool.disconnect(&host_id).await?;
    Ok(true)
}

#[tauri::command]
pub async fn ssh_status(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<String, String> {
    if pool.is_connected(&host_id).await {
        Ok("connected".into())
    } else {
        Ok("disconnected".into())
    }
}
```

**Step 3: Register commands in `lib.rs`**

Add `ssh_connect, ssh_disconnect, ssh_status` to imports and `generate_handler!`.

**Step 4: Verify**

Run: `cd src-tauri && cargo check`

**Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: add SSH connect/disconnect/status commands with managed pool"
```

---

## Task 5: SSH exec and SFTP Tauri commands

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

Expose the SSH pool's exec and SFTP primitives as Tauri commands.

**Step 1: Add commands to `commands.rs`**

```rust
use crate::ssh::{SshExecResult, SftpEntry};

#[tauri::command]
pub async fn ssh_exec(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    command: String,
) -> Result<SshExecResult, String> {
    pool.exec(&host_id, &command).await
}

#[tauri::command]
pub async fn sftp_read_file(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    path: String,
) -> Result<String, String> {
    pool.sftp_read(&host_id, &path).await
}

#[tauri::command]
pub async fn sftp_write_file(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    path: String,
    content: String,
) -> Result<bool, String> {
    pool.sftp_write(&host_id, &path, &content).await?;
    Ok(true)
}

#[tauri::command]
pub async fn sftp_list_dir(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    path: String,
) -> Result<Vec<SftpEntry>, String> {
    pool.sftp_list(&host_id, &path).await
}

#[tauri::command]
pub async fn sftp_remove_file(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    path: String,
) -> Result<bool, String> {
    pool.sftp_remove(&host_id, &path).await?;
    Ok(true)
}
```

**Step 2: Register all in `lib.rs`**

Add `ssh_exec, sftp_read_file, sftp_write_file, sftp_list_dir, sftp_remove_file` to imports and `generate_handler!`.

**Step 3: Verify**

Run: `cd src-tauri && cargo check`

**Step 4: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: add SSH exec and SFTP Tauri commands"
```

---

## Task 6: Remote business commands (read config + system status)

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

Compose the SSH primitives into business-level commands that return the same types as local commands.

**Step 1: Add `remote_read_raw_config`**

```rust
#[tauri::command]
pub async fn remote_read_raw_config(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Value, String> {
    let content = pool.sftp_read(&host_id, "~/.openclaw/openclaw.json").await
        .or_else(|_| pool.sftp_read(&host_id, ".openclaw/openclaw.json").await)?;
    let config: Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse remote config: {}", e))?;
    Ok(config)
}
```

**Step 2: Add `remote_get_system_status`**

```rust
#[tauri::command]
pub async fn remote_get_system_status(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Value, String> {
    // Get openclaw version
    let version_result = pool.exec(&host_id, "openclaw --version").await;
    let version = version_result
        .map(|r| r.stdout.trim().to_string())
        .unwrap_or_else(|_| "unknown".into());

    // Read config
    let config_content = pool.sftp_read(&host_id, "~/.openclaw/openclaw.json").await
        .or_else(|_| pool.sftp_read(&host_id, ".openclaw/openclaw.json").await)?;
    let config: Value = serde_json::from_str(&config_content)
        .map_err(|e| format!("Failed to parse config: {}", e))?;

    // Check gateway health
    let gateway_port = config
        .pointer("/gateway/port")
        .and_then(Value::as_u64)
        .unwrap_or(8080);
    let health_result = pool
        .exec(&host_id, &format!("curl -sf http://localhost:{}/health 2>/dev/null && echo OK || echo FAIL", gateway_port))
        .await;
    let healthy = health_result
        .map(|r| r.stdout.contains("OK"))
        .unwrap_or(false);

    // Count agents
    let agents = config
        .pointer("/agents/list")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);

    // Get global model
    let global_model = config
        .pointer("/agents/defaults/model")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    Ok(serde_json::json!({
        "healthy": healthy,
        "openclawVersion": version,
        "activeAgents": agents,
        "globalDefaultModel": global_model,
        "configPath": "~/.openclaw/openclaw.json",
        "openclawDir": "~/.openclaw",
    }))
}
```

**Step 3: Register in `lib.rs`**

Add `remote_read_raw_config, remote_get_system_status` to imports and `generate_handler!`.

**Step 4: Verify**

Run: `cd src-tauri && cargo check`

**Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: add remote config read and system status commands"
```

---

## Task 7: Frontend types and API functions

**Files:**
- Modify: `src/lib/types.ts`
- Modify: `src/lib/api.ts`

**Step 1: Add types to `types.ts`**

```typescript
export interface SshHost {
  id: string;
  label: string;
  host: string;
  port: number;
  username: string;
  authMethod: "key" | "ssh_config";
  keyPath?: string;
}

export interface SshExecResult {
  stdout: string;
  stderr: string;
  exitCode: number;
}

export interface SftpEntry {
  name: string;
  isDir: boolean;
  size: number;
}
```

**Step 2: Add API functions to `api.ts`**

Add to the `api` object:

```typescript
  // SSH host management
  listSshHosts: (): Promise<SshHost[]> =>
    invoke("list_ssh_hosts", {}),
  upsertSshHost: (host: SshHost): Promise<SshHost> =>
    invoke("upsert_ssh_host", { host }),
  deleteSshHost: (hostId: string): Promise<boolean> =>
    invoke("delete_ssh_host", { hostId }),

  // SSH connection
  sshConnect: (hostId: string): Promise<boolean> =>
    invoke("ssh_connect", { hostId }),
  sshDisconnect: (hostId: string): Promise<boolean> =>
    invoke("ssh_disconnect", { hostId }),
  sshStatus: (hostId: string): Promise<string> =>
    invoke("ssh_status", { hostId }),

  // SSH primitives
  sshExec: (hostId: string, command: string): Promise<SshExecResult> =>
    invoke("ssh_exec", { hostId, command }),
  sftpReadFile: (hostId: string, path: string): Promise<string> =>
    invoke("sftp_read_file", { hostId, path }),
  sftpWriteFile: (hostId: string, path: string, content: string): Promise<boolean> =>
    invoke("sftp_write_file", { hostId, path, content }),
  sftpListDir: (hostId: string, path: string): Promise<SftpEntry[]> =>
    invoke("sftp_list_dir", { hostId, path }),
  sftpRemoveFile: (hostId: string, path: string): Promise<boolean> =>
    invoke("sftp_remove_file", { hostId, path }),

  // Remote business commands
  remoteReadRawConfig: (hostId: string): Promise<unknown> =>
    invoke("remote_read_raw_config", { hostId }),
  remoteGetSystemStatus: (hostId: string): Promise<Record<string, unknown>> =>
    invoke("remote_get_system_status", { hostId }),
```

Don't forget to import the new types at the top of `api.ts`.

**Step 3: Verify**

Run: `npx tsc --noEmit`

**Step 4: Commit**

```bash
git add src/lib/types.ts src/lib/api.ts
git commit -m "feat: add SSH types and API functions"
```

---

## Task 8: Instance Tab Bar component

**Files:**
- Create: `src/components/InstanceTabBar.tsx`

This is the tab bar that sits above the main content area, showing Local + remote instances.

**Step 1: Create the component**

```tsx
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { api } from "@/lib/api";
import type { SshHost } from "@/lib/types";

interface InstanceTabBarProps {
  hosts: SshHost[];
  activeId: string;  // "local" or host.id
  connectionStatus: Record<string, "connected" | "disconnected" | "error">;
  onSelect: (id: string) => void;
  onHostsChange: () => void;
}

const statusDot: Record<string, string> = {
  connected: "bg-green-500",
  disconnected: "bg-gray-400",
  error: "bg-red-500",
};

export function InstanceTabBar({
  hosts,
  activeId,
  connectionStatus,
  onSelect,
  onHostsChange,
}: InstanceTabBarProps) {
  const [addOpen, setAddOpen] = useState(false);
  const [editHost, setEditHost] = useState<SshHost | null>(null);
  const [form, setForm] = useState({
    label: "",
    host: "",
    port: "22",
    username: "",
    authMethod: "ssh_config" as "key" | "ssh_config",
    keyPath: "",
  });

  const resetForm = () => {
    setForm({ label: "", host: "", port: "22", username: "", authMethod: "ssh_config", keyPath: "" });
    setEditHost(null);
  };

  const handleSave = () => {
    const host: SshHost = {
      id: editHost?.id || crypto.randomUUID(),
      label: form.label,
      host: form.host,
      port: parseInt(form.port) || 22,
      username: form.username,
      authMethod: form.authMethod,
      keyPath: form.authMethod === "key" ? form.keyPath : undefined,
    };
    api.upsertSshHost(host).then(() => {
      onHostsChange();
      setAddOpen(false);
      resetForm();
    });
  };

  const handleDelete = (hostId: string) => {
    api.deleteSshHost(hostId).then(() => {
      if (activeId === hostId) onSelect("local");
      onHostsChange();
    });
  };

  const openEdit = (host: SshHost) => {
    setEditHost(host);
    setForm({
      label: host.label,
      host: host.host,
      port: String(host.port),
      username: host.username,
      authMethod: host.authMethod,
      keyPath: host.keyPath || "",
    });
    setAddOpen(true);
  };

  return (
    <>
      <div className="flex items-center border-b border-border bg-muted/30 px-2 h-9 gap-1 shrink-0">
        {/* Local tab */}
        <button
          className={cn(
            "flex items-center gap-1.5 px-3 h-7 text-xs rounded-md transition-colors",
            activeId === "local"
              ? "bg-background shadow-sm font-medium"
              : "hover:bg-muted text-muted-foreground"
          )}
          onClick={() => onSelect("local")}
        >
          <span className="w-2 h-2 rounded-full bg-green-500" />
          Local
        </button>

        {/* Remote tabs */}
        {hosts.map((host) => (
          <button
            key={host.id}
            className={cn(
              "flex items-center gap-1.5 px-3 h-7 text-xs rounded-md transition-colors group",
              activeId === host.id
                ? "bg-background shadow-sm font-medium"
                : "hover:bg-muted text-muted-foreground"
            )}
            onClick={() => onSelect(host.id)}
            onContextMenu={(e) => {
              e.preventDefault();
              openEdit(host);
            }}
          >
            <span className={cn("w-2 h-2 rounded-full", statusDot[connectionStatus[host.id] || "disconnected"])} />
            {host.label}
            <span
              className="opacity-0 group-hover:opacity-60 ml-1 hover:opacity-100"
              onClick={(e) => { e.stopPropagation(); handleDelete(host.id); }}
            >
              &times;
            </span>
          </button>
        ))}

        {/* Add button */}
        <button
          className="flex items-center justify-center w-7 h-7 text-xs rounded-md hover:bg-muted text-muted-foreground"
          onClick={() => { resetForm(); setAddOpen(true); }}
        >
          +
        </button>
      </div>

      {/* Add/Edit Dialog */}
      <Dialog open={addOpen} onOpenChange={(open) => { if (!open) { setAddOpen(false); resetForm(); } }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{editHost ? "Edit Remote Instance" : "Add Remote Instance"}</DialogTitle>
          </DialogHeader>
          <div className="space-y-3">
            <div>
              <Label>Label</Label>
              <Input value={form.label} onChange={(e) => setForm((f) => ({ ...f, label: e.target.value }))} placeholder="Production VPS" />
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label>Host</Label>
                <Input value={form.host} onChange={(e) => setForm((f) => ({ ...f, host: e.target.value }))} placeholder="192.168.1.100 or ssh-alias" />
              </div>
              <div>
                <Label>Port</Label>
                <Input value={form.port} onChange={(e) => setForm((f) => ({ ...f, port: e.target.value }))} placeholder="22" />
              </div>
            </div>
            <div>
              <Label>Username</Label>
              <Input value={form.username} onChange={(e) => setForm((f) => ({ ...f, username: e.target.value }))} placeholder="root" />
            </div>
            <div>
              <Label>Auth Method</Label>
              <Select value={form.authMethod} onValueChange={(v) => setForm((f) => ({ ...f, authMethod: v as "key" | "ssh_config" }))}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="ssh_config">SSH Config / Agent</SelectItem>
                  <SelectItem value="key">Private Key</SelectItem>
                </SelectContent>
              </Select>
            </div>
            {form.authMethod === "key" && (
              <div>
                <Label>Key Path</Label>
                <Input value={form.keyPath} onChange={(e) => setForm((f) => ({ ...f, keyPath: e.target.value }))} placeholder="~/.ssh/id_rsa" />
              </div>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => { setAddOpen(false); resetForm(); }}>Cancel</Button>
            <Button onClick={handleSave} disabled={!form.label || !form.host || !form.username}>
              {editHost ? "Save" : "Add"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
```

**Step 2: Verify**

Run: `npx tsc --noEmit`

**Step 3: Commit**

```bash
git add src/components/InstanceTabBar.tsx
git commit -m "feat: add InstanceTabBar component with add/edit/delete"
```

---

## Task 9: Integrate Tab Bar into App.tsx + API context

**Files:**
- Modify: `src/App.tsx`
- Create: `src/lib/instance-context.tsx`

**Step 1: Create instance context**

```tsx
// src/lib/instance-context.tsx
import { createContext, useContext } from "react";

interface InstanceContextValue {
  instanceId: string;  // "local" or remote host id
  isRemote: boolean;
}

export const InstanceContext = createContext<InstanceContextValue>({
  instanceId: "local",
  isRemote: false,
});

export function useInstance() {
  return useContext(InstanceContext);
}
```

**Step 2: Modify App.tsx**

Key changes:
- Add state: `activeInstance`, `sshHosts`, `connectionStatus`
- Load SSH hosts on mount
- When switching to a remote tab, connect via `api.sshConnect()`
- Render `<InstanceTabBar>` above `<main>`
- Wrap page content in `<InstanceContext.Provider>`

Add imports:

```typescript
import { InstanceTabBar } from "./components/InstanceTabBar";
import { InstanceContext } from "./lib/instance-context";
import type { SshHost } from "./lib/types";
```

Add state after existing state declarations:

```typescript
const [activeInstance, setActiveInstance] = useState("local");
const [sshHosts, setSshHosts] = useState<SshHost[]>([]);
const [connectionStatus, setConnectionStatus] = useState<Record<string, string>>({});
```

Add useEffect to load hosts:

```typescript
useEffect(() => {
  api.listSshHosts().then(setSshHosts).catch((e) => console.error("Failed to load SSH hosts:", e));
}, []);

const refreshHosts = useCallback(() => {
  api.listSshHosts().then(setSshHosts).catch((e) => console.error("Failed to load SSH hosts:", e));
}, []);
```

Add handler for instance selection:

```typescript
const handleInstanceSelect = useCallback((id: string) => {
  setActiveInstance(id);
  if (id !== "local") {
    setConnectionStatus((prev) => ({ ...prev, [id]: prev[id] || "disconnected" }));
    api.sshConnect(id)
      .then(() => setConnectionStatus((prev) => ({ ...prev, [id]: "connected" })))
      .catch(() => setConnectionStatus((prev) => ({ ...prev, [id]: "error" })));
  }
}, []);
```

In the JSX, add the tab bar between `<div className="flex h-screen">` and the `<aside>` sidebar. Then wrap the `<main>` content in the context provider:

```tsx
<div className="flex flex-col h-screen">
  <InstanceTabBar
    hosts={sshHosts}
    activeId={activeInstance}
    connectionStatus={connectionStatus}
    onSelect={handleInstanceSelect}
    onHostsChange={refreshHosts}
  />
  <div className="flex flex-1 overflow-hidden">
    <aside ...> {/* existing sidebar */} </aside>
    <InstanceContext.Provider value={{ instanceId: activeInstance, isRemote: activeInstance !== "local" }}>
      <main ...> {/* existing route rendering */} </main>
    </InstanceContext.Provider>
    {/* Chat panel */}
  </div>
</div>
```

**Step 3: Verify**

Run: `npx tsc --noEmit`

**Step 4: Commit**

```bash
git add src/App.tsx src/lib/instance-context.tsx
git commit -m "feat: integrate Instance Tab Bar with connection management"
```

---

## Task 10: Home page shows remote status

**Files:**
- Modify: `src/pages/Home.tsx`

Make the Home page aware of the instance context. When on a remote instance, it calls `remoteGetSystemStatus` instead of the local status commands.

**Step 1: Modify Home.tsx**

Add at the top:

```typescript
import { useInstance } from "@/lib/instance-context";
```

Inside the component:

```typescript
const { instanceId, isRemote } = useInstance();
```

Modify the status-fetching useEffect to branch on `isRemote`:

```typescript
useEffect(() => {
  if (isRemote) {
    api.remoteGetSystemStatus(instanceId)
      .then((s) => {
        setStatus({ healthy: s.healthy as boolean, activeAgents: s.activeAgents as number, globalDefaultModel: s.globalDefaultModel as string | undefined });
        setStatusSettled(true);
        setVersion(s.openclawVersion as string);
      })
      .catch((e) => { console.error("Failed to fetch remote status:", e); setStatusSettled(true); });
  } else {
    fetchStatus();
    // ... existing polling logic
  }
}, [isRemote, instanceId]);
```

For remote, disable features not yet implemented (recipes, cook, model profiles, etc.) — show a "Not available for remote instances" message or simply hide those sections with `{!isRemote && (...)}`.

**Step 2: Verify**

Run: `npx tsc --noEmit`

**Step 3: Commit**

```bash
git add src/pages/Home.tsx
git commit -m "feat: Home page shows remote instance status via SSH"
```

---

## Task 11: End-to-end verification

**Step 1: Build and test**

Run: `cd src-tauri && cargo check && cd .. && npx tsc --noEmit`

**Step 2: Manual testing checklist**

- [ ] App starts, shows "Local" tab
- [ ] Click "+" to add a remote instance, form works
- [ ] Remote tab appears, clicking it triggers SSH connection
- [ ] Green dot appears on successful connection
- [ ] Home page shows remote openclaw version and gateway status
- [ ] Right-click remote tab to edit config
- [ ] Close (x) on tab removes the instance
- [ ] Switching back to Local tab works, shows local data

**Step 3: Final commit**

```bash
git add -A
git commit -m "feat: SSH remote management Phase 1 — connection, config, status display"
```
