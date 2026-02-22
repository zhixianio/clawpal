use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

use crate::node_client::NodeClient;
use crate::bridge_client::BridgeClient;
use crate::models::resolve_paths;
use crate::ssh::SshConnectionPool;

#[tauri::command]
pub async fn doctor_connect(
    client: State<'_, NodeClient>,
    app: AppHandle,
    url: String,
) -> Result<(), String> {
    client.connect(&url, app).await
}

#[tauri::command]
pub async fn doctor_disconnect(
    client: State<'_, NodeClient>,
    bridge: State<'_, BridgeClient>,
) -> Result<(), String> {
    let _ = bridge.disconnect().await;
    client.disconnect().await
}

#[tauri::command]
pub async fn doctor_bridge_connect(
    bridge: State<'_, BridgeClient>,
    app: AppHandle,
    addr: String,
) -> Result<(), String> {
    bridge.connect(&addr, app).await
}

#[tauri::command]
pub async fn doctor_bridge_disconnect(
    bridge: State<'_, BridgeClient>,
) -> Result<(), String> {
    bridge.disconnect().await
}

#[tauri::command]
pub async fn doctor_start_diagnosis(
    client: State<'_, NodeClient>,
    context: String,
) -> Result<(), String> {
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    // Fire-and-forget: results arrive via streaming chat events
    client.send_request_fire("agent", json!({
        "message": context,
        "idempotencyKey": idempotency_key,
        "agentId": "main",
        "sessionKey": "agent:main:clawpal-doctor",
    })).await
}

#[tauri::command]
pub async fn doctor_send_message(
    client: State<'_, NodeClient>,
    message: String,
) -> Result<(), String> {
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    // Fire-and-forget: results arrive via streaming chat events
    client.send_request_fire("agent", json!({
        "message": message,
        "idempotencyKey": idempotency_key,
        "agentId": "main",
        "sessionKey": "agent:main:clawpal-doctor",
    })).await
}

#[tauri::command]
pub async fn doctor_approve_invoke(
    client: State<'_, NodeClient>,
    bridge: State<'_, BridgeClient>,
    pool: State<'_, SshConnectionPool>,
    app: AppHandle,
    invoke_id: String,
    target: String,
) -> Result<Value, String> {
    // Try bridge first (invokes come from bridge in dual-connection mode)
    // Fall back to operator client (for operator-only mode)
    let invoke = match bridge.take_invoke(&invoke_id).await {
        Some(inv) => inv,
        None => client.take_invoke(&invoke_id).await
            .ok_or_else(|| format!("No pending invoke with id: {invoke_id}"))?,
    };

    let command = invoke.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let args = invoke.get("args").cloned().unwrap_or(Value::Null);

    // Route to local or remote execution
    let result = if target == "local" {
        execute_local_command(command, &args).await?
    } else {
        execute_remote_command(&pool, &target, command, &args).await?
    };

    // Send result back via bridge if connected, otherwise via operator
    if bridge.is_connected().await {
        bridge.send_invoke_result(&invoke_id, result.clone()).await?;
    } else {
        client.send_response(&invoke_id, result.clone()).await?;
    }

    let _ = app.emit("doctor:invoke-result", json!({
        "id": invoke_id,
        "result": result,
    }));

    Ok(result)
}

#[tauri::command]
pub async fn doctor_reject_invoke(
    client: State<'_, NodeClient>,
    bridge: State<'_, BridgeClient>,
    invoke_id: String,
    reason: String,
) -> Result<(), String> {
    // Remove from whichever client holds it
    let _invoke = match bridge.take_invoke(&invoke_id).await {
        Some(inv) => inv,
        None => client.take_invoke(&invoke_id).await
            .ok_or_else(|| format!("No pending invoke with id: {invoke_id}"))?,
    };

    if bridge.is_connected().await {
        bridge.send_invoke_error(&invoke_id, "REJECTED", &format!("Rejected by user: {reason}")).await
    } else {
        client.send_error_response(&invoke_id, &format!("Rejected by user: {reason}")).await
    }
}

