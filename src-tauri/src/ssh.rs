use std::collections::HashMap;

use base64::Engine;
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

/// Shell-quote a string using single quotes with proper escaping.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Base64 decode pipeline compatible with GNU coreutils and BSD/macOS.
fn base64_decode_pipeline() -> &'static str {
    "base64 -d 2>/dev/null || base64 -D 2>/dev/null"
}

/// Build a safe remote write command using base64 transport.
fn build_sftp_write_command(path: &str, b64: &str) -> String {
    let quoted = shell_quote(path);
    format!(
        "mkdir -p \"$(dirname {quoted})\" && printf '%s' '{b64}' | ({decode}) > {quoted}",
        decode = base64_decode_pipeline(),
    )
}

/// Check if an SSH exec error is likely transient (worth retrying) vs permanent.
fn is_transient_ssh_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    // Permanent errors — do not retry
    let permanent = [
        "authentication failed",
        "permission denied",
        "no such host",
        "host key verification",
        "no connection for id",
    ];
    if permanent.iter().any(|p| lower.contains(p)) {
        return false;
    }
    // Known transient patterns
    let transient = [
        "could not be executed",
        "broken pipe",
        "connection reset",
        "channel open",
        "session is closed",
        "end of file",
        "timed out",
    ];
    transient.iter().any(|t| lower.contains(t))
        || lower.contains("failed to exec") // our own wrapper message
}

