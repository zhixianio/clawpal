use std::collections::HashMap;
use std::io::ErrorKind;

// We test the public functions directly since they don't require Tauri state.
// Import the crate as a library.
use clawpal::cli_runner::*;
use clawpal::models::resolve_paths;

fn has_openclaw_binary() -> bool {
    match std::process::Command::new("openclaw")
        .arg("--version")
        .output()
    {
        Ok(_) => true,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            eprintln!("skipping test: `openclaw` not found in PATH");
            false
        }
        Err(err) => panic!("failed to probe `openclaw --version`: {err}"),
    }
}

#[test]
fn test_run_openclaw_version() {
    if !has_openclaw_binary() {
        return;
    }
    let output = run_openclaw(&["--version"]).expect("should run openclaw --version");
    assert_eq!(output.exit_code, 0, "exit code should be 0");
    assert!(
        !output.stdout.trim().is_empty(),
        "stdout should contain version string, got empty output"
    );
}

#[test]
fn test_run_openclaw_config_get() {
    if !has_openclaw_binary() {
        return;
    }
    let output = run_openclaw(&["config", "get", "agents", "--json"])
        .expect("should run openclaw config get");
    assert_eq!(output.exit_code, 0, "exit code should be 0");

    let json = parse_json_output(&output).expect("should parse JSON output");
    assert!(json.is_object(), "output should be a JSON object");
}

#[test]
fn test_run_openclaw_with_env_isolation() {
    if !has_openclaw_binary() {
        return;
    }
    // Create a temp dir to use as OPENCLAW_HOME
    let tmp = std::env::temp_dir().join(format!("clawpal-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let openclaw_home = tmp.join(".openclaw");
    std::fs::create_dir_all(&openclaw_home).unwrap();

    // Copy current config to temp
    let paths = resolve_paths();
    let dest = openclaw_home.join("openclaw.json");
    std::fs::copy(&paths.config_path, &dest).expect("should copy config");

    let mut env = HashMap::new();
    env.insert(
        "OPENCLAW_HOME".to_string(),
        openclaw_home.to_string_lossy().to_string(),
    );

    // Set a value in the sandbox
    let output = run_openclaw_with_env(
        &[
            "config",
            "set",
            "agents.defaults.model.primary",
            "test-model-12345",
        ],
        Some(&env),
    )
    .expect("should set config in sandbox");
    assert_eq!(output.exit_code, 0, "set should succeed: {}", output.stderr);

    // Read back from sandbox — should have our value
    let output = run_openclaw_with_env(
        &["config", "get", "agents.defaults.model.primary"],
        Some(&env),
    )
    .expect("should get config from sandbox");
    assert_eq!(output.exit_code, 0);
    assert!(
        output.stdout.contains("test-model-12345"),
        "sandbox should have test value, got: {}",
        output.stdout
    );

    // Read from real config — should NOT have our value
    let output = run_openclaw(&["config", "get", "agents.defaults.model.primary"])
        .expect("should get real config");
    assert_eq!(output.exit_code, 0);
    assert!(
        !output.stdout.contains("test-model-12345"),
        "real config should NOT have test value, got: {}",
        output.stdout
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_parse_json_output_with_leading_noise() {
    let output = CliOutput {
        stdout: "[plugins] loading...\n[plugins] done\n{\"key\": \"value\"}".to_string(),
        stderr: String::new(),
        exit_code: 0,
    };
    let json = parse_json_output(&output).expect("should parse JSON with leading noise");
    assert_eq!(json["key"], "value");
}

#[test]
fn test_parse_json_output_error() {
    let output = CliOutput {
        stdout: String::new(),
        stderr: "Error: something went wrong".to_string(),
        exit_code: 1,
    };
    let err = parse_json_output(&output).unwrap_err();
    assert!(err.contains("something went wrong"));
}

#[test]
fn test_command_queue_basic() {
    let queue = CommandQueue::new();

    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);

    // Enqueue
    let cmd1 = queue.enqueue(
        "Set model".to_string(),
        vec![
            "openclaw".into(),
            "config".into(),
            "set".into(),
            "foo".into(),
            "bar".into(),
        ],
    );
    assert_eq!(queue.len(), 1);
    assert!(!queue.is_empty());

    let cmd2 = queue.enqueue(
        "Add agent".to_string(),
        vec![
            "openclaw".into(),
            "agents".into(),
            "add".into(),
            "test".into(),
        ],
    );
    assert_eq!(queue.len(), 2);

    // List
    let list = queue.list();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].label, "Set model");
    assert_eq!(list[1].label, "Add agent");

    // Remove
    assert!(queue.remove(&cmd1.id));
    assert_eq!(queue.len(), 1);
    assert!(!queue.remove(&cmd1.id)); // already removed

    // Clear
    queue.clear();
    assert!(queue.is_empty());
    let _ = cmd2; // suppress warning
}

#[test]
fn test_remote_command_queues_isolation() {
    let queues = RemoteCommandQueues::new();

    // Enqueue to different hosts
    queues.enqueue(
        "host1",
        "Cmd A".to_string(),
        vec!["openclaw".into(), "a".into()],
    );
    queues.enqueue(
        "host2",
        "Cmd B".to_string(),
        vec!["openclaw".into(), "b".into()],
    );
    queues.enqueue(
        "host1",
        "Cmd C".to_string(),
        vec!["openclaw".into(), "c".into()],
    );

    assert_eq!(queues.len("host1"), 2);
    assert_eq!(queues.len("host2"), 1);
    assert_eq!(queues.len("host3"), 0);

    // Clear host1 doesn't affect host2
    queues.clear("host1");
    assert_eq!(queues.len("host1"), 0);
    assert_eq!(queues.len("host2"), 1);
}