#[tauri::command]
pub async fn collect_doctor_context() -> Result<String, String> {
    let paths = resolve_paths();

    let config_content = std::fs::read_to_string(&paths.config_path)
        .unwrap_or_else(|_| "(unable to read config)".into());

    let doctor_report = crate::doctor::run_doctor(&paths);

    let version = crate::cli_runner::run_openclaw(&["--version"])
        .map(|o| o.stdout)
        .unwrap_or_else(|_| "unknown".into());

    // Collect recent error log
    let error_log = crate::logging::read_log_tail("error.log", 100)
        .unwrap_or_default();

    let context = json!({
        "openclawVersion": version.trim(),
        "configPath": paths.config_path.to_string_lossy(),
        "configContent": config_content,
        "doctorReport": {
            "ok": doctor_report.ok,
            "score": doctor_report.score,
            "issues": doctor_report.issues.iter().map(|i| json!({
                "id": i.id,
                "severity": i.severity,
                "message": i.message,
            })).collect::<Vec<_>>(),
        },
        "errorLog": error_log,
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    });

    serde_json::to_string(&context).map_err(|e| format!("Failed to serialize context: {e}"))
}

#[tauri::command]
pub async fn collect_doctor_context_remote(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<String, String> {
    // Collect openclaw version
    let version_result = pool.exec_login(&host_id, "openclaw --version 2>/dev/null || echo unknown").await?;
    let version = version_result.stdout.trim().to_string();

    // Collect config path and content
    let config_path_result = pool.exec_login(&host_id, "openclaw config-path 2>/dev/null || echo ~/.config/openclaw/openclaw.json").await?;
    let config_path = config_path_result.stdout.trim().to_string();
    validate_not_sensitive(&config_path)?;
    let config_content = pool.sftp_read(&host_id, &config_path).await
        .unwrap_or_else(|_| "(unable to read remote config)".into());

    // Run doctor on remote
    let doctor_result = pool.exec_login(&host_id, "openclaw doctor --json 2>/dev/null").await?;
    let doctor_report: Value = serde_json::from_str(&doctor_result.stdout)
        .unwrap_or_else(|_| json!({
            "ok": false,
            "error": "Failed to parse doctor output",
            "raw": doctor_result.stdout.trim(),
        }));

    // Collect recent error log
    let error_log_result = pool.exec(&host_id, "tail -100 ~/.config/openclaw/error.log 2>/dev/null || echo ''").await?;
    let error_log = error_log_result.stdout;

    // System info
    let platform_result = pool.exec(&host_id, "uname -s").await?;
    let arch_result = pool.exec(&host_id, "uname -m").await?;

    let context = json!({
        "openclawVersion": version,
        "configPath": config_path,
        "configContent": config_content,
        "doctorReport": doctor_report,
        "errorLog": error_log,
        "platform": platform_result.stdout.trim().to_lowercase(),
        "arch": arch_result.stdout.trim(),
        "remote": true,
        "hostId": host_id,
    });

    serde_json::to_string(&context).map_err(|e| format!("Failed to serialize context: {e}"))
}

/// Sensitive paths that are ALWAYS blocked for both read and write.
/// Checked after tilde expansion, before any other path validation.
const SENSITIVE_PATH_PATTERNS: &[&str] = &[
    "/.ssh/",
    "/.ssh",
    "/.gnupg/",
    "/.gnupg",
    "/.aws/",
    "/.aws",
    "/.config/gcloud/",
    "/.azure/",
    "/.kube/config",
    "/.docker/config.json",
    "/.netrc",
    "/.npmrc",
    "/.env",
    "/.bash_history",
    "/.zsh_history",
    "/etc/shadow",
    "/etc/sudoers",
];

fn validate_not_sensitive(path: &str) -> Result<(), String> {
    let expanded = shellexpand::tilde(path).to_string();
    for pattern in SENSITIVE_PATH_PATTERNS {
        if expanded.contains(pattern) {
            return Err(format!(
                "Access to {path} is blocked — matches sensitive path pattern: {pattern}"
            ));
        }
    }
    Ok(())
}

/// Allowed directories for read_file / list_files (auto-executed without user approval).
/// Paths are canonicalized and must start with one of these prefixes.
fn allowed_read_dirs() -> Vec<std::path::PathBuf> {
    let paths = resolve_paths();
    let mut dirs = vec![
        paths.openclaw_dir.clone(),
        paths.clawpal_dir.clone(),
        paths.config_path.parent().unwrap_or(&paths.openclaw_dir).to_path_buf(),
    ];
    // Also allow /etc/openclaw for system-wide config
    let etc_openclaw = std::path::PathBuf::from("/etc/openclaw");
    if etc_openclaw.exists() {
        dirs.push(etc_openclaw);
    }
    dirs
}

/// Check that a resolved, canonicalized path falls within allowed directories.
fn validate_read_path(path: &str) -> Result<std::path::PathBuf, String> {
    validate_not_sensitive(path)?;
    let expanded = shellexpand::tilde(path).to_string();
    let canonical = std::fs::canonicalize(&expanded)
        .map_err(|e| format!("Cannot resolve path {path}: {e}"))?;
    let allowed = allowed_read_dirs();
    for dir in &allowed {
        if let Ok(canon_dir) = std::fs::canonicalize(dir) {
            if canonical.starts_with(&canon_dir) {
                return Ok(canonical);
            }
        }
    }
    Err(format!(
        "Path {path} is outside allowed directories. Reads are restricted to openclaw config and data directories."
    ))
}

/// Validate write path — must be within openclaw directories.
fn validate_write_path(path: &str) -> Result<std::path::PathBuf, String> {
    validate_not_sensitive(path)?;
    let expanded = shellexpand::tilde(path).to_string();
    let target = std::path::PathBuf::from(&expanded);
    // For writes, the file may not exist yet, so check the parent directory
    let parent = target.parent()
        .ok_or_else(|| format!("Invalid path: {path}"))?;
    let canon_parent = std::fs::canonicalize(parent)
        .map_err(|e| format!("Cannot resolve parent directory of {path}: {e}"))?;
    let allowed = allowed_read_dirs();
    for dir in &allowed {
        if let Ok(canon_dir) = std::fs::canonicalize(dir) {
            if canon_parent.starts_with(&canon_dir) {
                return Ok(target);
            }
        }
    }
    Err(format!(
        "Path {path} is outside allowed directories. Writes are restricted to openclaw config and data directories."
    ))
}

/// Allowed command prefixes for run_command.
const ALLOWED_COMMAND_PREFIXES: &[&str] = &[
    "openclaw ",
    "openclaw\t",
    "cat ",
    "ls ",
    "head ",
    "tail ",
    "wc ",
    "grep ",
    "find ",
    "systemctl status",
    "journalctl ",
    "ps ",
    "which ",
    "echo ",
    "date",
    "uname",
    "hostname",
    "df ",
    "free ",
    "uptime",
];

/// Maximum output size from run_command (256 KB).
const MAX_COMMAND_OUTPUT: usize = 256 * 1024;

/// Timeout for run_command (30 seconds).
const COMMAND_TIMEOUT_SECS: u64 = 30;

/// Shell metacharacters that enable command chaining / injection.
const DANGEROUS_PATTERNS: &[&str] = &[";", "|", "&&", "||", "`", "$(", ">", "<", "\n", "\r"];

fn validate_command(cmd: &str) -> Result<(), String> {
    let trimmed = cmd.trim();

    // Reject shell metacharacters that enable command chaining
    for pat in DANGEROUS_PATTERNS {
        if trimmed.contains(pat) {
            return Err(format!(
                "Command contains disallowed shell characters: {pat}"
            ));
        }
    }

    // Allow exact matches for simple commands
    let exact_allowed = ["date", "uname", "uptime", "hostname"];
    if exact_allowed.contains(&trimmed) {
        return Ok(());
    }
    for prefix in ALLOWED_COMMAND_PREFIXES {
        if trimmed.starts_with(prefix) {
            return Ok(());
        }
    }
    Err(format!(
        "Command not allowed: {trimmed}. Only openclaw, diagnostic, and read-only system commands are permitted."
    ))
}

fn truncate_output(s: &[u8]) -> String {
    let text = String::from_utf8_lossy(s);
    if text.len() > MAX_COMMAND_OUTPUT {
        let mut truncated = text[..MAX_COMMAND_OUTPUT].to_string();
        truncated.push_str("\n... (output truncated)");
        truncated
    } else {
        text.into_owned()
    }
}

/// Execute a command locally on behalf of the doctor agent.
async fn execute_local_command(command: &str, args: &Value) -> Result<Value, String> {
    match command {
        "read_file" => {
            let path = args.get("path").and_then(|v| v.as_str())
                .ok_or("read_file: missing 'path' argument")?;
            let canonical = validate_read_path(path)?;
            let content = tokio::fs::read_to_string(&canonical)
                .await
                .map_err(|e| format!("Failed to read {path}: {e}"))?;
            Ok(json!({"content": content}))
        }
        "list_files" => {
            let path = args.get("path").and_then(|v| v.as_str())
                .ok_or("list_files: missing 'path' argument")?;
            let canonical = validate_read_path(path)?;
            let mut entries = Vec::new();
            let mut dir = tokio::fs::read_dir(&canonical)
                .await
                .map_err(|e| format!("Failed to list {path}: {e}"))?;
            while let Some(entry) = dir.next_entry().await.map_err(|e| e.to_string())? {
                let meta = entry.metadata().await.map_err(|e| e.to_string())?;
                entries.push(json!({
                    "name": entry.file_name().to_string_lossy(),
                    "isDir": meta.is_dir(),
                    "size": meta.len(),
                }));
            }
            Ok(json!({"entries": entries}))
        }
        "read_config" => {
            let paths = resolve_paths();
            let content = std::fs::read_to_string(&paths.config_path)
                .map_err(|e| format!("Failed to read config: {e}"))?;
            Ok(json!({"content": content, "path": paths.config_path.to_string_lossy()}))
        }
        "system_info" => {
            let paths = resolve_paths();
            let version = crate::cli_runner::run_openclaw(&["--version"])
                .map(|o| o.stdout)
                .unwrap_or_else(|_| "unknown".into());
            Ok(json!({
                "platform": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "openclawVersion": version.trim(),
                "configPath": paths.config_path.to_string_lossy(),
                "openclawDir": paths.openclaw_dir.to_string_lossy(),
            }))
        }
        "validate_config" => {
            let paths = resolve_paths();
            let report = crate::doctor::run_doctor(&paths);
            Ok(json!({
                "ok": report.ok,
                "score": report.score,
                "issues": report.issues.iter().map(|i| json!({
                    "id": i.id,
                    "severity": i.severity,
                    "message": i.message,
                    "autoFixable": i.auto_fixable,
                })).collect::<Vec<_>>(),
            }))
        }
        "write_file" => {
            let path = args.get("path").and_then(|v| v.as_str())
                .ok_or("write_file: missing 'path' argument")?;
            let content = args.get("content").and_then(|v| v.as_str())
                .ok_or("write_file: missing 'content' argument")?;
            let validated = validate_write_path(path)?;
            // Refuse to write through symlinks to prevent escaping allowed directories
            if validated.is_symlink() {
                return Err(format!("write_file: refusing to write through symlink at {path}"));
            }
            tokio::fs::write(&validated, content)
                .await
                .map_err(|e| format!("Failed to write {path}: {e}"))?;
            Ok(json!({"ok": true}))
        }
        "run_command" => {
            let cmd = args.get("command").and_then(|v| v.as_str())
                .ok_or("run_command: missing 'command' argument")?;
            validate_command(cmd)?;
            let child = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("Failed to spawn command: {e}"))?;
            let output = tokio::time::timeout(
                std::time::Duration::from_secs(COMMAND_TIMEOUT_SECS),
                child.wait_with_output(),
            )
            .await
            .map_err(|_| format!("Command timed out after {COMMAND_TIMEOUT_SECS}s"))?
            .map_err(|e| format!("Failed to run command: {e}"))?;
            Ok(json!({
                "stdout": truncate_output(&output.stdout),
                "stderr": truncate_output(&output.stderr),
                "exitCode": output.status.code().unwrap_or(1),
            }))
        }
        _ => Err(format!("Unknown command: {command}")),
    }
}

