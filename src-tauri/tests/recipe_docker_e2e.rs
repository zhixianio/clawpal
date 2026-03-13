//! E2E test: import the bundled recipe library into a temporary ClawPal
//! workspace, then execute the three business recipes against a real OpenClaw
//! CLI running inside a Dockerized Ubuntu host exposed over SSH.
//!
//! Guarded by `CLAWPAL_RUN_DOCKER_RECIPE_E2E=1`.

use clawpal::cli_runner::{
    set_active_clawpal_data_override, set_active_openclaw_home_override, CliCache, CommandQueue,
    RemoteCommandQueues,
};
use clawpal::commands::{
    approve_recipe_workspace_source, execute_recipe_with_services, import_recipe_library,
    list_recipe_runs, read_recipe_workspace_source,
};
use clawpal::recipe_executor::ExecuteRecipeRequest;
use clawpal::recipe_planner::build_recipe_plan_from_source_text;
use clawpal::recipe_workspace::RecipeWorkspace;
use clawpal::ssh::{SshConnectionPool, SshHostConfig};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

const CONTAINER_NAME: &str = "clawpal-e2e-recipe-library";
const ROOT_PASSWORD: &str = "clawpal-e2e-pass";
const TEST_ANTHROPIC_KEY: &str = "test-anthropic-recipe-key";
const TEST_OPENAI_KEY: &str = "test-openai-recipe-key";

const DOCKERFILE: &str = r#"
FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get install -y openssh-server curl ca-certificates git xz-utils && \
    rm -rf /var/lib/apt/lists/* && \
    mkdir /var/run/sshd

RUN echo "root:ROOTPASS" | chpasswd && \
    sed -i 's/#PermitRootLogin.*/PermitRootLogin yes/' /etc/ssh/sshd_config && \
    sed -i 's/PermitRootLogin prohibit-password/PermitRootLogin yes/' /etc/ssh/sshd_config && \
    echo "PasswordAuthentication yes" >> /etc/ssh/sshd_config

RUN mkdir -p /root/.openclaw/agents/main/agent
RUN mkdir -p /root/.openclaw/instances/openclaw-recipe-e2e/workspace

RUN cat > /root/.openclaw/openclaw.json <<'OCEOF'
{
  "meta": {
    "lastTouchedVersion": "2026.3.2",
    "lastTouchedAt": "2026-03-12T17:59:58.553Z"
  },
  "gateway": {
    "port": 18789,
    "mode": "local",
    "auth": {
      "token": "gw-test-token-abc123"
    }
  },
  "models": {
    "providers": {
      "anthropic": {
        "baseUrl": "https://api.anthropic.com/v1",
        "models": [
          {
            "id": "claude-sonnet-4-20250514",
            "name": "Claude Sonnet 4"
          }
        ]
      }
    }
  },
  "agents": {
    "defaults": {
      "model": "anthropic/claude-sonnet-4-20250514",
      "workspace": "~/.openclaw/instances/openclaw-recipe-e2e/workspace"
    },
    "list": [
      {
        "id": "main",
        "model": "anthropic/claude-sonnet-4-20250514",
        "workspace": "~/.openclaw/instances/openclaw-recipe-e2e/workspace"
      }
    ]
  },
  "channels": {
    "discord": {
      "enabled": true,
      "groupPolicy": "allowlist",
      "streaming": "off",
      "guilds": {
        "guild-recipe-lab": {
          "channels": {
            "channel-general": {
              "systemPrompt": ""
            },
            "channel-support": {
              "systemPrompt": ""
            }
          }
        }
      }
    }
  }
}
OCEOF

RUN cat > /root/.openclaw/agents/main/agent/IDENTITY.md <<'IDEOF'
- Name: Main Agent
- Emoji: 🤖
IDEOF

RUN cat > /root/.openclaw/agents/main/agent/auth-profiles.json <<'AUTHEOF'
{
  "version": 1,
  "profiles": {
    "anthropic:default": {
      "type": "token",
      "provider": "anthropic",
      "token": "ANTHROPIC_KEY"
    },
    "openai:default": {
      "type": "token",
      "provider": "openai",
      "token": "OPENAI_KEY"
    }
  }
}
AUTHEOF

