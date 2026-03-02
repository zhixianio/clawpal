use super::{run_command, RunnerFailure, RunnerOutput};
use crate::install::types::InstallStep;
use dirs::home_dir;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

const DEFAULT_GATEWAY_PORT: u16 = 18789;
const PORT_STEP: u16 = 10;

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
    let suffix = instance_id
        .strip_prefix("docker:")
        .unwrap_or(instance_id)
        .trim();
    if suffix.is_empty() || suffix.eq_ignore_ascii_case("local") {
        return "docker-local".to_string();
    }
    if suffix.starts_with("docker-") {
        return sanitize_slug(suffix);
    }
    format!("docker-{}", sanitize_slug(suffix))
}

/// Extract the numeric instance number from a slug like "docker-local-2" → Some(2).
/// Returns None for the default instance ("docker-local") or non-numeric suffixes.
fn instance_number_from_slug(slug: &str) -> Option<u16> {
    slug.strip_prefix("docker-local-")
        .and_then(|n| n.parse::<u16>().ok())
        .filter(|&n| n >= 1)
}

/// Pre-computed per-instance values derived from a single slug computation.
struct DockerInstanceInfo {
    instance_id: String,
    slug: String,
}

impl DockerInstanceInfo {
    fn from_artifacts(artifacts: &HashMap<String, Value>) -> Self {
        let instance_id = docker_instance_id(artifacts);
        let slug = docker_slug_from_instance_id(&instance_id);
        Self { instance_id, slug }
    }

    fn is_default(&self) -> bool {
        self.slug == "docker-local"
    }

    /// Suffix for naming: "local-2" from slug "docker-local-2", empty for default.
    fn local_suffix(&self) -> &str {
        self.slug.strip_prefix("docker-").unwrap_or(&self.slug)
    }

    fn repo_dir(&self) -> Result<PathBuf, RunnerFailure> {
        let home = home_dir().ok_or_else(|| RunnerFailure {
            error_code: "env_missing".to_string(),
            summary: "install.docker.setupFailed".to_string(),
            details: "Unable to resolve HOME directory".to_string(),
            commands: vec![],
        })?;
        let dir_name = if self.is_default() {
            "openclaw-docker".to_string()
        } else {
            format!("openclaw-{}", self.slug)
        };
        Ok(home.join(".clawpal").join("install").join(dir_name))
    }

    fn image_tag(&self) -> String {
        if self.is_default() {
            "openclaw:local".to_string()
        } else {
            format!("openclaw:{}", self.slug)
        }
    }

    fn compose_project(&self) -> String {
        if self.is_default() {
            "openclaw".to_string()
        } else {
            format!("openclaw-{}", self.local_suffix())
        }
    }

    fn gateway_port(&self) -> u16 {
        if let Some(n) = instance_number_from_slug(&self.slug) {
            (n - 1)
                .checked_mul(PORT_STEP)
                .and_then(|offset| DEFAULT_GATEWAY_PORT.checked_add(offset))
                .unwrap_or(DEFAULT_GATEWAY_PORT)
        } else {
            DEFAULT_GATEWAY_PORT
        }
    }
}

fn default_docker_openclaw_home(slug: &str) -> String {
    match home_dir() {
        Some(home) => home
            .join(".clawpal")
            .join(slug)
            .to_string_lossy()
            .to_string(),
        None => "~/.clawpal/docker-local".to_string(),
    }
}

fn docker_openclaw_home(artifacts: &HashMap<String, Value>, slug: &str) -> String {
    artifact_string(artifacts, "docker_openclaw_home")
        .unwrap_or_else(|| default_docker_openclaw_home(slug))
}

fn docker_openclaw_state_dir(artifacts: &HashMap<String, Value>, slug: &str) -> String {
    format!("{}/.openclaw", docker_openclaw_home(artifacts, slug))
}

fn docker_instance_label(artifacts: &HashMap<String, Value>, info: &DockerInstanceInfo) -> String {
    if let Some(label) = artifact_string(artifacts, "docker_instance_label") {
        return label;
    }
    if info.is_default() {
        return "docker-local".to_string();
    }
    let suffix = info
        .instance_id
        .strip_prefix("docker:")
        .unwrap_or(&info.instance_id);
    if let Some(number) = suffix.strip_prefix("local-") {
        if !number.is_empty() {
            return format!("docker-local-{number}");
        }
    }
    if suffix.starts_with("docker-") {
        suffix.to_string()
    } else {
        format!("docker-{suffix}")
    }
}

