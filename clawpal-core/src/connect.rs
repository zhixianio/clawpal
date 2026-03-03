use thiserror::Error;
use uuid::Uuid;

use crate::instance::{Instance, InstanceRegistry, InstanceType, SshHostConfig};
use crate::ssh::SshSession;

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("docker home does not exist: {0}")]
    DockerHomeMissing(String),
    #[error("failed to update instance registry: {0}")]
    Registry(String),
    #[error("local home does not exist: {0}")]
    LocalHomeMissing(String),
    #[error("ssh connect failed: {0}")]
    Ssh(String),
}

pub type Result<T> = std::result::Result<T, ConnectError>;

pub async fn connect_docker(
    home: &str,
    label: Option<&str>,
    instance_id: Option<&str>,
) -> Result<Instance> {
    let expanded = shellexpand::tilde(home).to_string();
    if !std::path::Path::new(&expanded).exists() {
        return Err(ConnectError::DockerHomeMissing(expanded));
    }

    let id = match instance_id {
        Some(explicit) if !explicit.is_empty() => explicit.to_string(),
        _ => format!("docker:{}", slug_from_home(&expanded)),
    };
    let instance = Instance {
        id,
        instance_type: InstanceType::Docker,
        label: label.unwrap_or("docker-local").to_string(),
        openclaw_home: Some(expanded),
        clawpal_data_dir: None,
        ssh_host_config: None,
    };
    upsert_instance(instance)
}

pub async fn connect_local(
    home: &str,
    label: Option<&str>,
    instance_id: Option<&str>,
) -> Result<Instance> {
    let expanded = shellexpand::tilde(home).to_string();
    if !std::path::Path::new(&expanded).exists() {
        return Err(ConnectError::LocalHomeMissing(expanded));
    }
    let id = match instance_id {
        Some(explicit) if !explicit.is_empty() => explicit.to_string(),
        _ => format!("local:{}", slug_from_home(&expanded)),
    };
    let default_label = id
        .strip_prefix("wsl2:")
        .unwrap_or("local-instance")
        .to_string();
    let instance = Instance {
        id,
        instance_type: InstanceType::Local,
        label: label.unwrap_or(default_label.as_str()).to_string(),
        openclaw_home: Some(expanded),
        clawpal_data_dir: None,
        ssh_host_config: None,
    };
    upsert_instance(instance)
}

pub async fn connect_ssh(host_config: SshHostConfig) -> Result<Instance> {
    let session = SshSession::connect(&host_config)
        .await
        .map_err(|e| ConnectError::Ssh(e.to_string()))?;
    let _ = session
        .exec("echo connected")
        .await
        .map_err(|e| ConnectError::Ssh(e.to_string()))
        .and_then(|output| {
            if output.exit_code == 0 {
                Ok(output)
            } else {
                Err(ConnectError::Ssh(format!(
                    "remote connectivity probe failed with exit code {}: {}",
                    output.exit_code, output.stderr
                )))
            }
        })?;

    let instance = Instance {
        id: host_config.id.clone(),
        instance_type: InstanceType::RemoteSsh,
        label: host_config.label.clone(),
        openclaw_home: None,
        clawpal_data_dir: None,
        ssh_host_config: Some(host_config),
    };
    upsert_instance(instance)
}

fn upsert_instance(instance: Instance) -> Result<Instance> {
    let mut registry =
        InstanceRegistry::load().map_err(|e| ConnectError::Registry(e.to_string()))?;
    let _ = registry.remove(&instance.id);
    registry
        .add(instance.clone())
        .map_err(|e| ConnectError::Registry(e.to_string()))?;
    registry
        .save()
        .map_err(|e| ConnectError::Registry(e.to_string()))?;
    Ok(instance)
}

