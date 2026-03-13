pub mod config;
pub mod diagnostic;
pub mod registry;

use std::process::Stdio;
use std::sync::Arc;

use russh::client;
use russh_keys::key;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use crate::instance::SshHostConfig;

#[derive(Clone)]
pub struct SshSession {
    config: SshHostConfig,
    backend: Backend,
}

#[derive(Clone)]
enum Backend {
    Russh {
        handle: Arc<client::Handle<SshHandler>>,
    },
    Legacy,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Debug, Error)]
pub enum SshError {
    #[error("ssh connect failed: {0}")]
    Connect(String),
    #[error("ssh auth failed: {0}")]
    Auth(String),
    #[error("ssh open channel failed: {0}")]
    Channel(String),
    #[error("ssh command failed: {0}")]
    CommandFailed(String),
    #[error("invalid host config: {0}")]
    InvalidConfig(String),
    #[error("sftp failed: {0}")]
    Sftp(String),
}

pub type Result<T> = std::result::Result<T, SshError>;
const LEGACY_SSH_CONNECT_TIMEOUT_SECS: u64 = 12;
const LEGACY_SSH_SERVER_ALIVE_INTERVAL_SECS: u64 = 15;
const LEGACY_SSH_SERVER_ALIVE_COUNT_MAX: u64 = 2;
const RUSSH_CONNECT_TIMEOUT_SECS: u64 = 10;
const RUSSH_AUTH_TIMEOUT_SECS: u64 = 12;
const RUSSH_DISCONNECT_TIMEOUT_SECS: u64 = 3;
const RUSSH_EXEC_TIMEOUT_SECS: u64 = 25;
const RUSSH_SFTP_TIMEOUT_SECS: u64 = 30;

#[derive(Clone)]
struct SshHandler;

fn russh_exec_timeout_secs_from_env_var(raw: Option<String>) -> u64 {
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(RUSSH_EXEC_TIMEOUT_SECS)
}

fn russh_exec_timeout_secs() -> u64 {
    russh_exec_timeout_secs_from_env_var(std::env::var("CLAWPAL_RUSSH_EXEC_TIMEOUT_SECS").ok())
}

#[async_trait::async_trait]
impl client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &key::PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        // TODO: known_hosts verification
        Ok(true)
    }
}

#[derive(Debug, Clone)]
struct ResolvedTarget {
    host: String,
    port: u16,
    username: String,
    key_path: Option<String>,
}

impl SshSession {
    pub async fn connect(config: &SshHostConfig) -> Result<Self> {
        Self::connect_with_passphrase(config, None).await
    }

    pub async fn connect_with_passphrase(
        config: &SshHostConfig,
        passphrase: Option<&str>,
    ) -> Result<Self> {
        if config.host.trim().is_empty() {
            return Err(SshError::InvalidConfig("host is empty".to_string()));
        }
        if config.auth_method.trim().eq_ignore_ascii_case("password")
            && config
                .password
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .is_none()
        {
            return Err(SshError::InvalidConfig(
                "password auth selected but password is empty".to_string(),
            ));
        }
        let backend = match connect_and_auth(config, passphrase).await {
            Ok((handle, _)) => Backend::Russh {
                handle: Arc::new(handle),
            },
            Err(err) => {
                if config.auth_method.trim().eq_ignore_ascii_case("password")
                    || matches!(&err, SshError::Auth(_) | SshError::InvalidConfig(_))
                {
                    return Err(err);
                }
                Backend::Legacy
            }
        };
        let session = Self {
            config: config.clone(),
            backend,
        };
        if matches!(session.backend, Backend::Legacy) {
            session.verify_legacy_connectivity().await?;
        }
        Ok(session)
    }

    pub async fn exec(&self, cmd: &str) -> Result<ExecResult> {
        let handle = match &self.backend {
            Backend::Russh { handle } => handle.clone(),
            Backend::Legacy => return self.exec_legacy(cmd).await,
        };
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::Channel(e.to_string()))?;
        channel
            .exec(true, cmd)
            .await
            .map_err(|e| SshError::CommandFailed(e.to_string()))?;

