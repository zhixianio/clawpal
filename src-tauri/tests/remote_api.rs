//! Integration tests for remote SSH operations against a live VM.
//!
//! These tests require a reachable SSH host configured in ~/.ssh/config as "vm1".
//! Run with: cargo test --test remote_api -- --test-threads=1
//!
//! The tests run sequentially (--test-threads=1) because they share the SSH connection pool.

use clawpal::ssh::{SshConnectionPool, SshHostConfig};

/// Build a config that uses ssh_config auth (delegates to ~/.ssh/config for "vm1").
fn vm1_config() -> SshHostConfig {
    SshHostConfig {
        id: "vm1-test".into(),
        label: "VM1 Test".into(),
        host: "vm1".into(),
        port: 22,
        username: String::new(), // let ssh_config decide
        auth_method: "ssh_config".into(),
        key_path: None,
        password: None,
    }
}

// ---------------------------------------------------------------------------
// SSH layer tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_01_connect() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");
    assert!(
        pool.is_connected(&cfg.id).await,
        "should be connected after connect()"
    );
}

#[tokio::test]
async fn test_02_exec_simple() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    let result = pool.exec(&cfg.id, "echo hello").await.expect("exec failed");
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout.trim(), "hello");
}

#[tokio::test]
async fn test_03_exec_exit_code() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    let result = pool.exec(&cfg.id, "exit 42").await.expect("exec failed");
    assert_eq!(result.exit_code, 42);
}

#[tokio::test]
async fn test_04_exec_stderr() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    let result = pool
        .exec(&cfg.id, "echo oops >&2")
        .await
        .expect("exec failed");
    assert_eq!(result.exit_code, 0);
    assert!(result.stderr.contains("oops"));
}

#[tokio::test]
async fn test_05_sftp_write_read_roundtrip() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    let path = "/tmp/clawpal-test-roundtrip.txt";
    let content = "Hello from clawpal integration test!\nLine 2.\n";

    // Write
    pool.sftp_write(&cfg.id, path, content)
        .await
        .expect("sftp_write failed");

    // Read back
    let read = pool.sftp_read(&cfg.id, path).await.expect("sftp_read failed");
    assert_eq!(read, content);

    // Cleanup
    pool.sftp_remove(&cfg.id, path)
        .await
        .expect("sftp_remove failed");
}

#[tokio::test]
async fn test_06_sftp_write_binary_roundtrip() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    let path = "/tmp/clawpal-test-binary.bin";
    // Content with null bytes, unicode, special chars
    let content = "binary\x00test\nwith unicode: \u{1F980}\nand 'quotes'\n";

    pool.sftp_write(&cfg.id, path, content)
        .await
        .expect("sftp_write failed");

    let read = pool.sftp_read(&cfg.id, path).await.expect("sftp_read failed");
    assert_eq!(read, content);

    pool.sftp_remove(&cfg.id, path)
        .await
        .expect("sftp_remove failed");
}

#[tokio::test]
async fn test_07_sftp_list() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    // Create test directory with files
    pool.exec(&cfg.id, "mkdir -p /tmp/clawpal-test-list && touch /tmp/clawpal-test-list/a.txt /tmp/clawpal-test-list/b.txt")
        .await.expect("setup failed");

    let entries = pool
        .sftp_list(&cfg.id, "/tmp/clawpal-test-list")
        .await
        .expect("sftp_list failed");

    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"a.txt"), "should contain a.txt, got: {:?}", names);
    assert!(names.contains(&"b.txt"), "should contain b.txt, got: {:?}", names);

    // Cleanup
    pool.exec(&cfg.id, "rm -rf /tmp/clawpal-test-list")
        .await
        .expect("cleanup failed");
}

#[tokio::test]
async fn test_08_sftp_read_nonexistent() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    let result = pool.sftp_read(&cfg.id, "/tmp/this-file-does-not-exist-12345").await;
    assert!(result.is_err(), "reading nonexistent file should fail");
}

#[tokio::test]
async fn test_09_sftp_tilde_expansion() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    let path = "~/.clawpal-test-tilde.txt";
    pool.sftp_write(&cfg.id, path, "tilde test")
        .await
        .expect("sftp_write with tilde failed");

    let read = pool.sftp_read(&cfg.id, path).await.expect("sftp_read with tilde failed");
    assert_eq!(read, "tilde test");

    pool.sftp_remove(&cfg.id, path)
        .await
        .expect("sftp_remove with tilde failed");
}

#[tokio::test]
async fn test_10_disconnect_reconnect() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");
    assert!(pool.is_connected(&cfg.id).await);

    pool.disconnect(&cfg.id).await.expect("disconnect failed");
    assert!(!pool.is_connected(&cfg.id).await);

    // Reconnect
    pool.connect(&cfg).await.expect("reconnect failed");
    assert!(pool.is_connected(&cfg.id).await);

    // Verify it works
    let result = pool.exec(&cfg.id, "echo ok").await.expect("exec after reconnect failed");
    assert_eq!(result.stdout.trim(), "ok");
}

