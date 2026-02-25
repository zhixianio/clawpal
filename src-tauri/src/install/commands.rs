use super::session_store::InstallSessionStore;
use super::runners;
use super::types::{
    InstallMethod, InstallMethodCapability, InstallSession, InstallState, InstallStep,
    InstallStepResult,
};
use crate::ssh::SshConnectionPool;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use tauri::State;
use uuid::Uuid;

static TEST_SESSION_STORE: LazyLock<InstallSessionStore> = LazyLock::new(InstallSessionStore::new);

fn parse_method(raw: &str) -> Result<InstallMethod, String> {
    match raw {
        "local" => Ok(InstallMethod::Local),
        "wsl2" => Ok(InstallMethod::Wsl2),
        "docker" => Ok(InstallMethod::Docker),
        "remote_ssh" => Ok(InstallMethod::RemoteSsh),
        _ => Err(format!("unsupported install method: {raw}")),
    }
}

fn parse_step(raw: &str) -> Result<InstallStep, String> {
    match raw {
        "precheck" => Ok(InstallStep::Precheck),
        "install" => Ok(InstallStep::Install),
        "init" => Ok(InstallStep::Init),
        "verify" => Ok(InstallStep::Verify),
        _ => Err(format!("unsupported install step: {raw}")),
    }
}

fn create_session(
    store: &InstallSessionStore,
    method_raw: &str,
    options: Option<HashMap<String, Value>>,
) -> Result<InstallSession, String> {
    let method = parse_method(method_raw)?;
    let now = Utc::now().to_rfc3339();
    let session = InstallSession {
        id: format!("install-{}", Uuid::new_v4()),
        method,
        state: InstallState::SelectedMethod,
        current_step: None,
        logs: vec![],
        artifacts: options.unwrap_or_default(),
        created_at: now.clone(),
        updated_at: now,
    };
    store.insert(session.clone())?;
    Ok(session)
}

fn is_step_allowed(state: &InstallState, step: &InstallStep) -> bool {
    match step {
        InstallStep::Precheck => matches!(state, InstallState::SelectedMethod | InstallState::PrecheckFailed),
        InstallStep::Install => matches!(state, InstallState::PrecheckPassed | InstallState::InstallFailed),
        InstallStep::Init => matches!(state, InstallState::InstallPassed | InstallState::InitFailed),
        InstallStep::Verify => matches!(state, InstallState::InitPassed),
    }
}

fn running_state(step: &InstallStep) -> InstallState {
    match step {
        InstallStep::Precheck => InstallState::PrecheckRunning,
        InstallStep::Install => InstallState::InstallRunning,
        InstallStep::Init => InstallState::InitRunning,
        InstallStep::Verify => InstallState::InitPassed,
    }
}

fn success_state(step: &InstallStep) -> InstallState {
    match step {
        InstallStep::Precheck => InstallState::PrecheckPassed,
        InstallStep::Install => InstallState::InstallPassed,
        InstallStep::Init => InstallState::InitPassed,
        InstallStep::Verify => InstallState::Ready,
    }
}

fn failed_state(step: &InstallStep) -> InstallState {
    match step {
        InstallStep::Precheck => InstallState::PrecheckFailed,
        InstallStep::Install => InstallState::InstallFailed,
        InstallStep::Init => InstallState::InitFailed,
        InstallStep::Verify => InstallState::InitPassed,
    }
}

fn next_step(step: &InstallStep) -> Option<String> {
    match step {
        InstallStep::Precheck => Some("install".to_string()),
        InstallStep::Install => Some("init".to_string()),
        InstallStep::Init => Some("verify".to_string()),
        InstallStep::Verify => None,
    }
}

fn next_step_from_state(state: &InstallState) -> Option<String> {
    match state {
        InstallState::SelectedMethod | InstallState::PrecheckFailed => Some("precheck".to_string()),
        InstallState::PrecheckPassed | InstallState::InstallFailed => Some("install".to_string()),
        InstallState::InstallPassed | InstallState::InitFailed => Some("init".to_string()),
        InstallState::InitPassed => Some("verify".to_string()),
        _ => None,
    }
}

