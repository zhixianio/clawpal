use std::path::PathBuf;

use crate::install::{DockerInstallOptions, InstallError, Result, StepResult};

pub fn pull(options: &DockerInstallOptions) -> Result<StepResult> {
    if options.dry_run {
        return Ok(StepResult {
            step: "docker_pull".to_string(),
            ok: true,
            detail: "dry-run: docker compose pull".to_string(),
        });
    }

    ensure_command_exists("docker")?;
    ensure_command_exists("git")?;

    let repo = ensure_repo_checkout()?;
    let state_dir = openclaw_state_dir(options);
    match run_bash(
        "docker compose pull",
        Some(&repo),
        compose_env(&state_dir),
        "docker_pull",
    ) {
        Ok(step) => Ok(step),
        Err(err) => {
            let message = err.to_string();
            if needs_local_image_fallback(&message) {
                let built = run_bash(
                    "docker build -t openclaw:local -f Dockerfile .",
                    Some(&repo),
                    Vec::new(),
                    "docker_pull_fallback_build",
                )?;
                return Ok(StepResult {
                    step: "docker_pull".to_string(),
                    ok: true,
                    detail: format!(
                        "compose pull unavailable for openclaw image; built local image fallback: {}",
                        built.detail
                    ),
                });
            }
            Err(err)
        }
    }
}

pub fn configure(options: &DockerInstallOptions) -> Result<StepResult> {
    let state_dir = openclaw_state_dir(options);
    if options.dry_run {
        return Ok(StepResult {
            step: "docker_configure".to_string(),
            ok: true,
            detail: format!("dry-run: prepare {state_dir}"),
        });
    }

    run_bash(
        &format!(
            "mkdir -p \"{state}\" \"{state}/workspace\" && [ -f \"{state}/openclaw.json\" ] || printf '{{}}' > \"{state}/openclaw.json\"",
            state = state_dir
        ),
        None,
        Vec::new(),
        "docker_configure",
    )
}

pub fn up(options: &DockerInstallOptions) -> Result<StepResult> {
    if options.dry_run {
        return Ok(StepResult {
            step: "docker_up".to_string(),
            ok: true,
            detail: "dry-run: docker compose up -d".to_string(),
        });
    }

    ensure_command_exists("docker")?;
    let repo = ensure_repo_checkout()?;
    let state_dir = openclaw_state_dir(options);
    run_bash(
        "docker compose up -d",
        Some(&repo),
        compose_env(&state_dir),
        "docker_up",
    )
}

fn run_bash(
    command: &str,
    cwd: Option<&PathBuf>,
    envs: Vec<(&'static str, String)>,
    step: &str,
) -> Result<StepResult> {
    let mut cmd = std::process::Command::new("bash");
    cmd.args(["-ilc", command]);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    for (key, value) in envs {
        cmd.env(key, value);
    }

    let output = cmd
        .output()
        .map_err(|e| InstallError::Step(format!("{step} failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(InstallError::Step(format!(
            "{step} failed (code {:?}): {detail}",
            output.status.code()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if stdout.is_empty() {
        command.to_string()
    } else {
        stdout
    };

    Ok(StepResult {
        step: step.to_string(),
        ok: true,
        detail,
    })
}

fn ensure_command_exists(name: &str) -> Result<()> {
    if command_exists(name) {
        Ok(())
    } else {
        Err(InstallError::Step(format!(
            "required command not found in PATH: {name}"
        )))
    }
}

fn command_exists(name: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path_var) {
        #[cfg(windows)]
        {
            let with_exe = dir.join(format!("{name}.exe"));
            if with_exe.is_file() {
                return true;
            }
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let candidate = dir.join(name);
            if let Ok(meta) = std::fs::metadata(&candidate) {
                if meta.is_file() && (meta.permissions().mode() & 0o111 != 0) {
                    return true;
                }
            }
        }
    }
    false
}

fn needs_local_image_fallback(message: &str) -> bool {
    let lower = message.to_lowercase();
    (lower.contains("pull access denied for openclaw")
        || lower.contains("repository does not exist")
        || lower.contains("requested access to the resource is denied"))
        && lower.contains("docker_pull failed")
}

fn repo_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".clawpal")
        .join("install")
        .join("openclaw-docker")
}

fn ensure_repo_checkout() -> Result<PathBuf> {
    let repo = repo_dir();
    if repo.join(".git").exists() {
        return Ok(repo);
    }

    if let Some(parent) = repo.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            InstallError::Step(format!("failed to create {}: {e}", parent.display()))
        })?;
    }

    let command = format!(
        "if [ -d \"{repo}\"/.git ]; then echo 'repo exists'; else git clone https://github.com/openclaw/openclaw.git \"{repo}\"; fi",
        repo = repo.to_string_lossy()
    );
    run_bash(&command, None, Vec::new(), "docker_pull")?;
    Ok(repo)
}

