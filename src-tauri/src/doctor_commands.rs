use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::io::{BufRead, BufReader};
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, State};

use crate::doctor_runtime_bridge::emit_runtime_event;
use crate::models::resolve_paths;
use crate::runtime::types::{
    RuntimeAdapter, RuntimeDomain, RuntimeError, RuntimeEvent, RuntimeSessionKey,
};
use crate::runtime::zeroclaw::adapter::ZeroclawDoctorAdapter;
use crate::runtime::zeroclaw::install_adapter::ZeroclawInstallAdapter;
use crate::ssh::SshConnectionPool;

const DOCTOR_RESULT_STREAM_MAX_BYTES: usize = 10 * 1024;
const DOCTOR_RESULT_TEXT_MAX_BYTES: usize = 24 * 1024;

fn truncate_utf8_tail(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }
    let mut start = input.len().saturating_sub(max_bytes);
    while start < input.len() && !input.is_char_boundary(start) {
        start += 1;
    }
    input[start..].to_string()
}

fn clamp_doctor_result_stream(label: &str, input: &str) -> String {
    if input.len() <= DOCTOR_RESULT_STREAM_MAX_BYTES {
        return input.to_string();
    }
    let marker = format!(
        "[clawpal notice] {label} truncated (showing latest {} bytes)\n",
        DOCTOR_RESULT_STREAM_MAX_BYTES
    );
    let keep = DOCTOR_RESULT_STREAM_MAX_BYTES.saturating_sub(marker.len());
    let tail = truncate_utf8_tail(input, keep);
    format!("{marker}{tail}")
}

fn clamp_doctor_result_text(input: String) -> String {
    if input.len() <= DOCTOR_RESULT_TEXT_MAX_BYTES {
        return input;
    }
    let marker = format!(
        "[clawpal notice] command result payload truncated to {} bytes.\n",
        DOCTOR_RESULT_TEXT_MAX_BYTES
    );
    let keep = DOCTOR_RESULT_TEXT_MAX_BYTES.saturating_sub(marker.len());
    let tail = truncate_utf8_tail(&input, keep);
    format!("{marker}{tail}")
}

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
            let code = e.code.as_str().to_string();
            let message = e.message.clone();
            crate::logging::log_error(&format!(
                "doctor_start_diagnosis failed: code={code} instance={} agent={} message={message}",
                key.instance_id, key.agent_id
            ));
            emit_runtime_event(&app, RuntimeEvent::Error { error: e });
            // Do not reject the command: frontend consumes doctor:error events.
            Ok(())
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
            let code = e.code.as_str().to_string();
            let message = e.message.clone();
            crate::logging::log_error(&format!(
                "doctor_send_message failed: code={code} instance={} agent={} message={message}",
                key.instance_id, key.agent_id
            ));
            emit_runtime_event(&app, RuntimeEvent::Error { error: e });
            // Keep session alive and surface the runtime error event in UI.
            Ok(())
        }
    }
}

#[tauri::command]
pub async fn doctor_approve_invoke(
    app: AppHandle,
    pool: State<'_, SshConnectionPool>,
    invoke_id: String,
    target: String,
    instance_id: Option<String>,
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
            let raw_args = args
                .get("args")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            run_doctor_exec_tool(&pool, command, raw_args, &target).await
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
    let result_text = if let Some(stdout_raw) = result.get("stdout").and_then(|v| v.as_str()) {
        let stdout = clamp_doctor_result_stream("stdout", stdout_raw);
        let stderr_raw = result.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
        let stderr = clamp_doctor_result_stream("stderr", stderr_raw);
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
    let result_text = clamp_doctor_result_text(result_text);
    let is_install = domain.as_deref() == Some("install");
    let rt_domain = if is_install {
        RuntimeDomain::Install
    } else {
        RuntimeDomain::Doctor
    };
    let instance_scope = instance_id.unwrap_or_else(|| target.clone());
    let key = RuntimeSessionKey::new(
        "zeroclaw",
        rt_domain,
        instance_scope,
        agent_id.clone(),
        session_key.clone(),
    );
    let send_result = if is_install {
        ZeroclawInstallAdapter.send(&key, &result_text)
    } else {
        ZeroclawDoctorAdapter.send(&key, &result_text)
    };
    let events = match handle_runtime_send_result(rt_domain.as_str(), send_result) {
        Ok(events) => events,
        Err(err) => {
            emit_runtime_event(&app, RuntimeEvent::Error { error: err.clone() });
            return Err(format_runtime_send_error(rt_domain.as_str(), &err));
        }
    };
    for ev in events {
        register_runtime_invoke(&ev);
        emit_runtime_event(&app, ev);
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

    let doc_request = crate::openclaw_doc_resolver::DocResolveRequest {
        instance_scope: "local".to_string(),
        transport: "local".to_string(),
        openclaw_version: Some(version.trim().to_string()),
        doctor_issues: doctor_report
            .issues
            .iter()
            .map(|i| crate::openclaw_doc_resolver::DocResolveIssue {
                id: i.id.clone(),
                severity: i.severity.clone(),
                message: i.message.clone(),
            })
            .collect(),
        config_content: config_content.clone(),
        error_log: error_log.clone(),
        gateway_status: Some(if gateway_running {
            "running".to_string()
        } else {
            "stopped".to_string()
        }),
    };
    let doc_guidance =
        crate::openclaw_doc_resolver::resolve_local_doc_guidance(&doc_request, &paths).await;

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
        "docGuidance": doc_guidance,
    });

    serde_json::to_string(&context).map_err(|e| format!("Failed to serialize context: {e}"))
}

type RemoteExecFuture<'a> =
    Pin<Box<dyn Future<Output = Result<crate::ssh::SshExecResult, String>> + Send + 'a>>;
type RemoteStringFuture<'a> = Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;
type RemoteDocGuidanceFuture<'a> =
    Pin<Box<dyn Future<Output = crate::openclaw_doc_resolver::DocGuidance> + Send + 'a>>;

trait RemoteDoctorContextOps {
    fn exec_login<'a>(&'a self, host_id: &'a str, command: &'a str) -> RemoteExecFuture<'a>;
    fn exec<'a>(&'a self, host_id: &'a str, command: &'a str) -> RemoteExecFuture<'a>;
    fn sftp_read<'a>(&'a self, host_id: &'a str, path: &'a str) -> RemoteStringFuture<'a>;
    fn resolve_config_path<'a>(&'a self, host_id: &'a str) -> RemoteStringFuture<'a>;
}

struct SshPoolRemoteDoctorContextOps<'a> {
    pool: &'a SshConnectionPool,
}