// ---------------------------------------------------------------------------
// Unix implementation (uses openssh)
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod inner {
    use super::*;
    use std::sync::Arc;
    use openssh::{ControlPersist, ForwardType, KnownHosts, Session, SessionBuilder, Socket};
    use tokio::net::TcpStream;

    struct SshConnection {
        session: Arc<Session>,
        home_dir: String,
        config: SshHostConfig,
    }

    pub struct SshConnectionPool {
        connections: Mutex<HashMap<String, SshConnection>>,
        forwards: Mutex<HashMap<String, (u16, u16)>>,
        lifecycle: Mutex<()>,
    }

    impl SshConnectionPool {
        pub fn new() -> Self {
            Self {
                connections: Mutex::new(HashMap::new()),
                forwards: Mutex::new(HashMap::new()),
                lifecycle: Mutex::new(()),
            }
        }

        pub async fn connect(&self, config: &SshHostConfig) -> Result<(), String> {
            let _lifecycle_guard = self.lifecycle.lock().await;
            if config.auth_method == "password" {
                return Err(
                    "Password authentication is not supported with openssh. \
                     Please use SSH Config or Private Key mode instead. \
                     If your key is in ssh-agent, select SSH Config mode."
                        .into(),
                );
            }

            let dest = if config.username.is_empty() {
                config.host.clone()
            } else {
                format!("{}@{}", config.username, config.host)
            };

            let mut builder = SessionBuilder::default();
            builder.known_hosts_check(KnownHosts::Add);

            if config.port != 22 {
                builder.port(config.port);
            }

            builder.server_alive_interval(std::time::Duration::from_secs(30));
            builder.connect_timeout(std::time::Duration::from_secs(15));
            // Use a moderate ControlPersist so idle ControlMasters auto-exit
            // instead of living forever (which leaks sshd processes on the remote).
            // 3 min balances: short enough to limit accumulation, long enough to
            // survive browser-tab throttling of the 30s poll interval.
            builder.control_persist(ControlPersist::IdleFor(
                std::num::NonZeroUsize::new(3).unwrap(),
            ));
            // Clean up stale ControlMaster temp dirs from previous sessions
            // (e.g., after app crash or force-quit).
            builder.clean_history_control_directory(true);

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

            session
                .check()
                .await
                .map_err(|e| format!("SSH connection check failed: {e}"))?;

            let home_dir = Self::resolve_home_via_session(&session)
                .await
                .unwrap_or_else(|_| "/root".to_string());

            // Atomically swap old connection for new one — the pool always has an
            // entry for this id, so parallel exec_once() never sees "No connection".
            let old = {
                let mut pool = self.connections.lock().await;
                let old = pool.remove(&config.id);
                pool.insert(config.id.clone(), SshConnection { session: Arc::new(session), home_dir, config: config.clone() });
                old
            };
            // Best-effort cleanup of old session outside the lock
            if let Some(old) = old {
                match Arc::try_unwrap(old.session) {
                    Ok(old_session) => {
                        let _ = old_session.close().await;
                    }
                    Err(arc) => {
                        // In-flight commands hold references — spawn background cleanup
                        tokio::spawn(async move {
                            for _ in 0..120 {
                                if Arc::strong_count(&arc) <= 1 {
                                    break;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            }
                            if let Ok(session) = Arc::try_unwrap(arc) {
                                let _ = session.close().await;
                            }
                        });
                    }
                }
            }
            // Drop any cached local forward bound to an old session.
            self.forwards.lock().await.remove(&config.id);
            Ok(())
        }

        /// Reconnect an existing SSH connection by re-using its stored config.
        /// Skips explicit disconnect — connect() already handles old connection
        /// cleanup internally, which minimises the window where the pool has no
        /// entry for this id (avoids "No connection for id" from parallel commands).
        pub async fn reconnect(&self, id: &str) -> Result<(), String> {
            let config = {
                let pool = self.connections.lock().await;
                pool.get(id)
                    .map(|c| c.config.clone())
                    .ok_or_else(|| format!("No connection for id: {id}"))?
            };
            self.connect(&config).await
        }

        pub async fn disconnect(&self, id: &str) -> Result<(), String> {
            let _lifecycle_guard = self.lifecycle.lock().await;
            let conn = {
                let mut pool = self.connections.lock().await;
                pool.remove(id)
            };
            if let Some(conn) = conn {
                match Arc::try_unwrap(conn.session) {
                    Ok(session) => {
                        let _ = session.close().await;
                    }
                    Err(arc) => {
                        // Other references exist (in-flight exec). Spawn a
                        // background task that waits for them to finish, then
                        // explicitly closes the session so the ControlMaster
                        // is cleaned up instead of lingering for ControlPersist.
                        tokio::spawn(async move {
                            // Poll until we're the last reference (in-flight commands done)
                            for _ in 0..120 {
                                if Arc::strong_count(&arc) <= 1 {
                                    break;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            }
                            if let Ok(session) = Arc::try_unwrap(arc) {
                                let _ = session.close().await;
                            }
                            // If try_unwrap still fails after 60s, drop triggers Session::Drop
                        });
                    }
                }
            }
            self.forwards.lock().await.remove(id);
            Ok(())
        }

        pub async fn is_connected(&self, id: &str) -> bool {
            let session = {
                let pool = self.connections.lock().await;
                match pool.get(id) {
                    Some(conn) => Arc::clone(&conn.session),
                    None => return false,
                }
            };
            session.check().await.is_ok()
        }

        /// Create a local port forward: localhost:<local_port> → remote 127.0.0.1:<remote_port>.
        /// Binds to a random local port (port 0) and returns the actual port assigned.
        pub async fn request_port_forward(&self, id: &str, remote_port: u16) -> Result<u16, String> {
            let _lifecycle_guard = self.lifecycle.lock().await;
            // Reuse an existing forward when possible to avoid accumulating
            // duplicate local forwards for repeated doctor sessions.
            let cached = {
                let fwd = self.forwards.lock().await;
                fwd.get(id).copied()
            };
            if let Some((cached_remote_port, cached_local_port)) = cached {
                if cached_remote_port == remote_port {
                    let alive = match tokio::time::timeout(
                        std::time::Duration::from_millis(250),
                        TcpStream::connect(("127.0.0.1", cached_local_port)),
                    )
                    .await
                    {
                        Ok(Ok(_)) => true,
                        _ => false,
                    };
                    if alive {
                        return Ok(cached_local_port);
                    }
                    self.forwards.lock().await.remove(id);
                }
            }

            let session = {
                let pool = self.connections.lock().await;
                let conn = pool.get(id).ok_or_else(|| format!("No connection for id: {id}"))?;
                Arc::clone(&conn.session)
            };
            // Bind to port 0 = OS picks a free port
            let local_port = portpicker::pick_unused_port()
                .ok_or_else(|| "Could not find a free local port".to_string())?;
            session
                .request_port_forward(
                    ForwardType::Local,
                    Socket::TcpSocket { host: "127.0.0.1".into(), port: local_port },
                    Socket::TcpSocket { host: "127.0.0.1".into(), port: remote_port },
                )
                .await
                .map_err(|e| format!("SSH port forward failed: {e}"))?;
            self.forwards
                .lock()
                .await
                .insert(id.to_string(), (remote_port, local_port));
            Ok(local_port)
        }

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

        pub async fn resolve_path(&self, id: &str, path: &str) -> Result<String, String> {
            if path.starts_with("~/") || path == "~" {
                let home = self.get_home_dir(id).await?;
                Ok(path.replacen('~', &home, 1))
            } else {
                Ok(path.to_string())
            }
        }

        pub async fn exec(&self, id: &str, command: &str) -> Result<SshExecResult, String> {
            match self.exec_once(id, command).await {
                Ok(result) => Ok(result),
                Err(first_err) if is_transient_ssh_error(&first_err) => {
                    // Transient failure — ControlMaster may not be fully ready.
                    // Wait briefly and retry once before attempting reconnect.
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    match self.exec_once(id, command).await {
                        Ok(result) => Ok(result),
                        Err(_) => {
                            // Retry failed — try reconnect + one more attempt
                            if self.reconnect(id).await.is_ok() {
                                self.exec_once(id, command).await
                            } else {
                                Err(first_err)
                            }
                        }
                    }
                }
                Err(permanent_err) => Err(permanent_err),
            }
        }

        async fn exec_once(&self, id: &str, command: &str) -> Result<SshExecResult, String> {
            let session = {
                let pool = self.connections.lock().await;
                let conn = pool
                    .get(id)
                    .ok_or_else(|| format!("No connection for id: {id}"))?;
                Arc::clone(&conn.session)
            };

            let output = tokio::time::timeout(
                std::time::Duration::from_secs(120),
                session.raw_command(command).output(),
            )
            .await
            .map_err(|_| "Command timed out after 120s".to_string())?
            .map_err(|e| format!("Failed to exec command: {e}"))?;

            Ok(SshExecResult {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                exit_code: output.status.code().unwrap_or(1) as u32,
            })
        }

        /// Execute a command with login shell setup (sources profile for PATH).
        /// Forces bash to avoid zsh glob/nomatch quirks.
        pub async fn exec_login(&self, id: &str, command: &str) -> Result<SshExecResult, String> {
            let target_bin = command.split_whitespace().next().unwrap_or("");
            let wrapped = format!(
                concat!(
                    "setopt nonomatch 2>/dev/null; shopt -s nullglob 2>/dev/null; ",
                    ". \"$HOME/.profile\" 2>/dev/null; ",
                    ". \"$HOME/.bashrc\" 2>/dev/null; ",
                    ". \"$HOME/.zshrc\" 2>/dev/null; ",
                    "[ -d \"$HOME/.local/bin\" ] && export PATH=\"$HOME/.local/bin:$PATH\"; ",
                    "export NVM_DIR=\"${{NVM_DIR:-$HOME/.nvm}}\"; ",
                    "[ -s \"$NVM_DIR/nvm.sh\" ] && . \"$NVM_DIR/nvm.sh\" 2>/dev/null; ",
                    "for _fnm in \"$HOME/.fnm/fnm\" \"$HOME/.local/bin/fnm\"; do ",
                      "[ -x \"$_fnm\" ] && eval \"$($_fnm env --shell bash 2>/dev/null || $_fnm env 2>/dev/null)\" 2>/dev/null && break; ",
                    "done; ",
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

        pub async fn sftp_read(&self, id: &str, path: &str) -> Result<String, String> {
            let resolved = self.resolve_path(id, path).await?;
            let cmd = format!("cat {}", shell_quote(&resolved));
            let result = self.exec(id, &cmd).await?;
            if result.exit_code != 0 {
                return Err(format!(
                    "Failed to read {resolved}: {}",
                    result.stderr.trim()
                ));
            }
            Ok(result.stdout)
        }

        pub async fn sftp_write(&self, id: &str, path: &str, content: &str) -> Result<(), String> {
            let resolved = self.resolve_path(id, path).await?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
            let cmd = build_sftp_write_command(&resolved, &b64);
            let result = self.exec(id, &cmd).await?;
            if result.exit_code != 0 {
                return Err(format!(
                    "Failed to write {resolved}: {}",
                    result.stderr.trim()
                ));
            }
            Ok(())
        }

        pub async fn sftp_list(&self, id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
            let resolved = self.resolve_path(id, path).await?;
            let quoted = shell_quote(&resolved);
            // Use ls -lA for cross-platform compat (GNU stat vs BSD stat differ).
            let cmd = format!(
                "ls -lA {} 2>/dev/null || true",
                quoted
            );
            let result = self.exec(id, &cmd).await?;

            let mut entries = Vec::new();
            for line in result.stdout.lines() {
                // Skip "total NNN" header and empty lines
                if line.starts_with("total ") || line.trim().is_empty() {
                    continue;
                }
                // ls -l: perms links owner group size month day time name...
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 9 {
                    continue;
                }
                let perms = parts[0];
                let size: u64 = parts[4].parse().unwrap_or(0);
                // Name may contain spaces — rejoin from field 8 onward
                let name = parts[8..].join(" ");

                if name == "." || name == ".." || name.is_empty() {
                    continue;
                }

                entries.push(SftpEntry {
                    name,
                    is_dir: perms.starts_with('d'),
                    size,
                });
            }
            Ok(entries)
        }

        pub async fn sftp_remove(&self, id: &str, path: &str) -> Result<(), String> {
            let resolved = self.resolve_path(id, path).await?;
            let cmd = format!("rm {}", shell_quote(&resolved));
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
}

// ---------------------------------------------------------------------------
// Windows implementation (spawns ssh process directly, no openssh crate)
// ---------------------------------------------------------------------------

#[cfg(not(unix))]
mod inner {
    use super::*;
    use tokio::net::TcpStream;
    use tokio::process::Command;

    /// Create an ssh Command with hidden console window on Windows.
    fn ssh_command() -> Command {
        let mut cmd = Command::new("ssh");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd
    }

    struct SshConnection {
        config: SshHostConfig,
        home_dir: String,
    }

    struct PortForwardHandle {
        remote_port: u16,
        local_port: u16,
        child: tokio::process::Child,
    }

    impl SshConnection {
        /// Build common ssh args: [-p port] [-i key] [-o options] user@host
        fn ssh_args(&self) -> Vec<String> {
            let mut args = Vec::new();
            args.push("-o".into());
            args.push("BatchMode=yes".into());
            args.push("-o".into());
            args.push("StrictHostKeyChecking=accept-new".into());
            args.push("-o".into());
            args.push("ConnectTimeout=15".into());
            args.push("-o".into());
            args.push("ServerAliveInterval=30".into());
            if self.config.port != 22 {
                args.push("-p".into());
                args.push(self.config.port.to_string());
            }
            if self.config.auth_method == "key" {
                if let Some(ref key_path) = self.config.key_path {
                    args.push("-i".into());
                    args.push(shellexpand::tilde(key_path).to_string());
                }
            }
            let dest = if self.config.username.is_empty() {
                self.config.host.clone()
            } else {
                format!("{}@{}", self.config.username, self.config.host)
            };
            args.push(dest);
            args
        }
    }

    pub struct SshConnectionPool {
        connections: Mutex<HashMap<String, SshConnection>>,
        /// Tracked port-forward processes (killed on disconnect or new forward).
        port_forwards: Mutex<HashMap<String, PortForwardHandle>>,
        lifecycle: Mutex<()>,
    }

    impl SshConnectionPool {
        pub fn new() -> Self {
            Self {
                connections: Mutex::new(HashMap::new()),
                port_forwards: Mutex::new(HashMap::new()),
                lifecycle: Mutex::new(()),
            }
        }

        pub async fn connect(&self, config: &SshHostConfig) -> Result<(), String> {
            let _lifecycle_guard = self.lifecycle.lock().await;
            if config.auth_method == "password" {
                return Err(
                    "Password authentication is not supported. \
                     Please use SSH Config or Private Key mode instead."
                        .into(),
                );
            }

            // Test connection with a simple command
            let mut conn = SshConnection {
                config: config.clone(),
                home_dir: String::new(),
            };

            let mut args = conn.ssh_args();
            args.push("echo $HOME".into());

            let output = ssh_command()
                .args(&args)
                .output()
                .await
                .map_err(|e| format!("SSH connection failed: {e}"))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("SSH connection failed: {}", stderr.trim()));
            }

            let home = String::from_utf8_lossy(&output.stdout).trim().to_string();
            conn.home_dir = if home.is_empty() { "/root".into() } else { home };

            let mut pool = self.connections.lock().await;
            pool.insert(config.id.clone(), conn);
            drop(pool);
            // Old forwarding processes (if any) belong to a previous connection.
            let mut old_fwd = {
                let mut fwd = self.port_forwards.lock().await;
                fwd.remove(&config.id)
            };
            if let Some(ref mut pf) = old_fwd {
                let _ = pf.child.kill().await;
            }
            Ok(())
        }

        pub async fn disconnect(&self, id: &str) -> Result<(), String> {
            let _lifecycle_guard = self.lifecycle.lock().await;
            {
                let mut pool = self.connections.lock().await;
                pool.remove(id);
            }
            // Kill any tracked port-forward process for this host
            let mut old = {
                let mut fwd = self.port_forwards.lock().await;
                fwd.remove(id)
            };
            if let Some(ref mut pf) = old {
                let _ = pf.child.kill().await;
            }
            Ok(())
        }

        /// Reconnect an existing SSH connection by re-using its stored config.
        /// Skips explicit disconnect — connect() already handles old connection
        /// cleanup internally, which minimises the window where the pool has no
        /// entry for this id (avoids "No connection for id" from parallel commands).
        pub async fn reconnect(&self, id: &str) -> Result<(), String> {
            let config = {
                let pool = self.connections.lock().await;
                pool.get(id)
                    .map(|c| c.config.clone())
                    .ok_or_else(|| format!("No connection for id: {id}"))?
            };
            self.connect(&config).await
        }

        pub async fn is_connected(&self, id: &str) -> bool {
            let args = {
                let pool = self.connections.lock().await;
                match pool.get(id) {
                    Some(conn) => {
                        let mut a = conn.ssh_args();
                        a.push("true".into());
                        a
                    }
                    None => return false,
                }
            };
            ssh_command()
                .args(&args)
                .output()
                .await
                .map(|o| o.status.success())
                .unwrap_or(false)
        }

        /// Create a local port forward via `ssh -L -N`. Returns the local port.
        /// The ssh process is tracked and killed on disconnect or next forward request.
        pub async fn request_port_forward(&self, id: &str, remote_port: u16) -> Result<u16, String> {
            let _lifecycle_guard = self.lifecycle.lock().await;
            // Reuse live forward for the same remote port; otherwise replace.
            let mut to_kill = None;
            let mut candidate_reuse_port = None;
            {
                let mut fwd = self.port_forwards.lock().await;
                if let Some(existing) = fwd.get_mut(id) {
                    match existing.child.try_wait() {
                        Ok(None) if existing.remote_port == remote_port => {
                            candidate_reuse_port = Some(existing.local_port);
                        }
                        Ok(None) | Ok(Some(_)) | Err(_) => {
                            to_kill = fwd.remove(id);
                        }
                    }
                }
            }
            if let Some(port) = candidate_reuse_port {
                let alive = match tokio::time::timeout(
                    std::time::Duration::from_millis(250),
                    TcpStream::connect(("127.0.0.1", port)),
                )
                .await
                {
                    Ok(Ok(_)) => true,
                    _ => false,
                };
                if alive {
                    return Ok(port);
                }
                to_kill = {
                    let mut fwd = self.port_forwards.lock().await;
                    fwd.remove(id)
                };
            }
            if let Some(mut old) = to_kill {
                let _ = old.child.kill().await;
            }

            let args = {
                let pool = self.connections.lock().await;
                let conn = pool.get(id).ok_or_else(|| format!("No connection for id: {id}"))?;
                conn.ssh_args()
            };
            let local_port = portpicker::pick_unused_port()
                .ok_or_else(|| "Could not find a free local port".to_string())?;
            // -L: local forward, -N: no remote command (just forward)
            // No -f: Windows OpenSSH doesn't support it; we spawn detached instead.
            let mut cmd_args = vec![
                "-L".into(),
                format!("{}:127.0.0.1:{}", local_port, remote_port),
                "-N".into(),
            ];
            cmd_args.extend(args);
            let mut child = ssh_command()
                .args(&cmd_args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("SSH port forward failed: {e}"))?;

            // Give the child process a moment, then fail fast if it exited early.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!(
                    "SSH port forward exited early with status: {status}"
                ));
            }

            // Best-effort local liveness probe (short timeout, non-fatal).
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(300),
                TcpStream::connect(("127.0.0.1", local_port)),
            )
            .await;

            self.port_forwards.lock().await.insert(
                id.to_string(),
                PortForwardHandle {
                    remote_port,
                    local_port,
                    child,
                },
            );
            Ok(local_port)
        }

        pub async fn get_home_dir(&self, id: &str) -> Result<String, String> {
            let pool = self.connections.lock().await;
            let conn = pool
                .get(id)
                .ok_or_else(|| format!("No connection for id: {id}"))?;
            Ok(conn.home_dir.clone())
        }

        pub async fn resolve_path(&self, id: &str, path: &str) -> Result<String, String> {
            if path.starts_with("~/") || path == "~" {
                let home = self.get_home_dir(id).await?;
                Ok(path.replacen('~', &home, 1))
            } else {
                Ok(path.to_string())
            }
        }

        pub async fn exec(&self, id: &str, command: &str) -> Result<SshExecResult, String> {
            match self.exec_once(id, command).await {
                Ok(result) => Ok(result),
                Err(first_err) if is_transient_ssh_error(&first_err) => {
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    match self.exec_once(id, command).await {
                        Ok(result) => Ok(result),
                        Err(_) => {
                            if self.reconnect(id).await.is_ok() {
                                self.exec_once(id, command).await
                            } else {
                                Err(first_err)
                            }
                        }
                    }
                }
                Err(permanent_err) => Err(permanent_err),
            }
        }

        async fn exec_once(&self, id: &str, command: &str) -> Result<SshExecResult, String> {
            let args = {
                let pool = self.connections.lock().await;
                let conn = pool
                    .get(id)
                    .ok_or_else(|| format!("No connection for id: {id}"))?;
                let mut a = conn.ssh_args();
                a.push(command.into());
                a
            };

            let output = tokio::time::timeout(
                std::time::Duration::from_secs(120),
                ssh_command().args(&args).output(),
            )
            .await
            .map_err(|_| "Command timed out after 120s".to_string())?
            .map_err(|e| format!("Failed to exec command: {e}"))?;

            Ok(SshExecResult {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                exit_code: output.status.code().unwrap_or(1) as u32,
            })
        }

        pub async fn exec_login(&self, id: &str, command: &str) -> Result<SshExecResult, String> {
            let target_bin = command.split_whitespace().next().unwrap_or("");
            let wrapped = format!(
                concat!(
                    "setopt nonomatch 2>/dev/null; shopt -s nullglob 2>/dev/null; ",
                    ". \"$HOME/.profile\" 2>/dev/null; ",
                    ". \"$HOME/.bashrc\" 2>/dev/null; ",
                    ". \"$HOME/.zshrc\" 2>/dev/null; ",
                    "[ -d \"$HOME/.local/bin\" ] && export PATH=\"$HOME/.local/bin:$PATH\"; ",
                    "export NVM_DIR=\"${{NVM_DIR:-$HOME/.nvm}}\"; ",
                    "[ -s \"$NVM_DIR/nvm.sh\" ] && . \"$NVM_DIR/nvm.sh\" 2>/dev/null; ",
                    "for _fnm in \"$HOME/.fnm/fnm\" \"$HOME/.local/bin/fnm\"; do ",
                      "[ -x \"$_fnm\" ] && eval \"$($_fnm env --shell bash 2>/dev/null || $_fnm env 2>/dev/null)\" 2>/dev/null && break; ",
                    "done; ",
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

        pub async fn sftp_read(&self, id: &str, path: &str) -> Result<String, String> {
            let resolved = self.resolve_path(id, path).await?;
            let cmd = format!("cat {}", shell_quote(&resolved));
            let result = self.exec(id, &cmd).await?;
            if result.exit_code != 0 {
                return Err(format!(
                    "Failed to read {resolved}: {}",
                    result.stderr.trim()
                ));
            }
            Ok(result.stdout)
        }

        pub async fn sftp_write(&self, id: &str, path: &str, content: &str) -> Result<(), String> {
            let resolved = self.resolve_path(id, path).await?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
            let cmd = build_sftp_write_command(&resolved, &b64);
            let result = self.exec(id, &cmd).await?;
            if result.exit_code != 0 {
                return Err(format!(
                    "Failed to write {resolved}: {}",
                    result.stderr.trim()
                ));
            }
            Ok(())
        }

        pub async fn sftp_list(&self, id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
            let resolved = self.resolve_path(id, path).await?;
            let quoted = shell_quote(&resolved);
            // Use ls -lA for cross-platform compat (GNU stat vs BSD stat differ).
            let cmd = format!(
                "ls -lA {} 2>/dev/null || true",
                quoted
            );
            let result = self.exec(id, &cmd).await?;

            let mut entries = Vec::new();
            for line in result.stdout.lines() {
                // Skip "total NNN" header and empty lines
                if line.starts_with("total ") || line.trim().is_empty() {
                    continue;
                }
                // ls -l: perms links owner group size month day time name...
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 9 {
                    continue;
                }
                let perms = parts[0];
                let size: u64 = parts[4].parse().unwrap_or(0);
                // Name may contain spaces — rejoin from field 8 onward
                let name = parts[8..].join(" ");

                if name == "." || name == ".." || name.is_empty() {
                    continue;
                }

                entries.push(SftpEntry {
                    name,
                    is_dir: perms.starts_with('d'),
                    size,
                });
            }
            Ok(entries)
        }

        pub async fn sftp_remove(&self, id: &str, path: &str) -> Result<(), String> {
            let resolved = self.resolve_path(id, path).await?;
            let cmd = format!("rm {}", shell_quote(&resolved));
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
        fn default() -> Self { Self::new() }
    }
}

pub use inner::SshConnectionPool;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_decode_pipeline_is_cross_platform() {
        let pipe = base64_decode_pipeline();
        assert!(pipe.contains("base64 -d"), "expected GNU base64 flag");
        assert!(pipe.contains("base64 -D"), "expected BSD base64 flag");
    }

    #[test]
    fn test_build_sftp_write_command_uses_decode_fallback() {
        let cmd = build_sftp_write_command("/tmp/a.txt", "YWJj");
        assert!(cmd.contains("mkdir -p"));
        assert!(cmd.contains("base64 -d"));
        assert!(cmd.contains("base64 -D"));
    }
}