ARG NODE_VERSION=24.13.0
ARG OPENCLAW_VERSION=2026.3.2
ARG TARGETARCH
RUN case "${TARGETARCH}" in \
      amd64) NODE_ARCH="x64" ;; \
      arm64) NODE_ARCH="arm64" ;; \
      *) echo "Unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac && \
    curl --retry 5 --retry-all-errors --retry-delay 2 -fsSL \
      "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-${NODE_ARCH}.tar.xz" \
      -o /tmp/node.tar.xz && \
    tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 && \
    rm /tmp/node.tar.xz && \
    npm config set fetch-retries 5 && \
    npm config set fetch-retry-mintimeout 10000 && \
    npm config set fetch-retry-maxtimeout 120000 && \
    for attempt in 1 2 3; do \
      npm install -g "openclaw@${OPENCLAW_VERSION}" && break; \
      if [ "$attempt" -eq 3 ]; then exit 1; fi; \
      echo "openclaw install failed on attempt ${attempt}, retrying..." >&2; \
      sleep 5; \
    done

RUN echo "export ANTHROPIC_API_KEY=ANTHROPIC_KEY" >> /root/.bashrc && \
    echo "export OPENAI_API_KEY=OPENAI_KEY" >> /root/.bashrc && \
    echo "export ANTHROPIC_API_KEY=ANTHROPIC_KEY" >> /root/.profile && \
    echo "export OPENAI_API_KEY=OPENAI_KEY" >> /root/.profile

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D"]
"#;

struct TempDir(PathBuf);

