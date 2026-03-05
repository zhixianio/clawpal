use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTransferStats {
    pub host_id: String,
    pub upload_bytes_per_sec: u64,
    pub download_bytes_per_sec: u64,
    pub total_upload_bytes: u64,
    pub total_download_bytes: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Default)]
struct HostTransferCounter {
    total_upload_bytes: u64,
    total_download_bytes: u64,
    upload_bytes_per_sec: u64,
    download_bytes_per_sec: u64,
    updated_at_ms: u64,
    last_sample_at_ms: u64,
    last_sample_upload_bytes: u64,
    last_sample_download_bytes: u64,
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
    transfer_counters: Mutex<HashMap<String, HostTransferCounter>>,
    sftp_read_backoff_until_ms: Mutex<HashMap<String, u64>>,
}

const SSH_OP_MAX_CONCURRENCY_PER_HOST: usize = 2;
const SSH_TRANSFER_IDLE_DECAY_MS: u64 = 8_000;
const SFTP_READ_BACKOFF_MS: u64 = 60_000;

impl SshConnectionPool {
    fn format_connection_ids(connections: &HashMap<String, ConnectedHost>) -> String {
        let mut ids = connections.keys().cloned().collect::<Vec<String>>();
        ids.sort();
        ids.join(",")
    }

