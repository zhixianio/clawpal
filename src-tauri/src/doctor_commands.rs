use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

use crate::node_client::{NodeClient, GatewayCredentials};
use crate::bridge_client::{BridgeClient, extract_shell_command};
use crate::models::resolve_paths;
use crate::ssh::SshConnectionPool;

/// Create an SSH local port forward to a remote host's gateway (port 18789).
/// Returns the local port to connect to.
#[tauri::command]
pub async fn doctor_port_forward(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<u16, String> {
    pool.request_port_forward(&host_id, 18789).await
}

/// Read gateway auth token and device identity from a remote host via SSH.
/// Returns credentials needed to authenticate with that host's gateway.
#[tauri::command]
pub async fn doctor_read_remote_credentials(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<GatewayCredentials, String> {
    // Read auth token from remote config
    let config_result = pool.exec_login(&host_id,
        "cat \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}/openclaw.json\" 2>/dev/null || echo '{}'"
    ).await?;
    let token = serde_json::from_str::<Value>(config_result.stdout.trim())
        .ok()
        .and_then(|config| {
            config.get("gateway")?
                .get("auth")?
                .get("token")?
                .as_str()
                .map(|s| s.to_string())
        })
        .unwrap_or_default();

    // Read device identity
    let device_result = pool.exec_login(&host_id,
        "cat \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}/identity/device.json\" 2>/dev/null"
    ).await?;
    let device_json: Value = serde_json::from_str(device_result.stdout.trim())
        .map_err(|e| format!("Failed to parse remote device.json: {e}"))?;

    let device_id = device_json.get("deviceId")
        .and_then(|v| v.as_str())
        .ok_or("Missing deviceId in remote device.json")?
        .to_string();
    let private_key_pem = device_json.get("privateKeyPem")
        .and_then(|v| v.as_str())
        .ok_or("Missing privateKeyPem in remote device.json")?
        .to_string();

    Ok(GatewayCredentials { token, device_id, private_key_pem })
}

/// Auto-approve pending device pairing requests on a remote host.
/// When ClawPal connects to a remote gateway using the host's own device identity,
/// the gateway may require re-pairing (e.g. token rotation, repair).
/// This command SSHes into the host, lists pending requests, and approves them.
/// Returns the number of requests approved.
#[tauri::command]
pub async fn doctor_auto_pair(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<u32, String> {
    let result = pool.exec_login(&host_id, "openclaw devices list --json 2>/dev/null").await?;
    if result.exit_code != 0 {
        return Err(format!("openclaw devices list failed: {}", result.stderr.trim()));
    }
    let list: Value = serde_json::from_str(result.stdout.trim())
        .map_err(|e| format!("Failed to parse devices list: {e}"))?;

    let pending = list.get("pending").and_then(|v| v.as_array());
    let Some(pending) = pending else {
        return Ok(0);
    };

    let mut approved = 0u32;
    for req in pending {
        let request_id = req.get("requestId").and_then(|v| v.as_str()).unwrap_or("");
        if request_id.is_empty() { continue; }
        let approve_result = pool.exec_login(
            &host_id,
            &format!("openclaw devices approve {request_id} 2>&1"),
        ).await;
        if approve_result.is_ok() {
            approved += 1;
        }
    }
    Ok(approved)
}

#[tauri::command]
pub async fn doctor_connect(
    client: State<'_, NodeClient>,
    app: AppHandle,
    url: String,
    credentials: Option<GatewayCredentials>,
) -> Result<(), String> {
    client.connect(&url, app, credentials).await
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
    url: String,
    credentials: Option<GatewayCredentials>,
) -> Result<(), String> {
    bridge.connect(&url, app, credentials).await
}

#[tauri::command]
pub async fn doctor_bridge_disconnect(
    bridge: State<'_, BridgeClient>,
) -> Result<(), String> {
    bridge.disconnect().await
}

#[tauri::command]
pub async fn doctor_bridge_node_id(
    bridge: State<'_, BridgeClient>,
) -> Result<String, String> {
    bridge.node_id().await.ok_or_else(|| "Bridge not connected".into())
}

#[tauri::command]
pub async fn doctor_start_diagnosis(
    client: State<'_, NodeClient>,
    context: String,
    session_key: String,
    agent_id: String,
) -> Result<(), String> {
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    // Fire-and-forget: results arrive via streaming chat events
    client.send_request_fire("agent", json!({
        "message": context,
        "idempotencyKey": idempotency_key,
        "agentId": agent_id,
        "sessionKey": session_key,
    })).await
}

#[tauri::command]
pub async fn doctor_send_message(
    client: State<'_, NodeClient>,
    message: String,
    session_key: String,
    agent_id: String,
) -> Result<(), String> {
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    // Fire-and-forget: results arrive via streaming chat events
    client.send_request_fire("agent", json!({
        "message": message,
        "idempotencyKey": idempotency_key,
        "agentId": agent_id,
        "sessionKey": session_key,
    })).await
}

#[tauri::command]
pub async fn doctor_approve_invoke(
    bridge: State<'_, BridgeClient>,
    client: State<'_, NodeClient>,
    pool: State<'_, SshConnectionPool>,
    app: AppHandle,
    invoke_id: String,
    target: String,
    session_key: String,
    agent_id: String,
) -> Result<Value, String> {
    // Invokes come from the node connection (BridgeClient).
    // `expired` = true means the invoke was already auto-rejected with USER_PENDING
    // (gateway 30s timeout approaching), so the result must go via chat message.
    let (invoke, expired) = bridge.take_invoke(&invoke_id).await
        .ok_or_else(|| format!("No pending invoke with id: {invoke_id}"))?;

    let command = invoke.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let args = invoke.get("args").cloned().unwrap_or(Value::Null);
    // Use the gateway-assigned nodeId from the invoke request (not our hostname).
    // Mismatch here causes the gateway to ignore the result → agent sees "timeout".
    let node_id = invoke.get("nodeId").and_then(|v| v.as_str()).unwrap_or("").to_string();

    // Map standard node commands to internal execution.
    // Security: commands reach here only after user approval in the UI
    // (write → "Execute" button, read → "Allow" button).
    // User approval is the security boundary, not command validation.
    let result = match command {
        "system.run" => {
            // Gateway sends command as string or array ["/bin/sh", "-lc", "actual cmd"]
            let shell_cmd = extract_shell_command(&args);
            if shell_cmd.is_empty() {
                return Err("system.run: missing 'command' argument".into());
            }
            // Execute directly — user already approved this command.
            // Include executedOn metadata so the agent knows WHERE the command ran
            // (prevents it from claiming "command ran locally" on remote targets).
            if target == "local" {
                let mut v = run_command_local(&shell_cmd).await?;
                v["executedOn"] = json!("local");
                v
            } else {
                // If SSH fails, try reconnecting once before giving up.
                match run_command_remote(&pool, &target, &shell_cmd).await {
                    Ok(mut v) => {
                        v["executedOn"] = json!(format!("{target} (remote)"));
                        v
                    }
                    Err(e) => {
                        // Retry: reconnect SSH and try again
                        if let Ok(()) = pool.reconnect(&target).await {
                            match run_command_remote(&pool, &target, &shell_cmd).await {
                                Ok(mut v) => {
                                    v["executedOn"] = json!(format!("{target} (remote, reconnected)"));
                                    v
                                }
                                Err(e2) => json!({
                                    "stdout": "",
                                    "stderr": format!("Remote execution failed on '{target}' after reconnect: {e2}"),
                                    "exitCode": 255,
                                    "executedOn": format!("{target} (connection lost)"),
                                }),
                            }
                        } else {
                            json!({
                                "stdout": "",
                                "stderr": format!("Remote execution failed on '{target}': {e}. Ask the user to reconnect in the Instance tab."),
                                "exitCode": 255,
                                "executedOn": format!("{target} (connection lost)"),
                            })
                        }
                    }
                }
            }
        }
        // Fallback: pass through to internal handlers (for legacy/custom commands)
        _ => {
            if target == "local" {
                execute_local_command(command, &args).await?
            } else {
                execute_remote_command(&pool, &target, command, &args).await?
            }
        }
    };

    if expired {
        // Invoke was already auto-rejected with USER_PENDING — gateway discards late
        // invoke results. Send the output as a follow-up chat message instead so the
        // agent can continue with the information.
        let result_text = if let Some(stdout) = result.get("stdout").and_then(|v| v.as_str()) {
            let stderr = result.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
            let exit_code = result.get("exitCode").and_then(|v| v.as_i64()).unwrap_or(-1);
            let mut msg = format!("[User executed the previously pending command: `{command}`]\n");
            if !stdout.is_empty() {
                msg.push_str(&format!("stdout:\n```\n{stdout}\n```\n"));
            }
            if !stderr.is_empty() {
                msg.push_str(&format!("stderr:\n```\n{stderr}\n```\n"));
            }
            msg.push_str(&format!("exitCode: {exit_code}"));
            msg
        } else {
            format!("[User executed the previously pending command: `{command}`]\nResult: {result}")
        };
        let idempotency_key = uuid::Uuid::new_v4().to_string();
        let _ = client.send_request_fire("agent", json!({
            "message": result_text,
            "idempotencyKey": idempotency_key,
            "agentId": agent_id,
            "sessionKey": session_key,
        })).await;
    } else {
        // Normal path: send result back to the gateway via the node connection
        bridge.send_invoke_result(&invoke_id, &node_id, result.clone()).await?;
    }

    let _ = app.emit("doctor:invoke-result", json!({
        "id": invoke_id,
        "result": result,
    }));

    Ok(result)
}

#[tauri::command]
pub async fn doctor_reject_invoke(
    bridge: State<'_, BridgeClient>,
    invoke_id: String,
    reason: String,
) -> Result<(), String> {
    let (invoke, expired) = bridge.take_invoke(&invoke_id).await
        .ok_or_else(|| format!("No pending invoke with id: {invoke_id}"))?;
    if expired {
        // Already auto-rejected with USER_PENDING — no need to send another error
        return Ok(());
    }
    let node_id = invoke.get("nodeId").and_then(|v| v.as_str()).unwrap_or("");

    bridge.send_invoke_error(&invoke_id, node_id, "REJECTED", &format!("Rejected by user: {reason}")).await
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

    // Check if gateway process is running
    let gateway_running = std::process::Command::new("pgrep")
        .args(["-f", "openclaw-gateway"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

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
        "gatewayProcessRunning": gateway_running,
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

    // Resolve config path: check OPENCLAW_STATE_DIR / OPENCLAW_HOME, fallback to ~/.openclaw
    let config_path_result = pool.exec_login(&host_id,
        "echo \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}/openclaw.json\""
    ).await?;
    let config_path = config_path_result.stdout.trim().to_string();
    validate_not_sensitive(&config_path)?;
    let config_content = pool.sftp_read(&host_id, &config_path).await
        .unwrap_or_else(|_| "(unable to read remote config)".into());

    // Use `openclaw gateway status` — always returns useful text even when gateway is stopped.
    // `openclaw health --json` requires a running gateway + auth token and returns empty otherwise.
    let status_result = pool.exec_login(&host_id, "openclaw gateway status 2>&1").await?;
    let gateway_status = status_result.stdout.trim().to_string();

    // Check if gateway process is running (reliable even when health RPC fails)
    // Bracket trick: [o]penclaw-gateway prevents pgrep from matching its own sh -c process
    let pgrep_result = pool.exec(&host_id, "pgrep -f '[o]penclaw-gateway' >/dev/null 2>&1").await;
    let gateway_running = matches!(pgrep_result, Ok(r) if r.exit_code == 0);

    // Collect recent error log (logs live under $OPENCLAW_STATE_DIR/logs/)
    let error_log_result = pool.exec_login(&host_id,
        "tail -100 \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}/logs/gateway.err.log\" 2>/dev/null || echo ''"
    ).await?;
    let error_log = error_log_result.stdout;

    // System info
    let platform_result = pool.exec(&host_id, "uname -s").await?;
    let arch_result = pool.exec(&host_id, "uname -m").await?;

    let context = json!({
        "openclawVersion": version,
        "configPath": config_path,
        "configContent": config_content,
        "gatewayStatus": gateway_status,
        "gatewayProcessRunning": gateway_running,
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

/// Run a shell command locally (user-approved, no validate_command).
async fn run_command_local(cmd: &str) -> Result<Value, String> {
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

/// Run a shell command on a remote host via SSH (user-approved, no validate_command).
async fn run_command_remote(pool: &SshConnectionPool, host_id: &str, cmd: &str) -> Result<Value, String> {
    let result = pool.exec(host_id, cmd).await?;
    Ok(json!({
        "stdout": truncate_output(result.stdout.as_bytes()),
        "stderr": truncate_output(result.stderr.as_bytes()),
        "exitCode": result.exit_code,
    }))
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
            // Atomic write: write to temp file then rename to avoid symlink TOCTOU
            let parent = validated.parent()
                .ok_or_else(|| format!("Invalid path: {path}"))?;
            let tmp = parent.join(format!(".clawpal-tmp-{}", uuid::Uuid::new_v4()));
            tokio::fs::write(&tmp, content)
                .await
                .map_err(|e| { let _ = std::fs::remove_file(&tmp); format!("Failed to write {path}: {e}") })?;
            tokio::fs::rename(&tmp, &validated)
                .await
                .map_err(|e| { let _ = std::fs::remove_file(&tmp); format!("Failed to rename temp file to {path}: {e}") })?;
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
            let result = pool.exec_login(host_id,
                "echo \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}/openclaw.json\""
            ).await?;
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
            let result = pool.exec_login(host_id, "openclaw health --json 2>/dev/null").await?;
            if result.exit_code != 0 {
                return Ok(json!({
                    "ok": false,
                    "error": format!("openclaw health failed: {}", result.stderr.trim()),
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
            // Atomic write via temp file + mv to avoid symlink TOCTOU
            let tmp_name = format!(".clawpal-tmp-{}", uuid::Uuid::new_v4());
            let resolved = pool.resolve_path(host_id, path).await?;
            let esc = resolved.replace('\'', "'\\''");
            let parent_dir = std::path::Path::new(&resolved)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "/tmp".to_string());
            let tmp_path = format!("{}/{}", parent_dir, tmp_name);
            let tmp_esc = tmp_path.replace('\'', "'\\''");
            // Write content to temp file via SFTP
            if let Err(e) = pool.sftp_write(host_id, &tmp_path, content).await {
                let _ = pool.exec(host_id, &format!("rm -f '{tmp_esc}'")).await;
                return Err(format!("Failed to write temp file for {path}: {e}"));
            }
            // Atomic rename: mv temp -> target (overwrites without following symlinks)
            match pool.exec(host_id, &format!("mv -f '{tmp_esc}' '{esc}'")).await {
                Ok(mv_result) if mv_result.exit_code != 0 => {
                    let _ = pool.exec(host_id, &format!("rm -f '{tmp_esc}'")).await;
                    return Err(format!("Failed to rename temp file to {path}: {}", mv_result.stderr.trim()));
                }
                Err(e) => {
                    let _ = pool.exec(host_id, &format!("rm -f '{tmp_esc}'")).await;
                    return Err(format!("Failed to rename temp file to {path}: {e}"));
                }
                _ => {}
            }
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