impl TempDir {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(prefix: &str) -> TempDir {
    let path = std::env::temp_dir().join(format!("clawpal-{}-{}", prefix, Uuid::new_v4()));
    fs::create_dir_all(&path).expect("create temp dir");
    TempDir(path)
}

struct OverrideGuard;

impl OverrideGuard {
    fn new(openclaw_home: &Path, clawpal_data_dir: &Path) -> Self {
        set_active_openclaw_home_override(Some(openclaw_home.to_string_lossy().to_string()))
            .expect("set active openclaw home override");
        set_active_clawpal_data_override(Some(clawpal_data_dir.to_string_lossy().to_string()))
            .expect("set active clawpal data override");
        Self
    }
}

impl Drop for OverrideGuard {
    fn drop(&mut self) {
        let _ = set_active_openclaw_home_override(None);
        let _ = set_active_clawpal_data_override(None);
    }
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

struct ContainerCleanup;

impl Drop for ContainerCleanup {
    fn drop(&mut self) {
        cleanup_container();
        cleanup_image();
    }
}

fn should_run() -> bool {
    std::env::var("CLAWPAL_RUN_DOCKER_RECIPE_E2E")
        .ok()
        .as_deref()
        == Some("1")
}

fn docker_available() -> bool {
    Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
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
    let dockerfile = DOCKERFILE
        .replace("ROOTPASS", ROOT_PASSWORD)
        .replace("ANTHROPIC_KEY", TEST_ANTHROPIC_KEY)
        .replace("OPENAI_KEY", TEST_OPENAI_KEY);
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
        .map_err(|error| format!("docker build failed to spawn: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "docker build failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn start_container(ssh_port: u16) -> Result<(), String> {
    let output = Command::new("docker")
        .args([
            "run",
            "-d",
            "--name",
            CONTAINER_NAME,
            "-p",
            &format!("{ssh_port}:22"),
            &format!("{CONTAINER_NAME}:latest"),
        ])
        .output()
        .map_err(|error| format!("docker run failed: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "docker run failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn wait_for_ssh(port: u16, timeout_secs: u64) -> Result<(), String> {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);
    let addr = format!("127.0.0.1:{port}")
        .parse()
        .expect("parse docker ssh address");
    loop {
        if start.elapsed() > timeout {
            return Err("timeout waiting for SSH to become available".into());
        }
        if std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(1)).is_ok() {
            std::thread::sleep(std::time::Duration::from_millis(500));
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

fn docker_host_config(ssh_port: u16) -> SshHostConfig {
    SshHostConfig {
        id: "recipe-e2e-docker".into(),
        label: "Recipe E2E Docker".into(),
        host: "127.0.0.1".into(),
        port: ssh_port,
        username: "root".into(),
        auth_method: "password".into(),
        key_path: None,
        password: Some(ROOT_PASSWORD.into()),
        passphrase: None,
    }
}

fn recipe_library_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("examples")
        .join("recipe-library")
}

async fn execute_workspace_recipe(
    queue: &CommandQueue,
    cache: &CliCache,
    pool: &SshConnectionPool,
    remote_queues: &RemoteCommandQueues,
    host_id: &str,
    workspace_slug: &str,
    recipe_id: &str,
    params: Map<String, Value>,
) -> Result<clawpal::recipe_executor::ExecuteRecipeResult, String> {
    approve_recipe_workspace_source(workspace_slug.to_string())?;
    let source = read_recipe_workspace_source(workspace_slug.to_string())?;
    let mut plan = build_recipe_plan_from_source_text(recipe_id, &params, &source)?;
    plan.execution_spec.target = json!({
        "kind": "remote_ssh",
        "hostId": host_id,
    });

    execute_recipe_with_services(
        queue,
        cache,
        pool,
        remote_queues,
        ExecuteRecipeRequest {
            spec: plan.execution_spec,
            source_origin: Some("saved".into()),
            source_text: Some(source),
            workspace_slug: Some(workspace_slug.into()),
        },
    )
    .await
}

fn sample_dedicated_params() -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("agent_id".into(), Value::String("ops-bot".into()));
    params.insert("model".into(), Value::String("__default__".into()));
    params.insert("name".into(), Value::String("Ops Bot".into()));
    params.insert("emoji".into(), Value::String("🛰️".into()));
    params.insert(
        "persona".into(),
        Value::String("You coordinate incident response with crisp updates.".into()),
    );
    params
}

fn sample_agent_persona_params() -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("agent_id".into(), Value::String("main".into()));
    params.insert("persona_preset".into(), Value::String("coach".into()));
    params
}

fn sample_channel_persona_params() -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("guild_id".into(), Value::String("guild-recipe-lab".into()));
    params.insert("channel_id".into(), Value::String("channel-support".into()));
    params.insert("persona_preset".into(), Value::String("support".into()));
    params
}

#[tokio::test]
async fn e2e_recipe_library_import_and_execute_against_docker_openclaw() {
    if !should_run() {
        eprintln!("skip: set CLAWPAL_RUN_DOCKER_RECIPE_E2E=1 to enable");
        return;
    }
    if !docker_available() {
        eprintln!("skip: docker not available");
        return;
    }

    let ssh_port = portpicker::pick_unused_port().unwrap_or(2301);
    let test_root = temp_dir("recipe-docker-e2e");
    let _overrides = OverrideGuard::new(
        &test_root.path().join("openclaw-home"),
        &test_root.path().join("clawpal-data"),
    );
    let _exec_timeout = EnvVarGuard::set("CLAWPAL_RUSSH_EXEC_TIMEOUT_SECS", "60");
    let _cleanup = ContainerCleanup;

    cleanup_container();
    build_image().expect("docker image build should succeed");
    start_container(ssh_port).expect("docker container should start");
    wait_for_ssh(ssh_port, 45).expect("ssh should become available");

    let pool = SshConnectionPool::new();
    let queue = CommandQueue::new();
    let cache = CliCache::new();
    let remote_queues = RemoteCommandQueues::new();
    let host = docker_host_config(ssh_port);
    pool.connect(&host)
        .await
        .expect("ssh connect to docker recipe host should succeed");

    let import_result = import_recipe_library(recipe_library_root().to_string_lossy().to_string())
        .expect("import example recipe library");
    assert_eq!(import_result.imported.len(), 3);
    assert!(import_result.skipped.is_empty());
    assert_eq!(
        RecipeWorkspace::from_resolved_paths()
            .list_entries()
            .expect("list workspace recipes")
            .len(),
        3
    );

    let dedicated_result = execute_workspace_recipe(
        &queue,
        &cache,
        &pool,
        &remote_queues,
        &host.id,
        "dedicated-agent",
        "dedicated-agent",
        sample_dedicated_params(),
    )
    .await
    .expect("execute dedicated agent recipe");
    assert_eq!(dedicated_result.instance_id, host.id);
    assert_eq!(
        dedicated_result.summary,
        "Created dedicated agent Ops Bot (ops-bot)"
    );

    let remote_config_raw = pool
        .sftp_read(&host.id, "~/.openclaw/openclaw.json")
        .await
        .expect("read remote openclaw config");
    let remote_config: Value =
        serde_json::from_str(&remote_config_raw).expect("remote config should be valid json");
    let agents = remote_config
        .pointer("/agents/list")
        .and_then(Value::as_array)
        .expect("remote agents list");
    let dedicated_agent = agents
        .iter()
        .find(|agent| agent.get("id").and_then(Value::as_str) == Some("ops-bot"))
        .expect("ops-bot should exist in remote agents list");
    let dedicated_workspace = dedicated_agent
        .get("workspace")
        .and_then(Value::as_str)
        .expect("dedicated agent should have workspace");
    assert!(
        dedicated_workspace.starts_with('/') || dedicated_workspace.starts_with("~/"),
        "expected OpenClaw to return an absolute or home-relative workspace, got: {dedicated_workspace}"
    );
    assert_eq!(
        dedicated_agent.get("agentDir").and_then(Value::as_str),
        Some("/root/.openclaw/agents/ops-bot/agent")
    );
    if let Some(model) = dedicated_agent.get("model").and_then(Value::as_str) {
        assert_eq!(model, "anthropic/claude-sonnet-4-20250514");
    }

    let dedicated_identity = match pool
        .sftp_read(&host.id, "~/.openclaw/agents/ops-bot/agent/IDENTITY.md")
        .await
    {
        Ok(identity) => identity,
        Err(_) => pool
            .sftp_read(&host.id, &format!("{dedicated_workspace}/IDENTITY.md"))
            .await
            .expect("read dedicated agent identity"),
    };
    assert!(
        dedicated_identity.contains("Ops Bot"),
        "expected identity to preserve display name, got:\n{dedicated_identity}"
    );
    assert!(
        dedicated_identity.contains("🛰️"),
        "expected identity to preserve emoji, got:\n{dedicated_identity}"
    );
    assert!(
        dedicated_identity.contains("## Persona"),
        "expected identity to include persona section, got:\n{dedicated_identity}"
    );
    assert!(
        dedicated_identity.contains("incident response"),
        "expected identity to include persona content, got:\n{dedicated_identity}"
    );

    let agent_persona_result = execute_workspace_recipe(
        &queue,
        &cache,
        &pool,
        &remote_queues,
        &host.id,
        "agent-persona-pack",
        "agent-persona-pack",
        sample_agent_persona_params(),
    )
    .await
    .expect("execute agent persona recipe");
    assert_eq!(
        agent_persona_result.summary,
        "Updated persona for agent main"
    );

    let main_identity = pool
        .sftp_read(&host.id, "~/.openclaw/agents/main/agent/IDENTITY.md")
        .await
        .expect("read main identity");
    assert!(main_identity.contains("- Name: Main Agent"));
    assert!(main_identity.contains("- Emoji: 🤖"));
    assert!(main_identity.contains("## Persona"));
    assert!(main_identity.contains("focused coaching agent"));

    let channel_persona_result = execute_workspace_recipe(
        &queue,
        &cache,
        &pool,
        &remote_queues,
        &host.id,
        "channel-persona-pack",
        "channel-persona-pack",
        sample_channel_persona_params(),
    )
    .await
    .expect("execute channel persona recipe");
    assert_eq!(
        channel_persona_result.summary,
        "Updated persona for channel channel-support"
    );

    let updated_config_raw = pool
        .sftp_read(&host.id, "~/.openclaw/openclaw.json")
        .await
        .expect("read updated remote config");
    let updated_config: Value =
        serde_json::from_str(&updated_config_raw).expect("updated config should be valid json");
    assert_eq!(
        updated_config
            .pointer("/channels/discord/guilds/guild-recipe-lab/channels/channel-support/systemPrompt")
            .and_then(Value::as_str),
        Some(
            "You are the support concierge for this channel.\n\nWelcome users, ask clarifying questions, and turn vague requests into clean next steps.\n"
        )
    );

    let runs = list_recipe_runs(Some(host.id.clone())).expect("list recipe runs for docker host");
    assert_eq!(runs.len(), 3);
    assert!(runs.iter().all(|run| run.status == "succeeded"));
    assert!(runs
        .iter()
        .any(|run| run.summary == dedicated_result.summary));
    assert!(runs
        .iter()
        .any(|run| run.summary == agent_persona_result.summary));
    assert!(runs
        .iter()
        .any(|run| run.summary == channel_persona_result.summary));
}