    pub fn new() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            transfer_counters: Mutex::new(HashMap::new()),
            sftp_read_backoff_until_ms: Mutex::new(HashMap::new()),
        }
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn refresh_counter_speed(counter: &mut HostTransferCounter, now_ms: u64) {
        if counter.last_sample_at_ms == 0 {
            counter.last_sample_at_ms = now_ms;
            counter.last_sample_upload_bytes = counter.total_upload_bytes;
            counter.last_sample_download_bytes = counter.total_download_bytes;
            if counter.total_upload_bytes > 0 {
                counter.upload_bytes_per_sec = counter.total_upload_bytes;
            }
            if counter.total_download_bytes > 0 {
                counter.download_bytes_per_sec = counter.total_download_bytes;
            }
            return;
        }
        let elapsed = now_ms.saturating_sub(counter.last_sample_at_ms);
        if elapsed < 400 {
            return;
        }
        let upload_delta = counter
            .total_upload_bytes
            .saturating_sub(counter.last_sample_upload_bytes);
        let download_delta = counter
            .total_download_bytes
            .saturating_sub(counter.last_sample_download_bytes);
        let elapsed = elapsed.max(1);
        counter.upload_bytes_per_sec = if upload_delta == 0 {
            0
        } else {
            upload_delta
                .saturating_mul(1000)
                .saturating_add(elapsed.saturating_sub(1))
                / elapsed
        };
        counter.download_bytes_per_sec = if download_delta == 0 {
            0
        } else {
            download_delta
                .saturating_mul(1000)
                .saturating_add(elapsed.saturating_sub(1))
                / elapsed
        };
        counter.last_sample_at_ms = now_ms;
        counter.last_sample_upload_bytes = counter.total_upload_bytes;
        counter.last_sample_download_bytes = counter.total_download_bytes;
    }

    async fn record_transfer(&self, host_id: &str, upload_bytes: u64, download_bytes: u64) {
        if upload_bytes == 0 && download_bytes == 0 {
            return;
        }
        let now_ms = Self::now_ms();
        let mut guard = self.transfer_counters.lock().await;
        let counter = guard.entry(host_id.to_string()).or_default();
        if upload_bytes > 0 {
            counter.total_upload_bytes = counter.total_upload_bytes.saturating_add(upload_bytes);
        }
        if download_bytes > 0 {
            counter.total_download_bytes =
                counter.total_download_bytes.saturating_add(download_bytes);
        }
        counter.updated_at_ms = now_ms;
        Self::refresh_counter_speed(counter, now_ms);
    }

    pub async fn get_transfer_stats(&self, host_id: &str) -> SshTransferStats {
        let now_ms = Self::now_ms();
        let mut guard = self.transfer_counters.lock().await;
        let counter = guard.entry(host_id.to_string()).or_default();
        // Idle decay: keep last observed rate for a short window so sparse
        // remote calls don't instantly collapse to 0 between samples.
        if now_ms.saturating_sub(counter.updated_at_ms) > SSH_TRANSFER_IDLE_DECAY_MS {
            counter.upload_bytes_per_sec = 0;
            counter.download_bytes_per_sec = 0;
        }
        SshTransferStats {
            host_id: host_id.to_string(),
            upload_bytes_per_sec: counter.upload_bytes_per_sec,
            download_bytes_per_sec: counter.download_bytes_per_sec,
            total_upload_bytes: counter.total_upload_bytes,
            total_download_bytes: counter.total_download_bytes,
            updated_at_ms: counter.updated_at_ms,
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
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] connect_with_passphrase begin id={} host={} user={} port={} auth_method={}",
            config.id,
            config.host,
            config.username,
            config.port,
            config.auth_method
        ));
        let passphrase_owned = passphrase
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        let session = clawpal_core::ssh::SshSession::connect_with_passphrase(
            config,
            passphrase_owned.as_deref(),
        )
        .await
        .map_err(|error| {
            let message = error.to_string();
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] connect_with_passphrase session connect failed id={} error={}",
                config.id, message
            ));
            message
        })?;
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] connect_with_passphrase session created id={}",
            config.id
        ));
        let session = std::sync::Arc::new(session);
        let home = session
            .exec("echo $HOME")
            .await
            .map(|result| result.stdout.trim().to_string())
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/root".to_string());
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] connect_with_passphrase resolved_home id={} home={}",
            config.id, home
        ));

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
        self.transfer_counters
            .lock()
            .await
            .entry(config.id.clone())
            .or_default();
        self.sftp_read_backoff_until_ms
            .lock()
            .await
            .remove(&config.id);
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] connect_with_passphrase cached id={} total={}",
            config.id,
            self.connections.lock().await.len()
        ));
        Ok(())
    }

    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        crate::commands::logs::log_dev(format!("[dev][ssh_pool] disconnect begin id={id}"));
        if let Some(host) = self.connections.lock().await.remove(id) {
            let session = host.session.lock().await.clone();
            session.close().await;
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] disconnect removed id={} home={}",
                id, host.home_dir
            ));
            self.sftp_read_backoff_until_ms.lock().await.remove(id);
        } else {
            let known = {
                let guard = self.connections.lock().await;
                Self::format_connection_ids(&guard)
            };
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] disconnect missing id={id} known={}",
                known
            ));
        }
        crate::commands::logs::log_dev(format!("[dev][ssh_pool] disconnect done id={id}"));
        Ok(())
    }

    pub async fn reconnect(&self, id: &str) -> Result<(), String> {
        let (config, passphrase) = {
            let guard = self.connections.lock().await;
            let host = guard.get(id).ok_or_else(|| {
                crate::commands::logs::log_dev(format!(
                    "[dev][ssh_pool] reconnect missing connection id={id} known={}",
                    Self::format_connection_ids(&guard)
                ));
                format!("No connection for id: {id}")
            })?;
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] reconnect begin id={} passphrase_present={}",
                id,
                host.passphrase.is_some()
            ));
            (host.config.clone(), host.passphrase.clone())
        };
        if let Err(error) = self
            .connect_with_passphrase(&config, passphrase.as_deref())
            .await
        {
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] reconnect failed id={} error={}",
                id, error
            ));
            return Err(format!("ssh reconnect failed: {error}"));
        }
        crate::commands::logs::log_dev(format!("[dev][ssh_pool] reconnect success id={id}"));
        Ok(())
    }

    pub async fn is_connected(&self, id: &str) -> bool {
        self.connections.lock().await.contains_key(id)
    }

    pub async fn request_port_forward(&self, _id: &str, _remote_port: u16) -> Result<u16, String> {
        Err("Port forward is not supported in stateless ssh mode yet".to_string())
    }

    pub async fn get_home_dir(&self, id: &str) -> Result<String, String> {
        let guard = self.connections.lock().await;
        let conn = guard.get(id).ok_or_else(|| {
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] get_home_dir missing connection id={id} known={}",
                Self::format_connection_ids(&guard)
            ));
            format!("No connection for id: {id}")
        })?;
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] get_home_dir found id={id} home={}",
            conn.home_dir
        ));
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
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] exec start id={} command={}",
            id, command
        ));
        let _permit = conn.op_limiter.clone().acquire_owned().await.map_err(|e| {
            let message = format!("ssh limiter acquire failed: {e}");
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] exec acquire semaphore failed id={} error={}",
                id, message
            ));
            message
        })?;
        self.record_transfer(id, command.len() as u64, 0).await;
        let mut result = {
            let session = conn.session.lock().await.clone();
            session.exec(command).await
        };
        if let Err(err) = &result {
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] exec got session error id={} error={}",
                id, err
            ));
            if is_retryable_session_error(&err.to_string()) {
                self.refresh_session(&conn).await?;
                let session = conn.session.lock().await.clone();
                result = session.exec(command).await;
            }
        }
        let result = result.map_err(|e| {
            let message = e.to_string();
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] exec failed id={} error={}",
                id, message
            ));
            message
        })?;
        let stdout_bytes = result.stdout.len() as u64;
        let stderr_bytes = result.stderr.len() as u64;
        self.record_transfer(id, 0, stdout_bytes.saturating_add(stderr_bytes))
            .await;
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] exec success id={} exit={} stderr_len={}",
            id,
            result.exit_code,
            result.stderr.len()
        ));
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
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] sftp_read start id={} path={}",
            id, resolved
        ));
        let _permit = conn.op_limiter.clone().acquire_owned().await.map_err(|e| {
            let message = format!("ssh limiter acquire failed: {e}");
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] sftp_read acquire semaphore failed id={} error={}",
                id, message
            ));
            message
        })?;
        let now_ms = Self::now_ms();
        if self.is_sftp_read_backoff_active(id, now_ms).await {
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] sftp_read skip primary due backoff id={} path={}",
                id, resolved
            ));
            let bytes = self.exec_cat_read_with_retry(&conn, &resolved).await?;
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] sftp_read bytes id={} path={} bytes={} source=exec-fallback(backoff)",
                id,
                resolved,
                bytes.len()
            ));
            self.record_transfer(id, 0, bytes.len() as u64).await;
            return Ok(String::from_utf8_lossy(&bytes).to_string());
        }
        let mut bytes = {
            let session = conn.session.lock().await.clone();
            session.sftp_read(&resolved).await
        };
        if let Err(err) = &bytes {
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] sftp_read primary error id={} path={} error={}",
                id, resolved, err
            ));
            if should_backoff_sftp_read(&err.to_string()) {
                self.set_sftp_read_backoff(id, Self::now_ms()).await;
            }
            if is_retryable_session_error(&err.to_string()) {
                self.refresh_session(&conn).await?;
                let session = conn.session.lock().await.clone();
                bytes = session.sftp_read(&resolved).await;
            }
        }
        let bytes = match bytes {
            Ok(bytes) => {
                self.clear_sftp_read_backoff(id).await;
                bytes
            }
            Err(primary_err) => {
                let primary_msg = primary_err.to_string();
                if should_backoff_sftp_read(&primary_msg) {
                    self.set_sftp_read_backoff(id, Self::now_ms()).await;
                }
                if !should_attempt_sftp_exec_fallback(&primary_msg) {
                    crate::commands::logs::log_dev(format!(
                        "[dev][ssh_pool] sftp_read failed without fallback id={} path={} error={}",
                        id, resolved, primary_msg
                    ));
                    return Err(primary_msg);
                }
                match self.exec_cat_read_with_retry(&conn, &resolved).await {
                    Ok(bytes) => bytes,
                    Err(fallback_err) => {
                        crate::commands::logs::log_dev(format!(
                            "[dev][ssh_pool] sftp_read fallback failed id={} path={} error={}",
                            id, resolved, fallback_err
                        ));
                        return Err(format!(
                            "{primary_msg}; fallback via ssh cat failed: {fallback_err}"
                        ));
                    }
                }
            }
        };
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] sftp_read bytes id={} path={} bytes={}",
            id,
            resolved,
            bytes.len()
        ));
        self.record_transfer(id, 0, bytes.len() as u64).await;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    pub async fn sftp_write(&self, id: &str, path: &str, content: &str) -> Result<(), String> {
        let resolved = self.resolve_path(id, path).await?;
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] sftp_write start id={} path={}",
            id, resolved
        ));
        let conn = self.lookup_connected_host(id).await?;
        let _permit = conn.op_limiter.clone().acquire_owned().await.map_err(|e| {
            let message = format!("ssh limiter acquire failed: {e}");
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] sftp_write acquire semaphore failed id={} error={}",
                id, message
            ));
            message
        })?;
        let mut write_res = {
            let session = conn.session.lock().await.clone();
            session.sftp_write(&resolved, content.as_bytes()).await
        };
        if let Err(err) = &write_res {
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] sftp_write primary error id={} path={} error={}",
                id, resolved, err
            ));
            if is_retryable_session_error(&err.to_string()) {
                self.refresh_session(&conn).await?;
                let session = conn.session.lock().await.clone();
                write_res = session.sftp_write(&resolved, content.as_bytes()).await;
            }
        }
        write_res.map_err(|e| {
            let message = e.to_string();
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] sftp_write failed id={} path={} error={}",
                id, resolved, message
            ));
            message
        })?;
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] sftp_write success id={} path={}",
            id, resolved
        ));
        self.record_transfer(id, content.as_bytes().len() as u64, 0)
            .await;
        Ok(())
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
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] sftp_remove start id={} path={}",
            id, resolved
        ));
        let cmd = format!("rm -f {}", shell_quote(&resolved));
        let exec_result = self.exec(id, &cmd).await;
        if let Err(error) = exec_result {
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] sftp_remove exec failed id={} path={} error={}",
                id, resolved, error
            ));
            return Err(error);
        }
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] sftp_remove success id={} path={}",
            id, resolved
        ));
        Ok(())
    }

    async fn lookup_connected_host(&self, id: &str) -> Result<ConnectedHost, String> {
        let guard = self.connections.lock().await;
        let conn = guard.get(id).ok_or_else(|| {
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] lookup_connected_host missing id={} known={}",
                id,
                Self::format_connection_ids(&guard)
            ));
            format!("No connection for id: {id}")
        })?;
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] lookup_connected_host found id={} host={}",
            id, conn.config.host
        ));
        Ok(conn.clone())
    }

    async fn is_sftp_read_backoff_active(&self, id: &str, now_ms: u64) -> bool {
        let mut guard = self.sftp_read_backoff_until_ms.lock().await;
        let Some(until_ms) = guard.get(id).copied() else {
            return false;
        };
        if now_ms < until_ms {
            return true;
        }
        guard.remove(id);
        false
    }

    async fn set_sftp_read_backoff(&self, id: &str, now_ms: u64) {
        let until_ms = now_ms.saturating_add(SFTP_READ_BACKOFF_MS);
        self.sftp_read_backoff_until_ms
            .lock()
            .await
            .insert(id.to_string(), until_ms);
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] sftp_read backoff enabled id={} until_ms={}",
            id, until_ms
        ));
    }

    async fn clear_sftp_read_backoff(&self, id: &str) {
        self.sftp_read_backoff_until_ms.lock().await.remove(id);
    }

    async fn refresh_session(&self, conn: &ConnectedHost) -> Result<(), String> {
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] refresh_session begin id={}",
            conn.config.id
        ));
        let new_session = std::sync::Arc::new(
            clawpal_core::ssh::SshSession::connect_with_passphrase(
                &conn.config,
                conn.passphrase.as_deref(),
            )
            .await
            .map_err(|error| {
                let message = error.to_string();
                crate::commands::logs::log_dev(format!(
                    "[dev][ssh_pool] refresh_session connect failed id={} error={}",
                    conn.config.id, message
                ));
                message
            })?,
        );
        let mut guard = conn.session.lock().await;
        let old = std::mem::replace(&mut *guard, new_session);
        old.close().await;
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] refresh_session done id={}",
            conn.config.id
        ));
        Ok(())
    }

    async fn exec_cat_read_with_retry(
        &self,
        conn: &ConnectedHost,
        path: &str,
    ) -> Result<Vec<u8>, String> {
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] exec_cat_read_with_retry start id={} path={}",
            conn.config.id, path
        ));
        let mut out = {
            let session = conn.session.lock().await.clone();
            let cmd = format!("cat {}", shell_quote(path));
            session.exec(&cmd).await
        };
        if let Err(err) = &out {
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] exec_cat_read_with_retry primary error id={} path={} error={}",
                conn.config.id, path, err
            ));
            if is_retryable_session_error(&err.to_string()) {
                self.refresh_session(conn).await?;
                let session = conn.session.lock().await.clone();
                let cmd = format!("cat {}", shell_quote(path));
                out = session.exec(&cmd).await;
            }
        }
        let out = out.map_err(|error| {
            let message = error.to_string();
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] exec_cat_read_with_retry failed id={} path={} error={}",
                conn.config.id, path, message
            ));
            message
        })?;
        if out.exit_code != 0 {
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_pool] exec_cat_read_with_retry nonzero exit id={} path={} exit_code={} stderr={}",
                conn.config.id,
                path,
                out.exit_code,
                out.stderr
            ));
            return Err(format!(
                "cat exited with code {}: {}",
                out.exit_code, out.stderr
            ));
        }
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_pool] exec_cat_read_with_retry success id={} path={} bytes={}",
            conn.config.id,
            path,
            out.stdout.len()
        ));
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

