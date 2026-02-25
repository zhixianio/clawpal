use super::{classify_error_code, RunnerFailure, RunnerOutput};
use crate::cli_runner::run_openclaw_remote;
use crate::install::types::InstallStep;
use crate::ssh::SshConnectionPool;
use serde_json::Value;
use std::collections::HashMap;

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
                .map_err(|e| RunnerFailure {
                    error_code: classify_error_code(&e),
                    summary: "remote ssh precheck failed".to_string(),
                    details: e,
                    commands: vec!["openclaw check on remote".to_string()],
                })?;
            let openclaw_present = check.exit_code == 0;
            let details = if openclaw_present {
                "OpenClaw detected on remote host".to_string()
            } else {
                "OpenClaw not found on remote host; install step will run installer".to_string()
            };
            Ok(RunnerOutput {
                summary: "remote ssh precheck completed".to_string(),
                details,
                commands: vec!["ssh remote command -v openclaw".to_string()],
                artifacts: HashMap::from([("openclaw_present".to_string(), Value::Bool(openclaw_present))]),
            })
        }
        InstallStep::Install => {
            let already_present = artifacts
                .get("openclaw_present")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if already_present {
                return Ok(RunnerOutput {
                    summary: "remote ssh install skipped".to_string(),
                    details: "OpenClaw already present from precheck".to_string(),
                    commands: vec!["openclaw --version".to_string()],
                    artifacts: HashMap::from([("openclaw_present".to_string(), Value::Bool(true))]),
                });
            }
            let script = "curl -fsSL --proto '=https' --tlsv1.2 https://openclaw.ai/install.sh | bash -s -- --no-prompt --no-onboard";
            let result = pool.exec_login(host_id, script).await.map_err(|e| RunnerFailure {
                error_code: classify_error_code(&e),
                summary: "remote ssh install failed".to_string(),
                details: e,
                commands: vec![script.to_string()],
            })?;
            if result.exit_code != 0 {
                return Err(RunnerFailure {
                    error_code: classify_error_code(&result.stderr),
                    summary: "remote ssh install failed".to_string(),
                    details: if result.stderr.is_empty() {
                        result.stdout
                    } else {
                        result.stderr
                    },
                    commands: vec![script.to_string()],
                });
            }
            Ok(RunnerOutput {
                summary: "remote ssh install completed".to_string(),
                details: result.stdout,
                commands: vec![script.to_string()],
                artifacts: HashMap::from([("openclaw_present".to_string(), Value::Bool(true))]),
            })
        }
        InstallStep::Init => {
            let init_cmd = "mkdir -p ~/.openclaw && [ -f ~/.openclaw/openclaw.json ] || printf '{}' > ~/.openclaw/openclaw.json";
            let result = pool.exec_login(host_id, init_cmd).await.map_err(|e| RunnerFailure {
                error_code: classify_error_code(&e),
                summary: "remote ssh init failed".to_string(),
                details: e,
                commands: vec![init_cmd.to_string()],
            })?;
            if result.exit_code != 0 {
                return Err(RunnerFailure {
                    error_code: classify_error_code(&result.stderr),
                    summary: "remote ssh init failed".to_string(),
                    details: if result.stderr.is_empty() {
                        result.stdout
                    } else {
                        result.stderr
                    },
                    commands: vec![init_cmd.to_string()],
                });
            }
            Ok(RunnerOutput {
                summary: "remote ssh init completed".to_string(),
                details: "Initialized ~/.openclaw on remote host".to_string(),
                commands: vec![init_cmd.to_string()],
                artifacts: HashMap::new(),
            })
        }
        InstallStep::Verify => {
            let version = run_openclaw_remote(pool, host_id, &["--version"]).await.map_err(|e| RunnerFailure {
                error_code: classify_error_code(&e),
                summary: "remote ssh verify failed".to_string(),
                details: e,
                commands: vec!["openclaw --version".to_string()],
            })?;
            if version.exit_code != 0 {
                return Err(RunnerFailure {
                    error_code: classify_error_code(&version.stderr),
                    summary: "remote ssh verify failed".to_string(),
                    details: if version.stderr.is_empty() {
                        version.stdout
                    } else {
                        version.stderr
                    },
                    commands: vec!["openclaw --version".to_string()],
                });
            }
            Ok(RunnerOutput {
                summary: "remote ssh verify completed".to_string(),
                details: version.stdout,
                commands: vec!["openclaw --version".to_string()],
                artifacts: HashMap::new(),
            })
        }
    }
}