fn build_docker_instance_artifacts(
    artifacts: &HashMap<String, Value>,
    info: &DockerInstanceInfo,
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
            Value::String(info.instance_id.clone()),
        ),
        (
            "docker_instance_label".to_string(),
            Value::String(docker_instance_label(artifacts, info)),
        ),
        (
            "docker_openclaw_home".to_string(),
            Value::String(openclaw_home.to_string()),
        ),
        (
            "docker_clawpal_data_dir".to_string(),
            Value::String(format!("{openclaw_home}/data")),
        ),
        (
            "docker_image_tag".to_string(),
            Value::String(info.image_tag()),
        ),
        (
            "docker_compose_project".to_string(),
            Value::String(info.compose_project()),
        ),
        (
            "docker_gateway_port".to_string(),
            Value::Number(info.gateway_port().into()),
        ),
    ])
}

fn docker_compose_up_command(
    repo_str: &str,
    openclaw_state_dir: &str,
    info: &DockerInstanceInfo,
) -> String {
    let image_tag = info.image_tag();
    let project = info.compose_project();
    let port = info.gateway_port();
    format!(
        "cd \"{repo}\" && COMPOSE_PROJECT_NAME=\"{project}\" OPENCLAW_IMAGE=\"{image}\" OPENCLAW_GATEWAY_PORT=\"{port}\" OPENCLAW_CONFIG_DIR=\"{home}\" OPENCLAW_WORKSPACE_DIR=\"{home}/workspace\" OPENCLAW_GATEWAY_TOKEN=\"clawpal-install\" CLAUDE_AI_SESSION_KEY=\"dummy\" CLAUDE_WEB_SESSION_KEY=\"dummy\" CLAUDE_WEB_COOKIE=\"dummy\" docker compose up -d",
        repo = repo_str,
        project = project,
        image = image_tag,
        port = port,
        home = openclaw_state_dir,
    )
}

fn docker_verify_compose_command(
    repo_str: &str,
    openclaw_state_dir: &str,
    info: &DockerInstanceInfo,
) -> String {
    let image_tag = info.image_tag();
    let project = info.compose_project();
    let port = info.gateway_port();
    format!(
        "cd \"{repo}\" && COMPOSE_PROJECT_NAME=\"{project}\" OPENCLAW_IMAGE=\"{image}\" OPENCLAW_GATEWAY_PORT=\"{port}\" OPENCLAW_CONFIG_DIR=\"{home}\" OPENCLAW_WORKSPACE_DIR=\"{home}/workspace\" OPENCLAW_GATEWAY_TOKEN=\"clawpal-install\" CLAUDE_AI_SESSION_KEY=\"dummy\" CLAUDE_WEB_SESSION_KEY=\"dummy\" CLAUDE_WEB_COOKIE=\"dummy\" docker compose config",
        repo = repo_str,
        project = project,
        image = image_tag,
        port = port,
        home = openclaw_state_dir,
    )
}

pub fn docker_verify_compose_command_for_test(repo_str: &str) -> String {
    let info = DockerInstanceInfo::from_artifacts(&HashMap::new());
    let state_dir = docker_openclaw_state_dir(&HashMap::new(), &info.slug);
    docker_verify_compose_command(repo_str, &state_dir, &info)
}

