use super::{run_command, RunnerFailure, RunnerOutput};
use crate::install::types::InstallStep;
use dirs::home_dir;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

fn repo_dir() -> Result<PathBuf, RunnerFailure> {
    let home = home_dir().ok_or_else(|| RunnerFailure {
        error_code: "env_missing".to_string(),
        summary: "docker setup failed".to_string(),
        details: "Unable to resolve HOME directory".to_string(),
        commands: vec![],
    })?;
    Ok(home.join(".clawpal").join("install").join("openclaw-docker"))
}

fn repo_dir_str(path: &PathBuf) -> String {
    path.to_string_lossy().to_string()
}

pub fn run_step(
    step: &InstallStep,
    _artifacts: &HashMap<String, Value>,
) -> Result<RunnerOutput, RunnerFailure> {
    let repo = repo_dir()?;
    let repo_str = repo_dir_str(&repo);

    match step {
        InstallStep::Precheck => {
            let docker = run_command("docker", &["info"])?;
            let compose = run_command("docker", &["compose", "version"])?;
            let git = run_command("git", &["--version"])?;
            Ok(RunnerOutput {
                summary: "docker precheck completed".to_string(),
                details: "Docker, Docker Compose, and git are available".to_string(),
                commands: vec![docker.command_line, compose.command_line, git.command_line],
                artifacts: HashMap::from([(
                    "docker_repo_dir".to_string(),
                    Value::String(repo_str),
                )]),
            })
        }
        InstallStep::Install => {
            let cmd = format!(
                "mkdir -p \"{}\" && if [ -d \"{}/.git\" ]; then cd \"{}\" && git pull --ff-only; else git clone https://github.com/openclaw/openclaw.git \"{}\"; fi",
                repo_str, repo_str, repo_str, repo_str
            );
            let clone = run_command("bash", &["-lc", &cmd])?;
            Ok(RunnerOutput {
                summary: "docker install completed".to_string(),
                details: "Synced official OpenClaw repository for Docker setup".to_string(),
                commands: vec![clone.command_line],
                artifacts: HashMap::from([(
                    "docker_repo_dir".to_string(),
                    Value::String(repo_str),
                )]),
            })
        }
        InstallStep::Init => {
            let build_cmd = format!(
                "cd \"{}\" && docker build -t openclaw:local -f Dockerfile .",
                repo_str
            );
            let build = run_command("bash", &["-lc", &build_cmd])?;
            Ok(RunnerOutput {
                summary: "docker init completed".to_string(),
                details: if build.stderr.is_empty() {
                    "Built official OpenClaw Docker image (openclaw:local)".to_string()
                } else {
                    build.stderr
                },
                commands: vec![build.command_line],
                artifacts: HashMap::from([(
                    "docker_repo_dir".to_string(),
                    Value::String(repo_str),
                )]),
            })
        }
        InstallStep::Verify => {
            let compose_check = format!("cd \"{}\" && docker compose config", repo_str);
            let compose = run_command("bash", &["-lc", &compose_check])?;
            let inspect = run_command("docker", &["image", "inspect", "openclaw:local"])?;
            Ok(RunnerOutput {
                summary: "docker verify completed".to_string(),
                details: "Official Docker compose configuration and local image verified".to_string(),
                commands: vec![compose.command_line, inspect.command_line],
                artifacts: HashMap::new(),
            })
        }
    }
}