impl RemoteDoctorContextOps for SshPoolRemoteDoctorContextOps<'_> {
    fn exec_login<'a>(&'a self, host_id: &'a str, command: &'a str) -> RemoteExecFuture<'a> {
        Box::pin(async move { self.pool.exec_login(host_id, command).await })
    }

    fn exec<'a>(&'a self, host_id: &'a str, command: &'a str) -> RemoteExecFuture<'a> {
        Box::pin(async move { self.pool.exec(host_id, command).await })
    }

    fn sftp_read<'a>(&'a self, host_id: &'a str, path: &'a str) -> RemoteStringFuture<'a> {
        Box::pin(async move { self.pool.sftp_read(host_id, path).await })
    }

    fn resolve_config_path<'a>(&'a self, host_id: &'a str) -> RemoteStringFuture<'a> {
        Box::pin(async move { resolve_remote_config_path(self.pool, host_id).await })
    }
}

trait RemoteDocGuidanceResolver {
    fn resolve_remote_guidance<'a>(
        &'a self,
        host_id: &'a str,
        request: &'a crate::openclaw_doc_resolver::DocResolveRequest,
        paths: &'a crate::models::OpenClawPaths,
    ) -> RemoteDocGuidanceFuture<'a>;
}

struct OpenclawRemoteDocGuidanceResolver<'a> {
    pool: &'a SshConnectionPool,
}

impl RemoteDocGuidanceResolver for OpenclawRemoteDocGuidanceResolver<'_> {
    fn resolve_remote_guidance<'a>(
        &'a self,
        host_id: &'a str,
        request: &'a crate::openclaw_doc_resolver::DocResolveRequest,
        paths: &'a crate::models::OpenClawPaths,
    ) -> RemoteDocGuidanceFuture<'a> {
        Box::pin(async move {
            crate::openclaw_doc_resolver::resolve_remote_doc_guidance(
                self.pool, host_id, request, paths,
            )
            .await
        })
    }
}