        let exec_timeout_secs = russh_exec_timeout_secs();
        let wait_result = timeout(Duration::from_secs(exec_timeout_secs), async {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut exit_code = -1;
            while let Some(msg) = channel.wait().await {
                match msg {
                    russh::ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                    russh::ChannelMsg::ExtendedData { data, ext } => {
                        if ext == 1 {
                            stderr.extend_from_slice(&data);
                        }
                    }
                    russh::ChannelMsg::ExitStatus { exit_status } => {
                        exit_code = exit_status as i32;
                    }
                    _ => {}
                }
            }
            (stdout, stderr, exit_code)
        })
        .await;

        let (stdout, stderr, exit_code) = wait_result.map_err(|_| {
            SshError::CommandFailed(format!("russh exec timed out after {exec_timeout_secs}s"))
        })?;

        Ok(ExecResult {
            stdout: String::from_utf8_lossy(&stdout).trim_end().to_string(),
            stderr: String::from_utf8_lossy(&stderr).trim_end().to_string(),
            exit_code,
        })
    }

    pub async fn sftp_read(&self, path: &str) -> Result<Vec<u8>> {
        let handle = match &self.backend {
            Backend::Russh { handle } => handle.clone(),
            Backend::Legacy => return self.sftp_read_legacy(path).await,
        };
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::Sftp(e.to_string()))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| SshError::Sftp(e.to_string()))?;

        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| SshError::Sftp(e.to_string()))?;

        let resolved = resolve_remote_path(&self.config, path).await?;
        let read_result = timeout(Duration::from_secs(RUSSH_SFTP_TIMEOUT_SECS), async {
            let mut file = sftp
                .open(resolved.as_str())
                .await
                .map_err(|e| SshError::Sftp(format!("open {resolved}: {e}")))?;
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .await
                .map_err(|e| SshError::Sftp(e.to_string()))?;
            Ok::<Vec<u8>, SshError>(buf)
        })
        .await;
        match read_result {
            Ok(v) => v,
            Err(_) => Err(SshError::Sftp(format!(
                "russh sftp_read timed out after {RUSSH_SFTP_TIMEOUT_SECS}s"
            ))),
        }
    }

    pub async fn sftp_write(&self, path: &str, content: &[u8]) -> Result<()> {
        let handle = match &self.backend {
            Backend::Russh { handle } => handle.clone(),
            Backend::Legacy => return self.sftp_write_legacy(path, content).await,
        };

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::Sftp(e.to_string()))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| SshError::Sftp(e.to_string()))?;

        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| SshError::Sftp(e.to_string()))?;

        let resolved = resolve_remote_path(&self.config, path).await?;
        let parent = std::path::Path::new(&resolved)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());

        // Ensure parent dir exists before SFTP create on the same SSH connection.
        let mkdir_cmd = format!("mkdir -p {}", shell_escape(&parent));
        {
            let mut mkdir_ch = handle
                .channel_open_session()
                .await
                .map_err(|e| SshError::Sftp(format!("open mkdir channel: {e}")))?;
            mkdir_ch
                .exec(true, mkdir_cmd.as_str())
                .await
                .map_err(|e| SshError::Sftp(format!("mkdir exec: {e}")))?;
            let mkdir_wait = timeout(Duration::from_secs(5), async {
                let mut stderr = Vec::new();
                let mut exit_code = -1;
                while let Some(msg) = mkdir_ch.wait().await {
                    match msg {
                        russh::ChannelMsg::ExtendedData { data, ext: 1 } => {
                            stderr.extend_from_slice(&data);
                        }
                        russh::ChannelMsg::ExitStatus { exit_status } => {
                            exit_code = exit_status as i32;
                        }
                        _ => {}
                    }
                }
                (exit_code, stderr)
            })
            .await;
            let (exit_code, stderr) = mkdir_wait
                .map_err(|_| SshError::Sftp("mkdir wait timed out after 5s".to_string()))?;
            if exit_code != 0 {
                return Err(SshError::Sftp(format!(
                    "mkdir parent failed for {resolved}: {}",
                    String::from_utf8_lossy(&stderr).trim_end()
                )));
            }
        }

        let write_result = timeout(Duration::from_secs(RUSSH_SFTP_TIMEOUT_SECS), async {
            use tokio::io::AsyncWriteExt;
            let mut file = sftp
                .create(resolved.as_str())
                .await
                .map_err(|e| SshError::Sftp(format!("create {resolved}: {e}")))?;
            file.write_all(content)
                .await
                .map_err(|e| SshError::Sftp(e.to_string()))?;
            file.flush()
                .await
                .map_err(|e| SshError::Sftp(e.to_string()))?;
            Ok::<(), SshError>(())
        })
        .await;
        match write_result {
            Ok(v) => v,
            Err(_) => Err(SshError::Sftp(format!(
                "russh sftp_write timed out after {RUSSH_SFTP_TIMEOUT_SECS}s"
            ))),
        }
    }

    async fn exec_legacy(&self, cmd: &str) -> Result<ExecResult> {
        let output = self.run_legacy_ssh(&[cmd]).await?;
        Ok(ExecResult {
            stdout: String::from_utf8_lossy(&output.stdout)
                .trim_end()
                .to_string(),
            stderr: String::from_utf8_lossy(&output.stderr)
                .trim_end()
                .to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    async fn sftp_read_legacy(&self, path: &str) -> Result<Vec<u8>> {
        let escaped = shell_escape(path);
        let command = format!("cat {escaped}");
        let output = self.run_legacy_ssh(&[command.as_str()]).await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(SshError::CommandFailed(format!(
                "cat {path} exited with code {:?}: {stderr}",
                output.status.code()
            )));
        }
        Ok(output.stdout)
    }

    async fn sftp_write_legacy(&self, path: &str, content: &[u8]) -> Result<()> {
        let escaped = shell_escape(path);
        let command = format!("mkdir -p \"$(dirname {escaped})\" && cat > {escaped}");
        let destination = self.legacy_destination();

        let mut child = Command::new("ssh")
            .args(self.legacy_common_ssh_args())
            .arg(destination)
            .arg(command)
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SshError::Connect(e.to_string()))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(content)
                .await
                .map_err(|e| SshError::Sftp(e.to_string()))?;
        }
        let output = child
            .wait_with_output()
            .await
            .map_err(|e| SshError::Sftp(e.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(SshError::CommandFailed(format!(
                "write {path} exited with code {:?}: {stderr}",
                output.status.code()
            )));
        }
        Ok(())
    }

    fn legacy_destination(&self) -> String {
        if self.config.username.trim().is_empty() {
            self.config.host.clone()
        } else {
            format!("{}@{}", self.config.username, self.config.host)
        }
    }

    fn legacy_common_ssh_args(&self) -> Vec<String> {
        let mut args = vec![
            "-p".to_string(),
            self.config.port.to_string(),
            "-o".to_string(),
            format!("ConnectTimeout={LEGACY_SSH_CONNECT_TIMEOUT_SECS}"),
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "-o".to_string(),
            format!("ServerAliveInterval={LEGACY_SSH_SERVER_ALIVE_INTERVAL_SECS}"),
            "-o".to_string(),
            format!("ServerAliveCountMax={LEGACY_SSH_SERVER_ALIVE_COUNT_MAX}"),
        ];
        if let Some(key_path) = &self.config.key_path {
            if !key_path.trim().is_empty() {
                args.push("-i".to_string());
                args.push(key_path.clone());
            }
        }
        args
    }

    async fn run_legacy_ssh(&self, remote_args: &[&str]) -> Result<std::process::Output> {
        let mut cmd = Command::new("ssh");
        cmd.args(self.legacy_common_ssh_args())
            .arg(self.legacy_destination());
        for arg in remote_args {
            cmd.arg(arg);
        }
        // Ensure cancellation does not leak child ssh processes when outer futures time out.
        cmd.kill_on_drop(true);
        timeout(
            Duration::from_secs(
                LEGACY_SSH_CONNECT_TIMEOUT_SECS
                    + LEGACY_SSH_SERVER_ALIVE_INTERVAL_SECS * LEGACY_SSH_SERVER_ALIVE_COUNT_MAX
                    + 8,
            ),
            cmd.output(),
        )
        .await
        .map_err(|_| SshError::Connect("legacy ssh timed out".to_string()))?
        .map_err(|e| SshError::Connect(e.to_string()))
    }

    async fn verify_legacy_connectivity(&self) -> Result<()> {
        let output = self.run_legacy_ssh(&["true"]).await?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(SshError::Connect(format!(
            "legacy ssh connectivity check failed (exit {:?}): {}",
            output.status.code(),
            if stderr.is_empty() {
                "unknown error"
            } else {
                stderr.as_str()
            }
        )))
    }

    pub async fn close(&self) {
        if let Backend::Russh { handle } = &self.backend {
            let _ = timeout(
                Duration::from_secs(RUSSH_DISCONNECT_TIMEOUT_SECS),
                handle.disconnect(russh::Disconnect::ByApplication, "", "en"),
            )
            .await;
        }
    }
}