fn make_result(
    ok: bool,
    summary: String,
    details: String,
    next: Option<String>,
    error_code: Option<String>,
) -> InstallStepResult {
    InstallStepResult {
        ok,
        summary,
        details,
        commands: vec![],
        artifacts: HashMap::<String, Value>::new(),
        next_step: next,
        error_code,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstallOrchestratorDecision {
    pub step: Option<String>,
    pub reason: String,
    pub source: String,
    pub error_code: Option<String>,
    pub action_hint: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct ExternalDeciderOutput {
    pub step: Option<String>,
    pub reason: Option<String>,
}

fn parse_decider_json(raw: &str) -> Result<ExternalDeciderOutput, String> {
    if let Ok(parsed) = serde_json::from_str::<ExternalDeciderOutput>(raw) {
        return Ok(parsed);
    }
    let start = raw.find('{').ok_or_else(|| "missing '{' in decider output".to_string())?;
    let end = raw.rfind('}').ok_or_else(|| "missing '}' in decider output".to_string())?;
    let slice = &raw[start..=end];
    serde_json::from_str::<ExternalDeciderOutput>(slice)
        .map_err(|e| format!("invalid decider json: {e}"))
}

fn classify_orchestrator_error(raw: &str) -> (String, String) {
    let lower = raw.to_lowercase();
    if lower.contains("no compatible api key found")
        || lower.contains("no auth profile")
        || lower.contains("openrouter_api_key")
        || lower.contains("anthropic_api_key")
        || lower.contains("openai_api_key")
    {
        return ("auth_missing".to_string(), "open_settings_auth".to_string());
    }
    if lower.contains("no ssh host config with id")
        || lower.contains("remote ssh host not found")
        || lower.contains("remote ssh target missing")
    {
        return ("remote_target_missing".to_string(), "open_instances".to_string());
    }
    if lower.contains("cannot connect to the docker daemon")
        || lower.contains("docker: command not found")
        || lower.contains("command failed: docker")
    {
        return ("docker_unavailable".to_string(), "open_doctor".to_string());
    }
    if lower.contains("permission denied") || lower.contains("operation not permitted") {
        return ("permission_denied".to_string(), "open_doctor".to_string());
    }
    if lower.contains("timed out")
        || lower.contains("network")
        || lower.contains("failed to connect")
        || lower.contains("temporary failure")
    {
        return ("network_error".to_string(), "open_doctor".to_string());
    }
    ("orchestrator_error".to_string(), "resume".to_string())
}

fn make_orchestrator_error_decision(reason: String, source: &str) -> InstallOrchestratorDecision {
    let (error_code, action_hint) = classify_orchestrator_error(&reason);
    InstallOrchestratorDecision {
        step: None,
        reason,
        source: source.to_string(),
        error_code: Some(error_code),
        action_hint: Some(action_hint),
    }
}

fn zeroclaw_config_dir() -> Result<PathBuf, String> {
    let dir = crate::models::resolve_paths().clawpal_dir.join("zeroclaw-sidecar");
    std::fs::create_dir_all(&dir).map_err(|e| format!("failed to create zeroclaw config dir: {e}"))?;
    Ok(dir)
}

fn run_stdin_decider(session: &InstallSession, goal: &str, cmd: &PathBuf) -> Result<InstallOrchestratorDecision, String> {
    let payload = serde_json::json!({
        "goal": goal,
        "sessionId": session.id,
        "method": session.method.as_str(),
        "state": session.state.as_str(),
        "artifacts": session.artifacts,
    });
    let mut child = Command::new(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start decider '{}': {e}", cmd.display()))?;
    if let Some(stdin) = child.stdin.as_mut() {
        let body = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;
        stdin
            .write_all(&body)
            .map_err(|e| format!("failed to write decider stdin: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed to wait decider: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "decider exited with code {:?}: {}",
            output.status.code(),
            stderr
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed = parse_decider_json(&stdout)?;
    Ok(InstallOrchestratorDecision {
        step: parsed.step,
        reason: parsed.reason.unwrap_or_else(|| "sidecar decider".to_string()),
        source: "zeroclaw-sidecar".to_string(),
        error_code: None,
        action_hint: None,
    })
}

fn decider_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "zeroclaw-decider.exe"
    } else {
        "zeroclaw-decider"
    }
}

fn platform_sidecar_dir_name() -> &'static str {
    if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "darwin-aarch64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "darwin-x64"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "linux-x64"
    } else if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        "windows-x64"
    } else {
        "unknown"
    }
}

fn decider_legacy_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "zeroclaw.exe"
    } else {
        "zeroclaw"
    }
}