#[tauri::command]
pub async fn collect_doctor_context_remote(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<String, String> {
    let ops = SshPoolRemoteDoctorContextOps { pool: pool.inner() };
    let resolver = OpenclawRemoteDocGuidanceResolver { pool: pool.inner() };
    collect_doctor_context_remote_with_ops(&ops, &resolver, &host_id).await
}

async fn collect_doctor_context_remote_with_ops(
    ops: &impl RemoteDoctorContextOps,
    resolver: &impl RemoteDocGuidanceResolver,
    host_id: &str,
) -> Result<String, String> {
    // Collect openclaw version
    let version_result = ops
        .exec_login(
            host_id,
            clawpal_core::doctor::remote_openclaw_version_probe_script(),
        )
        .await?;
    let version = version_result.stdout.trim().to_string();

    // Resolve config path: check OPENCLAW_STATE_DIR / OPENCLAW_HOME, fallback to ~/.openclaw
    let config_path = ops.resolve_config_path(host_id).await?;
    validate_not_sensitive(&config_path)?;
    let config_content = ops
        .sftp_read(host_id, &config_path)
        .await
        .unwrap_or_else(|_| "(unable to read remote config)".into());

    // Use `openclaw gateway status` — always returns useful text even when gateway is stopped.
    // `openclaw health --json` requires a running gateway + auth token and returns empty otherwise.
    let status_result = ops
        .exec_login(
            host_id,
            clawpal_core::doctor::remote_openclaw_gateway_status_script(),
        )
        .await?;
    let gateway_status = status_result.stdout.trim().to_string();

    // Check if gateway process is running (reliable even when health RPC fails)
    // Bracket trick: [o]penclaw-gateway prevents pgrep from matching its own sh -c process
    let pgrep_result = ops
        .exec(
            host_id,
            clawpal_core::doctor::remote_openclaw_gateway_process_probe_script(),
        )
        .await;
    let gateway_running = matches!(pgrep_result, Ok(r) if r.exit_code == 0);

    // Collect recent error log (logs live under $OPENCLAW_STATE_DIR/logs/)
    let error_log_result = ops
        .exec_login(
            host_id,
            &clawpal_core::doctor::remote_gateway_error_log_tail_script(100),
        )
        .await?;
    let error_log = error_log_result.stdout;

    // System info
    let platform_result = ops
        .exec(host_id, clawpal_core::doctor::remote_uname_s_script())
        .await?;
    let arch_result = ops
        .exec(host_id, clawpal_core::doctor::remote_uname_m_script())
        .await?;

    let paths = resolve_paths();
    let doc_request = crate::openclaw_doc_resolver::DocResolveRequest {
        instance_scope: host_id.to_string(),
        transport: "remote_ssh".to_string(),
        openclaw_version: Some(version.clone()),
        doctor_issues: Vec::new(),
        config_content: config_content.clone(),
        error_log: error_log.clone(),
        gateway_status: Some(gateway_status.clone()),
    };
    let doc_guidance = resolver
        .resolve_remote_guidance(host_id, &doc_request, &paths)
        .await;

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
        "docGuidance": doc_guidance,
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
            .exec_login(target, clawpal_core::doctor::openclaw_which_probe_script())
            .await
            .map_err(|e| format!("probe which failed: {e}"))?;
        let version = pool
            .exec_login(
                target,
                clawpal_core::doctor::remote_openclaw_version_probe_script(),
            )
            .await
            .map_err(|e| format!("probe version failed: {e}"))?;
        let path_env = pool
            .exec_login(target, clawpal_core::doctor::shell_path_probe_script())
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

    let probe_cmd = clawpal_core::shell::wrap_login_shell_eval(
        clawpal_core::doctor::openclaw_which_probe_script(),
    );
    let which = std::process::Command::new("sh")
        .arg("-c")
        .arg(&probe_cmd)
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

async fn fix_openclaw_path_on_target(
    pool: &SshConnectionPool,
    target: &str,
) -> Result<Value, String> {
    if !target_is_remote_instance(target) {
        return Err("doctor fix-openclaw-path currently supports remote target only".to_string());
    }
    let find_dir = pool
        .exec_login(
            target,
            clawpal_core::doctor::remote_openclaw_fix_find_dir_script(),
        )
        .await?;
    let dir = find_dir.stdout.trim().to_string();
    if dir.is_empty() {
        return Err("cannot locate openclaw binary in known directories".to_string());
    }
    let patch_script = clawpal_core::doctor::remote_openclaw_fix_patch_script(&dir);
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

fn resolved_target<'a>(
    default_target: &'a str,
    parsed: &'a ParsedCliArgs,
) -> Result<&'a str, String> {
    match parsed.options.get("instance").and_then(|v| v.as_deref()) {
        Some(id) if id.trim().is_empty() => {
            Err("clawpal doctor --instance cannot be empty".to_string())
        }
        Some(id) => {
            if id == "local" || id.starts_with("docker:") {
                return Ok(id);
            }
            let registry = clawpal_core::instance::InstanceRegistry::load()
                .map_err(|e| format!("failed to load instance registry: {e}"))?;
            if registry.get(id).is_some() {
                Ok(id)
            } else {
                Err(format!("unknown instance id: {id}"))
            }
        }
        None => Ok(default_target),
    }
}

fn tool_stdout_json(value: Value) -> Result<Value, String> {
    let stdout = serde_json::to_string(&value)
        .map_err(|e| format!("failed to serialize tool output: {e}"))?;
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

fn sh_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

const DOCTOR_LOG_READ_MAX_LINES: usize = 400;

fn read_file_tail_lines(path: &std::path::Path, max_lines: usize) -> Result<String, String> {
    if max_lines == 0 {
        return Ok(String::new());
    }
    let file = std::fs::File::open(path).map_err(|e| format!("failed to read file: {e}"))?;
    let reader = BufReader::new(file);
    let mut ring: VecDeque<String> = VecDeque::with_capacity(max_lines + 1);
    for line in reader.lines() {
        let line = line.map_err(|e| format!("failed to read file: {e}"))?;
        ring.push_back(line);
        if ring.len() > max_lines {
            let _ = ring.pop_front();
        }
    }
    Ok(ring.into_iter().collect::<Vec<_>>().join("\n"))
}

fn format_runtime_send_error(domain: &str, err: &RuntimeError) -> String {
    format!(
        "{domain} runtime send failed [{}]: {}",
        err.code.as_str(),
        err.message
    )
}

fn handle_runtime_send_result(
    domain: &str,
    send_result: Result<Vec<RuntimeEvent>, RuntimeError>,
) -> Result<Vec<RuntimeEvent>, RuntimeError> {
    match send_result {
        Ok(events) => Ok(events),
        Err(err) => {
            crate::logging::log_error(&format_runtime_send_error(domain, &err));
            Err(err)
        }
    }
}

fn local_openclaw_root() -> Result<std::path::PathBuf, String> {
    let paths = resolve_paths();
    paths
        .config_path
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "failed to resolve local openclaw root".to_string())
}

async fn doctor_domain_remote_root(
    pool: &SshConnectionPool,
    target: &str,
    domain: &str,
) -> Result<String, String> {
    let base = pool
        .exec_login(
            target,
            clawpal_core::doctor::remote_openclaw_root_probe_script(),
        )
        .await?
        .stdout
        .trim()
        .to_string();
    clawpal_core::doctor::doctor_domain_remote_root(&base, domain)
}

async fn resolve_remote_config_path(
    pool: &SshConnectionPool,
    target: &str,
) -> Result<String, String> {
    let out = pool
        .exec_login(
            target,
            clawpal_core::doctor::remote_openclaw_config_path_probe_script(),
        )
        .await?;
    Ok(out.stdout.trim().to_string())
}

async fn doctor_file_read(
    pool: &SshConnectionPool,
    target: &str,
    domain: &str,
    path: Option<&str>,
) -> Result<Value, String> {
    if target_is_remote_instance(target) {
        let root = doctor_domain_remote_root(pool, target, domain).await?;
        let rel = match path {
            Some(p) => p.to_string(),
            None => match domain {
                "sessions" => {
                    let abs = resolve_remote_sessions_path(pool, target).await?;
                    clawpal_core::doctor::relpath_from_remote_abs(&root, &abs).ok_or_else(|| {
                        format!("failed to resolve sessions path under domain root: {root}")
                    })?
                }
                _ => clawpal_core::doctor::doctor_domain_default_relpath(domain)
                    .ok_or_else(|| "doctor file read requires --path for this domain".to_string())?
                    .to_string(),
            },
        };
        clawpal_core::doctor::validate_doctor_relative_path(&rel)?;
        let full_path = format!("{}/{}", root.trim_end_matches('/'), rel);
        validate_not_sensitive(&full_path)?;
        let content = if domain == "logs" {
            // Guardrail: logs can be very large; always tail a bounded window for doctor context.
            let cmd = format!(
                "tail -n {} {} 2>/dev/null || true",
                DOCTOR_LOG_READ_MAX_LINES,
                sh_single_quote(&full_path)
            );
            let out = pool.exec_login(target, &cmd).await?;
            out.stdout
        } else {
            pool.sftp_read(target, &full_path).await?
        };
        return Ok(json!({
            "target": target,
            "remote": true,
            "domain": domain,
            "root": root,
            "path": rel,
            "fullPath": full_path,
            "content": content,
        }));
    }
    let openclaw_root = local_openclaw_root()?;
    let root = clawpal_core::doctor::doctor_domain_local_root(&openclaw_root, domain)?;
    let rel = match path {
        Some(p) => p.to_string(),
        None => match domain {
            "sessions" => {
                let abs = clawpal_core::doctor::resolve_local_sessions_path(&openclaw_root);
                clawpal_core::doctor::relpath_from_local_abs(&root, &abs).ok_or_else(|| {
                    format!(
                        "failed to resolve sessions path under domain root: {}",
                        root.display()
                    )
                })?
            }
            _ => clawpal_core::doctor::doctor_domain_default_relpath(domain)
                .ok_or_else(|| "doctor file read requires --path for this domain".to_string())?
                .to_string(),
        },
    };
    clawpal_core::doctor::validate_doctor_relative_path(&rel)?;
    let full_path = root.join(&rel);
    validate_not_sensitive(&full_path.to_string_lossy())?;
    let content = if domain == "logs" {
        read_file_tail_lines(&full_path, DOCTOR_LOG_READ_MAX_LINES)?
    } else {
        std::fs::read_to_string(&full_path).map_err(|e| format!("failed to read file: {e}"))?
    };
    Ok(json!({
        "target": target,
        "remote": false,
        "domain": domain,
        "root": root.to_string_lossy(),
        "path": rel,
        "fullPath": full_path.to_string_lossy(),
        "content": content,
    }))
}

async fn doctor_file_write(
    pool: &SshConnectionPool,
    target: &str,
    domain: &str,
    path: Option<&str>,
    content: &str,
    backup: bool,
) -> Result<Value, String> {
    if target_is_remote_instance(target) {
        let root = doctor_domain_remote_root(pool, target, domain).await?;
        let rel = match path {
            Some(p) => p.to_string(),
            None => match domain {
                "sessions" => {
                    let abs = resolve_remote_sessions_path(pool, target).await?;
                    clawpal_core::doctor::relpath_from_remote_abs(&root, &abs).ok_or_else(|| {
                        format!("failed to resolve sessions path under domain root: {root}")
                    })?
                }
                _ => clawpal_core::doctor::doctor_domain_default_relpath(domain)
                    .ok_or_else(|| "doctor file write requires --path for this domain".to_string())?
                    .to_string(),
            },
        };
        clawpal_core::doctor::validate_doctor_relative_path(&rel)?;
        let full_path = format!("{}/{}", root.trim_end_matches('/'), rel);
        validate_not_sensitive(&full_path)?;
        let dirname_cmd = format!("mkdir -p \"$(dirname {})\"", sh_single_quote(&full_path));
        let mkdir_out = pool.exec_login(target, &dirname_cmd).await?;
        if mkdir_out.exit_code != 0 {
            return Err(format!(
                "doctor file write mkdir failed (exit {}): {}",
                mkdir_out.exit_code, mkdir_out.stderr
            ));
        }
        if backup {
            let backup_cmd = format!(
                "if [ -f {f} ]; then cp {f} {f}.bak.$(date +%Y%m%d%H%M%S); fi",
                f = sh_single_quote(&full_path)
            );
            let backup_out = pool.exec_login(target, &backup_cmd).await?;
            if backup_out.exit_code != 0 {
                return Err(format!(
                    "doctor file write backup failed (exit {}): {}",
                    backup_out.exit_code, backup_out.stderr
                ));
            }
        }
        pool.sftp_write(target, &full_path, content).await?;
        let verify = pool.sftp_read(target, &full_path).await?;
        if verify != content {
            return Err(
                "doctor file write verification failed: remote content mismatch".to_string(),
            );
        }
        return Ok(json!({
            "target": target,
            "remote": true,
            "domain": domain,
            "root": root,
            "path": rel,
            "fullPath": full_path,
            "written": true,
            "backup": backup,
        }));
    }
    let openclaw_root = local_openclaw_root()?;
    let root = clawpal_core::doctor::doctor_domain_local_root(&openclaw_root, domain)?;
    let rel = match path {
        Some(p) => p.to_string(),
        None => match domain {
            "sessions" => {
                let abs = clawpal_core::doctor::resolve_local_sessions_path(&openclaw_root);
                clawpal_core::doctor::relpath_from_local_abs(&root, &abs).ok_or_else(|| {
                    format!(
                        "failed to resolve sessions path under domain root: {}",
                        root.display()
                    )
                })?
            }
            _ => clawpal_core::doctor::doctor_domain_default_relpath(domain)
                .ok_or_else(|| "doctor file write requires --path for this domain".to_string())?
                .to_string(),
        },
    };
    clawpal_core::doctor::validate_doctor_relative_path(&rel)?;
    let full_path = root.join(&rel);
    validate_not_sensitive(&full_path.to_string_lossy())?;
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("failed to create parent dir: {e}"))?;
    }
    if backup && full_path.exists() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backup_path = full_path.with_extension(format!(
            "{}bak.{ts}",
            full_path
                .extension()
                .map(|ext| format!("{}.", ext.to_string_lossy()))
                .unwrap_or_default(),
        ));
        std::fs::copy(&full_path, backup_path)
            .map_err(|e| format!("failed to create backup file: {e}"))?;
    }
    std::fs::write(&full_path, content).map_err(|e| format!("failed to write file: {e}"))?;
    let verify = std::fs::read_to_string(&full_path)
        .map_err(|e| format!("failed to verify written file: {e}"))?;
    if verify != content {
        return Err("doctor file write verification failed: local content mismatch".to_string());
    }
    Ok(json!({
        "target": target,
        "remote": false,
        "domain": domain,
        "root": root.to_string_lossy(),
        "path": rel,
        "fullPath": full_path.to_string_lossy(),
        "written": true,
        "backup": backup,
    }))
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
        let config_path = resolve_remote_config_path(pool, target).await?;
        let raw = pool.sftp_read(target, &config_path).await?;
        let (rendered, deleted) = clawpal_core::doctor::delete_json_path_in_str(
            &raw,
            dotted_path,
            "remote config",
            "config",
        )?;
        if deleted {
            pool.sftp_write(target, &config_path, &rendered).await?;
        }
        return Ok(json!({
            "target": target,
            "remote": true,
            "configPath": config_path,
            "path": dotted_path,
            "deleted": deleted,
        }));
    }

    let config_path = clawpal_core::doctor::local_openclaw_config_path(&local_openclaw_root()?);
    let raw = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("failed to read local config: {e}"))?;
    let (rendered, deleted) =
        clawpal_core::doctor::delete_json_path_in_str(&raw, dotted_path, "local config", "config")?;
    if deleted {
        std::fs::write(&config_path, rendered)
            .map_err(|e| format!("failed to write local config: {e}"))?;
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
        let config_path = resolve_remote_config_path(pool, target).await?;
        let raw = pool.sftp_read(target, &config_path).await?;
        let selected =
            clawpal_core::doctor::select_json_value_from_str(&raw, dotted_path, "remote config")?;
        return Ok(json!({
            "target": target,
            "remote": true,
            "configPath": config_path,
            "path": dotted_path.unwrap_or(""),
            "value": selected,
        }));
    }

    let config_path = clawpal_core::doctor::local_openclaw_config_path(&local_openclaw_root()?);
    let raw = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("failed to read local config: {e}"))?;
    let selected =
        clawpal_core::doctor::select_json_value_from_str(&raw, dotted_path, "local config")?;
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
    let next_value =
        clawpal_core::doctor::parse_json_value_arg(value_json, "doctor config-upsert")?;
    if target_is_remote_instance(target) {
        let config_path = resolve_remote_config_path(pool, target).await?;
        let raw = pool.sftp_read(target, &config_path).await?;
        let rendered = clawpal_core::doctor::upsert_json_path_in_str(
            &raw,
            dotted_path,
            next_value.clone(),
            "remote config",
            "config",
        )?;
        pool.sftp_write(target, &config_path, &rendered).await?;
        return Ok(json!({
            "target": target,
            "remote": true,
            "configPath": config_path,
            "path": dotted_path,
            "upserted": true,
        }));
    }

    let config_path = clawpal_core::doctor::local_openclaw_config_path(&local_openclaw_root()?);
    let raw = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("failed to read local config: {e}"))?;
    let rendered = clawpal_core::doctor::upsert_json_path_in_str(
        &raw,
        dotted_path,
        next_value,
        "local config",
        "config",
    )?;
    std::fs::write(&config_path, rendered)
        .map_err(|e| format!("failed to write local config: {e}"))?;
    Ok(json!({
        "target": target,
        "remote": false,
        "configPath": config_path.to_string_lossy(),
        "path": dotted_path,
        "upserted": true,
    }))
}

