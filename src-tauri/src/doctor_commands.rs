use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, State};

use crate::doctor_runtime_bridge::emit_runtime_event;
use crate::models::resolve_paths;
use crate::runtime::types::{RuntimeAdapter, RuntimeDomain, RuntimeEvent, RuntimeSessionKey};
use crate::runtime::zeroclaw::adapter::ZeroclawDoctorAdapter;
use crate::runtime::zeroclaw::install_adapter::ZeroclawInstallAdapter;
use crate::ssh::SshConnectionPool;

fn zeroclaw_pending_invokes() -> &'static Mutex<HashMap<String, Value>> {
    static STORE: OnceLock<Mutex<HashMap<String, Value>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register_runtime_invoke(event: &RuntimeEvent) {
    if let RuntimeEvent::Invoke { payload } = event {
        if let Some(id) = payload.get("id").and_then(|v| v.as_str()) {
            if let Ok(mut guard) = zeroclaw_pending_invokes().lock() {
                guard.insert(id.to_string(), payload.clone());
            }
        }
    }
}

fn take_zeroclaw_invoke(invoke_id: &str) -> Option<Value> {
    if let Ok(mut guard) = zeroclaw_pending_invokes().lock() {
        return guard.remove(invoke_id);
    }
    None
}

#[tauri::command]
pub async fn doctor_connect(app: AppHandle) -> Result<(), String> {
    let _ = app.emit("doctor:connected", json!({ "engine": "zeroclaw" }));
    Ok(())
}

#[tauri::command]
pub async fn doctor_disconnect() -> Result<(), String> {
    Ok(())
}

#[tauri::command]
pub async fn doctor_start_diagnosis(
    app: AppHandle,
    context: String,
    session_key: String,
    agent_id: String,
    instance_id: Option<String>,
) -> Result<(), String> {
    let instance = instance_id.unwrap_or_else(|| "local".to_string());
    let key = RuntimeSessionKey::new(
        "zeroclaw",
        RuntimeDomain::Doctor,
        instance,
        agent_id.clone(),
        session_key.clone(),
    );
    let adapter = ZeroclawDoctorAdapter;
    match adapter.start(&key, &context) {
        Ok(events) => {
            for ev in events {
                register_runtime_invoke(&ev);
                emit_runtime_event(&app, ev);
            }
            Ok(())
        }
        Err(e) => {
            let code = e.code.as_str();
            emit_runtime_event(&app, RuntimeEvent::Error { error: e });
            Err(format!("zeroclaw start failed [{code}]"))
        }
    }
}

#[tauri::command]
pub async fn doctor_send_message(
    app: AppHandle,
    message: String,
    session_key: String,
    agent_id: String,
    instance_id: Option<String>,
) -> Result<(), String> {
    let instance = instance_id.unwrap_or_else(|| "local".to_string());
    let key = RuntimeSessionKey::new(
        "zeroclaw",
        RuntimeDomain::Doctor,
        instance,
        agent_id.clone(),
        session_key.clone(),
    );
    let adapter = ZeroclawDoctorAdapter;
    match adapter.send(&key, &message) {
        Ok(events) => {
            for ev in events {
                register_runtime_invoke(&ev);
                emit_runtime_event(&app, ev);
            }
            Ok(())
        }
        Err(e) => {
            let code = e.code.as_str();
            emit_runtime_event(&app, RuntimeEvent::Error { error: e });
            Err(format!("zeroclaw send failed [{code}]"))
        }
    }
}

#[tauri::command]
pub async fn doctor_approve_invoke(
    app: AppHandle,
    pool: State<'_, SshConnectionPool>,
    invoke_id: String,
    target: String,
    session_key: String,
    agent_id: String,
    domain: Option<String>,
) -> Result<Value, String> {
    let invoke = take_zeroclaw_invoke(&invoke_id)
        .ok_or_else(|| format!("No pending invoke with id: {invoke_id}"))?;

    let command = invoke.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let args = invoke.get("args").cloned().unwrap_or(Value::Null);
    // Map standard node commands to internal execution.
    // Security: commands reach here only after user approval in the UI
    // (write → "Execute" button, read → "Allow" button).
    // User approval is the security boundary, not command validation.
    let exec_result = match command {
        "clawpal" => run_clawpal_tool(&pool, &args, &target).await,
        "openclaw" => run_openclaw_tool(&pool, &args, &target).await,
        _ => {
            return Err(format!(
                "unsupported tool '{command}', expected 'clawpal' or 'openclaw'"
            ))
        }
    };
    let result = match exec_result {
        Ok(ok) => ok,
        Err(err) => json!({
            "stdout": "",
            "stderr": err,
            "exitCode": 2,
        }),
    };

    // Emit tool result first so UI can render it directly under the tool call
    // before any zeroclaw follow-up assistant message arrives.
    let _ = app.emit(
        "doctor:invoke-result",
        json!({
            "id": invoke_id,
            "result": result,
        }),
    );

    // Feed execution result back into zeroclaw session so it can continue the diagnosis.
    let command = command.to_string();
    let invoke_desc = describe_invoke(&command, &args);
    let result_text = if let Some(stdout) = result.get("stdout").and_then(|v| v.as_str()) {
        let stderr = result.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
        let exit_code = result
            .get("exitCode")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        let mut msg = format!("[Command executed: `{invoke_desc}`]\n");
        if !stdout.is_empty() {
            msg.push_str(&format!("stdout:\n```\n{stdout}\n```\n"));
        }
        if !stderr.is_empty() {
            msg.push_str(&format!("stderr:\n```\n{stderr}\n```\n"));
        }
        msg.push_str(&format!("exitCode: {exit_code}"));
        msg
    } else {
        format!("[Command executed: `{invoke_desc}`]\nResult: {result}")
    };
    let is_install = domain.as_deref() == Some("install");
    let rt_domain = if is_install {
        RuntimeDomain::Install
    } else {
        RuntimeDomain::Doctor
    };
    let key = RuntimeSessionKey::new(
        "zeroclaw",
        rt_domain,
        target.clone(),
        agent_id.clone(),
        session_key.clone(),
    );
    let send_result = if is_install {
        ZeroclawInstallAdapter.send(&key, &result_text)
    } else {
        ZeroclawDoctorAdapter.send(&key, &result_text)
    };
    if let Ok(events) = send_result {
        for ev in events {
            register_runtime_invoke(&ev);
            emit_runtime_event(&app, ev);
        }
    }

    Ok(result)
}

