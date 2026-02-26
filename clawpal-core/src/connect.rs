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
    #[error("ssh connect failed: {0}")]
    Ssh(String),
}

pub type Result<T> = std::result::Result<T, ConnectError>;

pub async fn connect_docker(home: &str, label: Option<&str>) -> Result<Instance> {
    let expanded = shellexpand::tilde(home).to_string();
    if !std::path::Path::new(&expanded).exists() {
        return Err(ConnectError::DockerHomeMissing(expanded));
    }

    let instance = Instance {
        id: format!("docker:{}", slug_from_home(&expanded)),
        instance_type: InstanceType::Docker,
        label: label.unwrap_or("Docker").to_string(),
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
        .map_err(|e| ConnectError::Ssh(e.to_string()))?;

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
        };
        let result = connect_ssh(config).await;
        assert!(result.is_err());
    }
}