/// Execute a command on a remote SSH host on behalf of the doctor agent.
/// Note: remote reads are not restricted to openclaw directories (unlike local reads)
/// because remote config locations vary. Security relies on the sensitive path blacklist
/// plus the frontend approval mechanism (first-time read requires user click).
async fn execute_remote_command(
    pool: &SshConnectionPool,
    host_id: &str,
    command: &str,
    args: &Value,
) -> Result<Value, String> {
    match command {
        "read_file" => {
            let path = args.get("path").and_then(|v| v.as_str())
                .ok_or("read_file: missing 'path' argument")?;
            validate_not_sensitive(path)?;
            let content = pool.sftp_read(host_id, path).await?;
            Ok(json!({"content": content}))
        }
        "list_files" => {
            let path = args.get("path").and_then(|v| v.as_str())
                .ok_or("list_files: missing 'path' argument")?;
            validate_not_sensitive(path)?;
            let entries = pool.sftp_list(host_id, path).await?;
            Ok(json!({"entries": entries.iter().map(|e| json!({
                "name": e.name,
                "isDir": e.is_dir,
                "size": e.size,
            })).collect::<Vec<_>>()}))
        }
        "read_config" => {
            let result = pool.exec_login(host_id, "openclaw config-path 2>/dev/null || echo ~/.config/openclaw/openclaw.json").await?;
            let config_path = result.stdout.trim().to_string();
            validate_not_sensitive(&config_path)?;
            let content = pool.sftp_read(host_id, &config_path).await
                .unwrap_or_else(|_| "(unable to read remote config)".into());
            Ok(json!({"content": content, "path": config_path}))
        }
        "system_info" => {
            let version_result = pool.exec_login(host_id, "openclaw --version 2>/dev/null || echo unknown").await?;
            let platform_result = pool.exec(host_id, "uname -s").await?;
            let arch_result = pool.exec(host_id, "uname -m").await?;
            let hostname_result = pool.exec(host_id, "hostname").await?;
            Ok(json!({
                "platform": platform_result.stdout.trim().to_lowercase(),
                "arch": arch_result.stdout.trim(),
                "openclawVersion": version_result.stdout.trim(),
                "hostname": hostname_result.stdout.trim(),
                "remote": true,
            }))
        }
        "validate_config" => {
            let result = pool.exec_login(host_id, "openclaw doctor --json 2>/dev/null").await?;
            if result.exit_code != 0 {
                return Ok(json!({
                    "ok": false,
                    "error": format!("openclaw doctor failed: {}", result.stderr.trim()),
                    "raw": result.stdout,
                }));
            }
            let parsed: Value = serde_json::from_str(&result.stdout)
                .unwrap_or_else(|_| json!({"raw": result.stdout.trim()}));
            Ok(parsed)
        }
        "write_file" => {
            let path = args.get("path").and_then(|v| v.as_str())
                .ok_or("write_file: missing 'path' argument")?;
            let content = args.get("content").and_then(|v| v.as_str())
                .ok_or("write_file: missing 'content' argument")?;
            validate_not_sensitive(path)?;
            // Best-effort symlink check (TOCTOU gap: file could change between check and write)
            let resolved = pool.resolve_path(host_id, path).await?;
            let stat_result = pool.exec(host_id, &format!("test -L '{}' && echo SYMLINK || echo OK", resolved.replace('\'', "'\\''"))).await?;
            if stat_result.stdout.trim() == "SYMLINK" {
                return Err(format!("write_file: refusing to write through symlink at {path}"));
            }
            pool.sftp_write(host_id, path, content).await?;
            Ok(json!({"ok": true}))
        }
        "run_command" => {
            let cmd = args.get("command").and_then(|v| v.as_str())
                .ok_or("run_command: missing 'command' argument")?;
            validate_command(cmd)?;
            let result = pool.exec(host_id, cmd).await?;
            Ok(json!({
                "stdout": truncate_output(result.stdout.as_bytes()),
                "stderr": truncate_output(result.stderr.as_bytes()),
                "exitCode": result.exit_code,
            }))
        }
        _ => Err(format!("Unknown command: {command}")),
    }
}
