use super::session_store::InstallSessionStore;
use super::types::{
    InstallMethod, InstallMethodCapability, InstallSession, InstallState, InstallStep,
    InstallStepResult,
};
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
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

fn create_session(store: &InstallSessionStore, method_raw: &str) -> Result<InstallSession, String> {
    let method = parse_method(method_raw)?;
    let now = Utc::now().to_rfc3339();
    let session = InstallSession {
        id: format!("install-{}", Uuid::new_v4()),
        method,
        state: InstallState::SelectedMethod,
        current_step: None,
        logs: vec![],
        artifacts: HashMap::<String, Value>::new(),
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
        InstallStep::Verify => InstallState::InitFailed,
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

fn run_step(store: &InstallSessionStore, session_id_raw: &str, step_raw: &str) -> Result<InstallStepResult, String> {
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

    session.state = success_state(&step);
    session.updated_at = Utc::now().to_rfc3339();
    store.upsert(session)?;

    let mut result = make_result(
        true,
        format!("{} completed", step.as_str()),
        format!("{} finished successfully", step.as_str()),
        next_step(&step),
        None,
    );
    result.commands = vec![format!("{}: simulated", step.as_str())];
    Ok(result)
}

#[tauri::command]
pub async fn install_create_session(
    method: String,
    store: State<'_, InstallSessionStore>,
) -> Result<InstallSession, String> {
    create_session(&store, method.trim())
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
    store: State<'_, InstallSessionStore>,
) -> Result<InstallStepResult, String> {
    run_step(&store, &session_id, &step)
}

#[tauri::command]
pub async fn install_list_methods() -> Result<Vec<InstallMethodCapability>, String> {
    Ok(list_method_capabilities())
}

pub async fn create_session_for_test(method: &str) -> Result<InstallSession, String> {
    create_session(&TEST_SESSION_STORE, method)
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
    run_step(&TEST_SESSION_STORE, session_id, step)
}

pub async fn list_methods_for_test() -> Result<Vec<InstallMethodCapability>, String> {
    Ok(list_method_capabilities())
}
