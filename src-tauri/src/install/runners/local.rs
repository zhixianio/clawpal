use super::{classify_error_code, run_command, RunnerFailure, RunnerOutput};
use crate::config_io::{ensure_dirs, write_text, DEFAULT_CONFIG};
use crate::install::types::InstallStep;
use crate::models::resolve_paths;
use serde_json::Value;
use std::collections::HashMap;
use std::process::Command;

fn detect_openclaw() -> Result<(bool, String), RunnerFailure> {
    let bin = clawpal_core::openclaw::resolve_openclaw_bin();
    let command_line = format!("{} --version", bin);
    match Command::new(bin).arg("--version").output() {
        Ok(output) => {
            let code = output.status.code().unwrap_or(-1);
            if code == 0 {
                Ok((true, command_line))
            } else {
                Ok((false, command_line))
            }
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                Ok((false, command_line))
            } else {
                Err(RunnerFailure {
                    error_code: classify_error_code(&e.to_string()),
                    summary: "install.local.precheck.failed".to_string(),
                    details: e.to_string(),
                    commands: vec![command_line],
                    ssh_diagnostic: None,
                })
            }
        }
    }
}

fn bool_artifact(artifacts: &HashMap<String, Value>, key: &str) -> bool {
    artifacts.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn run_step(
    step: &InstallStep,
    artifacts: &HashMap<String, Value>,
) -> Result<RunnerOutput, RunnerFailure> {
    match step {
        InstallStep::Precheck => {
            let (openclaw_present, command_line) = detect_openclaw()?;
            let mut next_artifacts = HashMap::new();
            next_artifacts.insert(
                "openclaw_present".to_string(),
                Value::Bool(openclaw_present),
            );
            let details = if openclaw_present {
                "install.local.precheck.detailsFound".to_string()
            } else {
                "install.local.precheck.detailsNotFound".to_string()
            };
            Ok(RunnerOutput {
                summary: "install.local.precheck.summary".to_string(),
                details,
                commands: vec![command_line],
                artifacts: next_artifacts,
            })
        }
        InstallStep::Install => {
            let already_present = bool_artifact(artifacts, "openclaw_present");
            if already_present {
                return Ok(RunnerOutput {
                    summary: "install.local.install.skipped".to_string(),
                    details: "install.local.install.skippedDetails".to_string(),
                    commands: vec![format!(
                        "{} --version",
                        clawpal_core::openclaw::resolve_openclaw_bin()
                    )],
                    artifacts: HashMap::from([("openclaw_present".to_string(), Value::Bool(true))]),
                });
            }

            let script = "mkdir -p ~/.clawpal/install/cache && INSTALLER=~/.clawpal/install/cache/openclaw-install.sh && ( [ -s \"$INSTALLER\" ] || curl -fsSL --proto '=https' --tlsv1.2 https://openclaw.ai/install.sh -o \"$INSTALLER\" ) && bash \"$INSTALLER\" --no-prompt --no-onboard";
            let install = run_command("bash", &["-ilc", script])?;
            Ok(RunnerOutput {
                summary: "install.local.install.summary".to_string(),
                details: if install.stderr.is_empty() {
                    install.stdout
                } else {
                    format!("{}\n{}", install.stdout, install.stderr)
                },
                commands: vec![install.command_line],
                artifacts: HashMap::from([("openclaw_present".to_string(), Value::Bool(true))]),
            })
        }
        InstallStep::Init => {
            let paths = resolve_paths();
            ensure_dirs(&paths).map_err(|e| RunnerFailure {
                error_code: classify_error_code(&e),
                summary: "install.local.init.failed".to_string(),
                details: e,
                commands: vec![format!("mkdir -p {}", paths.base_dir.display())],
                ssh_diagnostic: None,
            })?;
            if !paths.config_path.exists() {
                write_text(&paths.config_path, DEFAULT_CONFIG).map_err(|e| RunnerFailure {
                    error_code: classify_error_code(&e),
                    summary: "install.local.init.failed".to_string(),
                    details: e,
                    commands: vec![format!("write {}", paths.config_path.display())],
                    ssh_diagnostic: None,
                })?;
            }
            Ok(RunnerOutput {
                summary: "install.local.init.summary".to_string(),
                details: format!("install.local.init.details:{}", paths.base_dir.display()),
                commands: vec![format!("mkdir -p {}", paths.base_dir.display())],
                artifacts: HashMap::from([
                    (
                        "openclaw_dir".to_string(),
                        Value::String(paths.base_dir.to_string_lossy().to_string()),
                    ),
                    (
                        "openclaw_config".to_string(),
                        Value::String(paths.config_path.to_string_lossy().to_string()),
                    ),
                ]),
            })
        }
        InstallStep::Verify => {
            let version = run_command(
                clawpal_core::openclaw::resolve_openclaw_bin(),
                &["--version"],
            )?;
            let status = run_command(
                clawpal_core::openclaw::resolve_openclaw_bin(),
                &["config", "get", "agents", "--json"],
            )?;
            Ok(RunnerOutput {
                summary: "install.local.verify.summary".to_string(),
                details: format!("{}\n{}", version.stdout, status.stdout),
                commands: vec![version.command_line, status.command_line],
                artifacts: HashMap::new(),
            })
        }
    }
}
