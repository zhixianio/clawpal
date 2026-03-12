use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use clawpal_core::openclaw::OpenclawCli;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::models::resolve_paths;
use crate::recipe_executor::MaterializedExecutionPlan;
use crate::ssh::SshConnectionPool;

static ACTIVE_OPENCLAW_HOME_OVERRIDE: LazyLock<Mutex<Option<String>>> =
    LazyLock::new(|| Mutex::new(None));
static ACTIVE_CLAWPAL_DATA_OVERRIDE: LazyLock<Mutex<Option<String>>> =
    LazyLock::new(|| Mutex::new(None));

pub fn set_active_openclaw_home_override(path: Option<String>) -> Result<(), String> {
    let mut guard = ACTIVE_OPENCLAW_HOME_OVERRIDE
        .lock()
        .map_err(|_| "active openclaw home lock poisoned".to_string())?;
    let next = path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|raw| shellexpand::tilde(raw).to_string());
    *guard = next;
    Ok(())
}

pub fn get_active_openclaw_home_override() -> Option<String> {
    ACTIVE_OPENCLAW_HOME_OVERRIDE
        .lock()
        .ok()
        .and_then(|g| g.clone())
}

pub fn set_active_clawpal_data_override(path: Option<String>) -> Result<(), String> {
    let mut guard = ACTIVE_CLAWPAL_DATA_OVERRIDE
        .lock()
        .map_err(|_| "active clawpal data lock poisoned".to_string())?;
    let next = path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|raw| shellexpand::tilde(raw).to_string());
    *guard = next;
    Ok(())
}

pub fn get_active_clawpal_data_override() -> Option<String> {
    ACTIVE_CLAWPAL_DATA_OVERRIDE
        .lock()
        .ok()
        .and_then(|g| g.clone())
}

pub type CliOutput = clawpal_core::openclaw::CliOutput;

pub fn run_openclaw(args: &[&str]) -> Result<CliOutput, String> {
    run_openclaw_with_env(args, None)
}

pub fn run_openclaw_with_env(
    args: &[&str],
    env: Option<&HashMap<String, String>>,
) -> Result<CliOutput, String> {
    let mut merged_env = HashMap::new();
    if let Some(env_vars) = env {
        merged_env.extend(env_vars.clone());
    }
    if let Some(path) = get_active_openclaw_home_override() {
        if !merged_env.contains_key("OPENCLAW_HOME") {
            merged_env.insert("OPENCLAW_HOME".to_string(), path);
        }
    }
    let cli = OpenclawCli::new();
    cli.run_with_env(args, Some(&merged_env))
        .map_err(|e| e.to_string())
}

pub async fn run_openclaw_remote(
    pool: &SshConnectionPool,
    host_id: &str,
    args: &[&str],
) -> Result<CliOutput, String> {
    run_openclaw_remote_with_env(pool, host_id, args, None).await
}

pub async fn run_openclaw_remote_with_env(
    pool: &SshConnectionPool,
    host_id: &str,
    args: &[&str],
    env: Option<&HashMap<String, String>>,
) -> Result<CliOutput, String> {
    let cmd_str = build_remote_openclaw_command(args, env);
    let result = pool.exec_login(host_id, &cmd_str).await?;
    Ok(CliOutput {
        stdout: result.stdout,
        stderr: result.stderr,
        exit_code: result.exit_code as i32,
    })
}

fn build_remote_openclaw_command(args: &[&str], env: Option<&HashMap<String, String>>) -> String {
    let mut cmd_str = String::new();

    if let Some(env_vars) = env {
        for (k, v) in env_vars {
            cmd_str.push_str(&format!("export {}='{}'; ", k, v.replace('\'', "'\\''")));
        }
    }

    cmd_str.push_str(concat!(
        "clawpal_find_openclaw() { ",
        "if command -v openclaw >/dev/null 2>&1; then command -v openclaw; return 0; fi; ",
        "for cand in ",
        "\"$HOME/.npm-global/bin/openclaw\" ",
        "\"$HOME/.local/bin/openclaw\" ",
        "\"$HOME/.bun/bin/openclaw\" ",
        "\"$HOME/.cargo/bin/openclaw\" ",
        "\"/usr/local/bin/openclaw\" ",
        "\"/opt/homebrew/bin/openclaw\" ",
        "\"/usr/bin/openclaw\"; do ",
        "[ -x \"$cand\" ] && { printf '%s' \"$cand\"; return 0; }; ",
        "done; ",
        "return 1; ",
        "}; ",
        "clawpal_install_openclaw() { ",
        "platform=\"$(uname -s 2>/dev/null || printf unknown)\"; ",
        "if [ \"$platform\" = \"Linux\" ] || [ \"$platform\" = \"Darwin\" ] || grep -qi microsoft /proc/version 2>/dev/null; then ",
        "mkdir -p \"$HOME/.clawpal/install/cache\" && INSTALLER=\"$HOME/.clawpal/install/cache/openclaw-install.sh\" && ",
        "if command -v curl >/dev/null 2>&1; then ",
        "curl -fsSL --proto '=https' --tlsv1.2 https://openclaw.ai/install.sh -o \"$INSTALLER\"; ",
        "elif command -v wget >/dev/null 2>&1; then ",
        "wget -qO \"$INSTALLER\" https://openclaw.ai/install.sh; ",
        "else ",
        "echo 'openclaw install failed: curl or wget is required' >&2; return 1; ",
        "fi && bash \"$INSTALLER\" --no-prompt --no-onboard; ",
        "return $?; ",
        "fi; ",
        "if command -v powershell.exe >/dev/null 2>&1; then ",
        "powershell.exe -NoProfile -Command \"& ([scriptblock]::Create((iwr -useb https://openclaw.ai/install.ps1))) -InstallMethod npm -NoOnboard\"; ",
        "return $?; ",
        "fi; ",
        "if command -v powershell >/dev/null 2>&1; then ",
        "powershell -NoProfile -Command \"& ([scriptblock]::Create((iwr -useb https://openclaw.ai/install.ps1))) -InstallMethod npm -NoOnboard\"; ",
        "return $?; ",
        "fi; ",
        "if command -v cmd.exe >/dev/null 2>&1; then ",
        "cmd.exe /c \"curl -fsSL https://openclaw.ai/install.cmd -o install.cmd && install.cmd && del install.cmd\"; ",
        "return $?; ",
        "fi; ",
        "echo \"openclaw command not found after probe/auto-install (PATH=$PATH SHELL=$SHELL PLATFORM=$platform)\" >&2; ",
        "return 127; ",
        "}; ",
        "OPENCLAW_BIN=\"$(clawpal_find_openclaw 2>/dev/null || true)\"; ",
        "if [ -z \"$OPENCLAW_BIN\" ]; then ",
        "clawpal_install_openclaw || exit $?; ",
        "OPENCLAW_BIN=\"$(clawpal_find_openclaw 2>/dev/null || true)\"; ",
        "fi; ",
        "if [ -z \"$OPENCLAW_BIN\" ]; then ",
        "echo \"openclaw command not found after auto-install (PATH=$PATH SHELL=$SHELL)\" >&2; ",
        "exit 127; ",
        "fi; ",
        "\"$OPENCLAW_BIN\""
    ));
    for arg in args {
        cmd_str.push_str(&format!(" '{}'", arg.replace('\'', "'\\''")));
    }
    cmd_str
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn allowlisted_systemd_host_command_kind(command: &[String]) -> Option<&'static str> {
    match command {
        [bin, ..] if bin == "systemd-run" => Some("systemd-run"),
        [bin, user, action, ..]
            if bin == "systemctl"
                && user == "--user"
                && matches!(action.as_str(), "stop" | "reset-failed" | "daemon-reload") =>
        {
            Some("systemctl")
        }
        _ => None,
    }
}

fn is_allowlisted_systemd_host_command(command: &[String]) -> bool {
    allowlisted_systemd_host_command_kind(command).is_some()
}

fn build_remote_shell_command(
    command: &[String],
    env: Option<&HashMap<String, String>>,
) -> Result<String, String> {
    if command.is_empty() {
        return Err("host command is empty".to_string());
    }

    let mut shell = String::new();
    if let Some(env_vars) = env {
        for (key, value) in env_vars {
            shell.push_str(&format!("export {}={}; ", key, shell_quote(value)));
        }
    }
    shell.push_str(
        &command
            .iter()
            .map(|part| shell_quote(part))
            .collect::<Vec<_>>()
            .join(" "),
    );
    Ok(shell)
}

