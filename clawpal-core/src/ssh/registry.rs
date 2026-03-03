use std::collections::HashSet;

use crate::instance::{Instance, InstanceRegistry, InstanceType, SshHostConfig};

fn sanitize_id_segment(raw: &str) -> String {
    let lowered = raw.trim().to_ascii_lowercase();
    let mut out = String::with_capacity(lowered.len());
    let mut last_dash = false;
    for ch in lowered.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "remote".to_string()
    } else {
        trimmed
    }
}

fn canonical_ssh_host_id(host: &SshHostConfig) -> String {
    let explicit = host.id.trim();
    if !explicit.is_empty() {
        return explicit.to_string();
    }
    let base = if !host.host.trim().is_empty() {
        host.host.trim()
    } else if !host.label.trim().is_empty() {
        host.label.trim()
    } else {
        "remote"
    };
    format!("ssh:{}", sanitize_id_segment(base))
}

pub fn list_ssh_hosts() -> Result<Vec<SshHostConfig>, String> {
    let registry = InstanceRegistry::load().map_err(|e| e.to_string())?;
    let mut seen = HashSet::new();
    Ok(registry
        .list()
        .into_iter()
        .filter(|instance| matches!(instance.instance_type, InstanceType::RemoteSsh))
        .filter_map(|instance| instance.ssh_host_config)
        .filter(|host| {
            let key = format!(
                "{}@{}:{}",
                host.username,
                host.host.to_ascii_lowercase(),
                host.port
            );
            seen.insert(key)
        })
        .collect())
}

pub fn upsert_ssh_host(host: SshHostConfig) -> Result<SshHostConfig, String> {
    let mut registry = InstanceRegistry::load().map_err(|e| e.to_string())?;
    let mut host = host;
    let id = canonical_ssh_host_id(&host);
    host.id = id.clone();
    let duplicate_ids = registry
        .list()
        .into_iter()
        .filter(|instance| instance.id != id)
        .filter(|instance| matches!(instance.instance_type, InstanceType::RemoteSsh))
        .filter_map(|instance| instance.ssh_host_config.map(|cfg| (instance.id, cfg)))
        .filter(|(_, cfg)| {
            cfg.username == host.username
                && cfg.port == host.port
                && cfg.host.eq_ignore_ascii_case(&host.host)
        })
        .map(|(instance_id, _)| instance_id)
        .collect::<Vec<_>>();
    for duplicate_id in duplicate_ids {
        let _ = registry.remove(&duplicate_id);
    }
    let existing = registry.get(&id).cloned();
    if existing.is_some() {
        let _ = registry.remove(&id);
    }
    let instance = Instance {
        id,
        instance_type: InstanceType::RemoteSsh,
        label: host.label.clone(),
        openclaw_home: existing
            .as_ref()
            .and_then(|instance| instance.openclaw_home.clone()),
        clawpal_data_dir: existing
            .as_ref()
            .and_then(|instance| instance.clawpal_data_dir.clone()),
        ssh_host_config: Some(host.clone()),
    };
    registry.add(instance).map_err(|e| e.to_string())?;
    registry.save().map_err(|e| e.to_string())?;
    Ok(host)
}

pub fn delete_ssh_host(host_id: &str) -> Result<bool, String> {
    let mut registry = InstanceRegistry::load().map_err(|e| e.to_string())?;
    let removed = registry.remove(host_id).is_some();
    registry.save().map_err(|e| e.to_string())?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    fn temp_data_dir() -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("clawpal-core-ssh-registry-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn sample_host(id: &str) -> SshHostConfig {
        SshHostConfig {
            id: id.to_string(),
            label: "Remote".to_string(),
            host: "example.com".to_string(),
            port: 22,
            username: "ubuntu".to_string(),
            auth_method: "key".to_string(),
            key_path: Some("~/.ssh/id_ed25519".to_string()),
            password: None,
            passphrase: None,
        }
    }

    #[test]
    fn list_ssh_hosts_returns_saved_hosts() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = temp_data_dir();
        std::env::set_var("CLAWPAL_DATA_DIR", &dir);
        let host = sample_host("ssh:test-list");
        upsert_ssh_host(host).expect("upsert");

        let hosts = list_ssh_hosts().expect("list");
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].id, "ssh:test-list");
    }

    #[test]
    fn upsert_ssh_host_writes_registry() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = temp_data_dir();
        std::env::set_var("CLAWPAL_DATA_DIR", &dir);
        let host = sample_host("ssh:test-upsert");
        let saved = upsert_ssh_host(host).expect("upsert");
        assert_eq!(saved.id, "ssh:test-upsert");
    }

    #[test]
    fn delete_ssh_host_removes_record() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = temp_data_dir();
        std::env::set_var("CLAWPAL_DATA_DIR", &dir);
        let host = sample_host("ssh:test-delete");
        upsert_ssh_host(host).expect("upsert");
        let removed = delete_ssh_host("ssh:test-delete").expect("delete");
        assert!(removed);
    }

    #[test]
    fn upsert_ssh_host_replaces_duplicate_endpoint_records() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = temp_data_dir();
        std::env::set_var("CLAWPAL_DATA_DIR", &dir);

        let mut first = sample_host("ssh:test-dup-1");
        first.label = "vm1".to_string();
        first.host = "vm1".to_string();
        first.username = "ubuntu".to_string();
        upsert_ssh_host(first).expect("upsert first");

        let mut second = sample_host("ssh:test-dup-2");
        second.label = "vm1".to_string();
        second.host = "VM1".to_string();
        second.username = "ubuntu".to_string();
        upsert_ssh_host(second).expect("upsert second");

        let hosts = list_ssh_hosts().expect("list");
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].id, "ssh:test-dup-2");
    }

    #[test]
    fn upsert_ssh_host_generates_id_when_missing() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = temp_data_dir();
        std::env::set_var("CLAWPAL_DATA_DIR", &dir);
        let mut host = sample_host("");
        host.id = "".to_string();
        host.host = "Vm 1".to_string();
        let saved = upsert_ssh_host(host).expect("upsert");
        assert_eq!(saved.id, "ssh:vm-1");
    }
}