#[tauri::command]
pub async fn doctor_reject_invoke(invoke_id: String, _reason: String) -> Result<(), String> {
    if take_zeroclaw_invoke(&invoke_id).is_some() {
        // zeroclaw local pending invoke: just drop from pending queue.
        return Ok(());
    }
    Err(format!("No pending invoke with id: {invoke_id}"))
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
    let error_log = crate::logging::read_log_tail("error.log", 100).unwrap_or_default();

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
    let version_result = pool
        .exec_login(&host_id, "openclaw --version 2>/dev/null || echo unknown")
        .await?;
    let version = version_result.stdout.trim().to_string();

    // Resolve config path: check OPENCLAW_STATE_DIR / OPENCLAW_HOME, fallback to ~/.openclaw
    let config_path_result = pool
        .exec_login(
            &host_id,
            "echo \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}/openclaw.json\"",
        )
        .await?;
    let config_path = config_path_result.stdout.trim().to_string();
    validate_not_sensitive(&config_path)?;
    let config_content = pool
        .sftp_read(&host_id, &config_path)
        .await
        .unwrap_or_else(|_| "(unable to read remote config)".into());

    // Use `openclaw gateway status` — always returns useful text even when gateway is stopped.
    // `openclaw health --json` requires a running gateway + auth token and returns empty otherwise.
    let status_result = pool
        .exec_login(&host_id, "openclaw gateway status 2>&1")
        .await?;
    let gateway_status = status_result.stdout.trim().to_string();

    // Check if gateway process is running (reliable even when health RPC fails)
    // Bracket trick: [o]penclaw-gateway prevents pgrep from matching its own sh -c process
    let pgrep_result = pool
        .exec(&host_id, "pgrep -f '[o]penclaw-gateway' >/dev/null 2>&1")
        .await;
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

fn target_is_remote_instance(target: &str) -> bool {
    if target.starts_with("ssh:") {
        return true;
    }
    if target == "local" || target.starts_with("docker:") {
        return false;
    }
    if let Ok(registry) = clawpal_core::instance::InstanceRegistry::load() {
        if let Some(instance) = registry.get(target) {
            return matches!(
                instance.instance_type,
                clawpal_core::instance::InstanceType::RemoteSsh
            );
        }
    }
    false
}

async fn probe_openclaw_on_target(pool: &SshConnectionPool, target: &str) -> Result<Value, String> {
    if target_is_remote_instance(target) {
        let which = pool
            .exec_login(target, "command -v openclaw 2>/dev/null || true")
            .await
            .map_err(|e| format!("probe which failed: {e}"))?;
        let version = pool
            .exec_login(target, "openclaw --version 2>/dev/null || true")
            .await
            .map_err(|e| format!("probe version failed: {e}"))?;
        let path_env = pool
            .exec_login(target, "printf '%s' \"$PATH\"")
            .await
            .map_err(|e| format!("probe PATH failed: {e}"))?;
        let located = which.stdout.trim().to_string();
        return Ok(json!({
            "target": target,
            "remote": true,
            "openclawPath": if located.is_empty() { Value::Null } else { Value::String(located.clone()) },
            "openclawVersion": version.stdout.trim(),
            "path": path_env.stdout.trim(),
            "ok": !located.is_empty(),
        }));
    }

    let which = std::process::Command::new("sh")
        .arg("-lc")
        .arg("command -v openclaw 2>/dev/null || true")
        .output()
        .map_err(|e| format!("probe which failed: {e}"))?;
    let version = clawpal_core::openclaw::OpenclawCli::new()
        .run(&["--version"])
        .map(|o| o.stdout.trim().to_string())
        .unwrap_or_default();
    let path = std::env::var("PATH").unwrap_or_default();
    let located = String::from_utf8_lossy(&which.stdout).trim().to_string();
    Ok(json!({
        "target": target,
        "remote": false,
        "openclawPath": if located.is_empty() { Value::Null } else { Value::String(located.clone()) },
        "openclawVersion": version,
        "path": path,
        "ok": !located.is_empty(),
    }))
}

async fn fix_openclaw_path_on_target(pool: &SshConnectionPool, target: &str) -> Result<Value, String> {
    if !target_is_remote_instance(target) {
        return Err("doctor fix-openclaw-path currently supports remote target only".to_string());
    }
    let find_dir = pool.exec_login(
        target,
        "for d in \"$HOME/.npm-global/bin\" \"/opt/homebrew/bin\" \"/usr/local/bin\"; do [ -x \"$d/openclaw\" ] && echo \"$d\" && break; done",
    ).await?;
    let dir = find_dir.stdout.trim().to_string();
    if dir.is_empty() {
        return Err("cannot locate openclaw binary in known directories".to_string());
    }
    let escaped_dir = dir.replace('\'', "'\\''");
    let patch_script = format!(
        "line='export PATH=\"{escaped_dir}:$PATH\"'; \
for f in \"$HOME/.zshrc\" \"$HOME/.bashrc\"; do \
  touch \"$f\"; \
  grep -Fq \"$line\" \"$f\" || printf '\\n%s\\n' \"$line\" >> \"$f\"; \
done; \
command -v openclaw 2>/dev/null || true"
    );
    let apply = pool.exec_login(target, &patch_script).await?;
    let located = apply.stdout.trim().to_string();
    Ok(json!({
        "target": target,
        "updatedPathDir": dir,
        "openclawPathAfterFix": if located.is_empty() { Value::Null } else { Value::String(located.clone()) },
        "ok": !located.is_empty(),
    }))
}

#[derive(Default)]
struct ParsedCliArgs {
    positionals: Vec<String>,
    options: HashMap<String, Option<String>>,
}

fn parse_cli_args(tokens: &[&str]) -> ParsedCliArgs {
    let mut parsed = ParsedCliArgs::default();
    let mut index = 0usize;
    while index < tokens.len() {
        let token = tokens[index];
        if !token.starts_with("--") || token == "--" {
            parsed.positionals.push(token.to_string());
            index += 1;
            continue;
        }
        let key = token.trim_start_matches("--").to_string();
        if index + 1 < tokens.len() && !tokens[index + 1].starts_with("--") {
            parsed
                .options
                .insert(key, Some(tokens[index + 1].to_string()));
            index += 2;
        } else {
            parsed.options.insert(key, None);
            index += 1;
        }
    }
    parsed
}

fn tool_stdout_json(value: Value) -> Result<Value, String> {
    let stdout =
        serde_json::to_string(&value).map_err(|e| format!("failed to serialize tool output: {e}"))?;
    Ok(json!({
        "stdout": stdout,
        "stderr": "",
        "exitCode": 0
    }))
}

fn parse_tool_tokens(raw: &str) -> Result<Vec<String>, String> {
    shell_words::split(raw).map_err(|e| format!("invalid command args: {e}"))
}

fn describe_invoke(command: &str, args: &Value) -> String {
    let raw = args
        .get("args")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if raw.is_empty() {
        command.to_string()
    } else {
        format!("{command} {raw}")
    }
}

fn delete_json_path(value: &mut Value, dotted_path: &str) -> bool {
    let parts: Vec<&str> = dotted_path
        .split('.')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return false;
    }
    let mut cursor = value;
    for part in &parts[..parts.len() - 1] {
        if let Some(next) = cursor.get_mut(*part) {
            cursor = next;
        } else {
            return false;
        }
    }
    if let Some(obj) = cursor.as_object_mut() {
        return obj.remove(parts[parts.len() - 1]).is_some();
    }
    false
}

