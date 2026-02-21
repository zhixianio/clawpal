use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::models::resolve_paths;
use crate::ssh::SshConnectionPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

pub fn run_openclaw(args: &[&str]) -> Result<CliOutput, String> {
    run_openclaw_with_env(args, None)
}

pub fn run_openclaw_with_env(
    args: &[&str],
    env: Option<&HashMap<String, String>>,
) -> Result<CliOutput, String> {
    let mut cmd = Command::new("openclaw");
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    if let Some(env_vars) = env {
        for (k, v) in env_vars {
            cmd.env(k, v);
        }
    }

    let output = cmd
        .output()
        .map_err(|e| format!("failed to run openclaw: {e}"))?;

    let exit_code = output.status.code().unwrap_or(-1);
    Ok(CliOutput {
        stdout: String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string(),
        stderr: String::from_utf8_lossy(&output.stderr)
            .trim_end()
            .to_string(),
        exit_code,
    })
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
    let mut cmd_str = String::new();

    if let Some(env_vars) = env {
        for (k, v) in env_vars {
            cmd_str.push_str(&format!("{}='{}' ", k, v.replace('\'', "'\\''")));
        }
    }

    cmd_str.push_str("openclaw");
    for arg in args {
        cmd_str.push_str(&format!(" '{}'", arg.replace('\'', "'\\''")));
    }

    let result = pool.exec_login(host_id, &cmd_str).await?;
    Ok(CliOutput {
        stdout: result.stdout,
        stderr: result.stderr,
        exit_code: result.exit_code as i32,
    })
}