async fn resolve_remote_sessions_path(
    pool: &SshConnectionPool,
    target: &str,
) -> Result<String, String> {
    let out = pool
        .exec_login(
            target,
            clawpal_core::doctor::remote_sessions_discovery_script(),
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
        let selected =
            clawpal_core::doctor::select_json_value_from_str(&raw, dotted_path, "remote sessions")?;
        return Ok(json!({
            "target": target,
            "remote": true,
            "sessionsPath": sessions_path,
            "path": dotted_path.unwrap_or(""),
            "value": selected,
        }));
    }

    let sessions_path = clawpal_core::doctor::resolve_local_sessions_path(&local_openclaw_root()?);
    let raw = std::fs::read_to_string(&sessions_path)
        .map_err(|e| format!("failed to read local sessions: {e}"))?;
    let selected =
        clawpal_core::doctor::select_json_value_from_str(&raw, dotted_path, "local sessions")?;
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
    let next_value =
        clawpal_core::doctor::parse_json_value_arg(value_json, "doctor sessions-upsert")?;
    if target_is_remote_instance(target) {
        let sessions_path = resolve_remote_sessions_path(pool, target).await?;
        let raw = pool.sftp_read(target, &sessions_path).await?;
        let rendered = clawpal_core::doctor::upsert_json_path_in_str(
            &raw,
            dotted_path,
            next_value.clone(),
            "remote sessions",
            "sessions",
        )?;
        pool.sftp_write(target, &sessions_path, &rendered).await?;
        return Ok(json!({
            "target": target,
            "remote": true,
            "sessionsPath": sessions_path,
            "path": dotted_path,
            "upserted": true,
        }));
    }

    let sessions_path = clawpal_core::doctor::resolve_local_sessions_path(&local_openclaw_root()?);
    let raw = std::fs::read_to_string(&sessions_path)
        .map_err(|e| format!("failed to read local sessions: {e}"))?;
    let rendered = clawpal_core::doctor::upsert_json_path_in_str(
        &raw,
        dotted_path,
        next_value,
        "local sessions",
        "sessions",
    )?;
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
        let (rendered, deleted) = clawpal_core::doctor::delete_json_path_in_str(
            &raw,
            dotted_path,
            "remote sessions",
            "sessions",
        )?;
        if deleted {
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

    let sessions_path = clawpal_core::doctor::resolve_local_sessions_path(&local_openclaw_root()?);
    let raw = std::fs::read_to_string(&sessions_path)
        .map_err(|e| format!("failed to read local sessions: {e}"))?;
    let (rendered, deleted) = clawpal_core::doctor::delete_json_path_in_str(
        &raw,
        dotted_path,
        "local sessions",
        "sessions",
    )?;
    if deleted {
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
    "doctor probe-openclaw [--instance <id>]",
    "doctor fix-openclaw-path [--instance <id>]",
    "doctor file read --domain <config|sessions|logs|state> [--path <relpath>] [--instance <id>]",
    "doctor file write --domain <config|sessions|logs|state> [--path <relpath>] --content <text> [--backup] [--instance <id>]",
    "doctor config-read [<json.path>] [--instance <id>]",
    "doctor config-upsert <json.path> <json.value> [--instance <id>]",
    "doctor config-delete <json.path> [--instance <id>]",
    "doctor sessions-read [<json.path>] [--instance <id>]",
    "doctor sessions-upsert <json.path> <json.value> [--instance <id>]",
    "doctor sessions-delete <json.path> [--instance <id>]",
    "doctor exec --tool <command> [--args <argstring>] [--instance <id>]",
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

    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("probe-openclaw")
    {
        let parsed = parse_cli_args(&token_refs[2..]);
        let target = resolved_target(target, &parsed)?;
        let probed = probe_openclaw_on_target(pool, target).await?;
        return tool_stdout_json(probed);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("fix-openclaw-path")
    {
        let parsed = parse_cli_args(&token_refs[2..]);
        let target = resolved_target(target, &parsed)?;
        let fixed = fix_openclaw_path_on_target(pool, target).await?;
        return tool_stdout_json(fixed);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("file")
        && token_refs.get(2).copied() == Some("read")
    {
        let parsed = parse_cli_args(&token_refs[3..]);
        let target = resolved_target(target, &parsed)?;
        let domain = parsed
            .options
            .get("domain")
            .and_then(|v| v.as_deref())
            .ok_or_else(|| "clawpal doctor file read requires --domain".to_string())?;
        let path = parsed.options.get("path").and_then(|v| v.as_deref());
        let out = doctor_file_read(pool, target, domain, path).await?;
        return tool_stdout_json(out);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("file")
        && token_refs.get(2).copied() == Some("write")
    {
        let parsed = parse_cli_args(&token_refs[3..]);
        let target = resolved_target(target, &parsed)?;
        let domain = parsed
            .options
            .get("domain")
            .and_then(|v| v.as_deref())
            .ok_or_else(|| "clawpal doctor file write requires --domain".to_string())?;
        let path = parsed.options.get("path").and_then(|v| v.as_deref());
        let content = parsed
            .options
            .get("content")
            .and_then(|v| v.as_deref())
            .ok_or_else(|| "clawpal doctor file write requires --content".to_string())?;
        let backup = parsed.options.contains_key("backup");
        let out = doctor_file_write(pool, target, domain, path, content, backup).await?;
        return tool_stdout_json(out);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("config-read")
    {
        let parsed = parse_cli_args(&token_refs[2..]);
        let target = resolved_target(target, &parsed)?;
        let path = parsed.positionals.first().map(String::as_str);
        let out = doctor_config_read(pool, target, path).await?;
        return tool_stdout_json(out);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("config-upsert")
    {
        let parsed = parse_cli_args(&token_refs[2..]);
        let target = resolved_target(target, &parsed)?;
        let key_path = parsed
            .positionals
            .first()
            .map(String::as_str)
            .ok_or_else(|| {
                "clawpal doctor config-upsert requires <json.path> <json.value>".to_string()
            })?;
        let value_json = parsed
            .positionals
            .get(1)
            .map(String::as_str)
            .ok_or_else(|| {
                "clawpal doctor config-upsert requires <json.path> <json.value>".to_string()
            })?;
        let out = doctor_config_upsert(pool, target, key_path, value_json).await?;
        return tool_stdout_json(out);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("config-delete")
    {
        let parsed = parse_cli_args(&token_refs[2..]);
        let target = resolved_target(target, &parsed)?;
        let key_path = parsed
            .positionals
            .first()
            .map(String::as_str)
            .ok_or_else(|| "clawpal doctor config-delete requires <json.path>".to_string())?;
        let out = doctor_config_delete(pool, target, key_path).await?;
        return tool_stdout_json(out);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("sessions-read")
    {
        let parsed = parse_cli_args(&token_refs[2..]);
        let target = resolved_target(target, &parsed)?;
        let path = parsed.positionals.first().map(String::as_str);
        let out = doctor_sessions_read(pool, target, path).await?;
        return tool_stdout_json(out);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("sessions-upsert")
    {
        let parsed = parse_cli_args(&token_refs[2..]);
        let target = resolved_target(target, &parsed)?;
        let key_path = parsed
            .positionals
            .first()
            .map(String::as_str)
            .ok_or_else(|| {
                "clawpal doctor sessions-upsert requires <json.path> <json.value>".to_string()
            })?;
        let value_json = parsed
            .positionals
            .get(1)
            .map(String::as_str)
            .ok_or_else(|| {
                "clawpal doctor sessions-upsert requires <json.path> <json.value>".to_string()
            })?;
        let out = doctor_sessions_upsert(pool, target, key_path, value_json).await?;
        return tool_stdout_json(out);
    }
    if token_refs.first().copied() == Some("doctor")
        && token_refs.get(1).copied() == Some("sessions-delete")
    {
        let parsed = parse_cli_args(&token_refs[2..]);
        let target = resolved_target(target, &parsed)?;
        let key_path = parsed
            .positionals
            .first()
            .map(String::as_str)
            .ok_or_else(|| "clawpal doctor sessions-delete requires <json.path>".to_string())?;
        let out = doctor_sessions_delete(pool, target, key_path).await?;
        return tool_stdout_json(out);
    }
    if token_refs.first().copied() == Some("doctor") && token_refs.get(1).copied() == Some("exec") {
        let parsed = parse_cli_args(&token_refs[2..]);
        let target = resolved_target(target, &parsed)?;
        let tool = parsed
            .options
            .get("tool")
            .and_then(|v| v.as_deref())
            .ok_or_else(|| "clawpal doctor exec requires --tool".to_string())?;
        let tool_args = parsed
            .options
            .get("args")
            .and_then(|v| v.as_deref())
            .unwrap_or("");
        let out = run_doctor_exec_tool(pool, tool, tool_args, target).await?;
        return Ok(out);
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
                    let status = clawpal_core::health::check_instance(&instance)
                        .map_err(|e| e.to_string())?;
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
            let status =
                clawpal_core::health::check_instance(&instance).map_err(|e| e.to_string())?;
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
            let removed =
                clawpal_core::profile::delete_profile(&openclaw, id).map_err(|e| e.to_string())?;
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
            let saved = clawpal_core::profile::upsert_profile(&openclaw, profile)
                .map_err(|e| e.to_string())?;
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
            let instance = clawpal_core::connect::connect_docker(&home, label, None)
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
                passphrase: None,
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
                home: parsed.options.get("home").and_then(|v| v.as_ref()).cloned(),
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

async fn run_doctor_exec_tool(
    pool: &SshConnectionPool,
    tool: &str,
    raw_args: &str,
    target: &str,
) -> Result<Value, String> {
    let command = tool.trim();
    if command.is_empty() {
        return Err("doctor exec --tool cannot be empty".to_string());
    }
    let raw = raw_args.trim();
    let tokens = if raw.is_empty() {
        Vec::new()
    } else {
        parse_tool_tokens(raw)?
    };

    if target_is_remote_instance(target) {
        let mut cmd = sh_single_quote(command);
        for token in &tokens {
            cmd.push(' ');
            cmd.push_str(&sh_single_quote(token));
        }
        let out = pool.exec_login(target, &cmd).await?;
        return Ok(json!({
            "stdout": out.stdout,
            "stderr": out.stderr,
            "exitCode": out.exit_code,
        }));
    }

    let output = std::process::Command::new(command)
        .args(tokens.iter())
        .output()
        .map_err(|e| format!("failed to execute '{command}': {e}"))?;
    Ok(json!({
        "stdout": String::from_utf8_lossy(&output.stdout).to_string(),
        "stderr": String::from_utf8_lossy(&output.stderr).to_string(),
        "exitCode": output.status.code().unwrap_or(-1),
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
    use crate::openclaw_doc_resolver::{
        DocCitation, DocGuidance, DocResolveRequest, ResolverMeta, RootCauseHypothesis,
    };
    use std::collections::BTreeSet;
    use std::io::Write;

    fn prompt_supported_clawpal_commands_from(prompt: &str) -> BTreeSet<String> {
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

    fn doctor_prompt_supported_clawpal_commands() -> BTreeSet<String> {
        let prompt = crate::prompt_templates::doctor_domain_system();
        prompt_supported_clawpal_commands_from(&prompt)
    }

    fn install_prompt_supported_clawpal_commands() -> BTreeSet<String> {
        let prompt = crate::prompt_templates::install_domain_system();
        prompt_supported_clawpal_commands_from(&prompt)
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
    fn resolved_target_rejects_unknown_instance_id() {
        let parsed = ParsedCliArgs {
            positionals: Vec::new(),
            options: HashMap::from([("instance".to_string(), Some("ssh:not-exist".to_string()))]),
        };
        let result = resolved_target("local", &parsed);
        assert!(result.is_err());
        assert!(result
            .err()
            .unwrap_or_default()
            .contains("unknown instance id"));
    }

    #[test]
    fn resolved_target_allows_local_and_docker_targets() {
        let parsed_local = ParsedCliArgs {
            positionals: Vec::new(),
            options: HashMap::from([("instance".to_string(), Some("local".to_string()))]),
        };
        assert_eq!(
            resolved_target("ssh:vm1", &parsed_local).expect("local target"),
            "local"
        );

        let parsed_docker = ParsedCliArgs {
            positionals: Vec::new(),
            options: HashMap::from([("instance".to_string(), Some("docker:local".to_string()))]),
        };
        assert_eq!(
            resolved_target("ssh:vm1", &parsed_docker).expect("docker target"),
            "docker:local"
        );
    }

    #[test]
    fn parse_cli_args_supports_space_containing_option_values() {
        let tokens =
            parse_tool_tokens("connect docker --home \"/tmp/a b\" --label \"Docker Local\"")
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
    fn parse_cli_args_supports_doctor_instance_override() {
        let tokens = parse_tool_tokens("doctor config-read commands --instance ssh:vm1")
            .expect("parse tokens");
        let token_refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        let parsed = parse_cli_args(&token_refs[2..]);
        assert_eq!(
            parsed.positionals.first().map(String::as_str),
            Some("commands")
        );
        assert_eq!(
            resolved_target("local", &parsed).expect("resolve target"),
            "ssh:vm1"
        );
    }

    #[test]
    fn describe_invoke_appends_args_when_present() {
        let args = json!({
            "args": "doctor --fix",
            "instance": "c7c90e52-bbc7-44be-bfe7-a07302646435"
        });
        assert_eq!(describe_invoke("openclaw", &args), "openclaw doctor --fix");
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
        assert!(clawpal_core::doctor::delete_json_path(
            &mut doc,
            "commands.ownerDisplay"
        ));
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
        clawpal_core::doctor::upsert_json_path(&mut doc, "commands.ownerDisplay", json!("raw"))
            .expect("upsert");
        assert_eq!(doc["commands"]["ownerDisplay"], "raw");
        assert_eq!(doc["commands"]["other"], 1);
    }

    #[test]
    fn doctor_prompt_supported_commands_match_backend_list() {
        let prompt_commands = doctor_prompt_supported_clawpal_commands();
        let backend_commands = DOCTOR_SUPPORTED_CLAWPAL_COMMANDS
            .iter()
            .map(|v| v.to_string())
            .collect::<BTreeSet<_>>();
        assert_eq!(prompt_commands, backend_commands);
    }

    #[test]
    fn install_prompt_supported_commands_match_backend_list() {
        let prompt_commands = install_prompt_supported_clawpal_commands();
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
            prompt.contains(
                "NEVER invent non-existent clawpal commands (for example: doctor fix-config)."
            ),
            "prompt should explicitly forbid legacy doctor fix-config command"
        );
    }

    #[test]
    fn handle_runtime_send_result_propagates_error() {
        let err = crate::runtime::types::RuntimeError {
            code: crate::runtime::types::RuntimeErrorCode::SessionInvalid,
            message: "session expired".to_string(),
            action_hint: Some("restart doctor".to_string()),
        };
        let result = handle_runtime_send_result("doctor", Err(err.clone()));
        assert!(result.is_err());
        let got = result.err().expect("error expected");
        assert_eq!(got.code.as_str(), err.code.as_str());
        assert_eq!(got.message, err.message);
    }

    #[test]
    fn read_file_tail_lines_reads_last_lines() {
        let path =
            std::env::temp_dir().join(format!("clawpal-doctor-tail-{}.log", std::process::id()));
        let mut file = std::fs::File::create(&path).expect("create temp file");
        writeln!(file, "l1").expect("write line 1");
        writeln!(file, "l2").expect("write line 2");
        writeln!(file, "l3").expect("write line 3");
        drop(file);
        let content = read_file_tail_lines(&path, 2).expect("read tail");
        let _ = std::fs::remove_file(&path);
        assert_eq!(content, "l2\nl3");
    }

    fn fake_exec_result(stdout: &str, stderr: &str, exit_code: u32) -> crate::ssh::SshExecResult {
        crate::ssh::SshExecResult {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
        }
    }

    fn sample_doc_guidance() -> DocGuidance {
        DocGuidance {
            status: "ok".to_string(),
            source_strategy: "local-first".to_string(),
            root_cause_hypotheses: vec![RootCauseHypothesis {
                title: "Gateway mismatch".to_string(),
                reason: "Gateway process and status are inconsistent".to_string(),
                score: 0.88,
            }],
            fix_steps: vec!["Run openclaw gateway status".to_string()],
            confidence: 0.88,
            citations: vec![DocCitation {
                url: "https://docs.openclaw.ai/cli/gateway".to_string(),
                section: "Gateway".to_string(),
            }],
            version_awareness: "aligned".to_string(),
            resolver_meta: ResolverMeta {
                cache_hit: false,
                sources_checked: vec!["target-remote-docs".to_string()],
                rules_matched: vec!["gateway_connectivity".to_string()],
                fetched_pages: 1,
                fallback_used: false,
            },
        }
    }

    struct FakeRemoteContextOps {
        config_path_result: Result<String, String>,
        sftp_read_result: Result<String, String>,
        exec_login_results: Mutex<VecDeque<Result<crate::ssh::SshExecResult, String>>>,
        exec_results: Mutex<VecDeque<Result<crate::ssh::SshExecResult, String>>>,
    }

    impl FakeRemoteContextOps {
        fn next_exec_login(&self) -> Result<crate::ssh::SshExecResult, String> {
            self.exec_login_results
                .lock()
                .expect("lock exec_login queue")
                .pop_front()
                .unwrap_or_else(|| Err("missing fake exec_login result".to_string()))
        }

        fn next_exec(&self) -> Result<crate::ssh::SshExecResult, String> {
            self.exec_results
                .lock()
                .expect("lock exec queue")
                .pop_front()
                .unwrap_or_else(|| Err("missing fake exec result".to_string()))
        }
    }

    impl RemoteDoctorContextOps for FakeRemoteContextOps {
        fn exec_login<'a>(&'a self, _host_id: &'a str, _command: &'a str) -> RemoteExecFuture<'a> {
            let result = self.next_exec_login();
            Box::pin(async move { result })
        }

        fn exec<'a>(&'a self, _host_id: &'a str, _command: &'a str) -> RemoteExecFuture<'a> {
            let result = self.next_exec();
            Box::pin(async move { result })
        }

        fn sftp_read<'a>(&'a self, _host_id: &'a str, _path: &'a str) -> RemoteStringFuture<'a> {
            let result = self.sftp_read_result.clone();
            Box::pin(async move { result })
        }

        fn resolve_config_path<'a>(&'a self, _host_id: &'a str) -> RemoteStringFuture<'a> {
            let result = self.config_path_result.clone();
            Box::pin(async move { result })
        }
    }

    struct FakeDocGuidanceResolver {
        guidance: DocGuidance,
        host_ids: Mutex<Vec<String>>,
        requests: Mutex<Vec<DocResolveRequest>>,
    }

    impl FakeDocGuidanceResolver {
        fn new(guidance: DocGuidance) -> Self {
            Self {
                guidance,
                host_ids: Mutex::new(Vec::new()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn last_host_id(&self) -> Option<String> {
            self.host_ids.lock().expect("lock host ids").last().cloned()
        }

        fn last_request(&self) -> Option<DocResolveRequest> {
            self.requests.lock().expect("lock requests").last().cloned()
        }
    }

    impl RemoteDocGuidanceResolver for FakeDocGuidanceResolver {
        fn resolve_remote_guidance<'a>(
            &'a self,
            host_id: &'a str,
            request: &'a DocResolveRequest,
            _paths: &'a crate::models::OpenClawPaths,
        ) -> RemoteDocGuidanceFuture<'a> {
            self.host_ids
                .lock()
                .expect("lock host ids")
                .push(host_id.to_string());
            self.requests
                .lock()
                .expect("lock requests")
                .push(request.clone());
            let guidance = self.guidance.clone();
            Box::pin(async move { guidance })
        }
    }

    #[tokio::test]
    async fn collect_remote_context_with_mock_ops_includes_doc_guidance() {
        let ops = FakeRemoteContextOps {
            config_path_result: Ok("~/.openclaw/config.json".to_string()),
            sftp_read_result: Ok("{\"channels\":{}}".to_string()),
            exec_login_results: Mutex::new(VecDeque::from(vec![
                Ok(fake_exec_result("2026.3.0\n", "", 0)),
                Ok(fake_exec_result("gateway: running\n", "", 0)),
                Ok(fake_exec_result("error log line\n", "", 0)),
            ])),
            exec_results: Mutex::new(VecDeque::from(vec![
                Ok(fake_exec_result("", "", 0)),
                Ok(fake_exec_result("Linux\n", "", 0)),
                Ok(fake_exec_result("x86_64\n", "", 0)),
            ])),
        };
        let resolver = FakeDocGuidanceResolver::new(sample_doc_guidance());

        let context = collect_doctor_context_remote_with_ops(&ops, &resolver, "ssh:edge-1").await;
        assert!(context.is_ok());
        let parsed: Value = serde_json::from_str(&context.expect("remote context")).expect("json");

        assert_eq!(parsed["openclawVersion"], "2026.3.0");
        assert_eq!(parsed["configPath"], "~/.openclaw/config.json");
        assert_eq!(parsed["configContent"], "{\"channels\":{}}");
        assert_eq!(parsed["gatewayStatus"], "gateway: running");
        assert_eq!(parsed["gatewayProcessRunning"], true);
        assert_eq!(parsed["platform"], "linux");
        assert_eq!(parsed["arch"], "x86_64");
        assert_eq!(parsed["remote"], true);
        assert_eq!(parsed["hostId"], "ssh:edge-1");
        assert_eq!(parsed["docGuidance"]["status"], "ok");
        assert_eq!(resolver.last_host_id().as_deref(), Some("ssh:edge-1"));

        let request = resolver.last_request().expect("captured doc request");
        assert_eq!(request.instance_scope, "ssh:edge-1");
        assert_eq!(request.transport, "remote_ssh");
        assert_eq!(request.openclaw_version.as_deref(), Some("2026.3.0"));
        assert_eq!(request.config_content, "{\"channels\":{}}");
        assert_eq!(request.error_log, "error log line\n");
        assert_eq!(request.gateway_status.as_deref(), Some("gateway: running"));
    }

    #[tokio::test]
    async fn collect_remote_context_with_mock_ops_rejects_sensitive_config_path() {
        let ops = FakeRemoteContextOps {
            config_path_result: Ok("/home/dev/.ssh/id_rsa".to_string()),
            sftp_read_result: Ok("{}".to_string()),
            exec_login_results: Mutex::new(VecDeque::from(vec![Ok(fake_exec_result(
                "2026.3.0\n",
                "",
                0,
            ))])),
            exec_results: Mutex::new(VecDeque::new()),
        };
        let resolver = FakeDocGuidanceResolver::new(sample_doc_guidance());

        let err = collect_doctor_context_remote_with_ops(&ops, &resolver, "ssh:edge-1")
            .await
            .expect_err("sensitive path should fail");
        assert!(err.contains("blocked"));
        assert!(resolver.last_request().is_none());
    }

    #[tokio::test]
    async fn collect_remote_context_with_mock_ops_falls_back_when_config_read_fails() {
        let ops = FakeRemoteContextOps {
            config_path_result: Ok("~/.openclaw/config.json".to_string()),
            sftp_read_result: Err("read failed".to_string()),
            exec_login_results: Mutex::new(VecDeque::from(vec![
                Ok(fake_exec_result("2026.3.0\n", "", 0)),
                Ok(fake_exec_result("gateway: stopped\n", "", 0)),
                Ok(fake_exec_result("", "", 0)),
            ])),
            exec_results: Mutex::new(VecDeque::from(vec![
                Ok(fake_exec_result("", "", 1)),
                Ok(fake_exec_result("Darwin\n", "", 0)),
                Ok(fake_exec_result("arm64\n", "", 0)),
            ])),
        };
        let resolver = FakeDocGuidanceResolver::new(sample_doc_guidance());

        let context = collect_doctor_context_remote_with_ops(&ops, &resolver, "ssh:edge-2")
            .await
            .expect("remote context");
        let parsed: Value = serde_json::from_str(&context).expect("json");
        assert_eq!(parsed["configContent"], "(unable to read remote config)");
        assert_eq!(parsed["gatewayProcessRunning"], false);
        assert_eq!(parsed["platform"], "darwin");

        let request = resolver.last_request().expect("captured doc request");
        assert_eq!(request.config_content, "(unable to read remote config)");
    }

    #[tokio::test]
    async fn ssh_pool_remote_context_ops_forward_methods_return_errors_without_connection() {
        let pool = SshConnectionPool::new();
        let ops = SshPoolRemoteDoctorContextOps { pool: &pool };

        let login_err = RemoteDoctorContextOps::exec_login(&ops, "ssh:missing", "echo ok")
            .await
            .expect_err("expected missing connection for exec_login");
        assert!(login_err.contains("No connection"));

        let exec_err = RemoteDoctorContextOps::exec(&ops, "ssh:missing", "echo ok")
            .await
            .expect_err("expected missing connection for exec");
        assert!(exec_err.contains("No connection"));

        let read_err =
            RemoteDoctorContextOps::sftp_read(&ops, "ssh:missing", "~/.openclaw/config.toml")
                .await
                .expect_err("expected missing connection for sftp_read");
        assert!(read_err.contains("No connection"));

        let resolve_err = RemoteDoctorContextOps::resolve_config_path(&ops, "ssh:missing")
            .await
            .expect_err("expected missing connection for config path resolution");
        assert!(resolve_err.contains("No connection"));
    }
}