fn run_local_host_command(
    command: &[String],
    env: Option<&HashMap<String, String>>,
) -> Result<CliOutput, String> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| "host command is empty".to_string())?;
    let mut process = std::process::Command::new(program);
    process.args(args);
    if let Some(env_vars) = env {
        process.envs(env_vars);
    }
    let output = process.output().map_err(|error| {
        format!(
            "failed to start host command '{}': {}",
            command.join(" "),
            error
        )
    })?;
    Ok(CliOutput {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code().unwrap_or(1),
    })
}

fn run_allowlisted_systemd_local_command(command: &[String]) -> Result<Option<CliOutput>, String> {
    if !is_allowlisted_systemd_host_command(command) {
        return Ok(None);
    }
    run_local_host_command(command, None).map(Some)
}

async fn run_allowlisted_systemd_remote_command(
    pool: &SshConnectionPool,
    host_id: &str,
    command: &[String],
) -> Result<Option<CliOutput>, String> {
    if !is_allowlisted_systemd_host_command(command) {
        return Ok(None);
    }
    let shell = build_remote_shell_command(command, None)?;
    let output = pool.exec_login(host_id, &shell).await?;
    Ok(Some(CliOutput {
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code: output.exit_code as i32,
    }))
}

fn systemd_dropin_relative_path(target: &str, name: &str) -> String {
    format!("~/.config/systemd/user/{}.d/{}", target, name)
}

fn write_local_systemd_dropin(target: &str, name: &str, content: &str) -> Result<(), String> {
    let path =
        PathBuf::from(shellexpand::tilde(&systemd_dropin_relative_path(target, name)).to_string());
    crate::config_io::write_text(path.as_path(), content)
}

async fn write_remote_systemd_dropin(
    pool: &SshConnectionPool,
    host_id: &str,
    target: &str,
    name: &str,
    content: &str,
) -> Result<(), String> {
    let dir = format!("~/.config/systemd/user/{}.d", target);
    let resolved_dir = pool.resolve_path(host_id, &dir).await?;
    pool.exec(host_id, &format!("mkdir -p {}", shell_quote(&resolved_dir)))
        .await?;
    pool.sftp_write(
        host_id,
        &systemd_dropin_relative_path(target, name),
        content,
    )
    .await
}

pub fn parse_json_output(output: &CliOutput) -> Result<Value, String> {
    clawpal_core::openclaw::parse_json_output(output).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_remote_openclaw_command_reports_diagnostic_context_when_missing() {
        let cmd = build_remote_openclaw_command(&["agents", "list", "--json"], None);
        assert!(cmd.contains("clawpal_find_openclaw"));
        assert!(cmd.contains("https://openclaw.ai/install.sh"));
        assert!(cmd.contains("--no-prompt --no-onboard"));
        assert!(cmd.contains("https://openclaw.ai/install.ps1"));
        assert!(cmd.contains("install.cmd"));
        assert!(cmd.contains("openclaw command not found after auto-install"));
    }

    #[test]
    fn build_remote_openclaw_command_escapes_args_and_env() {
        let mut env = HashMap::new();
        env.insert("OPENCLAW_HOME".to_string(), "/tmp/a'b".to_string());
        let cmd = build_remote_openclaw_command(&["config", "get", "a'b"], Some(&env));
        assert!(cmd.contains("export OPENCLAW_HOME='/tmp/a'\\''b';"));
        assert!(cmd.contains(" 'a'\\''b'"));
    }

    #[test]
    fn allowlisted_systemd_host_commands_are_restricted_to_expected_shapes() {
        assert!(is_allowlisted_systemd_host_command(&[
            "systemd-run".into(),
            "--unit=clawpal-job-hourly".into(),
            "--".into(),
            "openclaw".into(),
            "doctor".into(),
            "run".into(),
        ]));
        assert!(is_allowlisted_systemd_host_command(&[
            "systemctl".into(),
            "--user".into(),
            "daemon-reload".into(),
        ]));
        assert!(!is_allowlisted_systemd_host_command(&[
            "systemctl".into(),
            "--system".into(),
            "daemon-reload".into(),
        ]));
        assert!(!is_allowlisted_systemd_host_command(&[
            "bash".into(),
            "-lc".into(),
            "echo nope".into(),
        ]));
    }

    #[test]
    fn rollback_command_supports_snapshot_id_prefix() {
        let command = vec![
            "__rollback__".to_string(),
            "snapshot_01".to_string(),
            "{\"ok\":true}".to_string(),
        ];

        assert_eq!(
            rollback_command_snapshot_id(&command).as_deref(),
            Some("snapshot_01")
        );
        assert_eq!(
            rollback_command_content(&command).expect("rollback content"),
            "{\"ok\":true}"
        );
    }

    #[test]
    fn preview_direct_apply_handles_config_set_and_unset_with_arrays() {
        let mut config = json!({
            "agents": {
                "list": [
                    {"id": "main", "model": {"primary": "old/model"}}
                ]
            }
        });

        let set_cmd = PendingCommand {
            id: "1".into(),
            label: "set".into(),
            command: vec![
                "openclaw".into(),
                "config".into(),
                "set".into(),
                "agents.list.0.model.primary".into(),
                "new/model".into(),
            ],
            created_at: String::new(),
        };
        let unset_cmd = PendingCommand {
            id: "2".into(),
            label: "unset".into(),
            command: vec![
                "openclaw".into(),
                "config".into(),
                "unset".into(),
                "agents.list.0.model.primary".into(),
            ],
            created_at: String::new(),
        };

        let domains = apply_direct_preview_command(&mut config, &set_cmd)
            .expect("set should succeed")
            .expect("must be direct");
        assert!(domains.agents);
        assert_eq!(
            config
                .pointer("/agents/list/0/model/primary")
                .and_then(Value::as_str),
            Some("new/model")
        );

        let domains = apply_direct_preview_command(&mut config, &unset_cmd)
            .expect("unset should succeed")
            .expect("must be direct");
        assert!(domains.agents);
        assert!(config.pointer("/agents/list/0/model/primary").is_none());
    }

    #[test]
    fn preview_direct_apply_supports_json_payloads_and_root_replace() {
        let mut config = json!({"gateway": {"port": 18789}});
        let set_root_cmd = PendingCommand {
            id: "1".into(),
            label: "replace".into(),
            command: vec![
                "openclaw".into(),
                "config".into(),
                "set".into(),
                ".".into(),
                "{\"gateway\":{\"port\":19789},\"bindings\":[]}".into(),
                "--json".into(),
            ],
            created_at: String::new(),
        };

        let domains = apply_direct_preview_command(&mut config, &set_root_cmd)
            .expect("root replace should succeed")
            .expect("must be direct");
        assert!(domains.generic);
        assert_eq!(config["gateway"]["port"], json!(19789));
        assert_eq!(config["bindings"], json!([]));
    }

    #[test]
    fn preview_direct_apply_supports_agents_add_and_delete() {
        let mut config = json!({
            "agents": {
                "list": [
                    {"id": "main"}
                ]
            }
        });
        let add_cmd = PendingCommand {
            id: "1".into(),
            label: "add".into(),
            command: vec![
                "openclaw".into(),
                "agents".into(),
                "add".into(),
                "helper".into(),
                "--non-interactive".into(),
                "--model".into(),
                "openai/gpt-5".into(),
                "--workspace".into(),
                "~/.openclaw/workspaces/helper".into(),
            ],
            created_at: String::new(),
        };
        let delete_cmd = PendingCommand {
            id: "2".into(),
            label: "delete".into(),
            command: vec![
                "openclaw".into(),
                "agents".into(),
                "delete".into(),
                "helper".into(),
                "--force".into(),
            ],
            created_at: String::new(),
        };

        let domains = apply_direct_preview_command(&mut config, &add_cmd)
            .expect("add should succeed")
            .expect("must be direct");
        assert!(domains.agents);
        let helper = config
            .pointer("/agents/list")
            .and_then(Value::as_array)
            .and_then(|list| {
                list.iter()
                    .find(|entry| entry.get("id").and_then(Value::as_str) == Some("helper"))
            })
            .expect("helper agent");
        assert_eq!(helper["model"], json!("openai/gpt-5"));
        assert_eq!(helper["workspace"], json!("~/.openclaw/workspaces/helper"));

        let domains = apply_direct_preview_command(&mut config, &delete_cmd)
            .expect("delete should succeed")
            .expect("must be direct");
        assert!(domains.agents);
        assert!(config
            .pointer("/agents/list")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .all(|entry| entry.get("id").and_then(Value::as_str) != Some("helper")));
    }

    #[test]
    fn preview_direct_apply_returns_none_for_unknown_commands() {
        let mut config = json!({});
        let unknown_cmd = PendingCommand {
            id: "1".into(),
            label: "fallback".into(),
            command: vec!["openclaw".into(), "gateway".into(), "restart".into()],
            created_at: String::new(),
        };

        let result = apply_direct_preview_command(&mut config, &unknown_cmd)
            .expect("parsing should succeed");
        assert!(result.is_none());
    }

    #[test]
    fn preview_direct_apply_skips_allowlisted_systemd_commands() {
        let mut config = json!({"gateway": {"port": 18789}});
        let host_cmd = PendingCommand {
            id: "1".into(),
            label: "Run hourly job".into(),
            command: vec![
                "systemd-run".into(),
                "--unit=clawpal-job-hourly".into(),
                "--".into(),
                "openclaw".into(),
                "doctor".into(),
                "run".into(),
            ],
            created_at: String::new(),
        };

        let touched = apply_direct_preview_command(&mut config, &host_cmd)
            .expect("preview should accept allowlisted host command")
            .expect("host command should be handled directly");

        assert_eq!(config["gateway"]["port"], json!(18789));
        assert!(!touched.agents && !touched.channels && !touched.bindings && !touched.generic);
    }

    #[test]
    fn preview_direct_apply_skips_internal_systemd_dropin_write_command() {
        let mut config = json!({"gateway": {"port": 18789}});
        let host_cmd = PendingCommand {
            id: "1".into(),
            label: "Write drop-in".into(),
            command: vec![
                crate::commands::INTERNAL_SYSTEMD_DROPIN_WRITE_COMMAND.into(),
                "openclaw-gateway.service".into(),
                "10-env.conf".into(),
                "[Service]\nEnvironment=OPENCLAW_CHANNEL=discord".into(),
            ],
            created_at: String::new(),
        };

        let touched = apply_direct_preview_command(&mut config, &host_cmd)
            .expect("preview should accept internal drop-in write")
            .expect("drop-in write should be handled directly");

        assert_eq!(config["gateway"]["port"], json!(18789));
        assert!(!touched.agents && !touched.channels && !touched.bindings && !touched.generic);
    }

    #[test]
    fn preview_side_effect_warning_marks_agent_commands() {
        let add_cmd = PendingCommand {
            id: "1".into(),
            label: "Create agent: helper".into(),
            command: vec![
                "openclaw".into(),
                "agents".into(),
                "add".into(),
                "helper".into(),
            ],
            created_at: String::new(),
        };
        let delete_cmd = PendingCommand {
            id: "2".into(),
            label: "Delete agent: helper".into(),
            command: vec![
                "openclaw".into(),
                "agents".into(),
                "delete".into(),
                "helper".into(),
            ],
            created_at: String::new(),
        };

        assert!(preview_side_effect_warning(&add_cmd)
            .expect("add warning")
            .contains("workspace/filesystem setup"));
        assert!(preview_side_effect_warning(&delete_cmd)
            .expect("delete warning")
            .contains("filesystem cleanup"));
    }

    #[test]
    fn preview_side_effect_warning_marks_systemd_commands() {
        let host_cmd = PendingCommand {
            id: "1".into(),
            label: "Run hourly job".into(),
            command: vec![
                "systemd-run".into(),
                "--unit=clawpal-job-hourly".into(),
                "--".into(),
                "openclaw".into(),
                "doctor".into(),
                "run".into(),
            ],
            created_at: String::new(),
        };
        let drop_in_cmd = PendingCommand {
            id: "2".into(),
            label: "Write drop-in".into(),
            command: vec![
                crate::commands::INTERNAL_SYSTEMD_DROPIN_WRITE_COMMAND.into(),
                "openclaw-gateway.service".into(),
                "10-env.conf".into(),
                "[Service]\nEnvironment=OPENCLAW_CHANNEL=discord".into(),
            ],
            created_at: String::new(),
        };

        assert!(preview_side_effect_warning(&host_cmd)
            .expect("systemd warning")
            .contains("host-side systemd changes"));
        assert!(preview_side_effect_warning(&drop_in_cmd)
            .expect("drop-in warning")
            .contains("does not write systemd drop-in"));
    }
}

