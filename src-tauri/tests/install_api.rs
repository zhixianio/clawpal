use clawpal::install::types::InstallSession;
use clawpal::install::commands::{
    create_session_for_test, failed_state_for_test, get_session_for_test, list_methods_for_test,
    orchestrator_next_for_test, orchestrator_next_with_sidecar_for_test,
    run_local_precheck_for_test, run_step_for_test,
};
use clawpal::install::runners::docker::docker_verify_compose_command_for_test;
use clawpal::cli_runner::set_active_openclaw_home_override;
use clawpal::cli_runner::set_active_clawpal_data_override;
use clawpal::models::resolve_paths;
use std::sync::Mutex;

static DECIDER_ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn install_session_serialization_roundtrip() {
    let json = r#"{
        "id": "sess-1",
        "method": "local",
        "state": "idle",
        "current_step": null,
        "logs": [],
        "artifacts": {},
        "created_at": "2026-02-24T00:00:00Z",
        "updated_at": "2026-02-24T00:00:00Z"
    }"#;

    let parsed: InstallSession = serde_json::from_str(json).expect("session json should deserialize");
    assert_eq!(parsed.method.as_str(), "local");
    assert_eq!(parsed.state.as_str(), "idle");
}

#[tokio::test]
async fn create_session_returns_selected_method_state() {
    let session = create_session_for_test("local")
        .await
        .expect("create session should succeed");
    assert_eq!(session.method.as_str(), "local");
    assert_eq!(session.state.as_str(), "selected_method");
}

#[tokio::test]
async fn create_session_rejects_unavailable_method_on_current_platform() {
    if cfg!(target_os = "windows") {
        return;
    }
    let err = create_session_for_test("wsl2")
        .await
        .expect_err("wsl2 should be unavailable on non-windows platforms");
    assert!(
        err.contains("unavailable"),
        "expected unavailable error, got: {err}"
    );
}

#[tokio::test]
async fn run_step_precheck_updates_state_and_next_step() {
    let session = create_session_for_test("local")
        .await
        .expect("create session should succeed");
    let result = run_step_for_test(&session.id, "precheck")
        .await
        .expect("precheck should execute");
    assert!(result.ok);
    assert_eq!(result.next_step.as_deref(), Some("install"));

    let refreshed = get_session_for_test(&session.id)
        .await
        .expect("get session should succeed");
    assert_eq!(refreshed.state.as_str(), "precheck_passed");
    let executed_commands = refreshed
        .artifacts
        .get("executed_commands")
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);
    assert!(executed_commands > 0);
}

#[tokio::test]
async fn invalid_step_does_not_mutate_session_state() {
    let session = create_session_for_test("local")
        .await
        .expect("create session should succeed");
    let result = run_step_for_test(&session.id, "verify")
        .await
        .expect("run_step should return a rejected result");
    assert!(!result.ok);
    assert_eq!(result.error_code.as_deref(), Some("validation_failed"));

    let refreshed = get_session_for_test(&session.id)
        .await
        .expect("get session should succeed");
    assert_eq!(refreshed.state.as_str(), "selected_method");
}

#[tokio::test]
async fn list_methods_returns_all_four_methods() {
    let methods = list_methods_for_test()
        .await
        .expect("list methods should succeed");
    let names: Vec<String> = methods.into_iter().map(|m| m.method).collect();
    assert_eq!(names, vec!["local", "wsl2", "docker", "remote_ssh"]);
}

#[tokio::test]
async fn orchestrator_next_returns_error_without_decider() {
    std::env::remove_var("CLAWPAL_ZEROCLAW_DECIDER");
    let session = create_session_for_test("docker")
        .await
        .expect("create session should succeed");
    let decision = orchestrator_next_for_test(&session.id, "install:docker")
        .await
        .expect("orchestrator next should succeed");
    assert_eq!(decision.source, "error");
    assert!(decision.step.is_none());
    assert!(decision.error_code.is_some());
    assert!(decision.action_hint.is_some());
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn orchestrator_sidecar_timeout_returns_error_decision() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    let _guard = DECIDER_ENV_LOCK.lock().expect("lock env guard");
    let session = create_session_for_test("docker")
        .await
        .expect("create session should succeed");

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("clawpal-install-timeout-{suffix}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let script_path = dir.join("slow-decider.sh");
    fs::write(
        &script_path,
        "#!/bin/sh\nsleep 2\necho '{\"step\":\"precheck\",\"reason\":\"slow\"}'\n",
    )
    .expect("write script");
    let mut perms = fs::metadata(&script_path).expect("stat script").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script_path, perms).expect("chmod script");

    std::env::set_var("CLAWPAL_ZEROCLAW_DECIDER", &script_path);
    std::env::set_var("CLAWPAL_ZEROCLAW_TIMEOUT_SECS", "1");
    let decision = orchestrator_next_with_sidecar_for_test(&session.id, "install:docker")
        .await
        .expect("orchestrator should return fallback decision");
    std::env::remove_var("CLAWPAL_ZEROCLAW_DECIDER");
    std::env::remove_var("CLAWPAL_ZEROCLAW_TIMEOUT_SECS");

    assert_eq!(decision.source, "error");
    assert!(decision.reason.to_lowercase().contains("timed out"), "reason={}", decision.reason);
    assert_eq!(decision.error_code.as_deref(), Some("network_error"));

    let _ = fs::remove_file(&script_path);
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn local_precheck_returns_command_summary() {
    let result = run_local_precheck_for_test()
        .await
        .expect("local precheck should succeed");
    assert!(!result.commands.is_empty());
    assert!(result.summary.contains("precheck"));
}

#[test]
fn verify_failure_maps_to_verify_failed_state() {
    let state = failed_state_for_test("verify").expect("should return failed-state mapping");
    assert_eq!(state, "verify_failed");
}

#[test]
fn docker_verify_command_sets_safe_env_defaults() {
    let command = docker_verify_compose_command_for_test("/tmp/openclaw");
    assert!(command.contains("OPENCLAW_CONFIG_DIR="));
    assert!(command.contains("OPENCLAW_WORKSPACE_DIR="));
    assert!(command.contains("COMPOSE_PROJECT_NAME="));
    assert!(command.contains("OPENCLAW_IMAGE="));
    assert!(command.contains("OPENCLAW_GATEWAY_PORT="));
    assert!(command.contains(".openclaw"));
    assert!(command.contains("docker compose config"));
}

#[test]
fn resolve_paths_uses_active_openclaw_home_override() {
    set_active_openclaw_home_override(Some("~/.clawpal/test-docker-openclaw".to_string()))
        .expect("set override should succeed");
    let paths = resolve_paths();
    let openclaw_dir = paths.openclaw_dir.to_string_lossy().to_string();
    assert!(
        openclaw_dir.contains(".clawpal/test-docker-openclaw/.openclaw"),
        "expected overridden openclaw dir, got {openclaw_dir}"
    );
    set_active_openclaw_home_override(None).expect("clear override should succeed");
}

#[test]
fn resolve_paths_uses_active_clawpal_data_override() {
    set_active_clawpal_data_override(Some("~/.clawpal/test-docker-data".to_string()))
        .expect("set data override should succeed");
    let paths = resolve_paths();
    let data_dir = paths.clawpal_dir.to_string_lossy().to_string();
    assert!(
        data_dir.contains(".clawpal/test-docker-data"),
        "expected overridden clawpal data dir, got {data_dir}"
    );
    set_active_clawpal_data_override(None).expect("clear data override should succeed");
}
