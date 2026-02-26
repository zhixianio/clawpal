use std::fs;
use std::net::TcpListener;
use std::process::Command;
use std::sync::Mutex;

use clawpal_core::install::{install_docker, DockerInstallOptions};
use uuid::Uuid;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn compose_env(state_dir: &str) -> [(&'static str, String); 6] {
    [
        ("OPENCLAW_CONFIG_DIR", state_dir.to_string()),
        ("OPENCLAW_WORKSPACE_DIR", format!("{state_dir}/workspace")),
        ("OPENCLAW_GATEWAY_TOKEN", "clawpal-install".to_string()),
        ("CLAUDE_AI_SESSION_KEY", "dummy".to_string()),
        ("CLAUDE_WEB_SESSION_KEY", "dummy".to_string()),
        ("CLAUDE_WEB_COOKIE", "dummy".to_string()),
    ]
}

fn should_run_live_docker_test() -> bool {
    std::env::var("CLAWPAL_RUN_DOCKER_LIVE_TESTS")
        .ok()
        .as_deref()
        == Some("1")
}

fn cleanup_compose(repo_dir: &str, state_dir: &str) {
    let mut down = Command::new("docker");
    down.args(["compose", "down", "-v"]).current_dir(repo_dir);
    for (key, value) in compose_env(state_dir) {
        down.env(key, value);
    }
    let _ = down.output();
}

fn is_local_port_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

#[test]
fn install_docker_real_run_smoke() {
    if !should_run_live_docker_test() {
        eprintln!("skip docker live test: set CLAWPAL_RUN_DOCKER_LIVE_TESTS=1 to enable");
        return;
    }

    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if !is_local_port_available(18789) {
        eprintln!("skip docker live test: local port 18789 is already occupied");
        return;
    }

    let data_dir = std::env::temp_dir().join(format!("clawpal-docker-live-data-{}", Uuid::new_v4()));
    let home_dir = std::env::temp_dir().join(format!("clawpal-docker-live-home-{}", Uuid::new_v4()));
    fs::create_dir_all(&data_dir).expect("create data dir");
    fs::create_dir_all(&home_dir).expect("create home dir");
    std::env::set_var("CLAWPAL_DATA_DIR", &data_dir);

    let options = DockerInstallOptions {
        home: Some(home_dir.to_string_lossy().to_string()),
        label: Some("Docker Live Test".to_string()),
        dry_run: false,
    };

    let repo_dir = std::env::var("HOME")
        .map(|h| format!("{h}/.clawpal/install/openclaw-docker"))
        .expect("HOME should be set");
    let state_dir = format!("{}/.openclaw", home_dir.to_string_lossy());
    let result = install_docker(options);
    if let Err(err) = result {
        let message = err.to_string();
        if message.contains("port is already allocated") {
            eprintln!("skip docker live test: openclaw gateway port 18789 already allocated");
            cleanup_compose(&repo_dir, &state_dir);
            return;
        }
        cleanup_compose(&repo_dir, &state_dir);
        panic!("install_docker should succeed in live mode: {message}");
    }
    let result = result.expect("result checked above");
    assert!(result.ok, "install_docker returned not ok");

    let mut ps = Command::new("docker");
    ps.args(["compose", "ps"]).current_dir(&repo_dir);
    for (key, value) in compose_env(&state_dir) {
        ps.env(key, value);
    }
    let ps_output = ps.output().expect("run docker compose ps");
    assert!(
        ps_output.status.success(),
        "docker compose ps failed: {}",
        String::from_utf8_lossy(&ps_output.stderr)
    );

    cleanup_compose(&repo_dir, &state_dir);
}