async fn connect_and_auth(
    config: &SshHostConfig,
    passphrase: Option<&str>,
) -> Result<(client::Handle<SshHandler>, ResolvedTarget)> {
    let resolved = resolve_target(config)?;
    let addr = format!("{}:{}", resolved.host, resolved.port);
    let ssh_config = Arc::new(client::Config::default());
    let mut handle = timeout(
        Duration::from_secs(RUSSH_CONNECT_TIMEOUT_SECS),
        client::connect(ssh_config, addr.clone(), SshHandler),
    )
    .await
    .map_err(|_| {
        SshError::Connect(format!(
            "russh TCP connect to {addr} timed out after {RUSSH_CONNECT_TIMEOUT_SECS}s"
        ))
    })?
    .map_err(|e| SshError::Connect(e.to_string()))?;

    if config.auth_method.trim().eq_ignore_ascii_case("password") {
        let password = config
            .password
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                SshError::InvalidConfig("password auth selected but password is empty".to_string())
            })?;
        let ok = timeout(
            Duration::from_secs(RUSSH_AUTH_TIMEOUT_SECS),
            handle.authenticate_password(&resolved.username, password),
        )
        .await
        .map_err(|_| {
            SshError::Auth(format!(
                "password authentication timed out after {RUSSH_AUTH_TIMEOUT_SECS}s"
            ))
        })?
        .map_err(|e| SshError::Auth(e.to_string()))?;
        if ok {
            return Ok((handle, resolved));
        }
        return Err(SshError::Auth("password authentication failed".to_string()));
    }

    let mut attempts: Vec<String> = Vec::new();
    for key_path in candidate_key_paths(&resolved) {
        let expanded = shellexpand::tilde(&key_path).to_string();
        let key_pair = passphrase
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .and_then(
                |phrase| match russh_keys::load_secret_key(&expanded, Some(phrase)) {
                    Ok(pair) => Some(pair),
                    Err(err) => {
                        attempts.push(format!(
                            "{expanded}: encrypted or passphrase mismatch ({err})"
                        ));
                        None
                    }
                },
            )
            .or_else(|| match russh_keys::load_secret_key(&expanded, None) {
                Ok(pair) => Some(pair),
                Err(err) => {
                    attempts.push(format!("{expanded}: failed to load ({err})"));
                    None
                }
            });
        let Some(key_pair) = key_pair else {
            continue;
        };
        let ok = timeout(
            Duration::from_secs(RUSSH_AUTH_TIMEOUT_SECS),
            handle.authenticate_publickey(&resolved.username, Arc::new(key_pair)),
        )
        .await
        .map_err(|_| {
            attempts.push(format!(
                "{expanded}: auth timed out after {RUSSH_AUTH_TIMEOUT_SECS}s"
            ));
            SshError::Auth(format!(
                "public key authentication timed out after {RUSSH_AUTH_TIMEOUT_SECS}s"
            ))
        })?
        .map_err(|e| {
            attempts.push(format!("{expanded}: auth request failed ({})", e));
            SshError::Auth(e.to_string())
        })?;
        if ok {
            return Ok((handle, resolved));
        }
        attempts.push(format!("{expanded}: auth rejected"));
    }

    let details = if attempts.is_empty() {
        "no candidate keys were available".to_string()
    } else {
        attempts.join("; ")
    };
    Err(SshError::Auth(format!(
        "public key authentication failed for {}@{}:{} after trying {}",
        resolved.username, resolved.host, resolved.port, details
    )))
}

