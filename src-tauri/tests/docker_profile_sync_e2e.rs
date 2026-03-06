//! E2E test: Docker Ubuntu container with OpenClaw config → ClawPal SSH connect
//! → profile sync → doctor check.
//!
//! This test spins up a Docker container running Ubuntu with SSH and the latest
//! real `openclaw` CLI (installed via npm), seeds OpenClaw configuration files, then:
//! 1. Connects via `SshConnectionPool` (password auth)
//! 2. Reads the OpenClaw config from the container
//! 3. Extracts model profiles from the config
//! 4. Resolves API keys from the remote auth store
//! 5. Runs `openclaw doctor --json` and verifies the output
//!
//! Requires Docker to be available. Guarded by `CLAWPAL_RUN_DOCKER_SYNC_E2E=1`.
//!
//! Run with:
//!   CLAWPAL_RUN_DOCKER_SYNC_E2E=1 cargo test -p clawpal --test docker_profile_sync_e2e -- --nocapture

use clawpal::ssh::{SshConnectionPool, SshHostConfig};
use std::process::Command;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CONTAINER_NAME: &str = "clawpal-e2e-docker-sync";
const SSH_PORT: u16 = 2299;
const ROOT_PASSWORD: &str = "clawpal-e2e-pass";

/// Dockerfile: Ubuntu + openssh-server + Node.js + real openclaw CLI (latest from npm) + seeded OpenClaw config.
const DOCKERFILE: &str = r#"
FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get install -y openssh-server && \
    rm -rf /var/lib/apt/lists/* && \
    mkdir /var/run/sshd

# Allow root login with password
RUN echo "root:ROOTPASS" | chpasswd && \
    sed -i 's/#PermitRootLogin.*/PermitRootLogin yes/' /etc/ssh/sshd_config && \
    sed -i 's/PermitRootLogin prohibit-password/PermitRootLogin yes/' /etc/ssh/sshd_config && \
    echo "PasswordAuthentication yes" >> /etc/ssh/sshd_config

# Seed OpenClaw configuration
RUN mkdir -p /root/.openclaw/agents/main/agent

# Main openclaw config (JSON5 compatible)
RUN cat > /root/.openclaw/openclaw.json <<'OCEOF'
{
  "gateway": {
    "port": 18789,
    "token": "gw-test-token-abc123"
  },
  "defaults": {
    "model": "anthropic/claude-sonnet-4-20250514"
  },
  "models": {
    "anthropic/claude-sonnet-4-20250514": {
      "provider": "anthropic",
      "model": "claude-sonnet-4-20250514"
    },
    "openai/gpt-4o": {
      "provider": "openai",
      "model": "gpt-4o"
    }
  },
  "agents": {
    "list": [
      { "id": "main", "model": "anthropic/claude-sonnet-4-20250514" }
    ]
  }
}
OCEOF

# Auth store with provider credentials
RUN cat > /root/.openclaw/agents/main/agent/auth-profiles.json <<'AUTHEOF'
{
  "version": 1,
  "profiles": {
    "anthropic:default": {
      "type": "token",
      "provider": "anthropic",
      "token": "e2e-anthropic-fake-key-00000000"
    },
    "openai:default": {
      "type": "token",
      "provider": "openai",
      "token": "e2e-openai-fake-key-11111111"
    }
  }
}
AUTHEOF

