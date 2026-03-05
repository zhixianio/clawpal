use super::{RunnerFailure, RunnerOutput};
use crate::cli_runner::run_openclaw_remote;
use crate::install::types::InstallStep;
use crate::ssh::SshConnectionPool;
use clawpal_core::ssh::diagnostic::{
    from_any_error, SshErrorCode, SshIntent, SshStage,
};
use serde_json::Value;
use std::collections::HashMap;

fn ssh_runner_failure(
    stage: SshStage,
    summary: &str,
    details: String,
    commands: Vec<String>,
) -> RunnerFailure {
    let report = from_any_error(stage, SshIntent::InstallStep, details.clone());
    let error_code = report
        .error_code
        .unwrap_or(SshErrorCode::Unknown)
        .as_str()
        .to_string();
    RunnerFailure {
        error_code,
        summary: summary.to_string(),
        details,
        commands,
        ssh_diagnostic: Some(report),
    }
}

pub async fn run_step(
    pool: &SshConnectionPool,
    host_id: &str,
    step: &InstallStep,
    artifacts: &HashMap<String, Value>,
) -> Result<RunnerOutput, RunnerFailure> {
    match step {
        InstallStep::Precheck => {
            let check = pool
                .exec_login(host_id, "command -v openclaw >/dev/null 2>&1")
                .await
                .map_err(|e| {
                    ssh_runner_failure(
                        SshStage::RemoteExec,
                        "install.ssh.precheck.failed",
                        e,
                        vec!["openclaw check on remote".to_string()],
                    )
                })?;
            let openclaw_present = check.exit_code == 0;
            let details = if openclaw_present {
                "install.ssh.precheck.detailsFound".to_string()
            } else {
                "install.ssh.precheck.detailsNotFound".to_string()
            };
            Ok(RunnerOutput {
                summary: "install.ssh.precheck.summary".to_string(),
                details,
                commands: vec!["ssh remote command -v openclaw".to_string()],
                artifacts: HashMap::from([(
                    "openclaw_present".to_string(),
                    Value::Bool(openclaw_present),
                )]),
            })
        }
        InstallStep::Install => {
            let already_present = artifacts
                .get("openclaw_present")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if already_present {
                return Ok(RunnerOutput {
                    summary: "install.ssh.install.skipped".to_string(),
                    details: "install.ssh.install.skippedDetails".to_string(),
                    commands: vec!["openclaw --version".to_string()],
                    artifacts: HashMap::from([("openclaw_present".to_string(), Value::Bool(true))]),
                });
            }
            let script = "mkdir -p ~/.clawpal/install/cache && INSTALLER=~/.clawpal/install/cache/openclaw-install.sh && ( [ -s \"$INSTALLER\" ] || curl -fsSL --proto '=https' --tlsv1.2 https://openclaw.ai/install.sh -o \"$INSTALLER\" ) && bash \"$INSTALLER\" --no-prompt --no-onboard";
            let result = pool
                .exec_login(host_id, script)
                .await
                .map_err(|e| {
                    ssh_runner_failure(
                        SshStage::RemoteExec,
                        "remote ssh install failed",
                        e,
                        vec![script.to_string()],
                    )
                })?;
            if result.exit_code != 0 {
                return Err(ssh_runner_failure(
                    SshStage::RemoteExec,
                    "install.ssh.install.failed",
                    if result.stderr.is_empty() {
                        result.stdout
                    } else {
                        result.stderr
                    },
                    vec![script.to_string()],
                ));
            }
            Ok(RunnerOutput {
                summary: "install.ssh.install.summary".to_string(),
                details: result.stdout,
                commands: vec![script.to_string()],
                artifacts: HashMap::from([("openclaw_present".to_string(), Value::Bool(true))]),
            })
        }
        InstallStep::Init => {
            let init_cmd = "mkdir -p ~/.openclaw && [ -f ~/.openclaw/openclaw.json ] || printf '{}' > ~/.openclaw/openclaw.json";
            let result = pool
                .exec_login(host_id, init_cmd)
                .await
                .map_err(|e| {
                    ssh_runner_failure(
                        SshStage::RemoteExec,
                        "install.ssh.init.failed",
                        e,
                        vec![init_cmd.to_string()],
                    )
                })?;
            if result.exit_code != 0 {
                return Err(ssh_runner_failure(
                    SshStage::RemoteExec,
                    "install.ssh.init.failed",
                    if result.stderr.is_empty() {
                        result.stdout
                    } else {
                        result.stderr
                    },
                    vec![init_cmd.to_string()],
                ));
            }
            Ok(RunnerOutput {
                summary: "install.ssh.init.summary".to_string(),
                details: "install.ssh.init.details".to_string(),
                commands: vec![init_cmd.to_string()],
                artifacts: HashMap::new(),
            })
        }
        InstallStep::Verify => {
            let version = run_openclaw_remote(pool, host_id, &["--version"])
                .await
                .map_err(|e| {
                    ssh_runner_failure(
                        SshStage::RemoteExec,
                        "remote ssh verify failed",
                        e,
                        vec!["openclaw --version".to_string()],
                    )
                })?;
            if version.exit_code != 0 {
                return Err(ssh_runner_failure(
                    SshStage::RemoteExec,
                    "install.ssh.verify.failed",
                    if version.stderr.is_empty() {
                        version.stdout
                    } else {
                        version.stderr
                    },
                    vec!["openclaw --version".to_string()],
                ));
            }
            Ok(RunnerOutput {
                summary: "install.ssh.verify.summary".to_string(),
                details: version.stdout,
                commands: vec!["openclaw --version".to_string()],
                artifacts: HashMap::new(),
            })
        }
    }
}