fn should_backoff_sftp_read(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    lowered.contains("timed out")
        || lowered.contains("timeout")
        || lowered.contains("open channel")
        || lowered.contains("connection reset")
        || lowered.contains("broken pipe")
        || lowered.contains("connection closed")
}

#[cfg(test)]
mod tests {
    use super::{
        build_login_shell_wrapper, shell_quote, should_attempt_sftp_exec_fallback,
        should_backoff_sftp_read, HostTransferCounter, SshConnectionPool,
    };

    #[test]
    fn shell_quote_escapes_single_quote() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn login_wrapper_sources_common_profile_files() {
        let wrapped = build_login_shell_wrapper("openclaw --version");
        assert!(wrapped.contains("*/zsh|*/bash) \"$LOGIN_SHELL\" -lc"));
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

    #[test]
    fn sftp_backoff_only_triggers_for_transport_like_errors() {
        assert!(should_backoff_sftp_read(
            "sftp failed: russh sftp_read timed out after 30s"
        ));
        assert!(should_backoff_sftp_read(
            "ssh open channel failed: channel closed"
        ));
        assert!(!should_backoff_sftp_read(
            "open /tmp/missing.json: No such file or directory"
        ));
    }

    #[test]
    fn transfer_speed_initial_sample_is_not_zero_for_non_zero_bytes() {
        let mut counter = HostTransferCounter {
            total_download_bytes: 24,
            ..Default::default()
        };
        SshConnectionPool::refresh_counter_speed(&mut counter, 1_000);
        assert_eq!(counter.download_bytes_per_sec, 24);
    }

    #[test]
    fn transfer_speed_uses_ceil_division_for_small_deltas() {
        let mut counter = HostTransferCounter::default();
        // Initialize sampling anchor.
        SshConnectionPool::refresh_counter_speed(&mut counter, 1_000);
        counter.total_download_bytes = 1;
        // 1 byte over 2000ms should still report 1 B/s instead of flooring to 0.
        SshConnectionPool::refresh_counter_speed(&mut counter, 3_000);
        assert_eq!(counter.download_bytes_per_sec, 1);
    }
}
