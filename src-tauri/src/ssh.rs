use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use russh::client;
use russh::keys::key;
use russh::{ChannelMsg, Disconnect};
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Data types
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
// Client handler (accepts all host keys for now)
// ---------------------------------------------------------------------------

struct SshHandler;

#[async_trait]
impl client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // TODO (Phase 3): verify against known_hosts
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Connection wrapper
// ---------------------------------------------------------------------------

/// Holds a live SSH session handle plus resolved remote home directory
/// and the original config for automatic reconnection.
struct SshConnection {
    handle: Arc<client::Handle<SshHandler>>,
    home_dir: String,
    config: SshHostConfig,
}

// ---------------------------------------------------------------------------
// Connection pool
// ---------------------------------------------------------------------------

/// A global pool of SSH connections keyed by instance ID.
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

    /// Establish an SSH connection for the given host config and store it in
    /// the pool under `config.id`.
    pub async fn connect(&self, config: &SshHostConfig) -> Result<(), String> {
        // Resolve connection params from ~/.ssh/config when using ssh_config mode
        let ssh_entry = if config.auth_method == "ssh_config" {
            parse_ssh_config(&config.host)
        } else {
            None
        };

        let connect_host = ssh_entry
            .as_ref()
            .and_then(|e| e.hostname.as_deref())
            .unwrap_or(&config.host);
        let connect_port = ssh_entry
            .as_ref()
            .and_then(|e| e.port)
            .unwrap_or(config.port);

        let ssh_config = Arc::new(client::Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(300)),
            keepalive_interval: Some(std::time::Duration::from_secs(30)),
            keepalive_max: 5,
            ..<_>::default()
        });

        let addr = (connect_host, connect_port);
        let handler = SshHandler;

        let mut session = client::connect(ssh_config, addr, handler)
            .await
            .map_err(|e| format!("SSH connect failed: {e}"))?;

        // Resolve username: UI value > ssh config value > system user
        let username = if !config.username.is_empty() {
            config.username.clone()
        } else if let Some(ref entry) = ssh_entry {
            entry.user.clone().unwrap_or_else(|| {
                std::env::var("USER")
                    .or_else(|_| std::env::var("USERNAME"))
                    .unwrap_or_else(|_| "root".to_string())
            })
        } else {
            std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_else(|_| "root".to_string())
        };

        // Authenticate
        let authenticated = match config.auth_method.as_str() {
            "key" => {
                let key_path = config
                    .key_path
                    .as_deref()
                    .unwrap_or("~/.ssh/id_rsa");
                let expanded = shellexpand::tilde(key_path).to_string();
                let key_pair = russh::keys::load_secret_key(&expanded, None)
                    .map_err(|e| format!("Failed to load SSH key {expanded}: {e}"))?;
                session
                    .authenticate_publickey(&username, Arc::new(key_pair))
                    .await
                    .map_err(|e| format!("Public key auth failed: {e}"))?
            }
            "ssh_config" => {
                // Try IdentityFile from ~/.ssh/config, then default key paths, then agent
                let identity_file = parse_ssh_config_identity(&config.host);

                // Build list of key paths to try
                let mut key_paths: Vec<String> = Vec::new();
                if let Some(ref p) = identity_file {
                    key_paths.push(shellexpand::tilde(p).to_string());
                }
                // Default key paths (same order as OpenSSH)
                let home = shellexpand::tilde("~").to_string();
                for name in ["id_ed25519", "id_rsa", "id_ecdsa"] {
                    let p = format!("{home}/.ssh/{name}");
                    if !key_paths.contains(&p) {
                        key_paths.push(p);
                    }
                }

                let mut authenticated = false;
                for path in &key_paths {
                    if let Ok(key_pair) = russh::keys::load_secret_key(path, None) {
                        match session
                            .authenticate_publickey(&username, Arc::new(key_pair))
                            .await
                        {
                            Ok(true) => { authenticated = true; break; }
                            _ => continue,
                        }
                    }
                }

                if !authenticated {
                    // All key files failed, try SSH agent as last resort
                    self.authenticate_with_agent(&mut session, &username).await?
                } else {
                    true
                }
            }
            "password" => {
                let password = config.password.as_deref()
                    .or(config.key_path.as_deref()) // backwards compat
                    .unwrap_or("");
                session
                    .authenticate_password(&username, password)
                    .await
                    .map_err(|e| format!("Password auth failed: {e}"))?
            }
            other => return Err(format!("Unknown auth_method: {other}")),
        };

        if !authenticated {
            return Err("SSH authentication failed (rejected by server)".into());
        }

        // Resolve remote $HOME so we can build absolute SFTP paths
        let home_dir = self.resolve_home(&session).await.unwrap_or_else(|_| "/root".to_string());

        let mut pool = self.connections.lock().await;
        pool.insert(
            config.id.clone(),
            SshConnection { handle: Arc::new(session), home_dir, config: config.clone() },
        );
        Ok(())
    }

    /// Try all keys offered by the ssh-agent until one succeeds.
    /// Only available on Unix (SSH agent uses a Unix domain socket).
    #[cfg(unix)]
    async fn authenticate_with_agent(
        &self,
        session: &mut client::Handle<SshHandler>,
        username: &str,
    ) -> Result<bool, String> {
        let mut agent = match russh::keys::agent::client::AgentClient::connect_env().await {
            Ok(a) => a,
            Err(_) => {
                // GUI apps may not have SSH_AUTH_SOCK; ask launchd for the socket path
                let output = tokio::task::spawn_blocking(|| {
                    std::process::Command::new("launchctl")
                        .args(["getenv", "SSH_AUTH_SOCK"])
                        .output()
                }).await
                    .map_err(|e| format!("Could not determine SSH agent socket: {e}"))?
                    .map_err(|e| format!("Could not determine SSH agent socket: {e}"))?;
                let sock_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if sock_path.is_empty() {
                    return Err("Could not connect to SSH agent: SSH_AUTH_SOCK not set and launchctl lookup failed".into());
                }
                russh::keys::agent::client::AgentClient::connect(
                    tokio::net::UnixStream::connect(&sock_path)
                        .await
                        .map_err(|e| format!("Could not connect to SSH agent at {sock_path}: {e}"))?
                )
            }
        };

        let identities = agent
            .request_identities()
            .await
            .map_err(|e| format!("Failed to list agent identities: {e}"))?;

        if identities.is_empty() {
            return Err("SSH agent has no identities loaded".into());
        }

        for identity in &identities {
            let (returned_agent, auth_result) = session
                .authenticate_future(username, identity.clone(), agent)
                .await;
            agent = returned_agent;
            match auth_result {
                Ok(true) => return Ok(true),
                Ok(false) => continue,
                Err(e) => {
                    // Log but try next key
                    eprintln!("Agent auth attempt failed: {e:?}");
                    continue;
                }
            }
        }

        Ok(false)
    }

    #[cfg(not(unix))]
    async fn authenticate_with_agent(
        &self,
        _session: &mut client::Handle<SshHandler>,
        _username: &str,
    ) -> Result<bool, String> {
        Err("SSH agent forwarding is not supported on Windows".into())
    }

    // -- resolve_home -----------------------------------------------------

    /// Run `echo $HOME` over a fresh channel to discover the remote home dir.
    async fn resolve_home(&self, handle: &client::Handle<SshHandler>) -> Result<String, String> {
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| format!("Failed to open channel for home resolve: {e}"))?;
        channel.exec(true, "echo $HOME").await.map_err(|e| format!("exec echo $HOME: {e}"))?;

        let mut stdout = Vec::new();
        loop {
            let Some(msg) = channel.wait().await else { break };
            if let ChannelMsg::Data { ref data } = msg {
                stdout.extend_from_slice(data);
            }
        }
        let home = String::from_utf8_lossy(&stdout).trim().to_string();
        if home.is_empty() {
            Err("Could not resolve remote $HOME".into())
        } else {
            Ok(home)
        }
    }

    /// Return the cached home directory for a connection.
    pub async fn get_home_dir(&self, id: &str) -> Result<String, String> {
        let pool = self.connections.lock().await;
        let conn = pool.get(id).ok_or_else(|| format!("No connection for id: {id}"))?;
        Ok(conn.home_dir.clone())
    }

    // -- disconnect -------------------------------------------------------

    /// Close and remove the connection for the given instance ID.
    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        let mut pool = self.connections.lock().await;
        if let Some(conn) = pool.remove(id) {
            conn.handle
                .disconnect(Disconnect::ByApplication, "", "")
                .await
                .map_err(|e| format!("SSH disconnect failed: {e}"))?;
        }
        Ok(())
    }

    // -- is_connected -----------------------------------------------------

    /// Check whether a connection exists (and the underlying handle is not
    /// closed) for the given instance ID.
    pub async fn is_connected(&self, id: &str) -> bool {
        let pool = self.connections.lock().await;
        match pool.get(id) {
            Some(conn) => !conn.handle.is_closed(),
            None => false,
        }
    }

    // -- ensure_alive / reconnect -----------------------------------------

    /// Check if the connection is alive. If stale, attempt to reconnect
    /// automatically using the stored config. Returns an error only if
    /// reconnection also fails.
    async fn ensure_alive(&self, id: &str) -> Result<(), String> {
        // Atomically check liveness and remove stale entry in one lock acquisition
        // to prevent TOCTOU races where two callers both try to reconnect.
        let stale_config = {
            let mut pool = self.connections.lock().await;
            match pool.get(id) {
                Some(conn) if conn.handle.is_closed() => {
                    // Remove stale entry while we still hold the lock
                    let conn = pool.remove(id).unwrap();
                    Some(conn.config)
                }
                Some(_) => None, // connection is alive
                None => return Err(format!("No connection for id: {id}")),
            }
        };

        if let Some(config) = stale_config {
            eprintln!("[ssh] Connection {id} is stale, attempting reconnect...");
            self.connect(&config).await.map_err(|e| {
                format!("Connection lost and reconnect failed: {e}")
            })?;
            eprintln!("[ssh] Reconnected {id} successfully");
        }
        Ok(())
    }

    // -- exec -------------------------------------------------------------

    /// Execute a command over SSH and return stdout, stderr and exit code.
    pub async fn exec(&self, id: &str, command: &str) -> Result<SshExecResult, String> {
        self.ensure_alive(id).await?;

        // Clone the handle so we don't hold the pool lock across network .await
        let handle = {
            let pool = self.connections.lock().await;
            let conn = pool.get(id).ok_or_else(|| format!("No connection for id: {id}"))?;
            Arc::clone(&conn.handle)
        };

        let mut channel = match handle.channel_open_session().await {
            Ok(ch) => ch,
            Err(e) => {
                // Channel open failed even after ensure_alive — connection
                // may have died between the check and now. Remove stale entry.
                let mut pool = self.connections.lock().await;
                pool.remove(id);
                return Err(format!("Failed to open channel: {e}"));
            }
        };

        channel
            .exec(true, command)
            .await
            .map_err(|e| format!("Failed to exec command: {e}"))?;

        let mut stdout_bytes: Vec<u8> = Vec::new();
        let mut stderr_bytes: Vec<u8> = Vec::new();
        let mut exit_code: u32 = 1; // default to failure

        loop {
            let Some(msg) = channel.wait().await else {
                break;
            };
            match msg {
                ChannelMsg::Data { ref data } => {
                    stdout_bytes.extend_from_slice(data);
                }
                ChannelMsg::ExtendedData { ref data, ext } => {
                    if ext == 1 {
                        // stderr
                        stderr_bytes.extend_from_slice(data);
                    }
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    exit_code = exit_status;
                }
                _ => {}
            }
        }

        Ok(SshExecResult {
            stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
            stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
            exit_code,
        })
    }

    /// Execute a command with login shell setup (sources profile for PATH).
    /// Also probes common Node version manager paths for Linux compatibility.
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

    /// Resolve a remote path, replacing leading `~/` with the cached home dir.
    pub async fn resolve_path(&self, id: &str, path: &str) -> Result<String, String> {
        if path.starts_with("~/") || path == "~" {
            let home = self.get_home_dir(id).await?;
            Ok(path.replacen('~', &home, 1))
        } else {
            Ok(path.to_string())
        }
    }

    // -- SFTP helpers (private) -------------------------------------------

    /// Open an SFTP session on the given connection. The caller is responsible
    /// for calling `sftp.close()` when done.
    async fn open_sftp(&self, id: &str) -> Result<SftpSession, String> {
        self.ensure_alive(id).await?;

        // Arc-clone the handle so we don't hold the pool lock across network .await
        let handle = {
            let pool = self.connections.lock().await;
            let conn = pool.get(id).ok_or_else(|| format!("No connection for id: {id}"))?;
            Arc::clone(&conn.handle)
        };

        let channel = match handle.channel_open_session().await {
            Ok(ch) => ch,
            Err(e) => {
                let mut pool = self.connections.lock().await;
                pool.remove(id);
                return Err(format!("Failed to open SFTP channel: {e}"));
            }
        };

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| format!("Failed to request SFTP subsystem: {e}"))?;

        let sftp = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            SftpSession::new(channel.into_stream()),
        )
        .await
        .map_err(|_| "SFTP session initialization timed out (15s)".to_string())?
        .map_err(|e| format!("Failed to initialize SFTP session: {e}"))?;

        Ok(sftp)
    }

    // -- sftp_read --------------------------------------------------------

    /// Read a remote file and return its contents as a String.
    pub async fn sftp_read(&self, id: &str, path: &str) -> Result<String, String> {
        let resolved = self.resolve_path(id, path).await?;
        let sftp = self.open_sftp(id).await?;
        let result = sftp.read(&resolved).await;
        if let Err(e) = sftp.close().await {
            eprintln!("[ssh] SFTP close error (non-fatal): {e}");
        }
        let data = result.map_err(|e| format!("SFTP read failed for {resolved}: {e}"))?;
        String::from_utf8(data).map_err(|e| format!("File is not valid UTF-8: {e}"))
    }

    // -- sftp_write -------------------------------------------------------

    /// Write a String to a remote file (creates or truncates).
    pub async fn sftp_write(&self, id: &str, path: &str, content: &str) -> Result<(), String> {
        let resolved = self.resolve_path(id, path).await?;
        let sftp = self.open_sftp(id).await?;

        let result = async {
            let mut file = sftp
                .create(&resolved)
                .await
                .map_err(|e| format!("SFTP create failed for {resolved}: {e}"))?;

            use tokio::io::AsyncWriteExt;
            file.write_all(content.as_bytes())
                .await
                .map_err(|e| format!("SFTP write failed for {resolved}: {e}"))?;
            file.flush()
                .await
                .map_err(|e| format!("SFTP flush failed for {resolved}: {e}"))?;
            file.shutdown()
                .await
                .map_err(|e| format!("SFTP shutdown failed for {resolved}: {e}"))?;
            Ok::<(), String>(())
        }
        .await;

        if let Err(e) = sftp.close().await {
            eprintln!("[ssh] SFTP close error (non-fatal): {e}");
        }
        result
    }

    // -- sftp_list --------------------------------------------------------

    /// List the entries in a remote directory.
    pub async fn sftp_list(&self, id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        let resolved = self.resolve_path(id, path).await?;
        let sftp = self.open_sftp(id).await?;
        let result = sftp.read_dir(&resolved).await;
        if let Err(e) = sftp.close().await {
            eprintln!("[ssh] SFTP close error (non-fatal): {e}");
        }
        let read_dir = result.map_err(|e| format!("SFTP read_dir failed for {resolved}: {e}"))?;

        Ok(read_dir
            .map(|entry| {
                let metadata = entry.metadata();
                SftpEntry {
                    name: entry.file_name(),
                    is_dir: metadata.is_dir(),
                    size: metadata.size.unwrap_or(0),
                }
            })
            .collect())
    }

    // -- sftp_remove ------------------------------------------------------

    /// Delete a remote file.
    pub async fn sftp_remove(&self, id: &str, path: &str) -> Result<(), String> {
        let resolved = self.resolve_path(id, path).await?;
        let sftp = self.open_sftp(id).await?;
        let result = sftp.remove_file(&resolved).await;
        if let Err(e) = sftp.close().await {
            eprintln!("[ssh] SFTP close error (non-fatal): {e}");
        }
        result.map_err(|e| format!("SFTP remove failed for {resolved}: {e}"))
    }
}

