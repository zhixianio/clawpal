use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallMethod {
    Local,
    Wsl2,
    Docker,
    RemoteSsh,
}

impl InstallMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            InstallMethod::Local => "local",
            InstallMethod::Wsl2 => "wsl2",
            InstallMethod::Docker => "docker",
            InstallMethod::RemoteSsh => "remote_ssh",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallState {
    Idle,
    SelectedMethod,
    PrecheckRunning,
    PrecheckFailed,
    PrecheckPassed,
    InstallRunning,
    InstallFailed,
    InstallPassed,
    InitRunning,
    InitFailed,
    InitPassed,
    VerifyRunning,
    VerifyFailed,
    Ready,
}

impl InstallState {
    pub fn as_str(&self) -> &'static str {
        match self {
            InstallState::Idle => "idle",
            InstallState::SelectedMethod => "selected_method",
            InstallState::PrecheckRunning => "precheck_running",
            InstallState::PrecheckFailed => "precheck_failed",
            InstallState::PrecheckPassed => "precheck_passed",
            InstallState::InstallRunning => "install_running",
            InstallState::InstallFailed => "install_failed",
            InstallState::InstallPassed => "install_passed",
            InstallState::InitRunning => "init_running",
            InstallState::InitFailed => "init_failed",
            InstallState::InitPassed => "init_passed",
            InstallState::VerifyRunning => "verify_running",
            InstallState::VerifyFailed => "verify_failed",
            InstallState::Ready => "ready",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallStep {
    Precheck,
    Install,
    Init,
    Verify,
}

impl InstallStep {
    pub fn as_str(&self) -> &'static str {
        match self {
            InstallStep::Precheck => "precheck",
            InstallStep::Install => "install",
            InstallStep::Init => "init",
            InstallStep::Verify => "verify",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InstallLogEntry {
    pub at: String,
    pub level: String,
    pub message: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InstallSession {
    pub id: String,
    pub method: InstallMethod,
    pub state: InstallState,
    pub current_step: Option<InstallStep>,
    pub logs: Vec<InstallLogEntry>,
    pub artifacts: HashMap<String, Value>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InstallStepResult {
    pub ok: bool,
    pub summary: String,
    pub details: String,
    pub commands: Vec<String>,
    pub artifacts: HashMap<String, Value>,
    pub next_step: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InstallMethodCapability {
    pub method: String,
    pub available: bool,
    pub hint: Option<String>,
}