fn upsert_json_path(value: &mut Value, dotted_path: &str, next_value: Value) -> Result<(), String> {
    let parts: Vec<&str> = dotted_path
        .split('.')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Err("doctor config-upsert requires non-empty <json.path>".to_string());
    }
    let mut cursor = value;
    for part in &parts[..parts.len() - 1] {
        if cursor.get(*part).is_none() {
            if let Some(obj) = cursor.as_object_mut() {
                obj.insert((*part).to_string(), json!({}));
            } else {
                return Err(format!("path segment '{part}' is not an object"));
            }
        }
        cursor = cursor
            .get_mut(*part)
            .ok_or_else(|| format!("path segment '{part}' is missing"))?;
        if !cursor.is_object() {
            return Err(format!("path segment '{part}' is not an object"));
        }
    }
    let leaf = parts[parts.len() - 1];
    let obj = cursor
        .as_object_mut()
        .ok_or_else(|| "target parent is not an object".to_string())?;
    obj.insert(leaf.to_string(), next_value);
    Ok(())
}

fn json_path_get<'a>(value: &'a Value, dotted_path: &str) -> Option<&'a Value> {
    let parts: Vec<&str> = dotted_path
        .split('.')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Some(value);
    }
    let mut cursor = value;
    for part in parts {
        cursor = cursor.get(part)?;
    }
    Some(cursor)
}

async fn doctor_config_delete(
    pool: &SshConnectionPool,
    target: &str,
    dotted_path: &str,
) -> Result<Value, String> {
    if dotted_path.trim().is_empty() {
        return Err("doctor config-delete requires <json.path>".to_string());
    }
    if target_is_remote_instance(target) {
        let config_path_result = pool
            .exec_login(
                target,
                "echo \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}/openclaw.json\"",
            )
            .await?;
        let config_path = config_path_result.stdout.trim().to_string();
        let raw = pool.sftp_read(target, &config_path).await?;
        let mut json: Value =
            serde_json::from_str(&raw).map_err(|e| format!("invalid remote config json: {e}"))?;
        let deleted = delete_json_path(&mut json, dotted_path);
        if deleted {
            let next =
                serde_json::to_string_pretty(&json).map_err(|e| format!("serialize config: {e}"))?;
            pool.sftp_write(target, &config_path, &next).await?;
        }
        return Ok(json!({
            "target": target,
            "remote": true,
            "configPath": config_path,
            "path": dotted_path,
            "deleted": deleted,
        }));
    }

    let paths = resolve_paths();
    let config_path = paths.config_path;
    let raw = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("failed to read local config: {e}"))?;
    let mut json: Value =
        serde_json::from_str(&raw).map_err(|e| format!("invalid local config json: {e}"))?;
    let deleted = delete_json_path(&mut json, dotted_path);
    if deleted {
        let next = serde_json::to_string_pretty(&json).map_err(|e| format!("serialize config: {e}"))?;
        std::fs::write(&config_path, next).map_err(|e| format!("failed to write local config: {e}"))?;
    }
    Ok(json!({
        "target": target,
        "remote": false,
        "configPath": config_path.to_string_lossy(),
        "path": dotted_path,
        "deleted": deleted,
    }))
}