// ---------------------------------------------------------------------------
// CommandQueue — Task 2
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingCommand {
    pub id: String,
    pub label: String,
    pub command: Vec<String>,
    pub created_at: String,
}

#[derive(Clone)]
pub struct CommandQueue {
    commands: Arc<Mutex<Vec<PendingCommand>>>,
}

impl CommandQueue {
    pub fn new() -> Self {
        Self {
            commands: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn enqueue(&self, label: String, command: Vec<String>) -> PendingCommand {
        let cmd = PendingCommand {
            id: Uuid::new_v4().to_string(),
            label,
            command,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        self.commands.lock().unwrap().push(cmd.clone());
        cmd
    }

    pub fn remove(&self, id: &str) -> bool {
        let mut cmds = self.commands.lock().unwrap();
        let before = cmds.len();
        cmds.retain(|c| c.id != id);
        cmds.len() < before
    }

    pub fn list(&self) -> Vec<PendingCommand> {
        self.commands.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.commands.lock().unwrap().clear();
    }

    pub fn is_empty(&self) -> bool {
        self.commands.lock().unwrap().is_empty()
    }

    pub fn len(&self) -> usize {
        self.commands.lock().unwrap().len()
    }
}

impl Default for CommandQueue {
    fn default() -> Self {
        Self::new()
    }
}

pub fn enqueue_materialized_plan(
    queue: &CommandQueue,
    plan: &MaterializedExecutionPlan,
) -> Vec<PendingCommand> {
    plan.commands
        .iter()
        .enumerate()
        .map(|(index, command)| {
            let label = format!(
                "[{}] {} ({}/{})",
                plan.execution_kind,
                plan.unit_name,
                index + 1,
                plan.commands.len()
            );
            queue.enqueue(label, command.clone())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tauri commands — Task 3
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn queue_command(
    queue: tauri::State<CommandQueue>,
    label: String,
    command: Vec<String>,
) -> Result<PendingCommand, String> {
    if command.is_empty() {
        return Err("command cannot be empty".into());
    }
    Ok(queue.enqueue(label, command))
}

#[tauri::command]
pub fn remove_queued_command(
    queue: tauri::State<CommandQueue>,
    id: String,
) -> Result<bool, String> {
    Ok(queue.remove(&id))
}

#[tauri::command]
pub fn list_queued_commands(
    queue: tauri::State<CommandQueue>,
) -> Result<Vec<PendingCommand>, String> {
    Ok(queue.list())
}

#[tauri::command]
pub fn discard_queued_commands(queue: tauri::State<CommandQueue>) -> Result<bool, String> {
    queue.clear();
    Ok(true)
}

#[tauri::command]
pub fn queued_commands_count(queue: tauri::State<CommandQueue>) -> Result<usize, String> {
    Ok(queue.len())
}

// ---------------------------------------------------------------------------
// Preview — sandbox execution with OPENCLAW_HOME
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
struct PreviewTouchedDomains {
    agents: bool,
    channels: bool,
    bindings: bool,
    generic: bool,
}

impl PreviewTouchedDomains {
    fn mark_path(&mut self, path: &str) {
        let normalized = path.trim();
        if normalized.is_empty() || normalized == "." {
            self.generic = true;
            return;
        }
        if normalized == "bindings" || normalized.starts_with("bindings.") {
            self.bindings = true;
            return;
        }
        if normalized == "channels" || normalized.starts_with("channels.") {
            self.channels = true;
            return;
        }
        if normalized == "agents" || normalized.starts_with("agents.") {
            self.agents = true;
            return;
        }
        self.generic = true;
    }

    fn merge(&mut self, other: Self) {
        self.agents |= other.agents;
        self.channels |= other.channels;
        self.bindings |= other.bindings;
        self.generic |= other.generic;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewValidationTarget {
    Agents,
    Channels,
    Bindings,
    Generic,
}

impl PreviewValidationTarget {
    fn label(self) -> &'static str {
        match self {
            Self::Agents => "agents",
            Self::Channels => "channels",
            Self::Bindings => "bindings",
            Self::Generic => "config",
        }
    }

    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Agents => &["agents", "list", "--json"],
            Self::Channels => &["config", "get", "channels", "--json"],
            Self::Bindings => &["config", "get", "bindings", "--json"],
            Self::Generic => &["config", "get", "agents", "--json"],
        }
    }
}

fn preview_validation_targets(touched: PreviewTouchedDomains) -> Vec<PreviewValidationTarget> {
    let mut targets = Vec::new();
    if touched.agents {
        targets.push(PreviewValidationTarget::Agents);
    }
    if touched.channels {
        targets.push(PreviewValidationTarget::Channels);
    }
    if touched.bindings {
        targets.push(PreviewValidationTarget::Bindings);
    }
    if touched.generic || targets.is_empty() {
        targets.push(PreviewValidationTarget::Generic);
    }
    targets
}

fn log_preview_stage(
    scope: &str,
    host_id: Option<&str>,
    queue_size: usize,
    stage: &str,
    outcome: &str,
    elapsed_ms: u128,
    detail: Option<&str>,
) {
    let mut line = format!(
        "[preview] scope={scope} queueSize={queue_size} stage={stage} outcome={outcome} elapsedMs={elapsed_ms}"
    );
    if let Some(host) = host_id {
        line.push_str(&format!(" hostId={host}"));
    }
    if let Some(extra) = detail.map(str::trim).filter(|s| !s.is_empty()) {
        let compact = extra.replace('\n', " ").replace('\r', " ");
        line.push_str(&format!(" detail={compact}"));
    }
    crate::logging::log_info(&line);
}

fn parse_preview_config(raw: &str) -> Value {
    clawpal_core::config::parse_config_json5(raw)
}

fn render_preview_config(config: &Value) -> Result<String, String> {
    clawpal_core::doctor::render_json_document(config, "preview config")
}

fn normalize_config_for_preview(raw: &str) -> String {
    fn sort_value(v: &Value) -> Value {
        match v {
            Value::Object(map) => {
                let sorted: serde_json::Map<String, Value> = map
                    .iter()
                    .collect::<std::collections::BTreeMap<_, _>>()
                    .into_iter()
                    .map(|(k, v)| (k.clone(), sort_value(v)))
                    .collect();
                Value::Object(sorted)
            }
            Value::Array(arr) => Value::Array(arr.iter().map(sort_value).collect()),
            other => other.clone(),
        }
    }

    match serde_json::from_str::<Value>(raw) {
        Ok(v) => serde_json::to_string_pretty(&sort_value(&v)).unwrap_or_else(|_| raw.to_string()),
        Err(_) => raw.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreviewPathSegment {
    Key(String),
    Index(usize),
}

fn parse_preview_path(path: &str) -> Vec<PreviewPathSegment> {
    path.split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .map(|segment| match segment.parse::<usize>() {
            Ok(index) => PreviewPathSegment::Index(index),
            Err(_) => PreviewPathSegment::Key(segment.to_string()),
        })
        .collect()
}

fn ensure_preview_child(next: Option<&PreviewPathSegment>) -> Value {
    match next {
        Some(PreviewPathSegment::Index(_)) => Value::Array(Vec::new()),
        _ => Value::Object(Default::default()),
    }
}

fn preview_upsert_path(
    cursor: &mut Value,
    segments: &[PreviewPathSegment],
    next_value: Value,
) -> Result<(), String> {
    if segments.is_empty() {
        *cursor = next_value;
        return Ok(());
    }

    match &segments[0] {
        PreviewPathSegment::Key(key) => {
            if !cursor.is_object() {
                if cursor.is_null() {
                    *cursor = Value::Object(Default::default());
                } else {
                    return Err(format!("path segment '{key}' is not an object"));
                }
            }

            let obj = cursor
                .as_object_mut()
                .ok_or_else(|| format!("path segment '{key}' is not an object"))?;
            if segments.len() == 1 {
                obj.insert(key.clone(), next_value);
                return Ok(());
            }
            let entry = obj
                .entry(key.clone())
                .or_insert_with(|| ensure_preview_child(segments.get(1)));
            if entry.is_null() {
                *entry = ensure_preview_child(segments.get(1));
            }
            preview_upsert_path(entry, &segments[1..], next_value)
        }
        PreviewPathSegment::Index(index) => {
            if !cursor.is_array() {
                if cursor.is_null() {
                    *cursor = Value::Array(Vec::new());
                } else {
                    return Err(format!("path segment '{index}' is not an array"));
                }
            }

            let arr = cursor
                .as_array_mut()
                .ok_or_else(|| format!("path segment '{index}' is not an array"))?;
            while arr.len() <= *index {
                arr.push(ensure_preview_child(segments.get(1)));
            }
            if segments.len() == 1 {
                arr[*index] = next_value;
                return Ok(());
            }
            if arr[*index].is_null() {
                arr[*index] = ensure_preview_child(segments.get(1));
            }
            preview_upsert_path(&mut arr[*index], &segments[1..], next_value)
        }
    }
}

fn preview_delete_path(cursor: &mut Value, segments: &[PreviewPathSegment]) -> bool {
    if segments.is_empty() {
        return false;
    }

    match &segments[0] {
        PreviewPathSegment::Key(key) => {
            let Some(obj) = cursor.as_object_mut() else {
                return false;
            };
            if segments.len() == 1 {
                return obj.remove(key).is_some();
            }
            let Some(next) = obj.get_mut(key) else {
                return false;
            };
            preview_delete_path(next, &segments[1..])
        }
        PreviewPathSegment::Index(index) => {
            let Some(arr) = cursor.as_array_mut() else {
                return false;
            };
            if *index >= arr.len() {
                return false;
            }
            if segments.len() == 1 {
                arr.remove(*index);
                return true;
            }
            preview_delete_path(&mut arr[*index], &segments[1..])
        }
    }
}

fn set_preview_path_value(config: &mut Value, path: &str, value: Value) -> Result<(), String> {
    if path.trim().is_empty() || path.trim() == "." {
        *config = value;
        return Ok(());
    }
    let segments = parse_preview_path(path);
    if segments.is_empty() {
        *config = value;
        return Ok(());
    }
    preview_upsert_path(config, &segments, value)
}

fn delete_preview_path_value(config: &mut Value, path: &str) -> bool {
    if path.trim().is_empty() || path.trim() == "." {
        *config = Value::Object(Default::default());
        return true;
    }
    let segments = parse_preview_path(path);
    if segments.is_empty() {
        *config = Value::Object(Default::default());
        return true;
    }
    preview_delete_path(config, &segments)
}

fn ensure_preview_agents_list(config: &mut Value) -> Result<&mut Vec<Value>, String> {
    if config.get("agents").is_none() {
        set_preview_path_value(config, "agents", Value::Object(Default::default()))?;
    }
    if config.pointer("/agents/list").is_none() {
        set_preview_path_value(config, "agents.list", Value::Array(Vec::new()))?;
    }
    config
        .pointer_mut("/agents/list")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "agents.list is not an array".to_string())
}

fn apply_direct_preview_command(
    config: &mut Value,
    cmd: &PendingCommand,
) -> Result<Option<PreviewTouchedDomains>, String> {
    let Some(first) = cmd.command.first().map(|s| s.as_str()) else {
        return Ok(None);
    };

    match first {
        crate::commands::INTERNAL_SYSTEMD_DROPIN_WRITE_COMMAND => {
            return Ok(Some(PreviewTouchedDomains::default()));
        }
        "__config_write__" | "__rollback__" => {
            let Some(content) = cmd.command.get(1) else {
                return Err(format!("{}: missing config payload", cmd.label));
            };
            *config = parse_preview_config(content);
            let mut touched = PreviewTouchedDomains::default();
            touched.generic = true;
            return Ok(Some(touched));
        }
        "openclaw" => {}
        _ if is_allowlisted_systemd_host_command(&cmd.command) => {
            return Ok(Some(PreviewTouchedDomains::default()));
        }
        _ => return Ok(None),
    }

    let args = &cmd.command[1..];
    let Some(subcommand_idx) = args
        .iter()
        .position(|arg| arg == "config" || arg == "agents")
    else {
        return Ok(None);
    };
    let command = &args[subcommand_idx..];
    match command {
        [action, op, path, value, rest @ ..] if action == "config" && op == "set" => {
            let parsed_value = if rest.iter().any(|arg| arg == "--json") {
                serde_json::from_str::<Value>(value)
                    .or_else(|_| json5::from_str::<Value>(value))
                    .map_err(|e| format!("{}: invalid JSON value for {path}: {e}", cmd.label))?
            } else {
                Value::String(value.clone())
            };
            set_preview_path_value(config, path, parsed_value)?;
            let mut touched = PreviewTouchedDomains::default();
            touched.mark_path(path);
            Ok(Some(touched))
        }
        [action, op, path, ..] if action == "config" && op == "unset" => {
            let _ = delete_preview_path_value(config, path);
            let mut touched = PreviewTouchedDomains::default();
            touched.mark_path(path);
            Ok(Some(touched))
        }
        [action, op, agent_id, rest @ ..] if action == "agents" && op == "add" => {
            let mut model: Option<String> = None;
            let mut workspace: Option<String> = None;
            let mut idx = 0usize;
            while idx < rest.len() {
                match rest[idx].as_str() {
                    "--model" => {
                        if let Some(next) = rest.get(idx + 1) {
                            model = Some(next.clone());
                            idx += 2;
                        } else {
                            return Err(format!("{}: missing --model value", cmd.label));
                        }
                    }
                    "--workspace" => {
                        if let Some(next) = rest.get(idx + 1) {
                            workspace = Some(next.clone());
                            idx += 2;
                        } else {
                            return Err(format!("{}: missing --workspace value", cmd.label));
                        }
                    }
                    _ => idx += 1,
                }
            }

            let list = ensure_preview_agents_list(config)?;
            list.retain(|entry| entry.get("id").and_then(Value::as_str) != Some(agent_id.as_str()));
            let mut agent = serde_json::Map::new();
            agent.insert("id".to_string(), Value::String(agent_id.clone()));
            if let Some(model_value) = model {
                agent.insert("model".to_string(), Value::String(model_value));
            }
            if let Some(workspace_value) = workspace {
                agent.insert("workspace".to_string(), Value::String(workspace_value));
            }
            list.push(Value::Object(agent));
            let mut touched = PreviewTouchedDomains::default();
            touched.agents = true;
            Ok(Some(touched))
        }
        [action, op, agent_id, ..] if action == "agents" && op == "delete" => {
            let list = ensure_preview_agents_list(config)?;
            list.retain(|entry| entry.get("id").and_then(Value::as_str) != Some(agent_id.as_str()));
            let mut touched = PreviewTouchedDomains::default();
            touched.agents = true;
            Ok(Some(touched))
        }
        _ => Ok(None),
    }
}

fn preview_side_effect_warning(cmd: &PendingCommand) -> Option<String> {
    if cmd.command.first().map(|value| value.as_str())
        == Some(crate::commands::INTERNAL_SYSTEMD_DROPIN_WRITE_COMMAND)
    {
        let target = cmd.command.get(1).map(String::as_str).unwrap_or("systemd");
        let name = cmd.command.get(2).map(String::as_str).unwrap_or("drop-in");
        return Some(format!(
            "{}: preview does not write systemd drop-in '{}:{}'; file creation will run during apply.",
            cmd.label, target, name
        ));
    }

    if let Some(kind) = allowlisted_systemd_host_command_kind(&cmd.command) {
        return Some(format!(
            "{}: preview does not execute allowlisted {} command '{}'; host-side systemd changes will run during apply.",
            cmd.label,
            kind,
            cmd.command.join(" ")
        ));
    }

    let [bin, category, action, target, ..] = cmd.command.as_slice() else {
        return None;
    };
    if bin == "openclaw" && category == "agents" {
        return match action.as_str() {
            "add" => Some(format!(
                "{}: preview only validates config changes; agent workspace/filesystem setup for '{}' will run during apply.",
                cmd.label, target
            )),
            "delete" => Some(format!(
                "{}: preview only validates config changes; any filesystem cleanup for '{}' is not simulated.",
                cmd.label, target
            )),
            _ => None,
        };
    }

    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewQueueResult {
    pub commands: Vec<PendingCommand>,
    pub config_before: String,
    pub config_after: String,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[tauri::command]
pub async fn preview_queued_commands(
    queue: tauri::State<'_, CommandQueue>,
) -> Result<PreviewQueueResult, String> {
    let commands = queue.list();
    if commands.is_empty() {
        return Err("No pending commands to preview".into());
    }

    tauri::async_runtime::spawn_blocking(move || {
        let paths = resolve_paths();
        let queue_size = commands.len();

        // Read current config
        let read_started = Instant::now();
        let config_before = crate::config_io::read_text(&paths.config_path)?;
        log_preview_stage(
            "local",
            None,
            queue_size,
            "readConfig",
            "success",
            read_started.elapsed().as_millis(),
            None,
        );
        let mut preview_config_json = parse_preview_config(&config_before);

        // Set up sandbox: symlink all entries from real .openclaw/ into sandbox,
        // but copy openclaw.json so commands modify the copy, not the original.
        // This ensures the CLI can find extensions, plugins, etc. for validation.
        let sandbox_started = Instant::now();
        let sandbox_root = paths.clawpal_dir.join("preview");
        let preview_dir = sandbox_root.join(".openclaw");
        // Clean previous sandbox if any
        let _ = std::fs::remove_dir_all(&sandbox_root);
        std::fs::create_dir_all(&preview_dir).map_err(|e| e.to_string())?;

        // Symlink all sibling entries from real .openclaw/
        if let Ok(entries) = std::fs::read_dir(&paths.base_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name == "openclaw.json" {
                    continue;
                }
                let target = preview_dir.join(&name);
                #[cfg(unix)]
                {
                    let _ = std::os::unix::fs::symlink(entry.path(), &target);
                }
            }
        }

        // Seed config file for sandbox preview. Source config may not exist yet
        // (fresh docker-local state), so write the already-loaded content.
        let preview_config = preview_dir.join("openclaw.json");
        crate::config_io::write_text(&preview_config, &config_before)?;
        log_preview_stage(
            "local",
            None,
            queue_size,
            "setupSandbox",
            "success",
            sandbox_started.elapsed().as_millis(),
            None,
        );

        let mut env = HashMap::new();
        env.insert(
            "OPENCLAW_HOME".to_string(),
            sandbox_root.to_string_lossy().to_string(),
        );

        // Execute preview queue in sandbox, applying config-like mutations directly
        // and only falling back to CLI for commands that cannot be safely simulated.
        let apply_started = Instant::now();
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut touched = PreviewTouchedDomains::default();
        let mut preview_dirty = false;
        for cmd in &commands {
            match apply_direct_preview_command(&mut preview_config_json, cmd) {
                Ok(Some(domains)) => {
                    touched.merge(domains);
                    if let Some(warning) = preview_side_effect_warning(cmd) {
                        warnings.push(warning);
                    }
                    preview_dirty = true;
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    errors.push(e);
                    break;
                }
            }

            if preview_dirty {
                let rendered = render_preview_config(&preview_config_json)?;
                crate::config_io::write_text(&preview_config, &rendered)?;
                preview_dirty = false;
            }

            let args: Vec<&str> = cmd.command.iter().skip(1).map(|s| s.as_str()).collect();
            let result = run_openclaw_with_env(&args, Some(&env));
            match result {
                Ok(output) if output.exit_code != 0 => {
                    let detail = if !output.stderr.is_empty() {
                        output.stderr.clone()
                    } else {
                        output.stdout.clone()
                    };
                    errors.push(format!("{}: {}", cmd.label, detail));
                    break;
                }
                Err(e) => {
                    errors.push(format!("{}: {}", cmd.label, e));
                    break;
                }
                Ok(_) => {
                    let latest = crate::config_io::read_text(&preview_config)
                        .unwrap_or_else(|_| config_before.clone());
                    preview_config_json = parse_preview_config(&latest);
                }
            }
        }
        let apply_outcome = if errors.is_empty() {
            "success"
        } else {
            "error"
        };
        let apply_detail = errors.first().map(String::as_str);
        log_preview_stage(
            "local",
            None,
            queue_size,
            "applyPreviewTransforms",
            apply_outcome,
            apply_started.elapsed().as_millis(),
            apply_detail,
        );

        if preview_dirty {
            let rendered = render_preview_config(&preview_config_json)?;
            crate::config_io::write_text(&preview_config, &rendered)?;
        }

        if errors.is_empty() {
            let validate_started = Instant::now();
            for target in preview_validation_targets(touched) {
                let output = run_openclaw_with_env(target.args(), Some(&env));
                match output {
                    Ok(output) if output.exit_code == 0 => {}
                    Ok(output) => {
                        let detail = if !output.stderr.is_empty() {
                            output.stderr.clone()
                        } else {
                            output.stdout.clone()
                        };
                        let message =
                            format!("Preview validation ({}) failed: {}", target.label(), detail);
                        errors.push(message.clone());
                        log_preview_stage(
                            "local",
                            None,
                            queue_size,
                            "cliValidate",
                            "error",
                            validate_started.elapsed().as_millis(),
                            Some(&message),
                        );
                        break;
                    }
                    Err(err) => {
                        let message =
                            format!("Preview validation ({}) failed: {}", target.label(), err);
                        errors.push(message.clone());
                        log_preview_stage(
                            "local",
                            None,
                            queue_size,
                            "cliValidate",
                            "error",
                            validate_started.elapsed().as_millis(),
                            Some(&message),
                        );
                        break;
                    }
                }
            }
            if errors.is_empty() {
                log_preview_stage(
                    "local",
                    None,
                    queue_size,
                    "cliValidate",
                    "success",
                    validate_started.elapsed().as_millis(),
                    None,
                );
            }
        }

        // Always read result config from sandbox (commands may have partially succeeded)
        // Replace sandbox paths with real paths so the diff doesn't show sandbox artifacts.
        let readback_started = Instant::now();
        let sandbox_prefix = sandbox_root.to_string_lossy().to_string();
        let real_home = paths
            .openclaw_dir
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let config_after_raw = crate::config_io::read_text(&preview_config)
            .unwrap_or_else(|_| config_before.clone())
            .replace(&sandbox_prefix, &real_home);
        log_preview_stage(
            "local",
            None,
            queue_size,
            "readbackConfig",
            "success",
            readback_started.elapsed().as_millis(),
            None,
        );

        let config_before = normalize_config_for_preview(&config_before);
        let config_after = normalize_config_for_preview(&config_after_raw);

        // Cleanup sandbox
        let cleanup_started = Instant::now();
        let _ = std::fs::remove_dir_all(paths.clawpal_dir.join("preview"));
        log_preview_stage(
            "local",
            None,
            queue_size,
            "cleanupSandbox",
            "success",
            cleanup_started.elapsed().as_millis(),
            None,
        );

        Ok(PreviewQueueResult {
            commands,
            config_before,
            config_after,
            warnings,
            errors,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// Apply — execute queue for real, rollback on failure
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyQueueResult {
    pub ok: bool,
    pub applied_count: usize,
    pub total_count: usize,
    pub error: Option<String>,
    pub rolled_back: bool,
}

fn rollback_command_snapshot_id(command: &[String]) -> Option<String> {
    if command.first().map(|value| value.as_str()) != Some("__rollback__") {
        return None;
    }
    if command.len() >= 3 {
        return command
            .get(1)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }
    None
}

fn rollback_command_content(command: &[String]) -> Result<String, String> {
    match command.first().map(|value| value.as_str()) {
        Some("__rollback__") if command.len() >= 3 => command
            .get(2)
            .cloned()
            .ok_or_else(|| "internal rollback is missing content".to_string()),
        Some("__rollback__") | Some("__config_write__") => command
            .get(1)
            .cloned()
            .ok_or_else(|| "internal config write is missing content".to_string()),
        _ => command
            .get(1)
            .cloned()
            .ok_or_else(|| "internal config write is missing content".to_string()),
    }
}

fn apply_internal_local_command(
    paths: &crate::models::OpenClawPaths,
    command: &[String],
) -> Result<bool, String> {
    fn content(command: &[String]) -> Result<String, String> {
        rollback_command_content(command)
    }
    match command.first().map(|value| value.as_str()) {
        Some("__config_write__") | Some("__rollback__") => {
            let content = content(command)?;
            crate::config_io::write_text(&paths.config_path, &content)?;
            Ok(true)
        }
        Some(crate::commands::INTERNAL_SETUP_IDENTITY_COMMAND) => {
            let agent_id = command
                .get(1)
                .ok_or_else(|| "setup_identity command missing agent id".to_string())?;
            let name = command
                .get(2)
                .ok_or_else(|| "setup_identity command missing name".to_string())?;
            crate::agent_identity::write_local_agent_identity(
                paths,
                agent_id,
                name,
                command.get(3).map(String::as_str),
            )?;
            Ok(true)
        }
        Some(crate::commands::INTERNAL_SYSTEMD_DROPIN_WRITE_COMMAND) => {
            let target = command
                .get(1)
                .map(String::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "systemd drop-in command missing target unit".to_string())?;
            let name = command
                .get(2)
                .map(String::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "systemd drop-in command missing name".to_string())?;
            let content = command
                .get(3)
                .map(String::as_str)
                .ok_or_else(|| "systemd drop-in command missing content".to_string())?;
            write_local_systemd_dropin(target, name, content)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

async fn apply_internal_remote_command(
    pool: &SshConnectionPool,
    host_id: &str,
    command: &[String],
) -> Result<bool, String> {
    fn content(command: &[String]) -> Result<String, String> {
        rollback_command_content(command)
    }
    match command.first().map(|value| value.as_str()) {
        Some("__config_write__") | Some("__rollback__") => {
            let content = content(command)?;
            pool.sftp_write(host_id, "~/.openclaw/openclaw.json", &content)
                .await?;
            Ok(true)
        }
        Some(crate::commands::INTERNAL_SETUP_IDENTITY_COMMAND) => {
            let agent_id = command
                .get(1)
                .ok_or_else(|| "setup_identity command missing agent id".to_string())?;
            let name = command
                .get(2)
                .ok_or_else(|| "setup_identity command missing name".to_string())?;
            crate::agent_identity::write_remote_agent_identity(
                pool,
                host_id,
                agent_id,
                name,
                command.get(3).map(String::as_str),
            )
            .await?;
            Ok(true)
        }
        Some(crate::commands::INTERNAL_SYSTEMD_DROPIN_WRITE_COMMAND) => {
            let target = command
                .get(1)
                .map(String::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "systemd drop-in command missing target unit".to_string())?;
            let name = command
                .get(2)
                .map(String::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "systemd drop-in command missing name".to_string())?;
            let content = command
                .get(3)
                .map(String::as_str)
                .ok_or_else(|| "systemd drop-in command missing content".to_string())?;
            write_remote_systemd_dropin(pool, host_id, target, name, content).await?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[tauri::command]
pub async fn apply_queued_commands(
    queue: tauri::State<'_, CommandQueue>,
    cache: tauri::State<'_, CliCache>,
    snapshot_recipe_id: Option<String>,
    run_id: Option<String>,
    snapshot_artifacts: Option<Vec<crate::recipe_store::Artifact>>,
) -> Result<ApplyQueueResult, String> {
    let commands = queue.list();
    if commands.is_empty() {
        return Err("No pending commands to apply".into());
    }

    let queue_handle = queue.inner().clone();
    let cache_handle = cache.inner().clone();

    tauri::async_runtime::spawn_blocking(move || {
        let paths = resolve_paths();
        let total_count = commands.len();

        // Save snapshot before applying (for rollback)
        let config_before = crate::config_io::read_text(&paths.config_path)?;
        // Build a descriptive label from command labels
        let mut summary = commands
            .iter()
            .map(|c| c.label.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        // Truncate to avoid excessively long snapshot IDs/filenames
        if summary.len() > 80 {
            summary.truncate(80);
            summary.push_str("...");
        }
        // Detect if this is a rollback operation
        let is_rollback = commands
            .iter()
            .any(|c| c.command.first().map(|s| s.as_str()) == Some("__rollback__"));
        let source = if is_rollback { "rollback" } else { "clawpal" };
        let can_rollback = !is_rollback;
        let snapshot_recipe_id = snapshot_recipe_id
            .clone()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or(summary);
        let _ = crate::history::add_snapshot(
            &paths.history_dir,
            &paths.metadata_path,
            Some(snapshot_recipe_id),
            source,
            can_rollback,
            &config_before,
            run_id.clone(),
            None,
            snapshot_artifacts.clone().unwrap_or_default(),
        );

        // Execute each command for real
        let mut applied_count = 0;
        for cmd in &commands {
            match apply_internal_local_command(&paths, &cmd.command) {
                Ok(true) => {
                    applied_count += 1;
                    continue;
                }
                Ok(false) => {}
                Err(e) => {
                    let _ = crate::config_io::write_text(&paths.config_path, &config_before);
                    queue_handle.clear();
                    return Ok(ApplyQueueResult {
                        ok: false,
                        applied_count,
                        total_count,
                        error: Some(format!(
                            "Step {} failed ({}): {}",
                            applied_count + 1,
                            cmd.label,
                            e
                        )),
                        rolled_back: true,
                    });
                }
            }
            let result = match run_allowlisted_systemd_local_command(&cmd.command) {
                Ok(Some(output)) => Ok(output),
                Ok(None) => {
                    let args: Vec<&str> = cmd.command.iter().skip(1).map(|s| s.as_str()).collect();
                    run_openclaw(&args)
                }
                Err(error) => Err(error),
            };
            match result {
                Ok(output) if output.exit_code != 0 => {
                    let detail = if !output.stderr.is_empty() {
                        output.stderr.clone()
                    } else {
                        output.stdout.clone()
                    };

                    // Rollback: restore config from snapshot
                    let _ = crate::config_io::write_text(&paths.config_path, &config_before);

                    queue_handle.clear();
                    return Ok(ApplyQueueResult {
                        ok: false,
                        applied_count,
                        total_count,
                        error: Some(format!(
                            "Step {} failed ({}): {}",
                            applied_count + 1,
                            cmd.label,
                            detail
                        )),
                        rolled_back: true,
                    });
                }
                Err(e) => {
                    let _ = crate::config_io::write_text(&paths.config_path, &config_before);
                    queue_handle.clear();
                    return Ok(ApplyQueueResult {
                        ok: false,
                        applied_count,
                        total_count,
                        error: Some(format!(
                            "Step {} failed ({}): {}",
                            applied_count + 1,
                            cmd.label,
                            e
                        )),
                        rolled_back: true,
                    });
                }
                Ok(_) => {
                    applied_count += 1;
                }
            }
        }

        // All succeeded — clear queue, invalidate cache, restart gateway
        queue_handle.clear();
        cache_handle.invalidate_all();

        // Restart gateway (best effort, don't fail the whole apply)
        let gateway_result = run_openclaw(&["gateway", "restart"]);
        if let Err(e) = &gateway_result {
            eprintln!("Warning: gateway restart failed after apply: {e}");
        }

        Ok(ApplyQueueResult {
            ok: true,
            applied_count,
            total_count,
            error: None,
            rolled_back: false,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// RemoteCommandQueues — Task 6: per-host command queues
// ---------------------------------------------------------------------------

pub struct RemoteCommandQueues {
    queues: Mutex<HashMap<String, Vec<PendingCommand>>>,
}

impl RemoteCommandQueues {
    pub fn new() -> Self {
        Self {
            queues: Mutex::new(HashMap::new()),
        }
    }

    pub fn enqueue(&self, host_id: &str, label: String, command: Vec<String>) -> PendingCommand {
        let cmd = PendingCommand {
            id: Uuid::new_v4().to_string(),
            label,
            command,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        self.queues
            .lock()
            .unwrap()
            .entry(host_id.to_string())
            .or_default()
            .push(cmd.clone());
        cmd
    }

    pub fn remove(&self, host_id: &str, id: &str) -> bool {
        let mut queues = self.queues.lock().unwrap();
        if let Some(cmds) = queues.get_mut(host_id) {
            let before = cmds.len();
            cmds.retain(|c| c.id != id);
            return cmds.len() < before;
        }
        false
    }

    pub fn list(&self, host_id: &str) -> Vec<PendingCommand> {
        self.queues
            .lock()
            .unwrap()
            .get(host_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn clear(&self, host_id: &str) {
        self.queues.lock().unwrap().remove(host_id);
    }

    pub fn len(&self, host_id: &str) -> usize {
        self.queues
            .lock()
            .unwrap()
            .get(host_id)
            .map(|v| v.len())
            .unwrap_or(0)
    }
}

impl Default for RemoteCommandQueues {
    fn default() -> Self {
        Self::new()
    }
}

pub fn enqueue_materialized_plan_remote(
    queues: &RemoteCommandQueues,
    host_id: &str,
    plan: &MaterializedExecutionPlan,
) -> Vec<PendingCommand> {
    plan.commands
        .iter()
        .enumerate()
        .map(|(index, command)| {
            let label = format!(
                "[{}] {} ({}/{})",
                plan.execution_kind,
                plan.unit_name,
                index + 1,
                plan.commands.len()
            );
            queues.enqueue(host_id, label, command.clone())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Remote queue management Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn remote_queue_command(
    queues: tauri::State<RemoteCommandQueues>,
    host_id: String,
    label: String,
    command: Vec<String>,
) -> Result<PendingCommand, String> {
    if command.is_empty() {
        return Err("command cannot be empty".into());
    }
    Ok(queues.enqueue(&host_id, label, command))
}

#[tauri::command]
pub fn remote_remove_queued_command(
    queues: tauri::State<RemoteCommandQueues>,
    host_id: String,
    id: String,
) -> Result<bool, String> {
    Ok(queues.remove(&host_id, &id))
}

#[tauri::command]
pub fn remote_list_queued_commands(
    queues: tauri::State<RemoteCommandQueues>,
    host_id: String,
) -> Result<Vec<PendingCommand>, String> {
    Ok(queues.list(&host_id))
}

#[tauri::command]
pub fn remote_discard_queued_commands(
    queues: tauri::State<RemoteCommandQueues>,
    host_id: String,
) -> Result<bool, String> {
    queues.clear(&host_id);
    Ok(true)
}

#[tauri::command]
pub fn remote_queued_commands_count(
    queues: tauri::State<RemoteCommandQueues>,
    host_id: String,
) -> Result<usize, String> {
    Ok(queues.len(&host_id))
}

// ---------------------------------------------------------------------------
// Remote preview — sandbox execution via SSH
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn remote_preview_queued_commands(
    pool: tauri::State<'_, SshConnectionPool>,
    queues: tauri::State<'_, RemoteCommandQueues>,
    host_id: String,
) -> Result<PreviewQueueResult, String> {
    let commands = queues.list(&host_id);
    if commands.is_empty() {
        return Err("No pending commands to preview".into());
    }
    let queue_size = commands.len();

    // Read current config via SSH
    let read_started = Instant::now();
    let config_before = pool
        .sftp_read(&host_id, "~/.openclaw/openclaw.json")
        .await?;
    log_preview_stage(
        "remote",
        Some(&host_id),
        queue_size,
        "readConfig",
        "success",
        read_started.elapsed().as_millis(),
        None,
    );
    let mut preview_config_json = parse_preview_config(&config_before);

    // Set up sandbox on remote: symlink all entries from real .openclaw/ into sandbox,
    // but copy openclaw.json so commands modify the copy, not the original.
    let sandbox_started = Instant::now();
    pool.exec(
        &host_id,
        concat!(
            "rm -rf ~/.clawpal/preview && ",
            "mkdir -p ~/.clawpal/preview/.openclaw && ",
            "for f in ~/.openclaw/*; do ",
            "  name=$(basename \"$f\"); ",
            "  [ \"$name\" = \"openclaw.json\" ] && continue; ",
            "  ln -s \"$f\" ~/.clawpal/preview/.openclaw/\"$name\"; ",
            "done && ",
            "cp ~/.openclaw/openclaw.json ~/.clawpal/preview/.openclaw/openclaw.json",
        ),
    )
    .await?;
    log_preview_stage(
        "remote",
        Some(&host_id),
        queue_size,
        "setupSandbox",
        "success",
        sandbox_started.elapsed().as_millis(),
        None,
    );

    // Execute each command in sandbox with OPENCLAW_HOME override.
    // OPENCLAW_HOME should point to the parent of .openclaw/ (CLI adds .openclaw/ itself).
    let apply_started = Instant::now();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut touched = PreviewTouchedDomains::default();
    let mut preview_dirty = false;
    for cmd in &commands {
        match apply_direct_preview_command(&mut preview_config_json, cmd) {
            Ok(Some(domains)) => {
                touched.merge(domains);
                if let Some(warning) = preview_side_effect_warning(cmd) {
                    warnings.push(warning);
                }
                preview_dirty = true;
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                errors.push(e);
                break;
            }
        }

        if preview_dirty {
            let rendered = render_preview_config(&preview_config_json)?;
            pool.sftp_write(
                &host_id,
                "~/.clawpal/preview/.openclaw/openclaw.json",
                &rendered,
            )
            .await?;
            preview_dirty = false;
        }

        let args: Vec<&str> = cmd.command.iter().skip(1).map(|s| s.as_str()).collect();
        let mut env = HashMap::new();
        env.insert(
            "OPENCLAW_HOME".to_string(),
            "~/.clawpal/preview".to_string(),
        );

        match run_openclaw_remote_with_env(&pool, &host_id, &args, Some(&env)).await {
            Ok(output) if output.exit_code != 0 => {
                let detail = if !output.stderr.is_empty() {
                    output.stderr.clone()
                } else {
                    output.stdout.clone()
                };
                errors.push(format!("{}: {}", cmd.label, detail));
                break;
            }
            Err(e) => {
                errors.push(format!("{}: {}", cmd.label, e));
                break;
            }
            Ok(_) => {
                let latest = pool
                    .sftp_read(&host_id, "~/.clawpal/preview/.openclaw/openclaw.json")
                    .await
                    .unwrap_or_else(|_| config_before.clone());
                preview_config_json = parse_preview_config(&latest);
            }
        }
    }
    let apply_outcome = if errors.is_empty() {
        "success"
    } else {
        "error"
    };
    let apply_detail = errors.first().map(String::as_str);
    log_preview_stage(
        "remote",
        Some(&host_id),
        queue_size,
        "applyPreviewTransforms",
        apply_outcome,
        apply_started.elapsed().as_millis(),
        apply_detail,
    );

    if preview_dirty {
        let rendered = render_preview_config(&preview_config_json)?;
        pool.sftp_write(
            &host_id,
            "~/.clawpal/preview/.openclaw/openclaw.json",
            &rendered,
        )
        .await?;
    }

    if errors.is_empty() {
        let validate_started = Instant::now();
        for target in preview_validation_targets(touched) {
            let mut env = HashMap::new();
            env.insert(
                "OPENCLAW_HOME".to_string(),
                "~/.clawpal/preview".to_string(),
            );
            match run_openclaw_remote_with_env(&pool, &host_id, target.args(), Some(&env)).await {
                Ok(output) if output.exit_code == 0 => {}
                Ok(output) => {
                    let detail = if !output.stderr.is_empty() {
                        output.stderr.clone()
                    } else {
                        output.stdout.clone()
                    };
                    let message =
                        format!("Preview validation ({}) failed: {}", target.label(), detail);
                    errors.push(message.clone());
                    log_preview_stage(
                        "remote",
                        Some(&host_id),
                        queue_size,
                        "cliValidate",
                        "error",
                        validate_started.elapsed().as_millis(),
                        Some(&message),
                    );
                    break;
                }
                Err(err) => {
                    let message =
                        format!("Preview validation ({}) failed: {}", target.label(), err);
                    errors.push(message.clone());
                    log_preview_stage(
                        "remote",
                        Some(&host_id),
                        queue_size,
                        "cliValidate",
                        "error",
                        validate_started.elapsed().as_millis(),
                        Some(&message),
                    );
                    break;
                }
            }
        }
        if errors.is_empty() {
            log_preview_stage(
                "remote",
                Some(&host_id),
                queue_size,
                "cliValidate",
                "success",
                validate_started.elapsed().as_millis(),
                None,
            );
        }
    }

    let readback_started = Instant::now();
    let raw = pool
        .sftp_read(&host_id, "~/.clawpal/preview/.openclaw/openclaw.json")
        .await
        .unwrap_or_else(|_| config_before.clone());
    // Replace sandbox paths with real paths in preview output.
    // The sandbox is at ~/.clawpal/preview, real OPENCLAW_HOME is ~.
    // Resolve ~ on remote to get absolute sandbox prefix.
    let resolved_home = pool
        .exec(&host_id, "echo $HOME")
        .await
        .map(|r| r.stdout.trim().to_string())
        .unwrap_or_default();
    let config_after = if !resolved_home.is_empty() {
        let sandbox_prefix = format!("{}/.clawpal/preview", resolved_home);
        raw.replace(&sandbox_prefix, &resolved_home)
    } else {
        raw
    };
    log_preview_stage(
        "remote",
        Some(&host_id),
        queue_size,
        "readbackConfig",
        "success",
        readback_started.elapsed().as_millis(),
        None,
    );

    let cleanup_started = Instant::now();
    let _ = pool.exec(&host_id, "rm -rf ~/.clawpal/preview").await;
    log_preview_stage(
        "remote",
        Some(&host_id),
        queue_size,
        "cleanupSandbox",
        "success",
        cleanup_started.elapsed().as_millis(),
        None,
    );

    Ok(PreviewQueueResult {
        commands,
        config_before: normalize_config_for_preview(&config_before),
        config_after: normalize_config_for_preview(&config_after),
        warnings,
        errors,
    })
}

// ---------------------------------------------------------------------------
// Remote apply — execute queue for real via SSH, rollback on failure
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn remote_apply_queued_commands(
    pool: tauri::State<'_, SshConnectionPool>,
    queues: tauri::State<'_, RemoteCommandQueues>,
    host_id: String,
    snapshot_recipe_id: Option<String>,
    run_id: Option<String>,
    snapshot_artifacts: Option<Vec<crate::recipe_store::Artifact>>,
) -> Result<ApplyQueueResult, String> {
    let commands = queues.list(&host_id);
    if commands.is_empty() {
        return Err("No pending commands to apply".into());
    }
    let total_count = commands.len();

    // Save snapshot on remote
    let config_before = pool
        .sftp_read(&host_id, "~/.openclaw/openclaw.json")
        .await?;
    let ts = chrono::Utc::now().timestamp();
    let mut summary: String = commands
        .iter()
        .map(|c| c.label.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if summary.len() > 80 {
        summary.truncate(80);
        summary.push_str("...");
    }
    // Sanitize summary for safe filename
    let safe_summary: String = summary
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | ' ' => '-',
            _ => c,
        })
        .collect();
    let is_rollback = commands
        .iter()
        .any(|c| c.command.first().map(|s| s.as_str()) == Some("__rollback__"));
    let source = if is_rollback { "rollback" } else { "clawpal" };
    let snapshot_filename = format!("{ts}-{source}-{safe_summary}.json");
    let snapshot_path = format!("~/.clawpal/snapshots/{snapshot_filename}");
    let _ = pool.exec(&host_id, "mkdir -p ~/.clawpal/snapshots").await;
    let _ = pool
        .sftp_write(&host_id, &snapshot_path, &config_before)
        .await;
    let snapshot_recipe_id = snapshot_recipe_id
        .clone()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or(summary.clone());
    let snapshot_created_at = chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| ts.to_string());
    let _ = crate::commands::config::record_remote_snapshot_metadata(
        &pool,
        &host_id,
        crate::history::SnapshotMeta {
            id: snapshot_filename.clone(),
            recipe_id: Some(snapshot_recipe_id),
            created_at: snapshot_created_at,
            config_path: snapshot_path.clone(),
            source: source.into(),
            can_rollback: !is_rollback,
            run_id: run_id.clone(),
            rollback_of: None,
            artifacts: snapshot_artifacts.clone().unwrap_or_default(),
        },
    )
    .await;

    // Execute each command
    let mut applied_count = 0;
    for cmd in &commands {
        match apply_internal_remote_command(&pool, &host_id, &cmd.command).await {
            Ok(true) => {
                applied_count += 1;
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                let _ = pool
                    .sftp_write(&host_id, "~/.openclaw/openclaw.json", &config_before)
                    .await;
                queues.clear(&host_id);
                return Ok(ApplyQueueResult {
                    ok: false,
                    applied_count,
                    total_count,
                    error: Some(format!(
                        "Step {} failed ({}): {}",
                        applied_count + 1,
                        cmd.label,
                        e
                    )),
                    rolled_back: true,
                });
            }
        }

        let result =
            match run_allowlisted_systemd_remote_command(&pool, &host_id, &cmd.command).await {
                Ok(Some(output)) => Ok(output),
                Ok(None) => {
                    let args: Vec<&str> = cmd.command.iter().skip(1).map(|s| s.as_str()).collect();
                    run_openclaw_remote(&pool, &host_id, &args).await
                }
                Err(error) => Err(error),
            };
        match result {
            Ok(output) if output.exit_code != 0 => {
                let detail = if !output.stderr.is_empty() {
                    output.stderr.clone()
                } else {
                    output.stdout.clone()
                };
                // Rollback
                let _ = pool
                    .sftp_write(&host_id, "~/.openclaw/openclaw.json", &config_before)
                    .await;
                queues.clear(&host_id);
                return Ok(ApplyQueueResult {
                    ok: false,
                    applied_count,
                    total_count,
                    error: Some(format!(
                        "Step {} failed ({}): {}",
                        applied_count + 1,
                        cmd.label,
                        detail
                    )),
                    rolled_back: true,
                });
            }
            Err(e) => {
                let _ = pool
                    .sftp_write(&host_id, "~/.openclaw/openclaw.json", &config_before)
                    .await;
                queues.clear(&host_id);
                return Ok(ApplyQueueResult {
                    ok: false,
                    applied_count,
                    total_count,
                    error: Some(format!(
                        "Step {} failed ({}): {}",
                        applied_count + 1,
                        cmd.label,
                        e
                    )),
                    rolled_back: true,
                });
            }
            Ok(_) => {
                applied_count += 1;
            }
        }
    }

    queues.clear(&host_id);
    let _ = pool.exec_login(&host_id, "openclaw gateway restart").await;

    Ok(ApplyQueueResult {
        ok: true,
        applied_count,
        total_count,
        error: None,
        rolled_back: false,
    })
}

// ---------------------------------------------------------------------------
// Read Cache — invalidated on Apply
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct CliCache {
    cache: Arc<Mutex<HashMap<String, (std::time::Instant, String)>>>,
}

impl CliCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get cached value if still valid.
    pub fn get(&self, key: &str, ttl: Option<std::time::Duration>) -> Option<String> {
        let cache = self.cache.lock().unwrap();
        cache.get(key).and_then(|(ts, val)| {
            if let Some(ttl) = ttl {
                if ts.elapsed() < ttl {
                    Some(val.clone())
                } else {
                    None
                }
            } else {
                Some(val.clone())
            }
        })
    }

    pub fn set(&self, key: String, value: String) {
        self.cache
            .lock()
            .unwrap()
            .insert(key, (std::time::Instant::now(), value));
    }

    /// Invalidate all cache entries (called after Apply).
    pub fn invalidate_all(&self) {
        self.cache.lock().unwrap().clear();
    }
}
