use serde::Serialize;
use std::path::Path;

use crate::instance::{Instance, InstanceType};
use crate::profile::ModelProfile;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrecheckIssue {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub auto_fixable: bool,
}

pub fn precheck_auth(profiles: &[ModelProfile]) -> Vec<PrecheckIssue> {
    let mut issues = Vec::new();
    for profile in profiles {
        if !profile.enabled {
            continue;
        }
        if profile.provider.trim().is_empty() || profile.model.trim().is_empty() {
            issues.push(PrecheckIssue {
                code: "AUTH_MISCONFIGURED".into(),
                severity: "error".into(),
                message: format!("Profile '{}' has empty provider or model", profile.id),
                auto_fixable: false,
            });
        }
    }
    issues
}

pub fn precheck_registry(registry_path: &Path) -> Vec<PrecheckIssue> {
    if !registry_path.exists() {
        return Vec::new();
    }
    let content = match std::fs::read_to_string(registry_path) {
        Ok(c) => c,
        Err(_) => {
            return vec![PrecheckIssue {
                code: "REGISTRY_CORRUPT".into(),
                severity: "error".into(),
                message: format!("Cannot read registry file: {}", registry_path.display()),
                auto_fixable: false,
            }];
        }
    };
    if serde_json::from_str::<serde_json::Value>(&content).is_err() {
        return vec![PrecheckIssue {
            code: "REGISTRY_CORRUPT".into(),
            severity: "error".into(),
            message: format!(
                "Registry file contains invalid JSON: {}",
                registry_path.display()
            ),
            auto_fixable: false,
        }];
    }
    Vec::new()
}

pub fn precheck_instance_state(instance: &Instance) -> Vec<PrecheckIssue> {
    if matches!(instance.instance_type, InstanceType::RemoteSsh) {
        return Vec::new();
    }
    if let Some(ref home) = instance.openclaw_home {
        if !Path::new(home).exists() {
            return vec![PrecheckIssue {
                code: "INSTANCE_ORPHANED".into(),
                severity: "warn".into(),
                message: format!(
                    "Instance '{}' references missing openclaw_home: {}",
                    instance.id, home
                ),
                auto_fixable: false,
            }];
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precheck_auth_detects_missing_provider() {
        let profiles = vec![ModelProfile {
            id: "test".into(),
            name: "Test".into(),
            provider: "".into(),
            model: "claude-sonnet".into(),
            auth_ref: String::new(),
            api_key: None,
            base_url: None,
            description: None,
            enabled: true,
        }];
        let issues = precheck_auth(&profiles);
        assert!(issues.iter().any(|i| i.code == "AUTH_MISCONFIGURED"));
    }

    #[test]
    fn precheck_auth_passes_valid_profiles() {
        let profiles = vec![ModelProfile {
            id: "ok".into(),
            name: "OK".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet".into(),
            auth_ref: "key-1".into(),
            api_key: None,
            base_url: None,
            description: None,
            enabled: true,
        }];
        let issues = precheck_auth(&profiles);
        assert!(issues.is_empty());
    }

    #[test]
    fn precheck_auth_skips_disabled_profiles() {
        let profiles = vec![ModelProfile {
            id: "disabled".into(),
            name: "Disabled".into(),
            provider: "".into(),
            model: "".into(),
            auth_ref: String::new(),
            api_key: None,
            base_url: None,
            description: None,
            enabled: false,
        }];
        let issues = precheck_auth(&profiles);
        assert!(issues.is_empty());
    }

    #[test]
    fn precheck_registry_detects_missing_file() {
        let issues = precheck_registry(Path::new("/nonexistent/registry.json"));
        assert!(issues.is_empty());
    }

    #[test]
    fn precheck_registry_detects_corrupt_json() {
        let dir = std::env::temp_dir().join(format!("precheck-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("instances.json");
        std::fs::write(&path, "{ corrupt json!!!").unwrap();
        let issues = precheck_registry(&path);
        assert!(issues.iter().any(|i| i.code == "REGISTRY_CORRUPT"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn precheck_instance_state_detects_missing_home() {
        let inst = Instance {
            id: "test".into(),
            instance_type: InstanceType::Local,
            label: "Test".into(),
            openclaw_home: Some("/nonexistent/path/openclaw".into()),
            clawpal_data_dir: None,
            ssh_host_config: None,
        };
        let issues = precheck_instance_state(&inst);
        assert!(issues.iter().any(|i| i.code == "INSTANCE_ORPHANED"));
    }

    #[test]
    fn precheck_instance_state_passes_when_no_home() {
        let inst = Instance {
            id: "remote".into(),
            instance_type: InstanceType::RemoteSsh,
            label: "Remote".into(),
            openclaw_home: None,
            clawpal_data_dir: None,
            ssh_host_config: None,
        };
        let issues = precheck_instance_state(&inst);
        assert!(issues.is_empty());
    }

    #[test]
    fn precheck_auth_detects_missing_model() {
        let profiles = vec![ModelProfile {
            id: "bad".into(),
            name: "Bad".into(),
            provider: "anthropic".into(),
            model: "".into(),
            auth_ref: String::new(),
            api_key: None,
            base_url: None,
            description: None,
            enabled: true,
        }];
        let issues = precheck_auth(&profiles);
        assert!(issues.iter().any(|i| i.code == "AUTH_MISCONFIGURED"));
    }

    #[test]
    fn precheck_auth_multiple_profiles() {
        let profiles = vec![
            ModelProfile {
                id: "good".into(),
                name: "Good".into(),
                provider: "anthropic".into(),
                model: "claude-3".into(),
                auth_ref: String::new(),
                api_key: None,
                base_url: None,
                description: None,
                enabled: true,
            },
            ModelProfile {
                id: "bad".into(),
                name: "Bad".into(),
                provider: "".into(),
                model: "".into(),
                auth_ref: String::new(),
                api_key: None,
                base_url: None,
                description: None,
                enabled: true,
            },
        ];
        let issues = precheck_auth(&profiles);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn precheck_auth_empty_profiles() {
        let issues = precheck_auth(&[]);
        assert!(issues.is_empty());
    }

    #[test]
    fn precheck_registry_valid_json_passes() {
        let dir = std::env::temp_dir().join(format!("precheck-valid-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("instances.json");
        std::fs::write(&path, r#"{"instances":[]}"#).unwrap();
        let issues = precheck_registry(&path);
        assert!(issues.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn precheck_instance_state_local_with_existing_home() {
        let home = std::env::temp_dir().join(format!("precheck-home-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let inst = Instance {
            id: "local".into(),
            instance_type: InstanceType::Local,
            label: "Local".into(),
            openclaw_home: Some(home.to_string_lossy().to_string()),
            clawpal_data_dir: None,
            ssh_host_config: None,
        };
        let issues = precheck_instance_state(&inst);
        assert!(issues.is_empty());
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn precheck_instance_state_no_home_dir_local() {
        let inst = Instance {
            id: "local".into(),
            instance_type: InstanceType::Local,
            label: "Local".into(),
            openclaw_home: None,
            clawpal_data_dir: None,
            ssh_host_config: None,
        };
        let issues = precheck_instance_state(&inst);
        assert!(issues.is_empty());
    }
}
