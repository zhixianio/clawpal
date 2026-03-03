use std::fs;
use std::sync::Mutex;

use clawpal_core::connect::{connect_docker, connect_ssh};
use clawpal_core::install::{install_docker, DockerInstallOptions};
use clawpal_core::instance::{InstanceRegistry, InstanceType, SshHostConfig};
use clawpal_core::ssh::registry as ssh_registry;
use uuid::Uuid;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn temp_dir(prefix: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

#[cfg(unix)]
fn create_fake_ssh_bin(success: bool) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let dir = temp_dir("clawpal-core-fake-ssh");
    let path = dir.join("ssh");
    let script = if success {
        "#!/bin/sh\necho connected\nexit 0\n"
    } else {
        "#!/bin/sh\necho connection failed >&2\nexit 255\n"
    };
    fs::write(&path, script).expect("write fake ssh");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod fake ssh");
    path
}

#[test]
fn install_docker_dry_run_registers_docker_instance() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data_dir = temp_dir("clawpal-core-workflow-data");
    let home_dir = temp_dir("clawpal-core-workflow-home");
    std::env::set_var("CLAWPAL_DATA_DIR", &data_dir);

    let result = install_docker(DockerInstallOptions {
        home: Some(home_dir.to_string_lossy().to_string()),
        label: Some("Docker Workflow".to_string()),
        dry_run: true,
    })
    .expect("install docker dry-run should succeed");

    assert!(result.ok);
    assert_eq!(result.instance_id.as_deref(), Some("docker:local"));

    let registry = InstanceRegistry::load().expect("load registry");
    let instance = registry
        .get("docker:local")
        .expect("docker instance should be saved");
    assert!(matches!(instance.instance_type, InstanceType::Docker));
    assert_eq!(instance.label, "Docker Workflow");
}

#[tokio::test]
async fn connect_docker_registers_instance() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data_dir = temp_dir("clawpal-core-connect-data");
    let home_dir = temp_dir("clawpal-core-connect-home");
    std::env::set_var("CLAWPAL_DATA_DIR", &data_dir);

    let instance = connect_docker(
        home_dir.to_string_lossy().as_ref(),
        Some("Connect Docker"),
        None,
    )
    .await
    .expect("connect docker should succeed");

    assert!(matches!(instance.instance_type, InstanceType::Docker));
    assert_eq!(instance.label, "Connect Docker");

    let registry = InstanceRegistry::load().expect("load registry");
    assert!(registry.get(&instance.id).is_some());
}

#[test]
fn ssh_registry_roundtrip_via_instance_registry() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data_dir = temp_dir("clawpal-core-ssh-registry-data");
    std::env::set_var("CLAWPAL_DATA_DIR", &data_dir);

    let host = SshHostConfig {
        id: "ssh:workflow-vm1".to_string(),
        label: "Workflow VM1".to_string(),
        host: "vm1".to_string(),
        port: 22,
        username: "root".to_string(),
        auth_method: "key".to_string(),
        key_path: None,
        password: None,
        passphrase: None,
    };

    ssh_registry::upsert_ssh_host(host.clone()).expect("upsert ssh host");
    let listed = ssh_registry::list_ssh_hosts().expect("list ssh hosts");
    assert!(listed.iter().any(|h| h.id == host.id));

    let removed = ssh_registry::delete_ssh_host(&host.id).expect("delete ssh host");
    assert!(removed);
}

#[test]
fn install_docker_reports_missing_docker_command() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data_dir = temp_dir("clawpal-core-install-error-data");
    let home_dir = temp_dir("clawpal-core-install-error-home");
    std::env::set_var("CLAWPAL_DATA_DIR", &data_dir);
    let original_path = std::env::var_os("PATH");
    let empty = temp_dir("clawpal-core-empty-path");
    std::env::set_var("PATH", &empty);

    let result = install_docker(DockerInstallOptions {
        home: Some(home_dir.to_string_lossy().to_string()),
        label: Some("Docker Missing".to_string()),
        dry_run: false,
    });
    assert!(result.is_err());
    let err_text = result.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(
        err_text.contains("required command not found in PATH: docker"),
        "unexpected error: {err_text}"
    );

    if let Some(path) = original_path {
        std::env::set_var("PATH", path);
    } else {
        std::env::remove_var("PATH");
    }
}

#[tokio::test]
#[cfg(unix)]
async fn connect_ssh_registers_remote_instance_with_fake_ssh() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data_dir = temp_dir("clawpal-core-connect-ssh-data");
    std::env::set_var("CLAWPAL_DATA_DIR", &data_dir);
    let ssh_path = create_fake_ssh_bin(true);
    let original_path = std::env::var_os("PATH");
    let fake_dir = ssh_path.parent().expect("fake ssh parent");
    let merged_path = format!(
        "{}:{}",
        fake_dir.display(),
        original_path
            .as_ref()
            .and_then(|v| v.to_str())
            .unwrap_or_default()
    );
    std::env::set_var("PATH", merged_path);

    let cfg = SshHostConfig {
        id: "ssh:workflow-connect".to_string(),
        label: "Workflow SSH".to_string(),
        host: "vm1".to_string(),
        port: 22,
        username: "root".to_string(),
        auth_method: "key".to_string(),
        key_path: None,
        password: None,
        passphrase: None,
    };
    let result = connect_ssh(cfg).await.expect("connect ssh");
    assert!(matches!(result.instance_type, InstanceType::RemoteSsh));

    if let Some(path) = original_path {
        std::env::set_var("PATH", path);
    } else {
        std::env::remove_var("PATH");
    }
}

#[tokio::test]
#[cfg(unix)]
async fn connect_ssh_returns_error_when_exec_fails() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let ssh_path = create_fake_ssh_bin(false);
    let original_path = std::env::var_os("PATH");
    let fake_dir = ssh_path.parent().expect("fake ssh parent");
    let merged_path = format!(
        "{}:{}",
        fake_dir.display(),
        original_path
            .as_ref()
            .and_then(|v| v.to_str())
            .unwrap_or_default()
    );
    std::env::set_var("PATH", merged_path);

    let cfg = SshHostConfig {
        id: "ssh:workflow-connect-fail".to_string(),
        label: "Workflow SSH Fail".to_string(),
        host: "vm1".to_string(),
        port: 22,
        username: "root".to_string(),
        auth_method: "key".to_string(),
        key_path: None,
        password: None,
        passphrase: None,
    };
    let result = connect_ssh(cfg).await;
    assert!(result.is_err());

    if let Some(path) = original_path {
        std::env::set_var("PATH", path);
    } else {
        std::env::remove_var("PATH");
    }
}
