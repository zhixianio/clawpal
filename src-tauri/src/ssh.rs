use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Semaphore};

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

#[derive(Clone)]
struct ConnectedHost {
    config: SshHostConfig,
    home_dir: String,
    passphrase: Option<String>,
    session: std::sync::Arc<Mutex<std::sync::Arc<clawpal_core::ssh::SshSession>>>,
    op_limiter: std::sync::Arc<Semaphore>,
}

pub struct SshConnectionPool {
    connections: Mutex<HashMap<String, ConnectedHost>>,
}

const SSH_OP_MAX_CONCURRENCY_PER_HOST: usize = 2;

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
        passphrase: Option<&str>,
    ) -> Result<(), String> {
        let passphrase_owned = passphrase
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        let session = std::sync::Arc::new(
            clawpal_core::ssh::SshSession::connect_with_passphrase(
                config,
                passphrase_owned.as_deref(),
            )
                .await
                .map_err(|e| e.to_string())?,
        );
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
                passphrase: passphrase_owned,
                session: std::sync::Arc::new(Mutex::new(session)),
                op_limiter: std::sync::Arc::new(Semaphore::new(SSH_OP_MAX_CONCURRENCY_PER_HOST)),
            },
        );
        Ok(())
    }

    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        if let Some(host) = self.connections.lock().await.remove(id) {
            let session = host.session.lock().await.clone();
            session.close().await;
        }
        Ok(())
    }

    pub async fn reconnect(&self, id: &str) -> Result<(), String> {
        let (config, passphrase) = {
            let guard = self.connections.lock().await;
            let host = guard
                .get(id)
                .ok_or_else(|| format!("No connection for id: {id}"))?;
            (host.config.clone(), host.passphrase.clone())
        };
        self.connect_with_passphrase(&config, passphrase.as_deref())
            .await
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
        let _permit = conn
            .op_limiter
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| format!("ssh limiter acquire failed: {e}"))?;
        let mut result = {
            let session = conn.session.lock().await.clone();
            session.exec(command).await
        };
        if let Err(err) = &result {
            if is_retryable_session_error(&err.to_string()) {
                self.refresh_session(&conn).await?;
                let session = conn.session.lock().await.clone();
                result = session.exec(command).await;
            }
        }
        let result = result.map_err(|e| e.to_string())?;
        Ok(SshExecResult {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code.max(0) as u32,
        })
    }

    pub async fn exec_login(&self, id: &str, command: &str) -> Result<SshExecResult, String> {
        let wrapped = build_login_shell_wrapper(command);
        self.exec(id, &wrapped).await
    }

    pub async fn sftp_read(&self, id: &str, path: &str) -> Result<String, String> {
        let resolved = self.resolve_path(id, path).await?;
        let conn = self.lookup_connected_host(id).await?;
        let _permit = conn
            .op_limiter
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| format!("ssh limiter acquire failed: {e}"))?;
        let mut bytes = {
            let session = conn.session.lock().await.clone();
            session.sftp_read(&resolved).await
        };
        if let Err(err) = &bytes {
            if is_retryable_session_error(&err.to_string()) {
                self.refresh_session(&conn).await?;
                let session = conn.session.lock().await.clone();
                bytes = session.sftp_read(&resolved).await;
            }
        }
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(primary_err) => {
                let primary_msg = primary_err.to_string();
                if !should_attempt_sftp_exec_fallback(&primary_msg) {
                    return Err(primary_msg);
                }
                match self.exec_cat_read_with_retry(&conn, &resolved).await {
                    Ok(bytes) => bytes,
                    Err(fallback_err) => {
                        return Err(format!(
                            "{primary_msg}; fallback via ssh cat failed: {fallback_err}"
                        ));
                    }
                }
            }
        };
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    pub async fn sftp_write(&self, id: &str, path: &str, content: &str) -> Result<(), String> {
        let resolved = self.resolve_path(id, path).await?;
        let conn = self.lookup_connected_host(id).await?;
        let _permit = conn
            .op_limiter
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| format!("ssh limiter acquire failed: {e}"))?;
        let mut write_res = {
            let session = conn.session.lock().await.clone();
            session.sftp_write(&resolved, content.as_bytes()).await
        };
        if let Err(err) = &write_res {
            if is_retryable_session_error(&err.to_string()) {
                self.refresh_session(&conn).await?;
                let session = conn.session.lock().await.clone();
                write_res = session.sftp_write(&resolved, content.as_bytes()).await;
            }
        }
        write_res.map_err(|e| e.to_string())
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

    async fn refresh_session(&self, conn: &ConnectedHost) -> Result<(), String> {
        let new_session = std::sync::Arc::new(
            clawpal_core::ssh::SshSession::connect_with_passphrase(
                &conn.config,
                conn.passphrase.as_deref(),
            )
                .await
                .map_err(|e| e.to_string())?,
        );
        let mut guard = conn.session.lock().await;
        let old = std::mem::replace(&mut *guard, new_session);
        old.close().await;
        Ok(())
    }

    async fn exec_cat_read_with_retry(
        &self,
        conn: &ConnectedHost,
        path: &str,
    ) -> Result<Vec<u8>, String> {
        let mut out = {
            let session = conn.session.lock().await.clone();
            let cmd = format!("cat {}", shell_quote(path));
            session.exec(&cmd).await
        };
        if let Err(err) = &out {
            if is_retryable_session_error(&err.to_string()) {
                self.refresh_session(conn).await?;
                let session = conn.session.lock().await.clone();
                let cmd = format!("cat {}", shell_quote(path));
                out = session.exec(&cmd).await;
            }
        }
        let out = out.map_err(|e| e.to_string())?;
        if out.exit_code != 0 {
            return Err(format!(
                "cat exited with code {}: {}",
                out.exit_code, out.stderr
            ));
        }
        Ok(out.stdout.into_bytes())
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

fn build_login_shell_wrapper(command: &str) -> String {
    clawpal_core::shell::wrap_login_shell_eval(command)
}

fn is_retryable_session_error(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    lowered.contains("ssh open channel failed")
        || lowered.contains("connection reset")
        || lowered.contains("broken pipe")
        || lowered.contains("connection closed")
}

fn should_attempt_sftp_exec_fallback(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    lowered.contains("timed out")
        || lowered.contains("timeout")
        || lowered.contains("sftp")
        || lowered.contains("open channel")
        || lowered.contains("connection reset")
        || lowered.contains("broken pipe")
        || lowered.contains("connection closed")
}

#[cfg(test)]
mod tests {
    use super::{build_login_shell_wrapper, shell_quote, should_attempt_sftp_exec_fallback};

    #[test]
    fn shell_quote_escapes_single_quote() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn login_wrapper_sources_common_profile_files() {
        let wrapped = build_login_shell_wrapper("openclaw --version");
        assert!(wrapped.contains("*/zsh|*/bash) \"$LOGIN_SHELL\" -ilc"));
        assert!(wrapped.contains("[ -f ~/.profile ]"));
    }

    #[test]
    fn sftp_fallback_is_enabled_for_timeout_like_errors() {
        assert!(should_attempt_sftp_exec_fallback(
            "sftp failed: russh sftp_read timed out after 30s"
        ));
        assert!(should_attempt_sftp_exec_fallback(
            "ssh open channel failed: channel closed"
        ));
        assert!(!should_attempt_sftp_exec_fallback(
            "open /tmp/missing.json: No such file or directory"
        ));
    }
}