fn slug_from_home(home: &str) -> String {
    let raw = std::path::Path::new(home)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("local");
    let mut slug = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        slug.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn connect_docker_registers_instance() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let data_dir = std::env::temp_dir().join(format!("clawpal-connect-{}", Uuid::new_v4()));
        let docker_home =
            std::env::temp_dir().join(format!("clawpal-connect-home-{}", Uuid::new_v4()));
        fs::create_dir_all(&data_dir).expect("create data dir");
        fs::create_dir_all(&docker_home).expect("create home dir");
        std::env::set_var("CLAWPAL_DATA_DIR", &data_dir);

        let instance = connect_docker(
            docker_home.to_str().unwrap_or_default(),
            Some("Docker Test"),
            None,
        )
        .await
        .expect("connect docker");
        assert!(matches!(instance.instance_type, InstanceType::Docker));
    }

    #[tokio::test]
    async fn connect_ssh_fails_with_empty_host() {
        let config = SshHostConfig {
            id: "ssh:test".to_string(),
            label: "SSH".to_string(),
            host: String::new(),
            port: 22,
            username: "ubuntu".to_string(),
            auth_method: "key".to_string(),
            key_path: None,
            password: None,
            passphrase: None,
        };
        let result = connect_ssh(config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connect_docker_returns_error_for_missing_home() {
        let result = connect_docker("/nonexistent/path/clawpal-test-12345", None, None).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("docker home does not exist"));
    }

    #[tokio::test]
    async fn connect_local_returns_error_for_missing_home() {
        let result = connect_local("/nonexistent/path/clawpal-test-12345", None, None).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("local home does not exist"));
    }

    #[tokio::test]
    async fn connect_docker_uses_explicit_instance_id() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let data_dir = std::env::temp_dir().join(format!("clawpal-connect-id-{}", Uuid::new_v4()));
        let docker_home =
            std::env::temp_dir().join(format!("clawpal-connect-id-home-{}", Uuid::new_v4()));
        fs::create_dir_all(&data_dir).expect("create data dir");
        fs::create_dir_all(&docker_home).expect("create home dir");
        std::env::set_var("CLAWPAL_DATA_DIR", &data_dir);

        let instance = connect_docker(
            docker_home.to_str().unwrap(),
            Some("My Docker"),
            Some("docker:custom-id"),
        )
        .await
        .expect("connect docker with explicit id");
        assert_eq!(instance.id, "docker:custom-id");
        assert_eq!(instance.label, "My Docker");
    }

    #[tokio::test]
    async fn connect_local_uses_explicit_instance_id() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let data_dir = std::env::temp_dir().join(format!("clawpal-local-id-{}", Uuid::new_v4()));
        let local_home =
            std::env::temp_dir().join(format!("clawpal-local-id-home-{}", Uuid::new_v4()));
        fs::create_dir_all(&data_dir).expect("create data dir");
        fs::create_dir_all(&local_home).expect("create home dir");
        std::env::set_var("CLAWPAL_DATA_DIR", &data_dir);

        let instance = connect_local(
            local_home.to_str().unwrap(),
            Some("My Local"),
            Some("local:custom-id"),
        )
        .await
        .expect("connect local with explicit id");
        assert_eq!(instance.id, "local:custom-id");
        assert_eq!(instance.label, "My Local");
    }

    #[test]
    fn slug_from_home_basic() {
        // Leading dots become hyphens, then leading hyphens are trimmed
        assert_eq!(slug_from_home("/home/user/.openclaw"), "openclaw");
    }

    #[test]
    fn slug_from_home_sanitizes_special_chars() {
        let slug = slug_from_home("/path/to/my dir@123");
        assert!(!slug.contains(' '));
        assert!(!slug.contains('@'));
        assert!(!slug.contains("--"));
    }

    #[test]
    fn slug_from_home_empty_dirname_generates_uuid() {
        // A path whose file_name() component becomes empty after sanitization
        let slug = slug_from_home("/");
        // Should be a UUID (36 chars with hyphens)
        assert!(!slug.is_empty());
    }

    #[test]
    fn connect_error_display_messages() {
        let err = ConnectError::DockerHomeMissing("/foo".to_string());
        assert!(err.to_string().contains("/foo"));

        let err = ConnectError::Registry("io error".to_string());
        assert!(err.to_string().contains("io error"));

        let err = ConnectError::Ssh("timeout".to_string());
        assert!(err.to_string().contains("timeout"));

        let err = ConnectError::LocalHomeMissing("/bar".to_string());
        assert!(err.to_string().contains("/bar"));
    }
}