fn resolve_decider_command_path() -> Option<PathBuf> {
    // Dev override (optional): allows testing custom decider command.
    if let Ok(raw) = std::env::var("CLAWPAL_ZEROCLAW_DECIDER") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if p.exists() {
                return Some(p);
            }
        }
    }

    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?.to_path_buf();
    let cwd = std::env::current_dir().ok()?;
    let bin_name = decider_file_name();
    let legacy_name = decider_legacy_file_name();
    let platform_dir = platform_sidecar_dir_name();
    let mut candidates: Vec<PathBuf> = vec![
        // Dev layouts: cwd may be repo root or src-tauri.
        cwd.join("src-tauri").join("resources").join("zeroclaw").join(bin_name),
        cwd.join("src-tauri")
            .join("resources")
            .join("zeroclaw")
            .join(platform_dir)
            .join(legacy_name),
        cwd.join("resources").join("zeroclaw").join(bin_name),
        cwd.join("resources")
            .join("zeroclaw")
            .join(platform_dir)
            .join(legacy_name),
        cwd.parent()
            .unwrap_or(&cwd)
            .join("src-tauri")
            .join("resources")
            .join("zeroclaw")
            .join(bin_name),
        cwd.parent()
            .unwrap_or(&cwd)
            .join("src-tauri")
            .join("resources")
            .join("zeroclaw")
            .join(platform_dir)
            .join(legacy_name),
        // macOS app bundle resource layout.
        exe_dir.join("../Resources/zeroclaw").join(bin_name),
        exe_dir
            .join("../Resources/zeroclaw")
            .join(platform_dir)
            .join(legacy_name),
        // Linux/Windows resource-adjacent layouts.
        exe_dir.join("resources").join("zeroclaw").join(bin_name),
        exe_dir
            .join("resources")
            .join("zeroclaw")
            .join(platform_dir)
            .join(legacy_name),
        // Co-located binary fallback.
        exe_dir.join(bin_name),
    ];
    candidates.dedup();
    candidates.into_iter().find(|p| p.exists())
}

