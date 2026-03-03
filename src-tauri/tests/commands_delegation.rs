use std::fs;
use std::sync::Mutex;

use clawpal::commands::{delete_ssh_host, list_ssh_hosts, upsert_ssh_host};
use clawpal::ssh::SshHostConfig;
use uuid::Uuid;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn temp_data_dir() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("clawpal-tauri-cmd-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

#[test]
fn ssh_host_crud_commands_delegate_to_core_registry() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data_dir = temp_data_dir();
    std::env::set_var("CLAWPAL_DATA_DIR", &data_dir);

    let host = SshHostConfig {
        id: "ssh:test-delegation".to_string(),
        label: "Delegation Test".to_string(),
        host: "vm1".to_string(),
        port: 22,
        username: "root".to_string(),
        auth_method: "key".to_string(),
        key_path: None,
        password: None,
        passphrase: None,
    };

    let saved = upsert_ssh_host(host.clone()).expect("upsert should succeed");
    assert_eq!(saved.id, host.id);

    let listed = list_ssh_hosts().expect("list should succeed");
    assert!(listed.iter().any(|h| h.id == host.id));

    let removed = delete_ssh_host(host.id.clone()).expect("delete should succeed");
    assert!(removed);

    let listed_after = list_ssh_hosts().expect("list should succeed");
    assert!(!listed_after.iter().any(|h| h.id == host.id));
}