async fn doctor_config_read(
    pool: &SshConnectionPool,
    target: &str,
    dotted_path: Option<&str>,
) -> Result<Value, String> {
    if target_is_remote_instance(target) {
        let config_path_result = pool
            .exec_login(
                target,
                "echo \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}/openclaw.json\"",
            )
            .await?;
        let config_path = config_path_result.stdout.trim().to_string();
        let raw = pool.sftp_read(target, &config_path).await?;
        let json: Value =
            serde_json::from_str(&raw).map_err(|e| format!("invalid remote config json: {e}"))?;
        let selected = dotted_path
            .and_then(|p| json_path_get(&json, p).cloned())
            .unwrap_or(json.clone());
        return Ok(json!({
            "target": target,
            "remote": true,
            "configPath": config_path,
            "path": dotted_path.unwrap_or(""),
            "value": selected,
        }));
    }

    let paths = resolve_paths();
    let config_path = paths.config_path;
    let raw = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("failed to read local config: {e}"))?;
    let json: Value =
        serde_json::from_str(&raw).map_err(|e| format!("invalid local config json: {e}"))?;
    let selected = dotted_path
        .and_then(|p| json_path_get(&json, p).cloned())
        .unwrap_or(json.clone());
    Ok(json!({
        "target": target,
        "remote": false,
        "configPath": config_path.to_string_lossy(),
        "path": dotted_path.unwrap_or(""),
        "value": selected,
    }))
}

async fn doctor_config_upsert(
    pool: &SshConnectionPool,
    target: &str,
    dotted_path: &str,
    value_json: &str,
) -> Result<Value, String> {
    if dotted_path.trim().is_empty() {
        return Err("doctor config-upsert requires <json.path>".to_string());
    }
    let next_value: Value = serde_json::from_str(value_json)
        .map_err(|e| format!("doctor config-upsert requires valid JSON value: {e}"))?;
    if target_is_remote_instance(target) {
        let config_path_result = pool
            .exec_login(
                target,
                "echo \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}/openclaw.json\"",
            )
            .await?;
        let config_path = config_path_result.stdout.trim().to_string();
        let raw = pool.sftp_read(target, &config_path).await?;
        let mut json: Value =
            serde_json::from_str(&raw).map_err(|e| format!("invalid remote config json: {e}"))?;
        upsert_json_path(&mut json, dotted_path, next_value)?;
        let rendered =
            serde_json::to_string_pretty(&json).map_err(|e| format!("serialize config: {e}"))?;
        pool.sftp_write(target, &config_path, &rendered).await?;
        return Ok(json!({
            "target": target,
            "remote": true,
            "configPath": config_path,
            "path": dotted_path,
            "upserted": true,
        }));
    }

    let paths = resolve_paths();
    let config_path = paths.config_path;
    let raw = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("failed to read local config: {e}"))?;
    let mut json: Value =
        serde_json::from_str(&raw).map_err(|e| format!("invalid local config json: {e}"))?;
    upsert_json_path(&mut json, dotted_path, next_value)?;
    let rendered =
        serde_json::to_string_pretty(&json).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&config_path, rendered).map_err(|e| format!("failed to write local config: {e}"))?;
    Ok(json!({
        "target": target,
        "remote": false,
        "configPath": config_path.to_string_lossy(),
        "path": dotted_path,
        "upserted": true,
    }))
}

