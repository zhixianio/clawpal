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

// ---------------------------------------------------------------------------
// Unix implementation (uses openssh)
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod inner {
    use super::*;
    use std::sync::Arc;
    use openssh::{KnownHosts, Session, SessionBuilder};

    struct SshConnection {
        session: Arc<Session>,
        home_dir: String,
    }

    pub struct SshConnectionPool {
        connections: Mutex<HashMap<String, SshConnection>>,
    }

    impl SshConnectionPool {
        pub fn new() -> Self {
            Self {
                connections: Mutex::new(HashMap::new()),
            }
        }

        pub async fn connect(&self, config: &SshHostConfig) -> Result<(), String> {
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
            builder.known_hosts_check(KnownHosts::Accept);

            if config.port != 22 {
                builder.port(config.port);
            }

            builder.server_alive_interval(std::time::Duration::from_secs(30));
            builder.connect_timeout(std::time::Duration::from_secs(15));

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

            let mut pool = self.connections.lock().await;
            pool.insert(config.id.clone(), SshConnection { session: Arc::new(session), home_dir });
            Ok(())
        }

        pub async fn disconnect(&self, id: &str) -> Result<(), String> {
            let conn = {
                let mut pool = self.connections.lock().await;
                pool.remove(id)
            };
            if let Some(conn) = conn {
                if let Ok(session) = Arc::try_unwrap(conn.session) {
                    session
                        .close()
                        .await
                        .map_err(|e| format!("SSH disconnect failed: {e}"))?;
                }
            }
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
            let session = {
                let pool = self.connections.lock().await;
                let conn = pool
                    .get(id)
                    .ok_or_else(|| format!("No connection for id: {id}"))?;
                Arc::clone(&conn.session)
            };

            let output = session
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
        /// Forces bash to avoid zsh glob/nomatch quirks.
        pub async fn exec_login(&self, id: &str, command: &str) -> Result<SshExecResult, String> {
            let target_bin = command.split_whitespace().next().unwrap_or("");
            let inner = format!(
                concat!(
                    "shopt -s nullglob 2>/dev/null; ",
                    ". \"$HOME/.profile\" 2>/dev/null; ",
                    ". \"$HOME/.bashrc\" 2>/dev/null; ",
                    "[ -d \"$HOME/.local/bin\" ] && export PATH=\"$HOME/.local/bin:$PATH\"; ",
                    "export NVM_DIR=\"${{NVM_DIR:-$HOME/.nvm}}\"; ",
                    "[ -s \"$NVM_DIR/nvm.sh\" ] && . \"$NVM_DIR/nvm.sh\" 2>/dev/null; ",
                    "for _fnm in \"$HOME/.fnm/fnm\" \"$HOME/.local/bin/fnm\"; do ",
                      "[ -x \"$_fnm\" ] && eval \"$($_fnm env --shell bash)\" 2>/dev/null && break; ",
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
            let wrapped = format!("export __CLP_CMD={}; bash -c \"$__CLP_CMD\"", shell_quote(&inner));
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
            let quoted = shell_quote(&resolved);
            let b64 = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
            let cmd = format!(
                "mkdir -p \"$(dirname {})\" && printf '%s' '{}' | base64 -d > {}",
                quoted, b64, quoted
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

        pub async fn sftp_list(&self, id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
            let resolved = self.resolve_path(id, path).await?;
            let quoted = shell_quote(&resolved);
            let cmd = format!(
                "stat --format='%n\t%F\t%s' {}/* 2>/dev/null || true",
                quoted
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
    use tokio::process::Command;

    struct SshConnection {
        config: SshHostConfig,
        home_dir: String,
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
    }

    impl SshConnectionPool {
        pub fn new() -> Self {
            Self {
                connections: Mutex::new(HashMap::new()),
            }
        }

        pub async fn connect(&self, config: &SshHostConfig) -> Result<(), String> {
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

            let output = Command::new("ssh")
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
            Ok(())
        }

        pub async fn disconnect(&self, id: &str) -> Result<(), String> {
            let mut pool = self.connections.lock().await;
            pool.remove(id);
            Ok(())
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
            Command::new("ssh")
                .args(&args)
                .output()
                .await
                .map(|o| o.status.success())
                .unwrap_or(false)
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
            let args = {
                let pool = self.connections.lock().await;
                let conn = pool
                    .get(id)
                    .ok_or_else(|| format!("No connection for id: {id}"))?;
                let mut a = conn.ssh_args();
                a.push(command.into());
                a
            };

            let output = Command::new("ssh")
                .args(&args)
                .output()
                .await
                .map_err(|e| format!("Failed to exec command: {e}"))?;

            Ok(SshExecResult {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                exit_code: output.status.code().unwrap_or(1) as u32,
            })
        }

        pub async fn exec_login(&self, id: &str, command: &str) -> Result<SshExecResult, String> {
            let target_bin = command.split_whitespace().next().unwrap_or("");
            let inner = format!(
                concat!(
                    "shopt -s nullglob 2>/dev/null; ",
                    ". \"$HOME/.profile\" 2>/dev/null; ",
                    ". \"$HOME/.bashrc\" 2>/dev/null; ",
                    "[ -d \"$HOME/.local/bin\" ] && export PATH=\"$HOME/.local/bin:$PATH\"; ",
                    "export NVM_DIR=\"${{NVM_DIR:-$HOME/.nvm}}\"; ",
                    "[ -s \"$NVM_DIR/nvm.sh\" ] && . \"$NVM_DIR/nvm.sh\" 2>/dev/null; ",
                    "for _fnm in \"$HOME/.fnm/fnm\" \"$HOME/.local/bin/fnm\"; do ",
                      "[ -x \"$_fnm\" ] && eval \"$($_fnm env --shell bash)\" 2>/dev/null && break; ",
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
            let wrapped = format!("export __CLP_CMD={}; bash -c \"$__CLP_CMD\"", shell_quote(&inner));
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
            let quoted = shell_quote(&resolved);
            let b64 = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
            let cmd = format!(
                "mkdir -p \"$(dirname {})\" && printf '%s' '{}' | base64 -d > {}",
                quoted, b64, quoted
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

        pub async fn sftp_list(&self, id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
            let resolved = self.resolve_path(id, path).await?;
            let quoted = shell_quote(&resolved);
            let cmd = format!(
                "stat --format='%n\t%F\t%s' {}/* 2>/dev/null || true",
                quoted
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
