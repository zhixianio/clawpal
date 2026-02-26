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
                summary: "wsl2 precheck completed".to_string(),
                details: status.stdout,
                commands: vec![status.command_line],
                artifacts: HashMap::new(),
            })
        }
        InstallStep::Install => {
            let script = "command -v openclaw >/dev/null 2>&1 || (mkdir -p ~/.clawpal/install/cache && INSTALLER=~/.clawpal/install/cache/openclaw-install.sh && ( [ -s \"$INSTALLER\" ] || curl -fsSL --proto '=https' --tlsv1.2 https://openclaw.ai/install.sh -o \"$INSTALLER\" ) && bash \"$INSTALLER\" --no-prompt --no-onboard)";
            let install = run_command(wsl(), &["bash", "-lc", script])?;
            Ok(RunnerOutput {
                summary: "wsl2 install completed".to_string(),
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
                &["bash", "-lc", "mkdir -p ~/.openclaw && [ -f ~/.openclaw/openclaw.json ] || printf '{}' > ~/.openclaw/openclaw.json"],
            )?;
            Ok(RunnerOutput {
                summary: "wsl2 init completed".to_string(),
                details: if init.stdout.is_empty() {
                    "Initialized ~/.openclaw inside WSL".to_string()
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
                &["bash", "-lc", "openclaw --version && openclaw config get agents --json >/dev/null"],
            )?;
            Ok(RunnerOutput {
                summary: "wsl2 verify completed".to_string(),
                details: verify.stdout,
                commands: vec![verify.command_line],
                artifacts: HashMap::new(),
            })
        }
    }
}
