use clawpal::install::types::InstallSession;
use clawpal::install::commands::{
    create_session_for_test, failed_state_for_test, get_session_for_test, list_methods_for_test,
    run_local_precheck_for_test, run_step_for_test,
};
use clawpal::install::runners::docker::docker_verify_compose_command_for_test;

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
async fn local_precheck_returns_command_summary() {
    let result = run_local_precheck_for_test()
        .await
        .expect("local precheck should succeed");
    assert!(!result.commands.is_empty());
    assert!(result.summary.contains("precheck"));
}

#[test]
fn verify_failure_keeps_init_passed_state() {
    let state = failed_state_for_test("verify").expect("should return failed-state mapping");
    assert_eq!(state, "init_passed");
}

#[test]
fn docker_verify_command_sets_safe_env_defaults() {
    let command = docker_verify_compose_command_for_test("/tmp/openclaw");
    assert!(command.contains("OPENCLAW_CONFIG_DIR="));
    assert!(command.contains("OPENCLAW_WORKSPACE_DIR="));
    assert!(command.contains("docker compose config"));
}