fn run_zeroclaw_agent_decider(
    session: &InstallSession,
    goal: &str,
    cmd: &PathBuf,
) -> Result<InstallOrchestratorDecision, String> {
    let cfg = zeroclaw_config_dir()?;
    let cfg_arg = cfg.to_string_lossy().to_string();
    let env_pairs = zeroclaw_env_pairs_from_clawpal();
    if env_pairs.is_empty() {
        return Err(
            "No compatible API key found in ClawPal model profiles. zeroclaw currently supports: openrouter, openai, anthropic, openai-codex."
                .to_string(),
        );
    }
    let provider = pick_zeroclaw_provider(&env_pairs);
    let auth = Command::new(cmd)
        .envs(env_pairs.clone())
        .args(["--config-dir", &cfg_arg, "auth", "status"])
        .output()
        .map_err(|e| format!("failed to check zeroclaw auth status: {e}"))?;
    let auth_text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&auth.stdout),
        String::from_utf8_lossy(&auth.stderr)
    );
    if auth_text.contains("No auth profiles configured") && env_pairs.is_empty() {
        return Err("zeroclaw sidecar is not configured: no auth profile. Run zeroclaw onboard/auth first.".to_string());
    }

    let mut allowed_steps = Vec::<&str>::new();
    for candidate in ["precheck", "install", "init", "verify"] {
        let parsed = parse_step(candidate)?;
        if is_step_allowed(&session.state, &parsed) {
            allowed_steps.push(candidate);
        }
    }
    let prompt = format!(
        "You are install orchestrator. Goal: {goal}. Method: {}. State: {}. Allowed steps: {}. Return ONLY JSON object with fields: step (string or null), reason (string).",
        session.method.as_str(),
        session.state.as_str(),
        allowed_steps.join(",")
    );
    let mut base_args = vec![
        "--config-dir".to_string(),
        cfg_arg.clone(),
        "agent".to_string(),
        "-m".to_string(),
        prompt,
    ];
    let mut candidates = Vec::<String>::new();
    if let Some(p) = provider {
        base_args.push("-p".to_string());
        base_args.push(p.to_string());
        candidates = candidate_models_for_provider(p);
    }
    let mut last_error = String::new();
    let mut output_text: Option<String> = None;
    for model in candidates.into_iter() {
        let mut args = base_args.clone();
        args.push("--model".to_string());
        args.push(model.clone());
        let output = Command::new(cmd)
            .envs(env_pairs.clone())
            .args(args)
            .output()
            .map_err(|e| format!("failed to run zeroclaw agent: {e}"))?;
        let merged = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if output.status.success() {
            output_text = Some(String::from_utf8_lossy(&output.stdout).to_string());
            break;
        }
        let lower = merged.to_lowercase();
        last_error = merged;
        if lower.contains("not_found_error") || lower.contains("model:") {
            continue;
        }
        return Err(format!("zeroclaw agent failed: {}", last_error.trim()));
    }
    if output_text.is_none() {
        let output = Command::new(cmd)
            .envs(env_pairs)
            .args(base_args)
            .output()
            .map_err(|e| format!("failed to run zeroclaw agent: {e}"))?;
        let merged = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !output.status.success() {
            if !last_error.is_empty() {
                return Err(format!(
                    "zeroclaw agent failed after model retries. last error: {}",
                    last_error.trim()
                ));
            }
            return Err(format!("zeroclaw agent failed: {}", merged.trim()));
        }
        output_text = Some(String::from_utf8_lossy(&output.stdout).to_string());
    }
    let parsed = parse_decider_json(output_text.as_deref().unwrap_or(""))?;
    Ok(InstallOrchestratorDecision {
        step: parsed.step,
        reason: parsed
            .reason
            .unwrap_or_else(|| "zeroclaw agent decision".to_string()),
        source: "zeroclaw-sidecar".to_string(),
        error_code: None,
        action_hint: None,
    })
}

fn zeroclaw_env_pairs_from_clawpal() -> Vec<(String, String)> {
    let provider_keys = crate::commands::collect_provider_api_keys_for_internal();
    let mut out = Vec::<(String, String)>::new();
    for (provider, key) in provider_keys {
        match provider.as_str() {
            "openrouter" => out.push(("OPENROUTER_API_KEY".to_string(), key)),
            "openai" | "openai-codex" => out.push(("OPENAI_API_KEY".to_string(), key)),
            "anthropic" => out.push(("ANTHROPIC_API_KEY".to_string(), key)),
            "gemini" | "google" => out.push(("GEMINI_API_KEY".to_string(), key)),
            _ => {}
        }
    }
    out
}

fn pick_zeroclaw_provider(env_pairs: &[(String, String)]) -> Option<&'static str> {
    if env_pairs.iter().any(|(k, _)| k == "OPENROUTER_API_KEY") {
        return Some("openrouter");
    }
    if env_pairs.iter().any(|(k, _)| k == "OPENAI_API_KEY") {
        return Some("openai");
    }
    if env_pairs.iter().any(|(k, _)| k == "ANTHROPIC_API_KEY") {
        return Some("anthropic");
    }
    None
}

fn default_model_for_provider(provider: &str) -> Option<&'static str> {
    match provider {
        // Avoid openrouter-style model ids when talking to Anthropic directly.
        "anthropic" => Some("claude-3-5-sonnet-latest"),
        "openai" => Some("gpt-4o-mini"),
        "openrouter" => Some("anthropic/claude-3.5-sonnet"),
        _ => None,
    }
}