fn resolve_local_sessions_path() -> std::path::PathBuf {
    let paths = resolve_paths();
    let openclaw_root = paths
        .config_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let agents_dir = openclaw_root.join("agents");
    if let Ok(agent_entries) = std::fs::read_dir(&agents_dir) {
        for agent_entry in agent_entries.flatten() {
            let candidate = agent_entry.path().join("sessions").join("sessions.json");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    openclaw_root
        .join("agents")
        .join("test")
        .join("sessions")
        .join("sessions.json")
}

async fn resolve_remote_sessions_path(pool: &SshConnectionPool, target: &str) -> Result<String, String> {
    let out = pool
        .exec_login(
            target,
            "root=\"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}\"; \
             first=\"$(find \"$root/agents\" -type f -path '*/sessions/sessions.json' 2>/dev/null | head -n 1)\"; \
             if [ -n \"$first\" ]; then printf '%s' \"$first\"; else printf '%s' \"$root/agents/test/sessions/sessions.json\"; fi",
        )
        .await?;
    Ok(out.stdout.trim().to_string())
}

async fn doctor_sessions_read(
    pool: &SshConnectionPool,
    target: &str,
    dotted_path: Option<&str>,
) -> Result<Value, String> {
    if target_is_remote_instance(target) {
        let sessions_path = resolve_remote_sessions_path(pool, target).await?;
        let raw = pool.sftp_read(target, &sessions_path).await?;
        let json: Value =
            serde_json::from_str(&raw).map_err(|e| format!("invalid remote sessions json: {e}"))?;
        let selected = dotted_path
            .and_then(|p| json_path_get(&json, p).cloned())
            .unwrap_or(json.clone());
        return Ok(json!({
            "target": target,
            "remote": true,
            "sessionsPath": sessions_path,
            "path": dotted_path.unwrap_or(""),
            "value": selected,
        }));
    }

    let sessions_path = resolve_local_sessions_path();
    let raw = std::fs::read_to_string(&sessions_path)
        .map_err(|e| format!("failed to read local sessions: {e}"))?;
    let json: Value =
        serde_json::from_str(&raw).map_err(|e| format!("invalid local sessions json: {e}"))?;
    let selected = dotted_path
        .and_then(|p| json_path_get(&json, p).cloned())
        .unwrap_or(json.clone());
    Ok(json!({
        "target": target,
        "remote": false,
        "sessionsPath": sessions_path.to_string_lossy(),
        "path": dotted_path.unwrap_or(""),
        "value": selected,
    }))
}

async fn doctor_sessions_upsert(
    pool: &SshConnectionPool,
    target: &str,
    dotted_path: &str,
    value_json: &str,
) -> Result<Value, String> {
    if dotted_path.trim().is_empty() {
        return Err("doctor sessions-upsert requires <json.path>".to_string());
    }
    let next_value: Value = serde_json::from_str(value_json)
        .map_err(|e| format!("doctor sessions-upsert requires valid JSON value: {e}"))?;
    if target_is_remote_instance(target) {
        let sessions_path = resolve_remote_sessions_path(pool, target).await?;
        let raw = pool.sftp_read(target, &sessions_path).await?;
        let mut json: Value =
            serde_json::from_str(&raw).map_err(|e| format!("invalid remote sessions json: {e}"))?;
        upsert_json_path(&mut json, dotted_path, next_value)?;
        let rendered =
            serde_json::to_string_pretty(&json).map_err(|e| format!("serialize sessions: {e}"))?;
        pool.sftp_write(target, &sessions_path, &rendered).await?;
        return Ok(json!({
            "target": target,
            "remote": true,
            "sessionsPath": sessions_path,
            "path": dotted_path,
            "upserted": true,
        }));
    }

    let sessions_path = resolve_local_sessions_path();
    let raw = std::fs::read_to_string(&sessions_path)
        .map_err(|e| format!("failed to read local sessions: {e}"))?;
    let mut json: Value =
        serde_json::from_str(&raw).map_err(|e| format!("invalid local sessions json: {e}"))?;
    upsert_json_path(&mut json, dotted_path, next_value)?;
    let rendered =
        serde_json::to_string_pretty(&json).map_err(|e| format!("serialize sessions: {e}"))?;
    std::fs::write(&sessions_path, rendered)
        .map_err(|e| format!("failed to write local sessions: {e}"))?;
    Ok(json!({
        "target": target,
        "remote": false,
        "sessionsPath": sessions_path.to_string_lossy(),
        "path": dotted_path,
        "upserted": true,
    }))
}

async fn doctor_sessions_delete(
    pool: &SshConnectionPool,
    target: &str,
    dotted_path: &str,
) -> Result<Value, String> {
    if dotted_path.trim().is_empty() {
        return Err("doctor sessions-delete requires <json.path>".to_string());
    }
    if target_is_remote_instance(target) {
        let sessions_path = resolve_remote_sessions_path(pool, target).await?;
        let raw = pool.sftp_read(target, &sessions_path).await?;
        let mut json: Value =
            serde_json::from_str(&raw).map_err(|e| format!("invalid remote sessions json: {e}"))?;
        let deleted = delete_json_path(&mut json, dotted_path);
        if deleted {
            let rendered =
                serde_json::to_string_pretty(&json).map_err(|e| format!("serialize sessions: {e}"))?;
            pool.sftp_write(target, &sessions_path, &rendered).await?;
        }
        return Ok(json!({
            "target": target,
            "remote": true,
            "sessionsPath": sessions_path,
            "path": dotted_path,
            "deleted": deleted,
        }));
    }

    let sessions_path = resolve_local_sessions_path();
    let raw = std::fs::read_to_string(&sessions_path)
        .map_err(|e| format!("failed to read local sessions: {e}"))?;
    let mut json: Value =
        serde_json::from_str(&raw).map_err(|e| format!("invalid local sessions json: {e}"))?;
    let deleted = delete_json_path(&mut json, dotted_path);
    if deleted {
        let rendered =
            serde_json::to_string_pretty(&json).map_err(|e| format!("serialize sessions: {e}"))?;
        std::fs::write(&sessions_path, rendered)
            .map_err(|e| format!("failed to write local sessions: {e}"))?;
    }
    Ok(json!({
        "target": target,
        "remote": false,
        "sessionsPath": sessions_path.to_string_lossy(),
        "path": dotted_path,
        "deleted": deleted,
    }))
}

#[cfg(test)]
const DOCTOR_SUPPORTED_CLAWPAL_COMMANDS: &[&str] = &[
    "instance list",
    "instance remove <id>",
    "health check [<id>] [--all]",
    "ssh list",
    "ssh connect <host_id>",
    "ssh disconnect <host_id>",
    "profile list",
    "profile add --provider <provider> --model <model> [--name <name>] [--api-key <key>]",
    "profile remove <id>",
    "profile test <id>",
    "connect docker --home <path> [--label <name>]",
    "connect ssh --host <host> [--port <port>] [--user <user>] [--id <id>] [--label <label>] [--key-path <path>]",
    "install local",
    "install docker [--home <path>] [--label <label>] [--dry-run] [pull|configure|up]",
    "doctor probe-openclaw",
    "doctor fix-openclaw-path",
    "doctor config-read [<json.path>]",
    "doctor config-upsert <json.path> <json.value>",
    "doctor config-delete <json.path>",
    "doctor sessions-read [<json.path>]",
    "doctor sessions-upsert <json.path> <json.value>",
    "doctor sessions-delete <json.path>",
];

async fn run_clawpal_tool(
    pool: &SshConnectionPool,
    args: &Value,
    target: &str,
) -> Result<Value, String> {
    let raw = args
        .get("args")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if raw.is_empty() {
        return Err("clawpal: missing args".to_string());
    }
    let tokens = parse_tool_tokens(raw)?;
    let token_refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
    if tokens.is_empty() {
        return Err("clawpal: missing args".to_string());
    }

    if token_refs.as_slice() == ["doctor", "probe-openclaw"] {
        let probed = probe_openclaw_on_target(pool, target).await?;
        return tool_stdout_json(probed);
    }
    if token_refs.as_slice() == ["doctor", "fix-openclaw-path"] {
        let fixed = fix_openclaw_path_on_target(pool, target).await?;
        return tool_stdout_json(fixed);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("config-read")
    {
        let path = token_refs.get(2).copied();
        let out = doctor_config_read(pool, target, path).await?;
        return tool_stdout_json(out);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("config-upsert")
    {
        let key_path = token_refs
            .get(2)
            .copied()
            .ok_or_else(|| "clawpal doctor config-upsert requires <json.path> <json.value>".to_string())?;
        let value_json = token_refs
            .get(3)
            .copied()
            .ok_or_else(|| "clawpal doctor config-upsert requires <json.path> <json.value>".to_string())?;
        let out = doctor_config_upsert(pool, target, key_path, value_json).await?;
        return tool_stdout_json(out);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("config-delete")
    {
        let key_path = token_refs
            .get(2)
            .copied()
            .ok_or_else(|| "clawpal doctor config-delete requires <json.path>".to_string())?;
        let out = doctor_config_delete(pool, target, key_path).await?;
        return tool_stdout_json(out);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("sessions-read")
    {
        let path = token_refs.get(2).copied();
        let out = doctor_sessions_read(pool, target, path).await?;
        return tool_stdout_json(out);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("sessions-upsert")
    {
        let key_path = token_refs
            .get(2)
            .copied()
            .ok_or_else(|| "clawpal doctor sessions-upsert requires <json.path> <json.value>".to_string())?;
        let value_json = token_refs
            .get(3)
            .copied()
            .ok_or_else(|| "clawpal doctor sessions-upsert requires <json.path> <json.value>".to_string())?;
        let out = doctor_sessions_upsert(pool, target, key_path, value_json).await?;
        return tool_stdout_json(out);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("sessions-delete")
    {
        let key_path = token_refs
            .get(2)
            .copied()
            .ok_or_else(|| "clawpal doctor sessions-delete requires <json.path>".to_string())?;
        let out = doctor_sessions_delete(pool, target, key_path).await?;
        return tool_stdout_json(out);
    }

    match (token_refs.first().copied(), token_refs.get(1).copied()) {
        (Some("instance"), Some("list")) => {
            let registry =
                clawpal_core::instance::InstanceRegistry::load().map_err(|e| e.to_string())?;
            tool_stdout_json(json!(registry.list()))
        }
        (Some("instance"), Some("remove")) => {
            let id = token_refs
                .get(2)
                .ok_or_else(|| "clawpal instance remove requires <id>".to_string())?;
            let mut registry =
                clawpal_core::instance::InstanceRegistry::load().map_err(|e| e.to_string())?;
            let removed = registry.remove(id).is_some();
            registry.save().map_err(|e| e.to_string())?;
            tool_stdout_json(json!({ "removed": removed, "id": id }))
        }
        (Some("health"), Some("check")) => {
            let parsed = parse_cli_args(&token_refs[2..]);
            let registry =
                clawpal_core::instance::InstanceRegistry::load().map_err(|e| e.to_string())?;
            if parsed.options.contains_key("all") {
                let mut output = Vec::new();
                for instance in registry.list() {
                    let status =
                        clawpal_core::health::check_instance(&instance).map_err(|e| e.to_string())?;
                    output.push(json!({ "id": instance.id, "status": status }));
                }
                return tool_stdout_json(Value::Array(output));
            }
            let target_id = parsed
                .positionals
                .first()
                .map(String::as_str)
                .unwrap_or("local");
            let instance = if target_id == "local" {
                clawpal_core::instance::Instance {
                    id: "local".to_string(),
                    instance_type: clawpal_core::instance::InstanceType::Local,
                    label: "Local".to_string(),
                    openclaw_home: None,
                    clawpal_data_dir: None,
                    ssh_host_config: None,
                }
            } else {
                registry
                    .get(target_id)
                    .cloned()
                    .ok_or_else(|| format!("instance '{target_id}' not found"))?
            };
            let status = clawpal_core::health::check_instance(&instance).map_err(|e| e.to_string())?;
            tool_stdout_json(json!({ "id": instance.id, "status": status }))
        }
        (Some("ssh"), Some("list")) => {
            let hosts = clawpal_core::ssh::registry::list_ssh_hosts().map_err(|e| e.to_string())?;
            tool_stdout_json(json!(hosts))
        }
        (Some("ssh"), Some("connect")) => {
            let host_id = token_refs
                .get(2)
                .ok_or_else(|| "clawpal ssh connect requires <host_id>".to_string())?;
            let registry =
                clawpal_core::instance::InstanceRegistry::load().map_err(|e| e.to_string())?;
            let instance = registry
                .get(host_id)
                .cloned()
                .ok_or_else(|| format!("instance '{host_id}' not found"))?;
            let host = instance
                .ssh_host_config
                .ok_or_else(|| format!("instance '{host_id}' is not an SSH instance"))?;
            let session = clawpal_core::ssh::SshSession::connect(&host)
                .await
                .map_err(|e| e.to_string())?;
            let output = session
                .exec("echo connected")
                .await
                .map_err(|e| e.to_string())?;
            tool_stdout_json(json!({
                "hostId": host_id,
                "connected": output.exit_code == 0,
                "stdout": output.stdout,
                "stderr": output.stderr,
                "exitCode": output.exit_code,
            }))
        }
        (Some("ssh"), Some("disconnect")) => {
            let host_id = token_refs
                .get(2)
                .ok_or_else(|| "clawpal ssh disconnect requires <host_id>".to_string())?;
            tool_stdout_json(json!({
                "hostId": host_id,
                "disconnected": true,
                "note": "stateless ssh mode has no persistent session",
            }))
        }
        (Some("profile"), Some("list")) => {
            let openclaw = clawpal_core::openclaw::OpenclawCli::new();
            let profiles =
                clawpal_core::profile::list_profiles(&openclaw).map_err(|e| e.to_string())?;
            tool_stdout_json(json!(profiles))
        }
        (Some("profile"), Some("remove")) => {
            let id = token_refs
                .get(2)
                .ok_or_else(|| "clawpal profile remove requires <id>".to_string())?;
            let openclaw = clawpal_core::openclaw::OpenclawCli::new();
            let removed = clawpal_core::profile::delete_profile(&openclaw, id)
                .map_err(|e| e.to_string())?;
            tool_stdout_json(json!({ "removed": removed, "id": id }))
        }
        (Some("profile"), Some("test")) => {
            let id = token_refs
                .get(2)
                .ok_or_else(|| "clawpal profile test requires <id>".to_string())?;
            let openclaw = clawpal_core::openclaw::OpenclawCli::new();
            let result =
                clawpal_core::profile::test_profile(&openclaw, id).map_err(|e| e.to_string())?;
            tool_stdout_json(json!(result))
        }
        (Some("profile"), Some("add")) => {
            let parsed = parse_cli_args(&token_refs[2..]);
            let provider = parsed
                .options
                .get("provider")
                .and_then(|v| v.as_ref())
                .cloned()
                .ok_or_else(|| "clawpal profile add requires --provider".to_string())?;
            let model = parsed
                .options
                .get("model")
                .and_then(|v| v.as_ref())
                .cloned()
                .ok_or_else(|| "clawpal profile add requires --model".to_string())?;
            let profile = clawpal_core::profile::ModelProfile {
                id: String::new(),
                name: parsed
                    .options
                    .get("name")
                    .and_then(|v| v.as_ref())
                    .cloned()
                    .unwrap_or_default(),
                provider,
                model,
                auth_ref: String::new(),
                api_key: parsed
                    .options
                    .get("api_key")
                    .or_else(|| parsed.options.get("api-key"))
                    .and_then(|v| v.as_ref())
                    .cloned(),
                base_url: None,
                description: None,
                enabled: true,
            };
            let openclaw = clawpal_core::openclaw::OpenclawCli::new();
            let saved =
                clawpal_core::profile::upsert_profile(&openclaw, profile).map_err(|e| e.to_string())?;
            tool_stdout_json(json!(saved))
        }
        (Some("connect"), Some("docker")) => {
            let parsed = parse_cli_args(&token_refs[2..]);
            let home = parsed
                .options
                .get("home")
                .and_then(|v| v.as_ref())
                .cloned()
                .ok_or_else(|| "clawpal connect docker requires --home".to_string())?;
            let label = parsed
                .options
                .get("label")
                .and_then(|v| v.as_ref())
                .map(String::as_str);
            let instance = clawpal_core::connect::connect_docker(&home, label)
                .await
                .map_err(|e| e.to_string())?;
            tool_stdout_json(json!(instance))
        }
        (Some("connect"), Some("ssh")) => {
            let parsed = parse_cli_args(&token_refs[2..]);
            let host = parsed
                .options
                .get("host")
                .and_then(|v| v.as_ref())
                .cloned()
                .ok_or_else(|| "clawpal connect ssh requires --host".to_string())?;
            let port = parsed
                .options
                .get("port")
                .and_then(|v| v.as_ref())
                .and_then(|v| v.parse::<u16>().ok())
                .unwrap_or(22);
            let username = parsed
                .options
                .get("user")
                .and_then(|v| v.as_ref())
                .cloned()
                .unwrap_or_else(|| "root".to_string());
            let id = parsed
                .options
                .get("id")
                .and_then(|v| v.as_ref())
                .cloned()
                .unwrap_or_else(|| format!("ssh:{host}"));
            let label = parsed
                .options
                .get("label")
                .and_then(|v| v.as_ref())
                .cloned()
                .unwrap_or_else(|| host.clone());
            let key_path = parsed
                .options
                .get("key_path")
                .or_else(|| parsed.options.get("key-path"))
                .and_then(|v| v.as_ref())
                .cloned();
            let config = clawpal_core::instance::SshHostConfig {
                id,
                label,
                host,
                port,
                username,
                auth_method: "key".to_string(),
                key_path,
                password: None,
            };
            let instance = clawpal_core::connect::connect_ssh(config)
                .await
                .map_err(|e| e.to_string())?;
            tool_stdout_json(json!(instance))
        }
        (Some("install"), Some("local")) => {
            let result = clawpal_core::install::install_local(
                clawpal_core::install::LocalInstallOptions::default(),
            )
            .map_err(|e| e.to_string())?;
            tool_stdout_json(json!(result))
        }
        (Some("install"), Some("docker")) => {
            let parsed = parse_cli_args(&token_refs[2..]);
            let subcommand = parsed.positionals.first().map(String::as_str);
            let options = clawpal_core::install::DockerInstallOptions {
                home: parsed
                    .options
                    .get("home")
                    .and_then(|v| v.as_ref())
                    .cloned(),
                label: parsed
                    .options
                    .get("label")
                    .and_then(|v| v.as_ref())
                    .cloned(),
                dry_run: parsed.options.contains_key("dry-run"),
            };
            let result = match subcommand {
                Some("pull") => clawpal_core::install::docker::pull(&options)
                    .map(|v| json!(v))
                    .map_err(|e| e.to_string())?,
                Some("configure") => clawpal_core::install::docker::configure(&options)
                    .map(|v| json!(v))
                    .map_err(|e| e.to_string())?,
                Some("up") => clawpal_core::install::docker::up(&options)
                    .map(|v| json!(v))
                    .map_err(|e| e.to_string())?,
                None => clawpal_core::install::install_docker(options)
                    .map(|v| json!(v))
                    .map_err(|e| e.to_string())?,
                Some(other) => {
                    return Err(format!(
                        "unsupported clawpal install docker subcommand: {other}"
                    ))
                }
            };
            tool_stdout_json(result)
        }
        _ => Err(format!("unsupported clawpal args: {raw}")),
    }
}

async fn run_openclaw_tool(
    pool: &SshConnectionPool,
    args: &Value,
    target: &str,
) -> Result<Value, String> {
    let raw = args
        .get("args")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if raw.is_empty() {
        return Err("openclaw: missing args".to_string());
    }
    let tokens = parse_tool_tokens(raw)?;
    validate_openclaw_tokens(&tokens)?;
    let parts: Vec<&str> = tokens.iter().map(String::as_str).collect();
    let output = if target_is_remote_instance(target) {
        crate::cli_runner::run_openclaw_remote(pool, target, &parts).await?
    } else {
        clawpal_core::openclaw::OpenclawCli::new()
            .run(&parts)
            .map_err(|e| e.to_string())?
    };
    Ok(json!({
        "stdout": output.stdout,
        "stderr": output.stderr,
        "exitCode": output.exit_code,
    }))
}

fn validate_openclaw_tokens(tokens: &[String]) -> Result<(), String> {
    let first = tokens
        .first()
        .map(String::as_str)
        .ok_or_else(|| "openclaw: missing args".to_string())?;
    let allowed = matches!(
        first,
        "--version"
            | "doctor"
            | "gateway"
            | "health"
            | "config"
            | "agents"
            | "models"
            | "auth"
            | "memory"
            | "security"
            | "channels"
            | "directory"
            | "cron"
            | "devices"
    );
    if !allowed {
        return Err(format!(
            "unsupported openclaw args: {} (allowed top-level commands: --version, doctor, gateway, health, config, agents, models, auth, memory, security, channels, directory, cron, devices)",
            tokens.join(" ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn prompt_supported_clawpal_commands() -> BTreeSet<String> {
        let prompt = crate::prompt_templates::doctor_domain_system();
        let mut in_section = false;
        let mut commands = BTreeSet::new();
        for line in prompt.lines() {
            let trimmed = line.trim();
            if trimmed == "For tool=\"clawpal\", you MUST use only these supported commands:" {
                in_section = true;
                continue;
            }
            if in_section {
                if let Some(cmd) = trimmed.strip_prefix("- ") {
                    commands.insert(cmd.to_string());
                    continue;
                }
                if !trimmed.is_empty() {
                    break;
                }
            }
        }
        commands
    }

    #[test]
    fn parse_tool_tokens_keeps_quoted_values() {
        let tokens = parse_tool_tokens(
            "profile add --provider openai --model gpt-4.1 --name \"My Profile\" --api-key \"sk test\"",
        )
        .expect("parse tokens");
        assert_eq!(
            tokens,
            vec![
                "profile",
                "add",
                "--provider",
                "openai",
                "--model",
                "gpt-4.1",
                "--name",
                "My Profile",
                "--api-key",
                "sk test"
            ]
        );
    }

    #[test]
    fn parse_cli_args_supports_space_containing_option_values() {
        let tokens = parse_tool_tokens("connect docker --home \"/tmp/a b\" --label \"Docker Local\"")
            .expect("parse tokens");
        let token_refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        let parsed = parse_cli_args(&token_refs[2..]);
        assert_eq!(
            parsed.options.get("home").and_then(|v| v.as_deref()),
            Some("/tmp/a b")
        );
        assert_eq!(
            parsed.options.get("label").and_then(|v| v.as_deref()),
            Some("Docker Local")
        );
    }

    #[test]
    fn describe_invoke_appends_args_when_present() {
        let args = json!({
            "args": "doctor --fix",
            "instance": "c7c90e52-bbc7-44be-bfe7-a07302646435"
        });
        assert_eq!(
            describe_invoke("openclaw", &args),
            "openclaw doctor --fix"
        );
    }

    #[test]
    fn validate_openclaw_tokens_rejects_unknown_command() {
        let err = validate_openclaw_tokens(&["foobar".to_string()]).unwrap_err();
        assert!(err.contains("unsupported openclaw args"));
    }

    #[test]
    fn validate_openclaw_tokens_accepts_doctor_fix() {
        let ok = validate_openclaw_tokens(&["doctor".to_string(), "--fix".to_string()]);
        assert!(ok.is_ok());
    }

    #[test]
    fn delete_json_path_removes_nested_field() {
        let mut doc = json!({
            "commands": {
                "ownerDisplay": "raw",
                "other": 1
            }
        });
        assert!(delete_json_path(&mut doc, "commands.ownerDisplay"));
        assert!(doc["commands"].get("ownerDisplay").is_none());
        assert_eq!(doc["commands"]["other"], 1);
    }

    #[test]
    fn upsert_json_path_sets_nested_field() {
        let mut doc = json!({
            "commands": {
                "other": 1
            }
        });
        upsert_json_path(&mut doc, "commands.ownerDisplay", json!("raw")).expect("upsert");
        assert_eq!(doc["commands"]["ownerDisplay"], "raw");
        assert_eq!(doc["commands"]["other"], 1);
    }

    #[test]
    fn doctor_prompt_supported_commands_match_backend_list() {
        let prompt_commands = prompt_supported_clawpal_commands();
        let backend_commands = DOCTOR_SUPPORTED_CLAWPAL_COMMANDS
            .iter()
            .map(|v| v.to_string())
            .collect::<BTreeSet<_>>();
        assert_eq!(prompt_commands, backend_commands);
    }

    #[test]
    fn doctor_prompt_rejects_legacy_fix_config_command() {
        let prompt = crate::prompt_templates::doctor_domain_system();
        assert!(
            prompt.contains("NEVER invent non-existent clawpal commands (for example: doctor fix-config)."),
            "prompt should explicitly forbid legacy doctor fix-config command"
        );
    }
}