impl Default for SshConnectionPool {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// SSH config parser
// ---------------------------------------------------------------------------

/// Parsed fields from an SSH config Host block.
struct SshConfigEntry {
    hostname: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    identity_file: Option<String>,
}

/// Parse `~/.ssh/config` and return the resolved entry for a given host alias.
fn parse_ssh_config(host_alias: &str) -> Option<SshConfigEntry> {
    let home = dirs::home_dir()?;
    let config_path = home.join(".ssh").join("config");
    let content = std::fs::read_to_string(&config_path).ok()?;

    let mut current_hosts: Vec<String> = Vec::new();
    let mut entries: Vec<(Vec<String>, SshConfigEntry)> = Vec::new();
    let mut entry = SshConfigEntry {
        hostname: None,
        user: None,
        port: None,
        identity_file: None,
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Split on first whitespace or '='
        let (key, value) = match trimmed.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };

        if key.eq_ignore_ascii_case("Host") {
            // Save previous block
            if !current_hosts.is_empty() {
                entries.push((current_hosts, entry));
            }
            current_hosts = value.split_whitespace().map(|s| s.to_string()).collect();
            entry = SshConfigEntry {
                hostname: None,
                user: None,
                port: None,
                identity_file: None,
            };
        } else if key.eq_ignore_ascii_case("HostName") {
            entry.hostname = Some(value.to_string());
        } else if key.eq_ignore_ascii_case("User") {
            entry.user = Some(value.to_string());
        } else if key.eq_ignore_ascii_case("Port") {
            entry.port = value.parse().ok();
        } else if key.eq_ignore_ascii_case("IdentityFile") {
            entry.identity_file = Some(value.to_string());
        }
    }
    // Save last block
    if !current_hosts.is_empty() {
        entries.push((current_hosts, entry));
    }

    // Find matching entry (exact match, no glob support for now)
    for (hosts, e) in entries {
        if hosts.iter().any(|h| h == host_alias || h == "*") {
            return Some(e);
        }
    }
    None
}

/// Convenience: extract just the IdentityFile for a host.
fn parse_ssh_config_identity(host_alias: &str) -> Option<String> {
    parse_ssh_config(host_alias).and_then(|e| e.identity_file)
}