pub fn parse_json_output(output: &CliOutput) -> Result<Value, String> {
    if output.exit_code != 0 {
        let details = if !output.stderr.is_empty() {
            &output.stderr
        } else {
            &output.stdout
        };
        return Err(format!(
            "openclaw command failed ({}): {}",
            output.exit_code, details
        ));
    }

    let raw = &output.stdout;
    // CLI may emit non-JSON noise (e.g. Doctor warnings with brackets) before
    // the actual JSON payload. Find the outermost JSON object/array by locating
    // the last `}` or `]` (whichever comes later), then walking backwards to
    // find its matching opener with correct nesting.
    let last_brace = raw.rfind('}');
    let last_bracket = raw.rfind(']');
    let end = match (last_brace, last_bracket) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    let start = match end {
        Some(e) => {
            let closer = raw.as_bytes()[e];
            let opener = if closer == b']' { b'[' } else { b'{' };
            let mut depth = 0i32;
            let mut pos = None;
            for i in (0..=e).rev() {
                let ch = raw.as_bytes()[i];
                if ch == closer { depth += 1; }
                else if ch == opener { depth -= 1; }
                if depth == 0 { pos = Some(i); break; }
            }
            pos
        }
        None => None,
    };
    let start = start.ok_or_else(|| format!("No JSON found in output: {raw}"))?;
    let end = end.unwrap();
    let json_str = &raw[start..=end];
    serde_json::from_str(json_str).map_err(|e| format!("Failed to parse JSON: {e}"))
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
pub fn discard_queued_commands(
    queue: tauri::State<CommandQueue>,
) -> Result<bool, String> {
    queue.clear();
    Ok(true)
}

#[tauri::command]
pub fn queued_commands_count(
    queue: tauri::State<CommandQueue>,
) -> Result<usize, String> {
    Ok(queue.len())
}

// ---------------------------------------------------------------------------
// Preview — sandbox execution with OPENCLAW_HOME
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewQueueResult {
    pub commands: Vec<PendingCommand>,
    pub config_before: String,
    pub config_after: String,
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

        // Read current config
        let config_before = crate::config_io::read_text(&paths.config_path)?;

        // Set up sandbox: symlink all entries from real .openclaw/ into sandbox,
        // but copy openclaw.json so commands modify the copy, not the original.
        // This ensures the CLI can find extensions, plugins, etc. for validation.
        let sandbox_root = paths.clawpal_dir.join("preview");
        let preview_dir = sandbox_root.join(".openclaw");
        // Clean previous sandbox if any
        let _ = std::fs::remove_dir_all(&sandbox_root);
        std::fs::create_dir_all(&preview_dir).map_err(|e| e.to_string())?;

        // Symlink all sibling entries from real .openclaw/
        if let Ok(entries) = std::fs::read_dir(&paths.base_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name == "openclaw.json" { continue; }
                let target = preview_dir.join(&name);
                #[cfg(unix)]
                { let _ = std::os::unix::fs::symlink(entry.path(), &target); }
            }
        }

        // Copy config file (the one we want to modify in-place)
        let preview_config = preview_dir.join("openclaw.json");
        std::fs::copy(&paths.config_path, &preview_config).map_err(|e| e.to_string())?;

        let mut env = HashMap::new();
        env.insert(
            "OPENCLAW_HOME".to_string(),
            sandbox_root.to_string_lossy().to_string(),
        );

        // Execute each command in sandbox
        let mut errors = Vec::new();
        for cmd in &commands {
            if cmd.command.first().map(|s| s.as_str()) == Some("__config_write__") {
                // Internal command: write config content directly
                if let Some(content) = cmd.command.get(1) {
                    if let Err(e) = std::fs::write(&preview_config, content) {
                        errors.push(format!("{}: {}", cmd.label, e));
                    }
                }
                continue;
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
                }
                Err(e) => {
                    errors.push(format!("{}: {}", cmd.label, e));
                    break;
                }
                _ => {}
            }
        }

        // Always read result config from sandbox (commands may have partially succeeded)
        let config_after_raw = crate::config_io::read_text(&preview_config)
            .unwrap_or_else(|_| config_before.clone());

        // Normalize both configs to sorted-key pretty JSON so the diff only
        // shows semantic changes, not key reordering by the CLI.
        fn sort_value(v: &Value) -> Value {
            match v {
                Value::Object(map) => {
                    let sorted: serde_json::Map<String, Value> = map.iter()
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
        let normalize_json = |s: &str| -> String {
            match serde_json::from_str::<Value>(s) {
                Ok(v) => serde_json::to_string_pretty(&sort_value(&v))
                    .unwrap_or_else(|_| s.to_string()),
                Err(_) => s.to_string(),
            }
        };
        let config_before = normalize_json(&config_before);
        let config_after = normalize_json(&config_after_raw);

        // Cleanup sandbox
        let _ = std::fs::remove_dir_all(paths.clawpal_dir.join("preview"));

        Ok(PreviewQueueResult {
            commands,
            config_before,
            config_after,
            errors,
        })
    }).await.map_err(|e| e.to_string())?
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

#[tauri::command]
pub async fn apply_queued_commands(
    queue: tauri::State<'_, CommandQueue>,
    cache: tauri::State<'_, CliCache>,
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
        let summary = commands.iter()
            .map(|c| c.label.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let _ = crate::history::add_snapshot(
            &paths.history_dir,
            &paths.metadata_path,
            Some(summary),
            "clawpal",
            true,
            &config_before,
            None,
        );

        // Execute each command for real
        let mut applied_count = 0;
        for cmd in &commands {
            if cmd.command.first().map(|s| s.as_str()) == Some("__config_write__") {
                // Internal command: write config content directly
                if let Some(content) = cmd.command.get(1) {
                    if let Err(e) = crate::config_io::write_text(&paths.config_path, content) {
                        let _ = crate::config_io::write_text(&paths.config_path, &config_before);
                        queue_handle.clear();
                        return Ok(ApplyQueueResult {
                            ok: false,
                            applied_count,
                            total_count,
                            error: Some(format!("Step {} failed ({}): {}", applied_count + 1, cmd.label, e)),
                            rolled_back: true,
                        });
                    }
                }
                applied_count += 1;
                continue;
            }
            let args: Vec<&str> = cmd.command.iter().skip(1).map(|s| s.as_str()).collect();
            let result = run_openclaw(&args);
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
    }).await.map_err(|e| e.to_string())?
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

    // Read current config via SSH
    let config_before = pool.sftp_read(&host_id, "~/.openclaw/openclaw.json").await?;

    // Set up sandbox on remote
    pool.exec(&host_id, "mkdir -p ~/.clawpal/preview/.openclaw")
        .await?;
    pool.exec(
        &host_id,
        "cp ~/.openclaw/openclaw.json ~/.clawpal/preview/.openclaw/openclaw.json",
    )
    .await?;

    // Execute each command in sandbox with OPENCLAW_HOME override
    let mut errors = Vec::new();
    for cmd in &commands {
        let args: Vec<&str> = cmd.command.iter().skip(1).map(|s| s.as_str()).collect();
        let mut env = HashMap::new();
        env.insert(
            "OPENCLAW_HOME".to_string(),
            "~/.clawpal/preview/.openclaw".to_string(),
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
            _ => {}
        }
    }

    let config_after = if errors.is_empty() {
        pool.sftp_read(&host_id, "~/.clawpal/preview/.openclaw/openclaw.json")
            .await?
    } else {
        config_before.clone()
    };

    let _ = pool.exec(&host_id, "rm -rf ~/.clawpal/preview").await;

    Ok(PreviewQueueResult {
        commands,
        config_before,
        config_after,
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
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
    let snapshot_path = format!("~/.clawpal/snapshots/{ts}-queue-apply.json");
    let _ = pool
        .exec(&host_id, "mkdir -p ~/.clawpal/snapshots")
        .await;
    let _ = pool
        .sftp_write(&host_id, &snapshot_path, &config_before)
        .await;

    // Execute each command
    let mut applied_count = 0;
    for cmd in &commands {
        let args: Vec<&str> = cmd.command.iter().skip(1).map(|s| s.as_str()).collect();
        match run_openclaw_remote(&pool, &host_id, &args).await {
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
    let _ = pool
        .exec_login(&host_id, "openclaw gateway restart")
        .await;

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
