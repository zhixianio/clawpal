use std::collections::HashMap;
use std::time::{Duration as StdDuration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::{timeout, Duration};

pub type SshHostConfig = clawpal_core::instance::SshHostConfig;

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

#[derive(Debug, Clone)]
struct ConnectedHost {
    config: SshHostConfig,
    home_dir: String,
    op_limiter: std::sync::Arc<Semaphore>,
    runtime: std::sync::Arc<Mutex<HostRuntimeState>>,
}

#[derive(Debug)]
struct HostRuntimeState {
    consecutive_timeouts: u32,
    cool_down_until: Option<Instant>,
}

pub struct SshConnectionPool {
    connections: Mutex<HashMap<String, ConnectedHost>>,
}

const SSH_OP_MAX_CONCURRENCY_PER_HOST: usize = 2;
const SSH_EXEC_TIMEOUT_SECS: u64 = 30;
const SSH_SFTP_TIMEOUT_SECS: u64 = 45;
const SSH_TIMEOUT_COOLDOWN_BASE_SECS: u64 = 20;
const SSH_TIMEOUT_COOLDOWN_MAX_SECS: u64 = 120;

impl SshConnectionPool {
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
        }
    }

    pub async fn connect(&self, config: &SshHostConfig) -> Result<(), String> {
        self.connect_with_passphrase(config, None).await
    }

    pub async fn connect_with_passphrase(
        &self,
        config: &SshHostConfig,
        _passphrase: Option<&str>,
    ) -> Result<(), String> {
        let session = clawpal_core::ssh::SshSession::connect(config)
            .await
            .map_err(|e| e.to_string())?;
        let home = session
            .exec("echo $HOME")
            .await
            .map(|r| r.stdout.trim().to_string())
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/root".to_string());

        self.connections.lock().await.insert(
            config.id.clone(),
            ConnectedHost {
                config: config.clone(),
                home_dir: home,
                op_limiter: std::sync::Arc::new(Semaphore::new(SSH_OP_MAX_CONCURRENCY_PER_HOST)),
                runtime: std::sync::Arc::new(Mutex::new(HostRuntimeState {
                    consecutive_timeouts: 0,
                    cool_down_until: None,
                })),
            },
        );
        Ok(())
    }

    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        self.connections.lock().await.remove(id);
        Ok(())
    }

    pub async fn reconnect(&self, id: &str) -> Result<(), String> {
        let config = {
            let guard = self.connections.lock().await;
            guard
                .get(id)
                .map(|c| c.config.clone())
                .ok_or_else(|| format!("No connection for id: {id}"))?
        };
        self.connect(&config).await
    }

    pub async fn is_connected(&self, id: &str) -> bool {
        self.connections.lock().await.contains_key(id)
    }

    pub async fn request_port_forward(&self, _id: &str, _remote_port: u16) -> Result<u16, String> {
        Err("Port forward is not supported in stateless ssh mode yet".to_string())
    }

    pub async fn get_home_dir(&self, id: &str) -> Result<String, String> {
        let guard = self.connections.lock().await;
        let conn = guard
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
        let conn = self.lookup_connected_host(id).await?;
        self.ensure_not_in_timeout_cooldown(id, &conn).await?;
        let _permit = conn
            .op_limiter
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| format!("ssh limiter acquire failed: {e}"))?;
        let session_connect = timeout(
            Duration::from_secs(SSH_EXEC_TIMEOUT_SECS),
            clawpal_core::ssh::SshSession::connect(&conn.config),
        ).await;
        let session = match session_connect {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => {
                self.mark_timeout_and_cooldown(&conn).await;
                return Err(format!("ssh connect timeout after {SSH_EXEC_TIMEOUT_SECS}s"));
            }
        };
        let result = timeout(Duration::from_secs(SSH_EXEC_TIMEOUT_SECS), session.exec(command))
            .await
            .map_err(|_| format!("ssh exec timeout after {SSH_EXEC_TIMEOUT_SECS}s"))?;
        let result = match result {
            Ok(v) => v,
            Err(e) => {
                let msg = e.to_string();
                if is_timeout_error_message(&msg) {
                    self.mark_timeout_and_cooldown(&conn).await;
                }
                return Err(msg);
            }
        };
        self.clear_timeout_cooldown(&conn).await;
        Ok(SshExecResult {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code.max(0) as u32,
        })
    }

    pub async fn exec_login(&self, id: &str, command: &str) -> Result<SshExecResult, String> {
        let wrapped = format!(
            "export CLAWPAL_LOGIN_CMD={cmd}; \
LOGIN_SHELL=\"${{SHELL:-/bin/sh}}\"; \
[ -x \"$LOGIN_SHELL\" ] || LOGIN_SHELL=\"/bin/sh\"; \
case \"$LOGIN_SHELL\" in \
  */zsh) \"$LOGIN_SHELL\" -lc '[ -f ~/.zprofile ] && . ~/.zprofile >/dev/null 2>&1 || true; [ -f ~/.zshrc ] && . ~/.zshrc >/dev/null 2>&1 || true; eval \"$CLAWPAL_LOGIN_CMD\"' ;; \
  */bash) \"$LOGIN_SHELL\" -lc '[ -f ~/.bash_profile ] && . ~/.bash_profile >/dev/null 2>&1 || true; [ -f ~/.bashrc ] && . ~/.bashrc >/dev/null 2>&1 || true; eval \"$CLAWPAL_LOGIN_CMD\"' ;; \
  *) \"$LOGIN_SHELL\" -lc 'eval \"$CLAWPAL_LOGIN_CMD\"' ;; \
esac",
            cmd = shell_quote(command)
        );
        self.exec(id, &wrapped).await
    }

    pub async fn sftp_read(&self, id: &str, path: &str) -> Result<String, String> {
        let resolved = self.resolve_path(id, path).await?;
        let conn = self.lookup_connected_host(id).await?;
        self.ensure_not_in_timeout_cooldown(id, &conn).await?;
        let _permit = conn
            .op_limiter
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| format!("ssh limiter acquire failed: {e}"))?;
        let session_connect = timeout(
            Duration::from_secs(SSH_EXEC_TIMEOUT_SECS),
            clawpal_core::ssh::SshSession::connect(&conn.config),
        ).await;
        let session = match session_connect {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => {
                self.mark_timeout_and_cooldown(&conn).await;
                return Err(format!("ssh connect timeout after {SSH_EXEC_TIMEOUT_SECS}s"));
            }
        };
        let bytes = timeout(
            Duration::from_secs(SSH_SFTP_TIMEOUT_SECS),
            session.sftp_read(&resolved),
        )
        .await
        .map_err(|_| format!("sftp read timeout after {SSH_SFTP_TIMEOUT_SECS}s"))?;
        let bytes = match bytes {
            Ok(v) => v,
            Err(e) => {
                let msg = e.to_string();
                if is_timeout_error_message(&msg) {
                    self.mark_timeout_and_cooldown(&conn).await;
                }
                return Err(msg);
            }
        };
        self.clear_timeout_cooldown(&conn).await;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    pub async fn sftp_write(&self, id: &str, path: &str, content: &str) -> Result<(), String> {
        let resolved = self.resolve_path(id, path).await?;
        let conn = self.lookup_connected_host(id).await?;
        self.ensure_not_in_timeout_cooldown(id, &conn).await?;
        let _permit = conn
            .op_limiter
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| format!("ssh limiter acquire failed: {e}"))?;
        let session_connect = timeout(
            Duration::from_secs(SSH_EXEC_TIMEOUT_SECS),
            clawpal_core::ssh::SshSession::connect(&conn.config),
        ).await;
        let session = match session_connect {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => {
                self.mark_timeout_and_cooldown(&conn).await;
                return Err(format!("ssh connect timeout after {SSH_EXEC_TIMEOUT_SECS}s"));
            }
        };
        let write_res = timeout(
            Duration::from_secs(SSH_SFTP_TIMEOUT_SECS),
            session.sftp_write(&resolved, content.as_bytes()),
        )
        .await
        .map_err(|_| format!("sftp write timeout after {SSH_SFTP_TIMEOUT_SECS}s"))?;
        match write_res {
            Ok(()) => {
                self.clear_timeout_cooldown(&conn).await;
                Ok(())
            }
            Err(e) => {
                let msg = e.to_string();
                if is_timeout_error_message(&msg) {
                    self.mark_timeout_and_cooldown(&conn).await;
                }
                Err(msg)
            }
        }
    }

    pub async fn sftp_list(&self, id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        let resolved = self.resolve_path(id, path).await?;
        let command = format!(
            "find {} -mindepth 1 -maxdepth 1 -printf '%f\\t%y\\t%s\\n' 2>/dev/null || true",
            shell_quote(&resolved)
        );
        let out = self.exec(id, &command).await?;
        let entries = out
            .stdout
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\t');
                let name = parts.next()?.to_string();
                let kind = parts.next().unwrap_or("f");
                let size = parts
                    .next()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);
                Some(SftpEntry {
                    name,
                    is_dir: kind == "d",
                    size,
                })
            })
            .collect();
        Ok(entries)
    }

    pub async fn sftp_remove(&self, id: &str, path: &str) -> Result<(), String> {
        let resolved = self.resolve_path(id, path).await?;
        let cmd = format!("rm -f {}", shell_quote(&resolved));
        let _ = self.exec(id, &cmd).await?;
        Ok(())
    }

    async fn lookup_connected_host(&self, id: &str) -> Result<ConnectedHost, String> {
        let guard = self.connections.lock().await;
        let conn = guard
            .get(id)
            .ok_or_else(|| format!("No connection for id: {id}"))?;
        Ok(conn.clone())
    }

    async fn ensure_not_in_timeout_cooldown(
        &self,
        id: &str,
        conn: &ConnectedHost,
    ) -> Result<(), String> {
        let mut state = conn.runtime.lock().await;
        if let Some(until) = state.cool_down_until {
            if Instant::now() < until {
                let left = until.saturating_duration_since(Instant::now()).as_secs();
                return Err(format!(
                    "ssh operations for {id} are cooling down after repeated timeouts; retry in {left}s"
                ));
            }
            state.cool_down_until = None;
        }
        Ok(())
    }

    async fn mark_timeout_and_cooldown(&self, conn: &ConnectedHost) {
        let mut state = conn.runtime.lock().await;
        state.consecutive_timeouts = state.consecutive_timeouts.saturating_add(1);
        let factor = 1_u64 << state.consecutive_timeouts.saturating_sub(1).min(3);
        let secs = (SSH_TIMEOUT_COOLDOWN_BASE_SECS * factor).min(SSH_TIMEOUT_COOLDOWN_MAX_SECS);
        state.cool_down_until = Some(Instant::now() + StdDuration::from_secs(secs));
    }

    async fn clear_timeout_cooldown(&self, conn: &ConnectedHost) {
        let mut state = conn.runtime.lock().await;
        state.consecutive_timeouts = 0;
        state.cool_down_until = None;
    }
}

impl Default for SshConnectionPool {
    fn default() -> Self {
        Self::new()
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn is_timeout_error_message(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    lowered.contains("timeout") || lowered.contains("timed out")
}

#[cfg(test)]
mod tests {
    use super::shell_quote;

    #[test]
    fn shell_quote_escapes_single_quote() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