fn resolve_target(config: &SshHostConfig) -> Result<ResolvedTarget> {
    let mut host = config.host.trim().to_string();
    let mut port = if config.port == 0 { 22 } else { config.port };
    let mut username = config.username.trim().to_string();
    let mut key_path = config.key_path.clone();

    if config.auth_method.trim().eq_ignore_ascii_case("ssh_config") {
        if let Some(entry) = resolve_ssh_config_entry(&host) {
            if let Some(host_name) = entry.host_name {
                host = host_name;
            }
            if username.is_empty() {
                if let Some(user) = entry.user {
                    username = user;
                }
            }
            if config.port == 22 {
                if let Some(p) = entry.port {
                    port = p;
                }
            }
            if key_path.is_none() {
                key_path = entry.identity_file;
            }
        }
    }

    if username.is_empty() {
        username = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "root".to_string());
    }

    Ok(ResolvedTarget {
        host,
        port,
        username,
        key_path,
    })
}

fn resolve_ssh_config_entry(host_alias: &str) -> Option<config::SshConfigHostSuggestion> {
    let home = dirs::home_dir()?;
    let path = home.join(".ssh").join("config");
    let data = std::fs::read_to_string(path).ok()?;
    config::parse_ssh_config_hosts(&data)
        .into_iter()
        .find(|h| h.host_alias == host_alias)
}

