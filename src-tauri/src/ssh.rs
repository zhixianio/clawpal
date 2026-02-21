use std::collections::HashMap;
use std::sync::Arc;

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
    session: Arc<Session>,
    home_dir: String,
}

/// Shell-quote a string using single quotes with proper escaping.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
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
        // Match previous behavior (accept all host keys). TODO: tighten to KnownHosts::Add
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
        pool.insert(config.id.clone(), SshConnection { session: Arc::new(session), home_dir });
        Ok(())
    }

    // -- disconnect -------------------------------------------------------

    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        let conn = {
            let mut pool = self.connections.lock().await;
            pool.remove(id)
        };
        if let Some(conn) = conn {
            // Arc<Session> unwrap - if we're the last holder, close it
            if let Ok(session) = Arc::try_unwrap(conn.session) {
                session
                    .close()
                    .await
                    .map_err(|e| format!("SSH disconnect failed: {e}"))?;
            }
        }
        Ok(())
    }

    // -- is_connected -----------------------------------------------------

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

    /// Write content to a remote file via base64 encoding.
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

    /// List a remote directory via `stat`.
    pub async fn sftp_list(&self, id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        let resolved = self.resolve_path(id, path).await?;
        let quoted = shell_quote(&resolved);
        // Use stat to get name, type, and size in a parseable format.
        // --format is GNU stat (Linux). Output: full_path\tfile_type\tsize
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