fn candidate_models_for_provider(provider: &str) -> Vec<String> {
    let mut out = Vec::<String>::new();
    if let Ok(profiles) = crate::commands::list_model_profiles() {
        for p in profiles.into_iter().filter(|p| p.enabled && p.provider.trim().eq_ignore_ascii_case(provider)) {
            let mut model = p.model.trim().to_string();
            if model.is_empty() {
                continue;
            }
            if provider != "openrouter" {
                if let Some((_, tail)) = model.split_once('/') {
                    model = tail.to_string();
                }
            }
            if !out.contains(&model) {
                out.push(model);
            }
        }
    }
    if let Some(default_model) = default_model_for_provider(provider) {
        let d = default_model.to_string();
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

fn run_external_decider(
    session: &InstallSession,
    goal: &str,
) -> Result<Option<InstallOrchestratorDecision>, String> {
    let Some(cmd) = resolve_decider_command_path() else {
        return Ok(None);
    };
    let is_stdin_decider = cmd
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with("zeroclaw-decider"))
        .unwrap_or(false);
    let decision = if is_stdin_decider {
        run_stdin_decider(session, goal, &cmd)?
    } else {
        run_zeroclaw_agent_decider(session, goal, &cmd)?
    };
    Ok(Some(decision))
}

fn orchestrator_next_internal(
    store: &InstallSessionStore,
    session_id: &str,
    goal: &str,
    allow_sidecar: bool,
) -> Result<InstallOrchestratorDecision, String> {
    let id = session_id.trim();
    if id.is_empty() {
        return Err("session_id is required".to_string());
    }
    let session = store
        .get(id)?
        .ok_or_else(|| format!("install session not found: {id}"))?;

    if allow_sidecar {
        let sidecar_decision = match run_external_decider(&session, goal) {
            Ok(v) => v,
            Err(err) => return Ok(make_orchestrator_error_decision(err, "error")),
        };
        if let Some(mut decision) = sidecar_decision {
            let inferred = next_step_from_state(&session.state);
            let decided_step = decision.step.clone();
            match decided_step {
                Some(step) => {
                    let parsed = match parse_step(&step) {
                        Ok(v) => v,
                        Err(_) => {
                            return Ok(make_orchestrator_error_decision(
                                format!("decider proposed unsupported step '{step}'"),
                                "error",
                            ))
                        }
                    };
                    if !is_step_allowed(&session.state, &parsed) {
                        if let Some(fixed) = inferred {
                            decision.step = Some(fixed.clone());
                            decision.reason = format!(
                                "{} | guardrail: replaced invalid step '{}' with '{}'",
                                decision.reason, step, fixed
                            );
                            decision.source = "zeroclaw-sidecar".to_string();
                            decision.error_code = None;
                            decision.action_hint = None;
                        } else {
                            return Ok(make_orchestrator_error_decision(
                                format!(
                                    "decider proposed invalid step '{step}' for state '{}'",
                                    session.state.as_str()
                                ),
                                "error",
                            ));
                        }
                    }
                }
                None => {
                    if let Some(fixed) = inferred {
                        decision.step = Some(fixed.clone());
                        decision.reason = format!(
                            "{} | guardrail: filled missing step with '{}'",
                            decision.reason, fixed
                        );
                        decision.source = "zeroclaw-sidecar".to_string();
                        decision.error_code = None;
                        decision.action_hint = None;
                    } else {
                        return Ok(make_orchestrator_error_decision(
                            "decider returned no step and no inferred fallback step is available"
                                .to_string(),
                            "error",
                        ));
                    }
                }
            }
            return Ok(decision);
        }
    }

    Ok(InstallOrchestratorDecision {
        step: next_step_from_state(&session.state),
        reason: format!("fallback by state '{}'", session.state.as_str()),
        source: "fallback".to_string(),
        error_code: None,
        action_hint: None,
    })
}

fn orchestrator_next(
    store: &InstallSessionStore,
    session_id: &str,
    goal: &str,
) -> Result<InstallOrchestratorDecision, String> {
    orchestrator_next_internal(store, session_id, goal, true)
}

fn append_executed_commands(session: &mut InstallSession, commands: &[String]) {
    if commands.is_empty() {
        return;
    }
    let key = "executed_commands".to_string();
    let next_values: Vec<Value> = commands
        .iter()
        .map(|cmd| Value::String(cmd.clone()))
        .collect();
    match session.artifacts.get_mut(&key) {
        Some(Value::Array(existing)) => {
            existing.extend(next_values);
        }
        _ => {
            session.artifacts.insert(key, Value::Array(next_values));
        }
    }
}

fn list_method_capabilities() -> Vec<InstallMethodCapability> {
    vec![
        InstallMethodCapability {
            method: "local".to_string(),
            available: true,
            hint: None,
        },
        InstallMethodCapability {
            method: "wsl2".to_string(),
            available: cfg!(target_os = "windows"),
            hint: Some("Requires WSL2 environment".to_string()),
        },
        InstallMethodCapability {
            method: "docker".to_string(),
            available: true,
            hint: Some("Requires Docker daemon to be running".to_string()),
        },
        InstallMethodCapability {
            method: "remote_ssh".to_string(),
            available: true,
            hint: Some("Requires reachable SSH host".to_string()),
        },
    ]
}

async fn run_remote_ssh_step(
    pool: &SshConnectionPool,
    host_id: &str,
    step: &InstallStep,
    artifacts: &HashMap<String, Value>,
) -> Result<runners::RunnerOutput, runners::RunnerFailure> {
    let status = if pool.is_connected(host_id).await {
        "connected".to_string()
    } else {
        "disconnected".to_string()
    };
    if status != "connected" {
        let hosts = crate::commands::list_ssh_hosts().map_err(|e| runners::RunnerFailure {
            error_code: "validation_failed".to_string(),
            summary: "remote ssh host lookup failed".to_string(),
            details: e,
            commands: vec![],
        })?;
        let host = hosts
            .into_iter()
            .find(|h| h.id == host_id)
            .ok_or_else(|| runners::RunnerFailure {
                error_code: "validation_failed".to_string(),
                summary: "remote ssh host not found".to_string(),
                details: format!("No SSH host config with id: {host_id}"),
                commands: vec![],
            })?;
        pool.connect(&host).await.map_err(|e| runners::RunnerFailure {
            error_code: runners::classify_error_code(&e),
            summary: "remote ssh connect failed".to_string(),
            details: e,
            commands: vec![format!("connect host {host_id}")],
        })?;
    }
    runners::remote_ssh::run_step(pool, host_id, step, artifacts).await
}

async fn run_step(
    store: &InstallSessionStore,
    pool: Option<&SshConnectionPool>,
    session_id_raw: &str,
    step_raw: &str,
) -> Result<InstallStepResult, String> {
    let session_id = session_id_raw.trim();
    if session_id.is_empty() {
        return Err("session_id is required".to_string());
    }

    let step = match parse_step(step_raw.trim()) {
        Ok(value) => value,
        Err(e) => {
            return Ok(make_result(
                false,
                "Install step rejected".to_string(),
                e,
                None,
                Some("validation_failed".to_string()),
            ))
        }
    };

    let mut session = match store.get(session_id)? {
        Some(value) => value,
        None => return Err(format!("install session not found: {session_id}")),
    };
    let method = session.method.clone();

    if !is_step_allowed(&session.state, &step) {
        session.state = failed_state(&step);
        session.updated_at = Utc::now().to_rfc3339();
        let blocked_state = session.state.as_str().to_string();
        store.upsert(session)?;
        return Ok(make_result(
            false,
            format!("{} blocked", step.as_str()),
            format!("Current state '{blocked_state}' does not allow this step"),
            None,
            Some("validation_failed".to_string()),
        ));
    }

    session.current_step = Some(step.clone());
    session.state = running_state(&step);
    session.updated_at = Utc::now().to_rfc3339();
    store.upsert(session.clone())?;

    let run_outcome = match method {
        InstallMethod::RemoteSsh => {
            let Some(host_id) = session
                .artifacts
                .get("ssh_host_id")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
            else {
                session.state = failed_state(&step);
                session.updated_at = Utc::now().to_rfc3339();
                store.upsert(session)?;
                return Ok(make_result(
                    false,
                    "remote ssh target missing".to_string(),
                    "Please select an existing remote instance before starting".to_string(),
                    None,
                    Some("validation_failed".to_string()),
                ));
            };
            let Some(pool) = pool else {
                session.state = failed_state(&step);
                session.updated_at = Utc::now().to_rfc3339();
                store.upsert(session)?;
                return Ok(make_result(
                    false,
                    "remote ssh unavailable".to_string(),
                    "SSH connection pool is unavailable".to_string(),
                    None,
                    Some("validation_failed".to_string()),
                ));
            };
            run_remote_ssh_step(pool, &host_id, &step, &session.artifacts).await
        }
        _ => runners::run_step(&method, &step, &session.artifacts),
    };
    match run_outcome {
        Ok(output) => {
            for (key, value) in &output.artifacts {
                session.artifacts.insert(key.clone(), value.clone());
            }
            append_executed_commands(&mut session, &output.commands);
            session.state = success_state(&step);
            session.updated_at = Utc::now().to_rfc3339();
            store.upsert(session)?;

            let mut result = make_result(
                true,
                output.summary,
                output.details,
                next_step(&step),
                None,
            );
            result.commands = output.commands;
            result.artifacts = output.artifacts;
            Ok(result)
        }
        Err(err) => {
            session.state = failed_state(&step);
            session.updated_at = Utc::now().to_rfc3339();
            store.upsert(session)?;

            let mut result = make_result(false, err.summary, err.details, None, Some(err.error_code));
            result.commands = err.commands;
            Ok(result)
        }
    }
}

#[tauri::command]
pub async fn install_create_session(
    method: String,
    options: Option<HashMap<String, Value>>,
    store: State<'_, InstallSessionStore>,
) -> Result<InstallSession, String> {
    create_session(&store, method.trim(), options)
}

#[tauri::command]
pub async fn install_get_session(
    session_id: String,
    store: State<'_, InstallSessionStore>,
) -> Result<InstallSession, String> {
    let id = session_id.trim();
    if id.is_empty() {
        return Err("session_id is required".to_string());
    }
    match store.get(id)? {
        Some(session) => Ok(session),
        None => Err(format!("install session not found: {id}")),
    }
}

#[tauri::command]
pub async fn install_run_step(
    session_id: String,
    step: String,
    pool: State<'_, SshConnectionPool>,
    store: State<'_, InstallSessionStore>,
) -> Result<InstallStepResult, String> {
    run_step(&store, Some(&pool), &session_id, &step).await
}

#[tauri::command]
pub async fn install_list_methods() -> Result<Vec<InstallMethodCapability>, String> {
    Ok(list_method_capabilities())
}

#[tauri::command]
pub async fn install_orchestrator_next(
    session_id: String,
    goal: String,
    store: State<'_, InstallSessionStore>,
) -> Result<InstallOrchestratorDecision, String> {
    orchestrator_next(&store, &session_id, &goal)
}

pub async fn create_session_for_test(method: &str) -> Result<InstallSession, String> {
    create_session(&TEST_SESSION_STORE, method, None)
}

pub async fn get_session_for_test(session_id: &str) -> Result<InstallSession, String> {
    let id = session_id.trim();
    if id.is_empty() {
        return Err("session_id is required".to_string());
    }
    TEST_SESSION_STORE
        .get(id)?
        .ok_or_else(|| format!("install session not found: {id}"))
}

pub async fn run_step_for_test(session_id: &str, step: &str) -> Result<InstallStepResult, String> {
    run_step(&TEST_SESSION_STORE, None, session_id, step).await
}

pub async fn list_methods_for_test() -> Result<Vec<InstallMethodCapability>, String> {
    Ok(list_method_capabilities())
}

pub async fn orchestrator_next_for_test(
    session_id: &str,
    goal: &str,
) -> Result<InstallOrchestratorDecision, String> {
    orchestrator_next_internal(&TEST_SESSION_STORE, session_id, goal, false)
}

pub async fn run_local_precheck_for_test() -> Result<InstallStepResult, String> {
    let output = runners::run_step(&InstallMethod::Local, &InstallStep::Precheck, &HashMap::new())
        .map_err(|e| format!("{}: {}", e.summary, e.details))?;
    let mut result = make_result(
        true,
        output.summary,
        output.details,
        next_step(&InstallStep::Precheck),
        None,
    );
    result.commands = output.commands;
    result.artifacts = output.artifacts;
    Ok(result)
}

pub fn failed_state_for_test(step: &str) -> Result<String, String> {
    let parsed = parse_step(step)?;
    Ok(failed_state(&parsed).as_str().to_string())
}
