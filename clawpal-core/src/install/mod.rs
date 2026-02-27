pub mod docker;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::health::check_instance;
use crate::instance::{Instance, InstanceRegistry, InstanceType};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DockerInstallOptions {
    pub home: Option<String>,
    pub label: Option<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocalInstallOptions {
    pub home: Option<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    pub step: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    pub ok: bool,
    pub instance_id: Option<String>,
    pub steps: Vec<StepResult>,
}

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("install step failed: {0}")]
    Step(String),
    #[error("failed to update instance registry: {0}")]
    Registry(String),
}

pub type Result<T> = std::result::Result<T, InstallError>;

pub fn install_docker(options: DockerInstallOptions) -> Result<InstallResult> {
    let mut steps = Vec::new();
    steps.push(docker::pull(&options)?);
    steps.push(docker::configure(&options)?);
    steps.push(docker::up(&options)?);

    let home = options
        .home
        .clone()
        .unwrap_or_else(|| "~/.clawpal/docker-local".to_string());
    let instance_id = "docker:local".to_string();
    let label = options.label.unwrap_or_else(|| "Docker Local".to_string());

    let mut registry =
        InstanceRegistry::load().map_err(|e| InstallError::Registry(e.to_string()))?;
    let _ = registry.remove(&instance_id);
    registry
        .add(Instance {
            id: instance_id.clone(),
            instance_type: InstanceType::Docker,
            label,
            openclaw_home: Some(home),
            clawpal_data_dir: None,
            ssh_host_config: None,
        })
        .map_err(|e| InstallError::Registry(e.to_string()))?;
    registry
        .save()
        .map_err(|e| InstallError::Registry(e.to_string()))?;

    Ok(InstallResult {
        ok: true,
        instance_id: Some(instance_id),
        steps,
    })
}

pub fn install_local(options: LocalInstallOptions) -> Result<InstallResult> {
    let mut steps = Vec::new();
    if options.dry_run {
        steps.push(StepResult {
            step: "local_install".to_string(),
            ok: true,
            detail: "dry-run: skipped local installer".to_string(),
        });
    } else {
        let output = std::process::Command::new("bash")
            .args(["-ilc", "command -v openclaw >/dev/null 2>&1 || true"])
            .output()
            .map_err(|e| InstallError::Step(e.to_string()))?;
        steps.push(StepResult {
            step: "local_install".to_string(),
            ok: output.status.success(),
            detail: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    let instance = Instance {
        id: "local".to_string(),
        instance_type: InstanceType::Local,
        label: "Local".to_string(),
        openclaw_home: options.home,
        clawpal_data_dir: None,
        ssh_host_config: None,
    };
    let health = check_instance(&instance).ok();
    steps.push(StepResult {
        step: "health_check".to_string(),
        ok: health.as_ref().map(|h| h.healthy).unwrap_or(false),
        detail: health
            .as_ref()
            .and_then(|h| h.version.clone())
            .unwrap_or_else(|| "health unavailable".to_string()),
    });

    Ok(InstallResult {
        ok: steps.iter().all(|step| step.ok),
        instance_id: Some("local".to_string()),
        steps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    fn temp_data_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("clawpal-core-install-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn install_docker_runs_pipeline_on_dry_run() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CLAWPAL_DATA_DIR", temp_data_dir());
        let options = DockerInstallOptions {
            dry_run: true,
            ..DockerInstallOptions::default()
        };
        let result = install_docker(options).expect("install_docker");
        assert!(result.ok);
        assert_eq!(result.steps.len(), 3);
    }

    #[test]
    fn install_local_returns_result_on_dry_run() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CLAWPAL_DATA_DIR", temp_data_dir());
        let options = LocalInstallOptions {
            dry_run: true,
            ..LocalInstallOptions::default()
        };
        let result = install_local(options).expect("install_local");
        assert_eq!(result.instance_id.as_deref(), Some("local"));
    }
}
