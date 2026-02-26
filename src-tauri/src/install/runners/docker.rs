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

fn artifact_string(artifacts: &HashMap<String, Value>, key: &str) -> Option<String> {
    artifacts
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .map(|raw| raw.to_string())
}

fn docker_instance_id(artifacts: &HashMap<String, Value>) -> String {
    artifact_string(artifacts, "docker_instance_id").unwrap_or_else(|| "docker:local".to_string())
}

fn sanitize_slug(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        }
    }
    let trimmed = out.trim_matches(|ch| ch == '-' || ch == '_').to_string();
    if trimmed.is_empty() {
        "docker-local".to_string()
    } else {
        trimmed
    }
}

fn docker_slug_from_instance_id(instance_id: &str) -> String {
    let suffix = instance_id.strip_prefix("docker:").unwrap_or(instance_id).trim();
    if suffix.is_empty() || suffix.eq_ignore_ascii_case("local") {
        return "docker-local".to_string();
    }
    if suffix.starts_with("docker-") {
        return sanitize_slug(suffix);
    }
    format!("docker-{}", sanitize_slug(suffix))
}

fn default_docker_openclaw_home(artifacts: &HashMap<String, Value>) -> String {
    let slug = docker_slug_from_instance_id(&docker_instance_id(artifacts));
    match home_dir() {
        Some(home) => home
            .join(".clawpal")
            .join(slug)
            .to_string_lossy()
            .to_string(),
        None => "~/.clawpal/docker-local".to_string(),
    }
}

fn docker_openclaw_home(artifacts: &HashMap<String, Value>) -> String {
    artifact_string(artifacts, "docker_openclaw_home")
        .unwrap_or_else(|| default_docker_openclaw_home(artifacts))
}

fn docker_openclaw_state_dir(artifacts: &HashMap<String, Value>) -> String {
    format!("{}/.openclaw", docker_openclaw_home(artifacts))
}

fn docker_instance_label(artifacts: &HashMap<String, Value>) -> String {
    if let Some(label) = artifact_string(artifacts, "docker_instance_label") {
        return label;
    }
    let instance_id = docker_instance_id(artifacts);
    if instance_id == "docker:local" {
        return "Docker Local".to_string();
    }
    let suffix = instance_id.strip_prefix("docker:").unwrap_or(&instance_id);
    if let Some(number) = suffix.strip_prefix("local-") {
        if !number.is_empty() {
            return format!("Docker Local {}", number);
        }
    }
    format!("Docker {}", suffix)
}

fn build_docker_instance_artifacts(
    artifacts: &HashMap<String, Value>,
    repo_str: &str,
    openclaw_home: &str,
) -> HashMap<String, Value> {
    HashMap::from([
        (
            "docker_repo_dir".to_string(),
            Value::String(repo_str.to_string()),
        ),
        (
            "docker_instance_id".to_string(),
            Value::String(docker_instance_id(artifacts)),
        ),
        (
            "docker_instance_label".to_string(),
            Value::String(docker_instance_label(artifacts)),
        ),
        (
            "docker_openclaw_home".to_string(),
            Value::String(openclaw_home.to_string()),
        ),
        (
            "docker_clawpal_data_dir".to_string(),
            Value::String(format!("{openclaw_home}/data")),
        ),
    ])
}

fn docker_verify_compose_command(repo_str: &str, openclaw_state_dir: &str) -> String {
    format!(
        "cd \"{repo}\" && OPENCLAW_CONFIG_DIR=\"{home}\" OPENCLAW_WORKSPACE_DIR=\"{home}/workspace\" OPENCLAW_GATEWAY_TOKEN=\"clawpal-install\" CLAUDE_AI_SESSION_KEY=\"dummy\" CLAUDE_WEB_SESSION_KEY=\"dummy\" CLAUDE_WEB_COOKIE=\"dummy\" docker compose config",
        repo = repo_str
        ,home = openclaw_state_dir
    )
}

pub fn docker_verify_compose_command_for_test(repo_str: &str) -> String {
    docker_verify_compose_command(repo_str, &docker_openclaw_state_dir(&HashMap::new()))
}

pub fn run_step(
    step: &InstallStep,
    artifacts: &HashMap<String, Value>,
) -> Result<RunnerOutput, RunnerFailure> {
    let repo = repo_dir()?;
    let repo_str = repo_dir_str(&repo);
    let openclaw_home = docker_openclaw_home(artifacts);
    let openclaw_state_dir = docker_openclaw_state_dir(artifacts);
    let step_artifacts = build_docker_instance_artifacts(artifacts, &repo_str, &openclaw_home);

    match step {
        InstallStep::Precheck => {
            let docker = run_command("docker", &["info"])?;
            let compose = run_command("docker", &["compose", "version"])?;
            let git = run_command("git", &["--version"])?;
            Ok(RunnerOutput {
                summary: "docker precheck completed".to_string(),
                details: "Docker, Docker Compose, and git are available".to_string(),
                commands: vec![docker.command_line, compose.command_line, git.command_line],
                artifacts: step_artifacts.clone(),
            })
        }
        InstallStep::Install => {
            let cmd = format!(
                "mkdir -p \"{}\" && if [ -d \"{}/.git\" ]; then echo \"Using existing OpenClaw repository checkout at {}\"; else git clone https://github.com/openclaw/openclaw.git \"{}\"; fi",
                repo_str, repo_str, repo_str, repo_str
            );
            let clone = run_command("bash", &["-lc", &cmd])?;
            Ok(RunnerOutput {
                summary: "docker install completed".to_string(),
                details: "Prepared official OpenClaw repository for Docker setup".to_string(),
                commands: vec![clone.command_line],
                artifacts: step_artifacts.clone(),
            })
        }
        InstallStep::Init => {
            let prep_cmd = format!(
                "mkdir -p \"{state}\" \"{state}/workspace\" && [ -f \"{state}/openclaw.json\" ] || printf '{{}}' > \"{state}/openclaw.json\"",
                state = openclaw_state_dir
            );
            let prep = run_command("bash", &["-lc", &prep_cmd])?;
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
                commands: vec![prep.command_line, build.command_line],
                artifacts: step_artifacts.clone(),
            })
        }
        InstallStep::Verify => {
            let compose_check = docker_verify_compose_command(&repo_str, &openclaw_state_dir);
            let compose = run_command("bash", &["-lc", &compose_check])?;
            let inspect = run_command("docker", &["image", "inspect", "openclaw:local"])?;
            Ok(RunnerOutput {
                summary: "docker verify completed".to_string(),
                details: "Official Docker compose configuration and local image verified".to_string(),
                commands: vec![compose.command_line, inspect.command_line],
                artifacts: step_artifacts,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_home_defaults_to_legacy_local_path() {
        let home = docker_openclaw_home(&HashMap::new());
        assert!(home.ends_with("/.clawpal/docker-local"), "home={home}");
    }

    #[test]
    fn docker_home_uses_instance_id_suffix() {
        let artifacts = HashMap::from([(
            "docker_instance_id".to_string(),
            Value::String("docker:local-2".to_string()),
        )]);
        let home = docker_openclaw_home(&artifacts);
        assert!(home.ends_with("/.clawpal/docker-local-2"), "home={home}");
    }
}