#[tokio::test]
async fn test_11_password_auth_rejected() {
    let pool = SshConnectionPool::new();
    let mut cfg = vm1_config();
    cfg.auth_method = "password".into();
    cfg.password = Some("test".into());

    let result = pool.connect(&cfg).await;
    assert!(result.is_err(), "password auth should be rejected");
    assert!(
        result.unwrap_err().contains("not supported"),
        "error should mention 'not supported'"
    );
}

// ---------------------------------------------------------------------------
// Remote config reading tests (uses actual openclaw config on vm1)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_20_read_remote_config() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    let raw = pool
        .sftp_read(&cfg.id, "~/.openclaw/openclaw.json")
        .await
        .expect("failed to read remote config");

    let config: serde_json::Value =
        serde_json::from_str(&raw).expect("config should be valid JSON");
    assert!(config.is_object(), "config should be a JSON object");
}

#[tokio::test]
async fn test_21_remote_openclaw_version() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    let result = pool
        .exec(&cfg.id, "openclaw --version")
        .await
        .expect("failed to run openclaw --version");
    assert_eq!(result.exit_code, 0, "openclaw --version should succeed");
    assert!(
        !result.stdout.trim().is_empty(),
        "version output should not be empty"
    );
}

#[tokio::test]
async fn test_22_remote_gateway_health() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    // Read gateway port from config
    let raw = pool
        .sftp_read(&cfg.id, "~/.openclaw/openclaw.json")
        .await
        .expect("failed to read config");
    let config: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let port = config
        .pointer("/gateway/port")
        .and_then(|v| v.as_u64())
        .unwrap_or(18789);

    // TCP health check via remote
    let result = pool
        .exec(
            &cfg.id,
            &format!(
                "timeout 2 bash -c 'echo > /dev/tcp/127.0.0.1/{}' 2>/dev/null && echo UP || echo DOWN",
                port
            ),
        )
        .await
        .expect("health check failed");

    let status = result.stdout.trim();
    assert!(
        status == "UP" || status == "DOWN",
        "health should be UP or DOWN, got: {}",
        status
    );
    println!("Gateway on vm1:{} is {}", port, status);
}

#[tokio::test]
async fn test_23_remote_agents_list() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    let result = pool
        .exec(&cfg.id, "openclaw agents list --json")
        .await
        .expect("agents list failed");
    assert_eq!(result.exit_code, 0);

    let agents: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("agents list should be valid JSON");
    assert!(agents.is_array(), "agents list should be an array");
    println!("Agents on vm1: {}", agents);
}

#[tokio::test]
async fn test_24_remote_doctor() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    let result = pool
        .exec(&cfg.id, "openclaw doctor 2>&1 || true")
        .await
        .expect("doctor failed");
    // Doctor may exit non-zero if issues found, that's OK
    assert!(
        !result.stdout.is_empty() || !result.stderr.is_empty(),
        "doctor should produce some output"
    );
    println!("Doctor exit_code: {}, output length: {} bytes", result.exit_code, result.stdout.len());
}

// ---------------------------------------------------------------------------
// Concurrent operations (ensure Arc<Session> works)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_30_concurrent_exec() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    // Fire 5 commands concurrently
    let mut handles = Vec::new();
    for i in 0..5 {
        let id = cfg.id.clone();
        let pool_ref = &pool;
        handles.push(async move {
            pool_ref
                .exec(&id, &format!("echo concurrent-{}", i))
                .await
        });
    }

    let results = futures::future::join_all(handles).await;
    for (i, r) in results.iter().enumerate() {
        let r = r.as_ref().expect(&format!("concurrent exec {} failed", i));
        assert_eq!(r.exit_code, 0);
        assert_eq!(r.stdout.trim(), format!("concurrent-{}", i));
    }
}

#[tokio::test]
async fn test_31_concurrent_sftp_read_write() {
    let pool = SshConnectionPool::new();
    let cfg = vm1_config();
    pool.connect(&cfg).await.expect("connect failed");

    // Write 5 files concurrently
    let mut write_handles = Vec::new();
    for i in 0..5 {
        let id = cfg.id.clone();
        let pool_ref = &pool;
        let path = format!("/tmp/clawpal-concurrent-{}.txt", i);
        let content = format!("content-{}", i);
        write_handles.push(async move { pool_ref.sftp_write(&id, &path, &content).await });
    }
    let results = futures::future::join_all(write_handles).await;
    for (i, r) in results.iter().enumerate() {
        r.as_ref()
            .expect(&format!("concurrent write {} failed", i));
    }

    // Read them back concurrently
    let mut read_handles = Vec::new();
    for i in 0..5 {
        let id = cfg.id.clone();
        let pool_ref = &pool;
        let path = format!("/tmp/clawpal-concurrent-{}.txt", i);
        read_handles.push(async move { pool_ref.sftp_read(&id, &path).await });
    }
    let results = futures::future::join_all(read_handles).await;
    for (i, r) in results.iter().enumerate() {
        let content = r
            .as_ref()
            .expect(&format!("concurrent read {} failed", i));
        assert_eq!(content, &format!("content-{}", i));
    }

    // Cleanup
    pool.exec(&cfg.id, "rm -f /tmp/clawpal-concurrent-*.txt")
        .await
        .expect("cleanup failed");
}
