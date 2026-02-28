use super::{run_command, RunnerFailure, RunnerOutput};
use crate::install::types::InstallStep;
use serde_json::Value;
use std::collections::HashMap;

fn wsl() -> &'static str {
    if cfg!(target_os = "windows") {
        "wsl.exe"
    } else {
        "wsl"
    }
}

pub fn run_step(
    step: &InstallStep,
    _artifacts: &HashMap<String, Value>,
) -> Result<RunnerOutput, RunnerFailure> {
    match step {
        InstallStep::Precheck => {
            let status = run_command(wsl(), &["--status"])?;
            Ok(RunnerOutput {
                summary: "install.wsl2.precheck.summary".to_string(),
                details: status.stdout,
                commands: vec![status.command_line],
                artifacts: HashMap::new(),
            })
        }
        InstallStep::Install => {
            let script = "export PATH=\"$HOME/.npm-global/bin:$PATH\"; command -v openclaw >/dev/null 2>&1 || (if command -v curl >/dev/null 2>&1; then curl -fsSL https://openclaw.ai/install.sh | bash -s -- --no-prompt --no-onboard; elif command -v wget >/dev/null 2>&1; then wget -qO- https://openclaw.ai/install.sh | bash -s -- --no-prompt --no-onboard; else echo 'curl or wget is required to install openclaw' >&2; exit 1; fi)";
            let install = run_command(wsl(), &["bash", "-ilc", script])?;
            Ok(RunnerOutput {
                summary: "install.wsl2.install.summary".to_string(),
                details: if install.stderr.is_empty() {
                    install.stdout
                } else {
                    format!("{}\n{}", install.stdout, install.stderr)
                },
                commands: vec![install.command_line],
                artifacts: HashMap::new(),
            })
        }
        InstallStep::Init => {
            let init = run_command(
                wsl(),
                &["bash", "-ilc", "mkdir -p ~/.openclaw && [ -f ~/.openclaw/openclaw.json ] || printf '{}' > ~/.openclaw/openclaw.json"],
            )?;
            Ok(RunnerOutput {
                summary: "install.wsl2.init.summary".to_string(),
                details: if init.stdout.is_empty() {
                    "install.wsl2.init.details".to_string()
                } else {
                    init.stdout
                },
                commands: vec![init.command_line],
                artifacts: HashMap::new(),
            })
        }
        InstallStep::Verify => {
            let verify = run_command(
                wsl(),
                &[
                    "bash",
                    "-ilc",
                    "export PATH=\"$HOME/.npm-global/bin:$PATH\"; openclaw --version && openclaw config get agents --json >/dev/null",
                ],
            )?;
            Ok(RunnerOutput {
                summary: "install.wsl2.verify.summary".to_string(),
                details: verify.stdout,
                commands: vec![verify.command_line],
                artifacts: HashMap::new(),
            })
        }
    }
}
