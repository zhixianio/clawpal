use std::collections::HashMap;
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::instance::{Instance, InstanceType};
use crate::openclaw::{parse_json_output, CliOutput, OpenclawCli};

const HEALTH_SSH_CONNECT_TIMEOUT_SECS: u64 = 10;
const HEALTH_SSH_SERVER_ALIVE_INTERVAL_SECS: u64 = 10;
const HEALTH_SSH_SERVER_ALIVE_COUNT_MAX: u64 = 2;
const HEALTH_REMOTE_COMMAND_TIMEOUT_SECS: u64 = 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HealthStatus {
    pub healthy: bool,
    pub active_agents: u32,
    pub version: Option<String>,
}

#[derive(Debug, Error)]
pub enum HealthError {
    #[error("missing ssh host config for remote instance '{0}'")]
    MissingSshConfig(String),
    #[error("health check command failed: {0}")]
    Command(String),
    #[error("failed to run ssh: {0}")]
    SshIo(#[from] std::io::Error),
    #[error("json parse failed: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, HealthError>;

pub fn check_instance(instance: &Instance) -> Result<HealthStatus> {
    let cli = OpenclawCli::new();
    check_instance_with_cli(instance, &cli)
}

fn check_instance_with_cli(instance: &Instance, cli: &OpenclawCli) -> Result<HealthStatus> {
    match instance.instance_type {
        InstanceType::Local | InstanceType::Docker => check_local_or_docker(instance, cli),
        InstanceType::RemoteSsh => check_remote_ssh(instance),
    }
}

fn check_local_or_docker(instance: &Instance, cli: &OpenclawCli) -> Result<HealthStatus> {
    let mut env = HashMap::new();
    if let Some(home) = &instance.openclaw_home {
        env.insert("OPENCLAW_HOME".to_string(), home.clone());
    }

    let agents_output = cli
        .run_with_env(&["agents", "list", "--json"], Some(&env))
        .map_err(|e| HealthError::Command(e.to_string()))?;
    let active_agents = parse_active_agents(&agents_output)?;

    let version_output = cli
        .run_with_env(&["--version"], Some(&env))
        .map_err(|e| HealthError::Command(e.to_string()))?;
    let version = if version_output.exit_code == 0 {
        Some(version_output.stdout.trim().to_string())
    } else {
        None
    };

    Ok(HealthStatus {
        healthy: agents_output.exit_code == 0,
        active_agents,
        version,
    })
}

fn check_remote_ssh(instance: &Instance) -> Result<HealthStatus> {
    let ssh = instance
        .ssh_host_config
        .as_ref()
        .ok_or_else(|| HealthError::MissingSshConfig(instance.id.clone()))?;

    let destination = if ssh.username.trim().is_empty() {
        ssh.host.clone()
    } else {
        format!("{}@{}", ssh.username, ssh.host)
    };

    let mut base_args = vec![
        "-p".to_string(),
        ssh.port.to_string(),
        "-o".to_string(),
        format!("ConnectTimeout={HEALTH_SSH_CONNECT_TIMEOUT_SECS}"),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        format!("ServerAliveInterval={HEALTH_SSH_SERVER_ALIVE_INTERVAL_SECS}"),
        "-o".to_string(),
        format!("ServerAliveCountMax={HEALTH_SSH_SERVER_ALIVE_COUNT_MAX}"),
    ];
    if let Some(key_path) = ssh.key_path.clone() {
        if !key_path.trim().is_empty() {
            base_args.push("-i".to_string());
            base_args.push(key_path);
        }
    }
    base_args.push(destination);

    let agents_command = if let Some(home) = &instance.openclaw_home {
        format!("OPENCLAW_HOME='{}' openclaw agents list --json", home)
    } else {
        "openclaw agents list --json".to_string()
    };
    let mut agents_args = base_args.clone();
    agents_args.push(wrap_remote_health_command(&agents_command));
    let agents_output = run_ssh_command(&agents_args)?;
    let active_agents = parse_active_agents(&agents_output)?;

    let version_command = if let Some(home) = &instance.openclaw_home {
        format!("OPENCLAW_HOME='{}' openclaw --version", home)
    } else {
        "openclaw --version".to_string()
    };
    let mut version_args = base_args;
    version_args.push(wrap_remote_health_command(&version_command));
    let version_output = run_ssh_command(&version_args)?;
    let version = if version_output.exit_code == 0 {
        Some(version_output.stdout.trim().to_string())
    } else {
        None
    };

    Ok(HealthStatus {
        healthy: agents_output.exit_code == 0,
        active_agents,
        version,
    })
}

fn run_ssh_command(args: &[String]) -> Result<CliOutput> {
    let output = Command::new("ssh").args(args).output()?;
    Ok(CliOutput {
        stdout: String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string(),
        stderr: String::from_utf8_lossy(&output.stderr)
            .trim_end()
            .to_string(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}

fn wrap_remote_health_command(command: &str) -> String {
    let login_wrapped = crate::shell::wrap_login_shell_eval(command);
    let escaped = shell_escape(&login_wrapped);
    format!(
        "if command -v timeout >/dev/null 2>&1; then timeout {HEALTH_REMOTE_COMMAND_TIMEOUT_SECS}s sh -c {escaped}; else sh -c {escaped}; fi"
    )
}

fn shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn parse_active_agents(output: &CliOutput) -> Result<u32> {
    if output.exit_code != 0 {
        return Ok(0);
    }
    let json = parse_json_output(output).map_err(|e| HealthError::Command(e.to_string()))?;
    Ok(count_agents(&json))
}

fn count_agents(value: &Value) -> u32 {
    if let Some(array) = value.as_array() {
        return array.len() as u32;
    }
    if let Some(array) = value.get("agents").and_then(Value::as_array) {
        return array.len() as u32;
    }
    if value.get("agents").is_some() {
        return 1;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::{Instance, InstanceType, SshHostConfig};
    use serde_json::json;
    use uuid::Uuid;

    #[cfg(unix)]
    fn create_fake_openclaw_script() -> String {
        let dir = std::env::temp_dir().join(format!("clawpal-core-health-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("fake-openclaw.sh");
        std::fs::write(
            &path,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"openclaw 1.2.3\"; exit 0; fi\necho '[{\"id\":\"main\"}]'\n",
        )
        .expect("write script");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path.to_string_lossy().to_string()
    }

    #[test]
    #[cfg(unix)]
    fn check_instance_reports_local_health() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let cli = OpenclawCli::with_bin(create_fake_openclaw_script());
        let instance = Instance {
            id: "local".to_string(),
            instance_type: InstanceType::Local,
            label: "Local".to_string(),
            openclaw_home: None,
            clawpal_data_dir: None,
            ssh_host_config: None,
        };
        let status = check_instance_with_cli(&instance, &cli).expect("check");
        assert!(status.healthy);
        assert_eq!(status.active_agents, 1);
        assert_eq!(status.version.as_deref(), Some("openclaw 1.2.3"));
    }

    #[cfg(unix)]
    fn create_fake_openclaw_unhealthy_script() -> String {
        let dir = std::env::temp_dir().join(format!("clawpal-core-health-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("fake-openclaw-unhealthy.sh");
        std::fs::write(
            &path,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"openclaw 9.9.9\"; exit 0; fi\necho 'failed' >&2\nexit 2\n",
        )
        .expect("write script");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path.to_string_lossy().to_string()
    }

    #[test]
    #[cfg(unix)]
    fn check_instance_reports_unhealthy_when_agents_command_fails() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let cli = OpenclawCli::with_bin(create_fake_openclaw_unhealthy_script());
        let instance = Instance {
            id: "local".to_string(),
            instance_type: InstanceType::Local,
            label: "Local".to_string(),
            openclaw_home: None,
            clawpal_data_dir: None,
            ssh_host_config: None,
        };
        let status = check_instance_with_cli(&instance, &cli).expect("check");
        assert!(!status.healthy);
        assert_eq!(status.active_agents, 0);
        assert_eq!(status.version.as_deref(), Some("openclaw 9.9.9"));
    }

    #[test]
    fn wrap_remote_health_command_uses_login_shell_for_bash_and_zsh() {
        let wrapped = wrap_remote_health_command("openclaw --version");
        assert!(wrapped.contains("*/zsh|*/bash) \"$LOGIN_SHELL\" -lc"));
        assert!(wrapped.contains("*) \"$LOGIN_SHELL\" -lc"));
    }

    #[test]
    fn count_agents_array() {
        assert_eq!(count_agents(&json!([{"id":"a"},{"id":"b"}])), 2);
    }

    #[test]
    fn count_agents_wrapped() {
        assert_eq!(count_agents(&json!({"agents":[{"id":"a"}]})), 1);
    }

    #[test]
    fn count_agents_object_fallback() {
        // agents exists but is not an array → returns 1
        assert_eq!(count_agents(&json!({"agents":{"id":"main"}})), 1);
    }

    #[test]
    fn count_agents_empty() {
        assert_eq!(count_agents(&json!({})), 0);
        assert_eq!(count_agents(&json!([])), 0);
    }

    #[test]
    fn parse_active_agents_nonzero_exit() {
        let output = CliOutput {
            stdout: "error".to_string(),
            stderr: String::new(),
            exit_code: 1,
        };
        assert_eq!(parse_active_agents(&output).unwrap(), 0);
    }

    #[test]
    fn parse_active_agents_valid_json() {
        let output = CliOutput {
            stdout: "[{\"id\":\"a\"},{\"id\":\"b\"}]".to_string(),
            stderr: String::new(),
            exit_code: 0,
        };
        assert_eq!(parse_active_agents(&output).unwrap(), 2);
    }

    #[test]
    fn shell_escape_simple() {
        assert_eq!(shell_escape("hello"), "'hello'");
    }

    #[test]
    fn shell_escape_quotes() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn health_error_display() {
        let e = HealthError::MissingSshConfig("my-instance".to_string());
        assert!(e.to_string().contains("my-instance"));

        let e = HealthError::Command("timeout".to_string());
        assert!(e.to_string().contains("timeout"));
    }

    #[test]
    fn check_remote_ssh_fails_without_ssh_config() {
        let instance = Instance {
            id: "ssh:no-config".to_string(),
            instance_type: InstanceType::RemoteSsh,
            label: "No Config".to_string(),
            openclaw_home: None,
            clawpal_data_dir: None,
            ssh_host_config: None,
        };
        let result = check_instance(&instance);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing ssh"));
    }

    #[test]
    #[cfg(unix)]
    fn check_instance_remote_ssh_path_works_with_fake_ssh() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("clawpal-core-health-ssh-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let ssh_bin = dir.join("ssh");
        std::fs::write(
            &ssh_bin,
            "#!/bin/sh\ncase \"$*\" in\n  *\"openclaw agents list --json\"*) echo '[{\"id\":\"main\"}]'; exit 0 ;;\n  *\"openclaw --version\"*) echo 'openclaw 2.0.0'; exit 0 ;;\n  *) echo 'unexpected command' >&2; exit 1 ;;\nesac\n",
        )
        .expect("write fake ssh");
        std::fs::set_permissions(&ssh_bin, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake ssh");

        let original_path = std::env::var_os("PATH");
        let merged_path = format!(
            "{}:{}",
            dir.display(),
            original_path
                .as_ref()
                .and_then(|v| v.to_str())
                .unwrap_or_default()
        );
        std::env::set_var("PATH", merged_path);

        let instance = Instance {
            id: "ssh:remote".to_string(),
            instance_type: InstanceType::RemoteSsh,
            label: "Remote".to_string(),
            openclaw_home: None,
            clawpal_data_dir: None,
            ssh_host_config: Some(SshHostConfig {
                id: "ssh:remote".to_string(),
                label: "Remote".to_string(),
                host: "vm1".to_string(),
                port: 22,
                username: "root".to_string(),
                auth_method: "key".to_string(),
                key_path: None,
                password: None,
                passphrase: None,
            }),
        };
        let status = check_instance(&instance).expect("remote health");
        assert!(status.healthy);
        assert_eq!(status.active_agents, 1);
        assert_eq!(status.version.as_deref(), Some("openclaw 2.0.0"));

        if let Some(path) = original_path {
            std::env::set_var("PATH", path);
        } else {
            std::env::remove_var("PATH");
        }
    }
}