# Install Node.js (pinned) + openclaw CLI (pinned) for reproducible builds.
# Node: official binary tarball — no apt source or remote script execution.
# openclaw: exact published version — no floating @latest tag.
ARG NODE_VERSION=24.13.0
ARG OPENCLAW_VERSION=2026.3.2
RUN apt-get update && \
    apt-get install -y curl ca-certificates xz-utils && \
    rm -rf /var/lib/apt/lists/* && \
    curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
      -o /tmp/node.tar.xz && \
    tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 && \
    rm /tmp/node.tar.xz && \
    npm install -g "openclaw@${OPENCLAW_VERSION}"

# Set env vars that ClawPal profile sync checks
RUN echo "export ANTHROPIC_API_KEY=e2e-anthropic-fake-key-00000000" >> /root/.bashrc && \
    echo "export OPENAI_API_KEY=e2e-openai-fake-key-11111111" >> /root/.bashrc

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D"]
"#;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn should_run() -> bool {
    std::env::var("CLAWPAL_RUN_DOCKER_SYNC_E2E").ok().as_deref() == Some("1")
}

fn docker_available() -> bool {
    Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn cleanup_container() {
    let _ = Command::new("docker")
        .args(["rm", "-f", CONTAINER_NAME])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

fn cleanup_image() {
    let _ = Command::new("docker")
        .args(["rmi", "-f", &format!("{CONTAINER_NAME}:latest")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

fn build_image() -> Result<(), String> {
    let dockerfile = DOCKERFILE.replace("ROOTPASS", ROOT_PASSWORD);
    let output = Command::new("docker")
        .args([
            "build",
            "-t",
            &format!("{CONTAINER_NAME}:latest"),
            "-f",
            "-",
            ".",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir(std::env::temp_dir())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(dockerfile.as_bytes())?;
            }
            child.wait_with_output()
        })
        .map_err(|e| format!("docker build failed to spawn: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("docker build failed: {stderr}"));
    }
    Ok(())
}

fn start_container() -> Result<(), String> {
    let output = Command::new("docker")
        .args([
            "run",
            "-d",
            "--name",
            CONTAINER_NAME,
            "-p",
            &format!("{}:22", SSH_PORT),
            &format!("{CONTAINER_NAME}:latest"),
        ])
        .output()
        .map_err(|e| format!("docker run failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("docker run failed: {stderr}"));
    }
    Ok(())
}

fn wait_for_ssh(timeout_secs: u64) -> Result<(), String> {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);
    loop {
        if start.elapsed() > timeout {
            return Err("timeout waiting for SSH to become available".into());
        }
        let result = std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{SSH_PORT}").parse().unwrap(),
            std::time::Duration::from_secs(1),
        );
        if result.is_ok() {
            // Give sshd a moment to fully start
            std::thread::sleep(std::time::Duration::from_millis(500));
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

fn docker_host_config() -> SshHostConfig {
    SshHostConfig {
        id: "e2e-docker-sync".into(),
        label: "E2E Docker Sync".into(),
        host: "127.0.0.1".into(),
        port: SSH_PORT,
        username: "root".into(),
        auth_method: "password".into(),
        key_path: None,
        password: Some(ROOT_PASSWORD.into()),
        passphrase: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Full e2e: build image → start container → SSH connect → read config →
/// extract profiles → resolve keys → doctor check → verify → cleanup.
#[tokio::test]
async fn e2e_docker_profile_sync_and_doctor() {
    if !should_run() {
        eprintln!("skip: set CLAWPAL_RUN_DOCKER_SYNC_E2E=1 to enable");
        return;
    }
    if !docker_available() {
        eprintln!("skip: docker not available");
        return;
    }

    // Cleanup any leftover container from previous runs
    cleanup_container();

    // Build & start
    build_image().expect("docker build should succeed");
    start_container().expect("docker run should succeed");

    // Ensure cleanup on exit (manual drop guard)
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            cleanup_container();
            cleanup_image();
        }
    }
    let _cleanup = Cleanup;

    wait_for_ssh(30).expect("SSH should become available");

    // --- Step 1: SSH connect ---
    let pool = SshConnectionPool::new();
    let cfg = docker_host_config();
    pool.connect(&cfg)
        .await
        .expect("SSH connect to Docker container should succeed");
    assert!(pool.is_connected(&cfg.id).await);
    eprintln!("[e2e] SSH connected to Docker container");

    // --- Step 2: Read OpenClaw config ---
    let config_raw = pool
        .sftp_read(&cfg.id, "~/.openclaw/openclaw.json")
        .await
        .expect("should read openclaw.json from container");
    let config: serde_json::Value =
        serde_json::from_str(&config_raw).expect("openclaw.json should be valid JSON");
    assert!(config.is_object(), "config should be a JSON object");

    // Verify config structure
    let gateway_port = config
        .pointer("/gateway/port")
        .and_then(|v| v.as_u64())
        .expect("gateway.port should exist");
    assert_eq!(gateway_port, 18789);

    let default_model = config
        .pointer("/defaults/model")
        .and_then(|v| v.as_str())
        .expect("defaults.model should exist");
    assert_eq!(default_model, "anthropic/claude-sonnet-4-20250514");
    eprintln!("[e2e] Config verified: gateway port={gateway_port}, default model={default_model}");

    // --- Step 3: Read auth store ---
    let auth_raw = pool
        .sftp_read(&cfg.id, "~/.openclaw/agents/main/agent/auth-profiles.json")
        .await
        .expect("should read auth-profiles.json from container");
    let auth: serde_json::Value =
        serde_json::from_str(&auth_raw).expect("auth-profiles.json should be valid JSON");

    let anthropic_token = auth
        .pointer("/profiles/anthropic:default/token")
        .and_then(|v| v.as_str())
        .expect("anthropic:default token should exist");
    assert_eq!(anthropic_token, "e2e-anthropic-fake-key-00000000");

    let openai_token = auth
        .pointer("/profiles/openai:default/token")
        .and_then(|v| v.as_str())
        .expect("openai:default token should exist");
    assert_eq!(openai_token, "e2e-openai-fake-key-11111111");
    eprintln!("[e2e] Auth store verified: 2 provider credentials found");

    // --- Step 4: Extract model profiles from config ---
    // Verify models are defined in the config
    let models = config
        .get("models")
        .and_then(|v| v.as_object())
        .expect("models should be an object");
    assert!(
        models.contains_key("anthropic/claude-sonnet-4-20250514"),
        "should have anthropic model"
    );
    assert!(
        models.contains_key("openai/gpt-4o"),
        "should have openai model"
    );
    eprintln!(
        "[e2e] Model profiles extracted: {} models found",
        models.len()
    );

    // --- Step 5: Run openclaw --version ---
    let version_result = pool
        .exec(&cfg.id, "openclaw --version")
        .await
        .expect("openclaw --version should succeed");
    assert_eq!(version_result.exit_code, 0);
    // Version string comes from the real openclaw binary; just verify it's non-empty
    // and looks like a semver or calver (e.g. "2026.3.2" or "1.2.3").
    assert!(
        !version_result.stdout.trim().is_empty(),
        "openclaw --version should produce non-empty output"
    );
    assert!(
        version_result.stdout.chars().any(|c| c.is_ascii_digit()),
        "version output should contain a version number: {}",
        version_result.stdout.trim()
    );
    eprintln!("[e2e] OpenClaw version: {}", version_result.stdout.trim());

    // --- Step 6: Run doctor check ---
    let doctor_result = pool
        .exec(&cfg.id, "openclaw doctor --json")
        .await
        .expect("openclaw doctor should succeed");
    assert_eq!(
        doctor_result.exit_code, 0,
        "doctor should exit 0, stderr: {}",
        doctor_result.stderr
    );

    let doctor: serde_json::Value =
        serde_json::from_str(&doctor_result.stdout).expect("doctor output should be valid JSON");
    assert_eq!(
        doctor.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "doctor should report ok=true"
    );
    assert_eq!(
        doctor.get("score").and_then(|v| v.as_u64()),
        Some(100),
        "doctor should report score=100"
    );

    let checks = doctor
        .get("checks")
        .and_then(|v| v.as_array())
        .expect("doctor should have checks array");
    assert!(!checks.is_empty(), "doctor should have at least one check");
    for check in checks {
        let status = check.get("status").and_then(|v| v.as_str()).unwrap_or("");
        assert_eq!(status, "ok", "check {:?} should be ok", check.get("id"));
    }
    eprintln!("[e2e] Doctor check passed: {} checks all ok", checks.len());

    // --- Step 7: Verify env vars accessible via exec ---
    let env_result = pool
        .exec(&cfg.id, "bash -l -c 'echo $ANTHROPIC_API_KEY'")
        .await
        .expect("should read env var");
    assert_eq!(
        env_result.stdout.trim(),
        "e2e-anthropic-fake-key-00000000",
        "ANTHROPIC_API_KEY should be set in remote env"
    );
    eprintln!("[e2e] Remote env vars verified");

    // --- Step 8: Verify agents list ---
    let agents_result = pool
        .exec(&cfg.id, "openclaw agents list --json")
        .await
        .expect("agents list should succeed");
    assert_eq!(agents_result.exit_code, 0);
    let agents: serde_json::Value =
        serde_json::from_str(&agents_result.stdout).expect("agents list should be valid JSON");
    assert!(agents.is_array(), "agents list should be an array");
    let agents_arr = agents.as_array().unwrap();
    assert_eq!(agents_arr.len(), 1, "should have 1 agent");
    assert_eq!(
        agents_arr[0].get("id").and_then(|v| v.as_str()),
        Some("main"),
        "agent id should be 'main'"
    );
    eprintln!("[e2e] Agents list verified: {:?}", agents);

    // --- Step 9: SFTP list the config directory ---
    let entries = pool
        .sftp_list(&cfg.id, "~/.openclaw")
        .await
        .expect("sftp_list ~/.openclaw should succeed");
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.contains(&"openclaw.json"),
        "config dir should contain openclaw.json, got: {:?}",
        names
    );
    assert!(
        names.contains(&"agents"),
        "config dir should contain agents/, got: {:?}",
        names
    );
    eprintln!("[e2e] Config directory listing verified: {:?}", names);

    // --- Step 10: Disconnect ---
    pool.disconnect(&cfg.id)
        .await
        .expect("disconnect should succeed");
    assert!(!pool.is_connected(&cfg.id).await);
    eprintln!("[e2e] Disconnected. Test passed!");
}

/// Verify password auth works (basic sanity check).
#[tokio::test]
async fn e2e_docker_password_auth_connect() {
    if !should_run() {
        eprintln!("skip: set CLAWPAL_RUN_DOCKER_SYNC_E2E=1 to enable");
        return;
    }
    if !docker_available() {
        eprintln!("skip: docker not available");
        return;
    }

    // Reuse container from previous test if running together, or build fresh
    let needs_setup = Command::new("docker")
        .args(["inspect", CONTAINER_NAME])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| !s.success())
        .unwrap_or(true);

    if needs_setup {
        cleanup_container();
        build_image().expect("docker build");
        start_container().expect("docker run");
        wait_for_ssh(30).expect("SSH available");
    }

    struct Cleanup {
        should_cleanup: bool,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if self.should_cleanup {
                cleanup_container();
                cleanup_image();
            }
        }
    }
    let _cleanup = Cleanup {
        should_cleanup: needs_setup,
    };

    let pool = SshConnectionPool::new();
    let cfg = docker_host_config();

    // Verify password auth connects
    pool.connect(&cfg)
        .await
        .expect("password auth connect should succeed");
    assert!(pool.is_connected(&cfg.id).await);

    // Quick exec smoke test
    let result = pool
        .exec(&cfg.id, "whoami")
        .await
        .expect("exec whoami should succeed");
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout.trim(), "root");

    pool.disconnect(&cfg.id).await.expect("disconnect");
    eprintln!("[e2e] Password auth test passed");
}

/// Verify wrong password is rejected.
#[tokio::test]
async fn e2e_docker_wrong_password_rejected() {
    if !should_run() {
        eprintln!("skip: set CLAWPAL_RUN_DOCKER_SYNC_E2E=1 to enable");
        return;
    }
    if !docker_available() {
        eprintln!("skip: docker not available");
        return;
    }

    // Container must be running
    let running = Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", CONTAINER_NAME])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false);

    if !running {
        eprintln!("skip: container not running (run e2e_docker_profile_sync_and_doctor first)");
        return;
    }

    let pool = SshConnectionPool::new();
    let mut cfg = docker_host_config();
    cfg.password = Some("wrong-password".into());
    cfg.id = "e2e-docker-sync-wrong-pw".into();

    let result = pool.connect(&cfg).await;
    assert!(
        result.is_err(),
        "connect with wrong password should fail, got: {:?}",
        result
    );
    eprintln!("[e2e] Wrong password correctly rejected");
}