pub fn run_step(
    step: &InstallStep,
    artifacts: &HashMap<String, Value>,
) -> Result<RunnerOutput, RunnerFailure> {
    let info = DockerInstanceInfo::from_artifacts(artifacts);
    let repo = info.repo_dir()?;
    let repo_str = repo.to_string_lossy().to_string();
    let openclaw_home = docker_openclaw_home(artifacts, &info.slug);
    let openclaw_state_dir = docker_openclaw_state_dir(artifacts, &info.slug);
    let step_artifacts =
        build_docker_instance_artifacts(artifacts, &info, &repo_str, &openclaw_home);

    match step {
        InstallStep::Precheck => {
            let docker = run_command("docker", &["info"])?;
            let compose = run_command("docker", &["compose", "version"])?;
            let git = run_command("git", &["--version"])?;
            Ok(RunnerOutput {
                summary: "install.docker.precheck.summary".to_string(),
                details: "install.docker.precheck.details".to_string(),
                commands: vec![docker.command_line, compose.command_line, git.command_line],
                artifacts: step_artifacts.clone(),
            })
        }
        InstallStep::Install => {
            let cmd = format!(
                "mkdir -p \"{}\" && if [ -d \"{}/.git\" ]; then echo \"Using existing OpenClaw repository checkout at {}\"; else git clone https://github.com/openclaw/openclaw.git \"{}\"; fi",
                repo_str, repo_str, repo_str, repo_str
            );
            let clone = run_command("bash", &["-ilc", &cmd])?;
            Ok(RunnerOutput {
                summary: "install.docker.install.summary".to_string(),
                details: "install.docker.install.details".to_string(),
                commands: vec![clone.command_line],
                artifacts: step_artifacts.clone(),
            })
        }
        InstallStep::Init => {
            let prep_cmd = format!(
                "mkdir -p \"{state}\" \"{state}/workspace\" && [ -f \"{state}/openclaw.json\" ] || printf '{{}}' > \"{state}/openclaw.json\"",
                state = openclaw_state_dir
            );
            let prep = run_command("bash", &["-ilc", &prep_cmd])?;
            let image_tag = info.image_tag();
            let build_cmd = format!(
                "cd \"{}\" && docker build -t {} -f Dockerfile .",
                repo_str, image_tag
            );
            let build = run_command("bash", &["-ilc", &build_cmd])?;
            let up_cmd = docker_compose_up_command(&repo_str, &openclaw_state_dir, &info);
            let up = run_command("bash", &["-ilc", &up_cmd])?;
            Ok(RunnerOutput {
                summary: "install.docker.init.summary".to_string(),
                details: if build.stderr.is_empty() {
                    "install.docker.init.details".to_string()
                } else {
                    build.stderr
                },
                commands: vec![prep.command_line, build.command_line, up.command_line],
                artifacts: step_artifacts.clone(),
            })
        }
        InstallStep::Verify => {
            let compose_check =
                docker_verify_compose_command(&repo_str, &openclaw_state_dir, &info);
            let compose = run_command("bash", &["-ilc", &compose_check])?;
            let image_tag = info.image_tag();
            let inspect = run_command("docker", &["image", "inspect", &image_tag])?;

            // Check if container is actually running
            let project = info.compose_project();
            let ps_cmd = format!(
                "cd \"{repo}\" && COMPOSE_PROJECT_NAME=\"{project}\" docker compose ps --format json",
                repo = repo_str,
                project = project,
            );
            let mut commands = vec![compose.command_line, inspect.command_line];
            let mut final_artifacts = step_artifacts;
            match run_command("bash", &["-ilc", &ps_cmd]) {
                Ok(ps) => {
                    commands.push(ps.command_line);
                    // Parse container state from JSON lines output
                    let running = ps.stdout.lines().any(|line| {
                        serde_json::from_str::<Value>(line)
                            .ok()
                            .and_then(|v| {
                                v.get("State").and_then(Value::as_str).map(str::to_string)
                            })
                            .map(|s| s == "running")
                            .unwrap_or(false)
                    });
                    final_artifacts
                        .insert("docker_container_running".to_string(), Value::Bool(running));
                }
                Err(_) => {
                    // Non-fatal: container status check failed, skip
                    final_artifacts
                        .insert("docker_container_running".to_string(), Value::Bool(false));
                }
            }

            Ok(RunnerOutput {
                summary: "install.docker.verify.summary".to_string(),
                details: "install.docker.verify.details".to_string(),
                commands,
                artifacts: final_artifacts,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifacts_with_id(id: &str) -> HashMap<String, Value> {
        HashMap::from([(
            "docker_instance_id".to_string(),
            Value::String(id.to_string()),
        )])
    }

    // --- openclaw home ---

    #[test]
    fn docker_home_defaults_to_legacy_local_path() {
        let info = DockerInstanceInfo::from_artifacts(&HashMap::new());
        let home = docker_openclaw_home(&HashMap::new(), &info.slug);
        assert!(home.ends_with("/.clawpal/docker-local"), "home={home}");
    }

    #[test]
    fn docker_home_uses_instance_id_suffix() {
        let a = artifacts_with_id("docker:local-2");
        let info = DockerInstanceInfo::from_artifacts(&a);
        let home = docker_openclaw_home(&a, &info.slug);
        assert!(home.ends_with("/.clawpal/docker-local-2"), "home={home}");
    }

    // --- repo dir ---

    #[test]
    fn repo_dir_defaults_to_openclaw_docker() {
        let info = DockerInstanceInfo::from_artifacts(&HashMap::new());
        let path = info.repo_dir().expect("should resolve");
        assert!(
            path.ends_with("install/openclaw-docker"),
            "path={}",
            path.display()
        );
    }

    #[test]
    fn repo_dir_uses_instance_slug() {
        let a = artifacts_with_id("docker:local-2");
        let info = DockerInstanceInfo::from_artifacts(&a);
        let path = info.repo_dir().expect("should resolve");
        assert!(
            path.ends_with("install/openclaw-docker-local-2"),
            "path={}",
            path.display()
        );
    }

    // --- image tag ---

    #[test]
    fn image_tag_defaults_to_openclaw_local() {
        let info = DockerInstanceInfo::from_artifacts(&HashMap::new());
        assert_eq!(info.image_tag(), "openclaw:local");
    }

    #[test]
    fn image_tag_uses_instance_slug() {
        let a = artifacts_with_id("docker:local-2");
        let info = DockerInstanceInfo::from_artifacts(&a);
        assert_eq!(info.image_tag(), "openclaw:docker-local-2");
    }

    // --- compose project (no double prefix) ---

    #[test]
    fn compose_project_defaults_to_openclaw() {
        let info = DockerInstanceInfo::from_artifacts(&HashMap::new());
        assert_eq!(info.compose_project(), "openclaw");
    }

    #[test]
    fn compose_project_uses_local_suffix() {
        let a = artifacts_with_id("docker:local-3");
        let info = DockerInstanceInfo::from_artifacts(&a);
        assert_eq!(info.compose_project(), "openclaw-local-3");
    }

    // --- gateway port ---

    #[test]
    fn gateway_port_defaults_to_18789() {
        let info = DockerInstanceInfo::from_artifacts(&HashMap::new());
        assert_eq!(info.gateway_port(), 18789);
    }

    #[test]
    fn gateway_port_instance_1_is_default() {
        let a = artifacts_with_id("docker:local-1");
        let info = DockerInstanceInfo::from_artifacts(&a);
        assert_eq!(info.gateway_port(), 18789);
    }

    #[test]
    fn gateway_port_offsets_by_instance_number() {
        let a2 = artifacts_with_id("docker:local-2");
        assert_eq!(
            DockerInstanceInfo::from_artifacts(&a2).gateway_port(),
            18799
        );

        let a3 = artifacts_with_id("docker:local-3");
        assert_eq!(
            DockerInstanceInfo::from_artifacts(&a3).gateway_port(),
            18809
        );
    }

    #[test]
    fn gateway_port_zero_falls_back_to_default() {
        let a = artifacts_with_id("docker:local-0");
        let info = DockerInstanceInfo::from_artifacts(&a);
        assert_eq!(info.gateway_port(), 18789);
    }

    #[test]
    fn gateway_port_overflow_falls_back_to_default() {
        let a = artifacts_with_id("docker:local-60000");
        let info = DockerInstanceInfo::from_artifacts(&a);
        assert_eq!(info.gateway_port(), 18789);
    }

    #[test]
    fn gateway_port_non_numeric_suffix_falls_back() {
        let a = artifacts_with_id("docker:staging");
        let info = DockerInstanceInfo::from_artifacts(&a);
        assert_eq!(info.gateway_port(), 18789);
    }

    #[test]
    fn gateway_port_uppercase_local_falls_back() {
        // "docker:LOCAL-2" slug goes through sanitize_slug which lowercases
        let a = artifacts_with_id("docker:LOCAL-2");
        let info = DockerInstanceInfo::from_artifacts(&a);
        assert_eq!(info.gateway_port(), 18799);
    }

    #[test]
    fn gateway_port_empty_number_falls_back() {
        let a = artifacts_with_id("docker:local-");
        let info = DockerInstanceInfo::from_artifacts(&a);
        assert_eq!(info.gateway_port(), 18789);
    }

    // --- verify compose command ---

    #[test]
    fn verify_compose_command_includes_new_env_vars() {
        let cmd = docker_verify_compose_command_for_test("/tmp/openclaw");
        assert!(cmd.contains("COMPOSE_PROJECT_NAME="), "cmd={cmd}");
        assert!(cmd.contains("OPENCLAW_IMAGE="), "cmd={cmd}");
        assert!(cmd.contains("OPENCLAW_GATEWAY_PORT="), "cmd={cmd}");
        assert!(cmd.contains("OPENCLAW_CONFIG_DIR="), "cmd={cmd}");
        assert!(cmd.contains("docker compose config"), "cmd={cmd}");
    }
}