fn candidate_key_paths(target: &ResolvedTarget) -> Vec<String> {
    if let Some(path) = &target.key_path {
        return vec![path.clone()];
    }
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let ssh = home.join(".ssh");
    vec![
        ssh.join("id_ed25519").to_string_lossy().to_string(),
        ssh.join("id_rsa").to_string_lossy().to_string(),
    ]
}

async fn resolve_remote_path(config: &SshHostConfig, path: &str) -> Result<String> {
    if !path.starts_with('~') {
        return Ok(path.to_string());
    }
    let session = SshSession::connect(config).await?;
    let home = session.exec("echo $HOME").await?;
    if home.exit_code != 0 || home.stdout.trim().is_empty() {
        return Err(SshError::InvalidConfig(
            "cannot resolve remote home directory".to_string(),
        ));
    }
    Ok(path.replacen('~', home.stdout.trim(), 1))
}

fn shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_rejects_empty_host() {
        let cfg = SshHostConfig {
            id: "ssh:bad".to_string(),
            label: "Bad".to_string(),
            host: String::new(),
            port: 22,
            username: "ubuntu".to_string(),
            auth_method: "key".to_string(),
            key_path: None,
            password: None,
            passphrase: None,
        };
        let result = SshSession::connect(&cfg).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connect_rejects_password_mode_without_password() {
        let cfg = SshHostConfig {
            id: "ssh:badpwd".to_string(),
            label: "BadPwd".to_string(),
            host: "127.0.0.1".to_string(),
            port: 22,
            username: "ubuntu".to_string(),
            auth_method: "password".to_string(),
            key_path: None,
            password: None,
            passphrase: None,
        };
        let result = SshSession::connect(&cfg).await;
        assert!(result.is_err());
        assert!(result
            .err()
            .map(|e| e.to_string().contains("password"))
            .unwrap_or(false));
    }

    #[test]
    fn resolve_target_uses_explicit_values() {
        let cfg = SshHostConfig {
            id: "ssh:test".to_string(),
            label: "Test".to_string(),
            host: "example.com".to_string(),
            port: 2022,
            username: "alice".to_string(),
            auth_method: "key".to_string(),
            key_path: Some("~/.ssh/id_test".to_string()),
            password: None,
            passphrase: None,
        };
        let resolved = resolve_target(&cfg).expect("resolve");
        assert_eq!(resolved.host, "example.com");
        assert_eq!(resolved.port, 2022);
        assert_eq!(resolved.username, "alice");
        assert_eq!(resolved.key_path.as_deref(), Some("~/.ssh/id_test"));
    }

    #[test]
    fn legacy_args_include_timeout_and_keepalive() {
        let cfg = SshHostConfig {
            id: "ssh:test".to_string(),
            label: "Test".to_string(),
            host: "example.com".to_string(),
            port: 22,
            username: "alice".to_string(),
            auth_method: "key".to_string(),
            key_path: None,
            password: None,
            passphrase: None,
        };
        let session = SshSession {
            config: cfg,
            backend: Backend::Legacy,
        };
        let args = session.legacy_common_ssh_args();
        let joined = args.join(" ");
        assert!(joined.contains("ConnectTimeout="));
        assert!(joined.contains("BatchMode=yes"));
        assert!(joined.contains("ServerAliveInterval="));
        assert!(joined.contains("ServerAliveCountMax="));
    }

    #[test]
    fn legacy_args_include_identity_file_when_set() {
        let cfg = SshHostConfig {
            id: "ssh:test".to_string(),
            label: "Test".to_string(),
            host: "example.com".to_string(),
            port: 22,
            username: "alice".to_string(),
            auth_method: "key".to_string(),
            key_path: Some("~/.ssh/my_key".to_string()),
            password: None,
            passphrase: None,
        };
        let session = SshSession {
            config: cfg,
            backend: Backend::Legacy,
        };
        let args = session.legacy_common_ssh_args();
        assert!(args.contains(&"-i".to_string()));
        assert!(args.contains(&"~/.ssh/my_key".to_string()));
    }

    #[test]
    fn legacy_args_omit_identity_file_when_empty() {
        let cfg = SshHostConfig {
            id: "ssh:test".to_string(),
            label: "Test".to_string(),
            host: "example.com".to_string(),
            port: 22,
            username: "alice".to_string(),
            auth_method: "key".to_string(),
            key_path: Some("   ".to_string()),
            password: None,
            passphrase: None,
        };
        let session = SshSession {
            config: cfg,
            backend: Backend::Legacy,
        };
        let args = session.legacy_common_ssh_args();
        assert!(!args.contains(&"-i".to_string()));
    }

    #[test]
    fn legacy_destination_with_username() {
        let cfg = SshHostConfig {
            id: "ssh:test".to_string(),
            label: "Test".to_string(),
            host: "myhost.com".to_string(),
            port: 22,
            username: "deploy".to_string(),
            auth_method: "key".to_string(),
            key_path: None,
            password: None,
            passphrase: None,
        };
        let session = SshSession {
            config: cfg,
            backend: Backend::Legacy,
        };
        assert_eq!(session.legacy_destination(), "deploy@myhost.com");
    }

    #[test]
    fn legacy_destination_without_username() {
        let cfg = SshHostConfig {
            id: "ssh:test".to_string(),
            label: "Test".to_string(),
            host: "myhost.com".to_string(),
            port: 22,
            username: String::new(),
            auth_method: "key".to_string(),
            key_path: None,
            password: None,
            passphrase: None,
        };
        let session = SshSession {
            config: cfg,
            backend: Backend::Legacy,
        };
        assert_eq!(session.legacy_destination(), "myhost.com");
    }

    #[test]
    fn legacy_destination_whitespace_username() {
        let cfg = SshHostConfig {
            id: "ssh:test".to_string(),
            label: "Test".to_string(),
            host: "myhost.com".to_string(),
            port: 22,
            username: "   ".to_string(),
            auth_method: "key".to_string(),
            key_path: None,
            password: None,
            passphrase: None,
        };
        let session = SshSession {
            config: cfg,
            backend: Backend::Legacy,
        };
        // trim().is_empty() → falls through to just host
        assert_eq!(session.legacy_destination(), "myhost.com");
    }

    #[test]
    fn legacy_args_use_custom_port() {
        let cfg = SshHostConfig {
            id: "ssh:test".to_string(),
            label: "Test".to_string(),
            host: "example.com".to_string(),
            port: 2222,
            username: "user".to_string(),
            auth_method: "key".to_string(),
            key_path: None,
            password: None,
            passphrase: None,
        };
        let session = SshSession {
            config: cfg,
            backend: Backend::Legacy,
        };
        let args = session.legacy_common_ssh_args();
        let port_idx = args.iter().position(|a| a == "-p").expect("-p flag");
        assert_eq!(args[port_idx + 1], "2222");
    }

    #[test]
    fn resolve_target_defaults_port_zero_to_22() {
        let cfg = SshHostConfig {
            id: "ssh:test".to_string(),
            label: "Test".to_string(),
            host: "example.com".to_string(),
            port: 0,
            username: "alice".to_string(),
            auth_method: "key".to_string(),
            key_path: None,
            password: None,
            passphrase: None,
        };
        let resolved = resolve_target(&cfg).expect("resolve");
        assert_eq!(resolved.port, 22);
    }

    #[test]
    fn resolve_target_falls_back_username_from_env() {
        let cfg = SshHostConfig {
            id: "ssh:test".to_string(),
            label: "Test".to_string(),
            host: "example.com".to_string(),
            port: 22,
            username: String::new(),
            auth_method: "key".to_string(),
            key_path: None,
            password: None,
            passphrase: None,
        };
        let resolved = resolve_target(&cfg).expect("resolve");
        // Should fall back to $USER / $USERNAME / "root" — just ensure it's not empty
        assert!(!resolved.username.is_empty());
    }

    #[test]
    fn shell_escape_handles_simple_string() {
        assert_eq!(shell_escape("hello"), "'hello'");
    }

    #[test]
    fn shell_escape_handles_single_quotes() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn shell_escape_handles_spaces_and_special() {
        assert_eq!(shell_escape("my file (1)"), "'my file (1)'");
    }

    #[test]
    fn ssh_error_display_messages() {
        let e = SshError::Connect("timeout".to_string());
        assert!(e.to_string().contains("timeout"));

        let e = SshError::Auth("bad key".to_string());
        assert!(e.to_string().contains("bad key"));

        let e = SshError::Channel("closed".to_string());
        assert!(e.to_string().contains("closed"));

        let e = SshError::CommandFailed("exit 1".to_string());
        assert!(e.to_string().contains("exit 1"));

        let e = SshError::InvalidConfig("empty".to_string());
        assert!(e.to_string().contains("empty"));

        let e = SshError::Sftp("io error".to_string());
        assert!(e.to_string().contains("io error"));
    }

    #[test]
    fn candidate_key_paths_returns_explicit_path_only() {
        let target = ResolvedTarget {
            host: "example.com".to_string(),
            port: 22,
            username: "alice".to_string(),
            key_path: Some("/custom/key".to_string()),
        };
        let paths = candidate_key_paths(&target);
        assert_eq!(paths, vec!["/custom/key"]);
    }

    #[test]
    fn candidate_key_paths_returns_defaults_when_no_explicit() {
        let target = ResolvedTarget {
            host: "example.com".to_string(),
            port: 22,
            username: "alice".to_string(),
            key_path: None,
        };
        let paths = candidate_key_paths(&target);
        // Should include id_ed25519 and id_rsa at minimum (may be empty if no home dir)
        for p in &paths {
            assert!(p.contains("id_ed25519") || p.contains("id_rsa"));
        }
    }

    #[test]
    fn russh_exec_timeout_secs_uses_default_without_env_override() {
        assert_eq!(
            russh_exec_timeout_secs_from_env_var(None),
            RUSSH_EXEC_TIMEOUT_SECS
        );
        assert_eq!(
            russh_exec_timeout_secs_from_env_var(Some(String::new())),
            RUSSH_EXEC_TIMEOUT_SECS
        );
        assert_eq!(
            russh_exec_timeout_secs_from_env_var(Some("not-a-number".into())),
            RUSSH_EXEC_TIMEOUT_SECS
        );
    }

    #[test]
    fn russh_exec_timeout_secs_accepts_positive_env_override() {
        assert_eq!(russh_exec_timeout_secs_from_env_var(Some("60".into())), 60);
        assert_eq!(russh_exec_timeout_secs_from_env_var(Some("5".into())), 5);
    }
}