fn openclaw_home(options: &DockerInstallOptions) -> String {
    options
        .home
        .as_ref()
        .map(|v| shellexpand::tilde(v).to_string())
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            format!("{home}/.clawpal/docker-local")
        })
}

fn openclaw_state_dir(options: &DockerInstallOptions) -> String {
    format!("{}/.openclaw", openclaw_home(options))
}

fn compose_env(state_dir: &str) -> Vec<(&'static str, String)> {
    vec![
        ("OPENCLAW_CONFIG_DIR", state_dir.to_string()),
        ("OPENCLAW_WORKSPACE_DIR", format!("{state_dir}/workspace")),
        ("OPENCLAW_GATEWAY_TOKEN", "clawpal-install".to_string()),
        ("CLAUDE_AI_SESSION_KEY", "dummy".to_string()),
        ("CLAUDE_WEB_SESSION_KEY", "dummy".to_string()),
        ("CLAUDE_WEB_COOKIE", "dummy".to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_returns_step_result_on_dry_run() {
        let options = DockerInstallOptions {
            dry_run: true,
            ..DockerInstallOptions::default()
        };
        let result = pull(&options).expect("pull");
        assert!(result.ok);
    }

    #[test]
    fn configure_returns_step_result_on_dry_run() {
        let options = DockerInstallOptions {
            dry_run: true,
            ..DockerInstallOptions::default()
        };
        let result = configure(&options).expect("configure");
        assert!(result.ok);
    }

    #[test]
    fn up_returns_step_result_on_dry_run() {
        let options = DockerInstallOptions {
            dry_run: true,
            ..DockerInstallOptions::default()
        };
        let result = up(&options).expect("up");
        assert!(result.ok);
    }

    #[test]
    fn detects_pull_access_denied_for_fallback() {
        let msg = "docker_pull failed (code Some(1)): pull access denied for openclaw";
        assert!(needs_local_image_fallback(msg));
    }

    #[test]
    fn command_exists_returns_false_when_path_is_empty_dir() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let empty =
            std::env::temp_dir().join(format!("clawpal-empty-path-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&empty).expect("create empty dir");
        let original = std::env::var_os("PATH");
        std::env::set_var("PATH", &empty);
        let exists = command_exists("docker");
        if let Some(path) = original {
            std::env::set_var("PATH", path);
        } else {
            std::env::remove_var("PATH");
        }
        assert!(!exists);
    }

    #[test]
    fn needs_local_image_fallback_repo_does_not_exist() {
        let msg = "docker_pull failed (code Some(1)): repository does not exist or may require authentication";
        assert!(needs_local_image_fallback(msg));
    }

    #[test]
    fn needs_local_image_fallback_access_denied() {
        let msg = "docker_pull failed: requested access to the resource is denied";
        assert!(needs_local_image_fallback(msg));
    }

    #[test]
    fn needs_local_image_fallback_unrelated_error() {
        let msg = "docker_pull failed: network timeout";
        assert!(!needs_local_image_fallback(msg));
    }

    #[test]
    fn needs_local_image_fallback_missing_docker_pull_prefix() {
        // Must contain "docker_pull failed" to trigger
        let msg = "pull access denied for openclaw";
        assert!(!needs_local_image_fallback(msg));
    }

    #[test]
    fn compose_env_returns_expected_keys() {
        let env = compose_env("/tmp/test-state");
        let keys: Vec<&str> = env.iter().map(|(k, _)| *k).collect();
        assert!(keys.contains(&"OPENCLAW_CONFIG_DIR"));
        assert!(keys.contains(&"OPENCLAW_WORKSPACE_DIR"));
        assert!(keys.contains(&"OPENCLAW_GATEWAY_TOKEN"));
    }

    #[test]
    fn openclaw_state_dir_uses_home_option() {
        let options = DockerInstallOptions {
            home: Some("/custom/home".to_string()),
            ..DockerInstallOptions::default()
        };
        let dir = openclaw_state_dir(&options);
        assert_eq!(dir, "/custom/home/.openclaw");
    }

    #[test]
    fn openclaw_state_dir_default() {
        let options = DockerInstallOptions::default();
        let dir = openclaw_state_dir(&options);
        assert!(dir.ends_with("/.openclaw"));
    }

    #[test]
    fn openclaw_home_expands_tilde() {
        let options = DockerInstallOptions {
            home: Some("~/my-openclaw".to_string()),
            ..DockerInstallOptions::default()
        };
        let home = openclaw_home(&options);
        assert!(!home.starts_with('~'));
        assert!(home.ends_with("/my-openclaw"));
    }
}
