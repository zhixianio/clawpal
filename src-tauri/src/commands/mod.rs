use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use tauri::{Manager, State};

use crate::access_discovery::probe_engine::{build_probe_plan_for_local, run_probe_with_redaction};
use crate::access_discovery::store::AccessDiscoveryStore;
use crate::access_discovery::types::{CapabilityProfile, ExecutionExperience};
use crate::config_io::{ensure_dirs, read_openclaw_config, write_json, write_text};
use crate::doctor::{apply_auto_fixes, run_doctor, DoctorReport};
use crate::history::{add_snapshot, list_snapshots, read_snapshot};
use crate::install::session_store::InstallSessionStore;
use crate::install::types::InstallState;
use crate::models::resolve_paths;
use crate::ssh::{SftpEntry, SshConnectionPool, SshExecResult, SshHostConfig};

pub mod agent;
pub mod backup;
pub mod config;
pub mod cron;
pub mod discover_local;
pub mod discovery;
pub mod doctor;
pub mod gateway;
pub mod logs;
pub mod precheck;
pub mod preferences;
pub mod profiles;
pub mod rescue;
pub mod sessions;
pub mod watchdog;

#[allow(unused_imports)]
pub use agent::*;
#[allow(unused_imports)]
pub use backup::*;
#[allow(unused_imports)]
pub use config::*;
#[allow(unused_imports)]
pub use cron::*;
#[allow(unused_imports)]
pub use discover_local::*;
#[allow(unused_imports)]
pub use discovery::*;
#[allow(unused_imports)]
pub use doctor::*;
#[allow(unused_imports)]
pub use gateway::*;
#[allow(unused_imports)]
pub use logs::*;
#[allow(unused_imports)]
pub use precheck::*;
#[allow(unused_imports)]
pub use preferences::*;
#[allow(unused_imports)]
pub use profiles::*;
#[allow(unused_imports)]
pub use rescue::*;
#[allow(unused_imports)]
pub use sessions::*;
#[allow(unused_imports)]
pub use watchdog::*;

/// Escape a string for safe inclusion in a single-quoted shell argument.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

use crate::recipe::{
    build_candidate_config_from_template, collect_change_paths, format_diff,
    load_recipes_with_fallback, ApplyResult, PreviewResult,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    pub healthy: bool,
    pub config_path: String,
    pub openclaw_dir: String,
    pub clawpal_dir: String,
    pub openclaw_version: String,
    pub active_agents: u32,
    pub snapshots: usize,
    pub channels: ChannelSummary,
    pub models: ModelSummary,
    pub memory: MemorySummary,
    pub sessions: SessionSummary,
    pub openclaw_update: OpenclawUpdateCheck,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenclawUpdateCheck {
    pub installed_version: String,
    pub latest_version: Option<String>,
    pub upgrade_available: bool,
    pub channel: Option<String>,
    pub details: Option<String>,
    pub source: String,
    pub checked_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogProviderCache {
    pub cli_version: String,
    pub updated_at: u64,
    pub providers: Vec<ModelCatalogProvider>,
    pub source: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenclawCommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl From<crate::cli_runner::CliOutput> for OpenclawCommandOutput {
    fn from(value: crate::cli_runner::CliOutput) -> Self {
        Self {
            stdout: value.stdout,
            stderr: value.stderr,
            exit_code: value.exit_code,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RescueBotCommandResult {
    pub command: Vec<String>,
    pub output: OpenclawCommandOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RescueBotManageResult {
    pub action: String,
    pub profile: String,
    pub main_port: u16,
    pub rescue_port: u16,
    pub min_recommended_port: u16,
    pub was_already_configured: bool,
    pub commands: Vec<RescueBotCommandResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RescuePrimaryCheckItem {
    pub id: String,
    pub title: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RescuePrimaryIssue {
    pub id: String,
    pub code: String,
    pub severity: String,
    pub message: String,
    pub auto_fixable: bool,
    pub fix_hint: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RescuePrimaryDiagnosisResult {
    pub status: String,
    pub checked_at: String,
    pub target_profile: String,
    pub rescue_profile: String,
    pub rescue_configured: bool,
    pub rescue_port: Option<u16>,
    pub checks: Vec<RescuePrimaryCheckItem>,
    pub issues: Vec<RescuePrimaryIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RescuePrimaryRepairStep {
    pub id: String,
    pub title: String,
    pub ok: bool,
    pub detail: String,
    pub command: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RescuePrimaryRepairResult {
    pub attempted_at: String,
    pub target_profile: String,
    pub rescue_profile: String,
    pub selected_issue_ids: Vec<String>,
    pub applied_issue_ids: Vec<String>,
    pub skipped_issue_ids: Vec<String>,
    pub failed_issue_ids: Vec<String>,
    pub steps: Vec<RescuePrimaryRepairStep>,
    pub before: RescuePrimaryDiagnosisResult,
    pub after: RescuePrimaryDiagnosisResult,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractModelProfilesResult {
    pub created: usize,
    pub reused: usize,
    pub skipped_invalid: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractModelProfileEntry {
    pub provider: String,
    pub model: String,
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenclawUpdateCache {
    pub checked_at: u64,
    pub latest_version: Option<String>,
    pub channel: Option<String>,
    pub details: Option<String>,
    pub source: String,
    pub installed_version: Option<String>,
    pub ttl_seconds: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSummary {
    pub global_default_model: Option<String>,
    pub agent_overrides: Vec<String>,
    pub channel_overrides: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSummary {
    pub configured_channels: usize,
    pub channel_model_overrides: usize,
    pub channel_examples: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFileSummary {
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySummary {
    pub file_count: usize,
    pub total_bytes: u64,
    pub files: Vec<MemoryFileSummary>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionSummary {
    pub agent: String,
    pub session_files: usize,
    pub archive_files: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFile {
    pub path: String,
    pub relative_path: String,
    pub agent: String,
    pub kind: String,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAnalysis {
    pub agent: String,
    pub session_id: String,
    pub file_path: String,
    pub size_bytes: u64,
    pub message_count: usize,
    pub user_message_count: usize,
    pub assistant_message_count: usize,
    pub last_activity: Option<String>,
    pub age_days: f64,
    pub total_tokens: u64,
    pub model: Option<String>,
    pub category: String,
    pub kind: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionAnalysis {
    pub agent: String,
    pub total_files: usize,
    pub total_size_bytes: u64,
    pub empty_count: usize,
    pub low_value_count: usize,
    pub valuable_count: usize,
    pub sessions: Vec<SessionAnalysis>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub total_session_files: usize,
    pub total_archive_files: usize,
    pub total_bytes: u64,
    pub by_agent: Vec<AgentSessionSummary>,
}

pub type ModelProfile = clawpal_core::profile::ModelProfile;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogModel {
    pub id: String,
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogProvider {
    pub provider: String,
    pub base_url: Option<String>,
    pub models: Vec<ModelCatalogModel>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelNode {
    pub path: String,
    pub channel_type: Option<String>,
    pub mode: Option<String>,
    pub allowlist: Vec<String>,
    pub model: Option<String>,
    pub has_model_field: bool,
    pub display_name: Option<String>,
    pub name_status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscordGuildChannel {
    pub guild_id: String,
    pub guild_name: String,
    pub channel_id: String,
    pub channel_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_agent_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAuthSuggestion {
    pub auth_ref: Option<String>,
    pub has_key: bool,
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelBinding {
    pub scope: String,
    pub scope_id: String,
    pub model_profile_id: Option<String>,
    pub model_value: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub id: String,
    pub recipe_id: Option<String>,
    pub created_at: String,
    pub source: String,
    pub can_rollback: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback_of: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub items: Vec<HistoryItem>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixResult {
    pub ok: bool,
    pub applied: Vec<String>,
    pub remaining_issues: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOverview {
    pub id: String,
    pub name: Option<String>,
    pub emoji: Option<String>,
    pub model: Option<String>,
    pub channels: Vec<String>,
    pub online: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusLight {
    pub healthy: bool,
    pub active_agents: u32,
    pub global_default_model: Option<String>,
    pub fallback_models: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusExtra {
    pub openclaw_version: Option<String>,
    pub duplicate_installs: Vec<String>,
}

/// Clear cached openclaw version — call after upgrade so status shows new version.
pub fn clear_openclaw_version_cache() {
    *OPENCLAW_VERSION_CACHE.lock().unwrap() = None;
}

static OPENCLAW_VERSION_CACHE: std::sync::Mutex<Option<Option<String>>> =
    std::sync::Mutex::new(None);

/// Fast status: reads config + quick TCP probe of gateway port.
/// Local status extra: openclaw version (cached) + no duplicate detection needed locally.
fn local_health_instance() -> clawpal_core::instance::Instance {
    clawpal_core::instance::Instance {
        id: "local".to_string(),
        instance_type: clawpal_core::instance::InstanceType::Local,
        label: "Local".to_string(),
        openclaw_home: crate::cli_runner::get_active_openclaw_home_override(),
        clawpal_data_dir: crate::cli_runner::get_active_clawpal_data_override(),
        ssh_host_config: None,
    }
}

/// Returns cached catalog instantly without calling CLI. Returns empty if no cache.
/// Refresh catalog from CLI and update cache. Returns the fresh catalog.
/// Read Discord guild/channels from persistent cache. Fast, no subprocess.
/// Resolve Discord guild/channel names via openclaw CLI and persist to cache.
#[tauri::command]
pub fn update_channel_config(
    path: String,
    channel_type: Option<String>,
    mode: Option<String>,
    allowlist: Vec<String>,
    model: Option<String>,
) -> Result<bool, String> {
    if path.trim().is_empty() {
        return Err("channel path is required".into());
    }
    let paths = resolve_paths();
    let mut cfg = read_openclaw_config(&paths)?;
    let current = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    set_nested_value(
        &mut cfg,
        &format!("{path}.type"),
        channel_type.map(Value::String),
    )?;
    set_nested_value(&mut cfg, &format!("{path}.mode"), mode.map(Value::String))?;
    let allowlist_values = allowlist.into_iter().map(Value::String).collect::<Vec<_>>();
    set_nested_value(
        &mut cfg,
        &format!("{path}.allowlist"),
        Some(Value::Array(allowlist_values)),
    )?;
    set_nested_value(&mut cfg, &format!("{path}.model"), model.map(Value::String))?;
    write_config_with_snapshot(&paths, &current, &cfg, "update-channel")?;
    Ok(true)
}

/// List current channel→agent bindings from config.
#[tauri::command]
pub fn delete_channel_node(path: String) -> Result<bool, String> {
    if path.trim().is_empty() {
        return Err("channel path is required".into());
    }
    let paths = resolve_paths();
    let mut cfg = read_openclaw_config(&paths)?;
    let current = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    let before = cfg.to_string();
    set_nested_value(&mut cfg, &path, None)?;
    if cfg.to_string() == before {
        return Ok(false);
    }
    write_config_with_snapshot(&paths, &current, &cfg, "delete-channel")?;
    Ok(true)
}

#[tauri::command]
pub fn set_global_model(model_value: Option<String>) -> Result<bool, String> {
    let paths = resolve_paths();
    let mut cfg = read_openclaw_config(&paths)?;
    let current = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    let model = model_value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    // If existing model is an object (has fallbacks etc.), only update "primary" inside it
    if let Some(existing) = cfg.pointer_mut("/agents/defaults/model") {
        if let Some(model_obj) = existing.as_object_mut() {
            let sync_model_value = match model.clone() {
                Some(v) => {
                    model_obj.insert("primary".into(), Value::String(v.clone()));
                    Some(v)
                }
                None => {
                    model_obj.remove("primary");
                    None
                }
            };
            write_config_with_snapshot(&paths, &current, &cfg, "set-global-model")?;
            maybe_sync_main_auth_for_model_value(&paths, sync_model_value)?;
            return Ok(true);
        }
    }
    // Fallback: plain string or missing — set the whole value
    set_nested_value(&mut cfg, "agents.defaults.model", model.map(Value::String))?;
    write_config_with_snapshot(&paths, &current, &cfg, "set-global-model")?;
    let model_to_sync = cfg
        .pointer("/agents/defaults/model")
        .and_then(read_model_value);
    maybe_sync_main_auth_for_model_value(&paths, model_to_sync)?;
    Ok(true)
}

#[tauri::command]
pub fn set_agent_model(agent_id: String, model_value: Option<String>) -> Result<bool, String> {
    if agent_id.trim().is_empty() {
        return Err("agent id is required".into());
    }
    let paths = resolve_paths();
    let mut cfg = read_openclaw_config(&paths)?;
    let current = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    let value = model_value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    set_agent_model_value(&mut cfg, &agent_id, value)?;
    write_config_with_snapshot(&paths, &current, &cfg, "set-agent-model")?;
    Ok(true)
}

#[tauri::command]
pub fn set_channel_model(path: String, model_value: Option<String>) -> Result<bool, String> {
    if path.trim().is_empty() {
        return Err("channel path is required".into());
    }
    let paths = resolve_paths();
    let mut cfg = read_openclaw_config(&paths)?;
    let current = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    let value = model_value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    set_nested_value(&mut cfg, &format!("{path}.model"), value.map(Value::String))?;
    write_config_with_snapshot(&paths, &current, &cfg, "set-channel-model")?;
    Ok(true)
}

#[tauri::command]
pub fn list_model_bindings() -> Result<Vec<ModelBinding>, String> {
    let paths = resolve_paths();
    let cfg = read_openclaw_config(&paths)?;
    let profiles = load_model_profiles(&paths);
    Ok(collect_model_bindings(&cfg, &profiles))
}

fn local_cli_cache_key(suffix: &str) -> String {
    let paths = resolve_paths();
    format!("local:{}:{}", paths.openclaw_dir.to_string_lossy(), suffix)
}

/// Check if an agent has active sessions by examining sessions/sessions.json.
/// Returns true if the file exists and is larger than 2 bytes (i.e. not just "{}").
fn agent_has_sessions(base_dir: &std::path::Path, agent_id: &str) -> bool {
    let sessions_file = base_dir
        .join("agents")
        .join(agent_id)
        .join("sessions")
        .join("sessions.json");
    match std::fs::metadata(&sessions_file) {
        Ok(m) => m.len() > 2, // "{}" is 2 bytes = empty
        Err(_) => false,
    }
}

/// Parse the JSON output of `openclaw agents list --json` into Vec<AgentOverview>.
/// `online_set`: if Some, use it to determine online status; if None, check local sessions.
fn parse_agents_cli_output(
    json: &Value,
    online_set: Option<&std::collections::HashSet<String>>,
) -> Result<Vec<AgentOverview>, String> {
    let arr = json
        .as_array()
        .ok_or("agents list output is not an array")?;
    let paths = if online_set.is_none() {
        Some(resolve_paths())
    } else {
        None
    };
    let mut agents = Vec::new();
    for entry in arr {
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("main")
            .to_string();
        let name = entry
            .get("identityName")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let emoji = entry
            .get("identityEmoji")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let model = entry
            .get("model")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let workspace = entry
            .get("workspace")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let online = match online_set {
            Some(set) => set.contains(&id),
            None => agent_has_sessions(paths.as_ref().unwrap().base_dir.as_path(), &id),
        };
        agents.push(AgentOverview {
            id,
            name,
            emoji,
            model,
            channels: Vec::new(),
            online,
            workspace,
        });
    }
    if agents.is_empty() {
        agents.push(AgentOverview {
            id: "main".into(),
            name: None,
            emoji: None,
            model: None,
            channels: Vec::new(),
            online: false,
            workspace: None,
        });
    }
    Ok(agents)
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        if let Some(home) = std::env::var("HOME").ok() {
            return format!("{}{}", home, &path[1..]);
        }
    }
    path.to_string()
}

fn analyze_sessions_sync() -> Result<Vec<AgentSessionAnalysis>, String> {
    let paths = resolve_paths();
    let agents_root = paths.base_dir.join("agents");
    if !agents_root.exists() {
        return Ok(Vec::new());
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as f64;

    let mut results: Vec<AgentSessionAnalysis> = Vec::new();
    let entries = fs::read_dir(&agents_root).map_err(|e| e.to_string())?;

    for entry in entries.flatten() {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }
        let agent = entry.file_name().to_string_lossy().to_string();

        // Load sessions.json metadata for this agent
        let sessions_json_path = entry_path.join("sessions").join("sessions.json");
        let sessions_meta: HashMap<String, Value> = if sessions_json_path.exists() {
            let text = fs::read_to_string(&sessions_json_path).unwrap_or_default();
            serde_json::from_str(&text).unwrap_or_default()
        } else {
            HashMap::new()
        };

        // Build sessionId -> metadata lookup
        let mut meta_by_id: HashMap<String, &Value> = HashMap::new();
        for (_key, val) in &sessions_meta {
            if let Some(sid) = val.get("sessionId").and_then(Value::as_str) {
                meta_by_id.insert(sid.to_string(), val);
            }
        }

        let mut agent_sessions: Vec<SessionAnalysis> = Vec::new();

        for (kind_name, dir_name) in [("sessions", "sessions"), ("archive", "sessions_archive")] {
            let dir = entry_path.join(dir_name);
            if !dir.exists() {
                continue;
            }
            let files = match fs::read_dir(&dir) {
                Ok(f) => f,
                Err(_) => continue,
            };
            for file_entry in files.flatten() {
                let file_path = file_entry.path();
                let fname = file_entry.file_name().to_string_lossy().to_string();
                if !fname.ends_with(".jsonl") {
                    continue;
                }

                let metadata = match file_entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let size_bytes = metadata.len();

                // Extract session ID from filename (e.g. "abc123.jsonl" or "abc123-topic-456.jsonl")
                let session_id = fname.trim_end_matches(".jsonl").to_string();

                // Parse JSONL to count messages
                let mut message_count = 0usize;
                let mut user_message_count = 0usize;
                let mut assistant_message_count = 0usize;
                let mut last_activity: Option<String> = None;

                if let Ok(file) = fs::File::open(&file_path) {
                    let reader = BufReader::new(file);
                    for line in reader.lines() {
                        let line = match line {
                            Ok(l) => l,
                            Err(_) => continue,
                        };
                        if line.trim().is_empty() {
                            continue;
                        }
                        let obj: Value = match serde_json::from_str(&line) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        if obj.get("type").and_then(Value::as_str) == Some("message") {
                            message_count += 1;
                            if let Some(ts) = obj.get("timestamp").and_then(Value::as_str) {
                                last_activity = Some(ts.to_string());
                            }
                            let role = obj.pointer("/message/role").and_then(Value::as_str);
                            match role {
                                Some("user") => user_message_count += 1,
                                Some("assistant") => assistant_message_count += 1,
                                _ => {}
                            }
                        }
                    }
                }

                // Look up metadata from sessions.json
                // For topic files like "abc-topic-123", try the base session ID "abc"
                let base_id = if session_id.contains("-topic-") {
                    session_id.split("-topic-").next().unwrap_or(&session_id)
                } else {
                    &session_id
                };
                let meta = meta_by_id.get(base_id);

                let total_tokens = meta
                    .and_then(|m| m.get("totalTokens"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let model = meta
                    .and_then(|m| m.get("model"))
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
                let updated_at = meta
                    .and_then(|m| m.get("updatedAt"))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);

                let age_days = if updated_at > 0.0 {
                    (now - updated_at) / (1000.0 * 60.0 * 60.0 * 24.0)
                } else {
                    // Fall back to file modification time
                    metadata
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| (now - d.as_millis() as f64) / (1000.0 * 60.0 * 60.0 * 24.0))
                        .unwrap_or(0.0)
                };

                // Classify
                let category = if size_bytes < 500 || message_count == 0 {
                    "empty"
                } else if user_message_count <= 1 && age_days > 7.0 {
                    "low_value"
                } else {
                    "valuable"
                };

                agent_sessions.push(SessionAnalysis {
                    agent: agent.clone(),
                    session_id,
                    file_path: file_path.to_string_lossy().to_string(),
                    size_bytes,
                    message_count,
                    user_message_count,
                    assistant_message_count,
                    last_activity,
                    age_days,
                    total_tokens,
                    model,
                    category: category.to_string(),
                    kind: kind_name.to_string(),
                });
            }
        }

        // Sort: empty first, then low_value, then valuable; within each by age descending
        agent_sessions.sort_by(|a, b| {
            let cat_order = |c: &str| match c {
                "empty" => 0,
                "low_value" => 1,
                _ => 2,
            };
            cat_order(&a.category).cmp(&cat_order(&b.category)).then(
                b.age_days
                    .partial_cmp(&a.age_days)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });

        let total_files = agent_sessions.len();
        let total_size_bytes = agent_sessions.iter().map(|s| s.size_bytes).sum();
        let empty_count = agent_sessions
            .iter()
            .filter(|s| s.category == "empty")
            .count();
        let low_value_count = agent_sessions
            .iter()
            .filter(|s| s.category == "low_value")
            .count();
        let valuable_count = agent_sessions
            .iter()
            .filter(|s| s.category == "valuable")
            .count();

        if total_files > 0 {
            results.push(AgentSessionAnalysis {
                agent,
                total_files,
                total_size_bytes,
                empty_count,
                low_value_count,
                valuable_count,
                sessions: agent_sessions,
            });
        }
    }

    results.sort_by(|a, b| b.total_size_bytes.cmp(&a.total_size_bytes));
    Ok(results)
}

fn delete_sessions_by_ids_sync(agent_id: &str, session_ids: &[String]) -> Result<usize, String> {
    if agent_id.trim().is_empty() {
        return Err("agent id is required".into());
    }
    if agent_id.contains("..") || agent_id.contains('/') || agent_id.contains('\\') {
        return Err("invalid agent id".into());
    }
    let paths = resolve_paths();
    let agent_dir = paths.base_dir.join("agents").join(agent_id);

    let mut deleted = 0usize;

    // Search in both sessions and sessions_archive
    let dirs = ["sessions", "sessions_archive"];

    for sid in session_ids {
        if sid.contains("..") || sid.contains('/') || sid.contains('\\') {
            continue;
        }
        for dir_name in &dirs {
            let dir = agent_dir.join(dir_name);
            if !dir.exists() {
                continue;
            }
            let jsonl_path = dir.join(format!("{}.jsonl", sid));
            if jsonl_path.exists() {
                if fs::remove_file(&jsonl_path).is_ok() {
                    deleted += 1;
                }
            }
            // Also clean up related files (topic files, .lock, .deleted.*)
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let fname = entry.file_name().to_string_lossy().to_string();
                    if fname.starts_with(sid.as_str()) && fname != format!("{}.jsonl", sid) {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
    }

    // Remove entries from sessions.json (in sessions dir)
    let sessions_json_path = agent_dir.join("sessions").join("sessions.json");
    if sessions_json_path.exists() {
        if let Ok(text) = fs::read_to_string(&sessions_json_path) {
            if let Ok(mut data) = serde_json::from_str::<serde_json::Map<String, Value>>(&text) {
                let id_set: HashSet<&str> = session_ids.iter().map(String::as_str).collect();
                data.retain(|_key, val| {
                    let sid = val.get("sessionId").and_then(Value::as_str).unwrap_or("");
                    !id_set.contains(sid)
                });
                let _ = fs::write(
                    &sessions_json_path,
                    serde_json::to_string(&data).unwrap_or_default(),
                );
            }
        }
    }

    Ok(deleted)
}

fn preview_session_sync(agent_id: &str, session_id: &str) -> Result<Vec<Value>, String> {
    if agent_id.contains("..") || agent_id.contains('/') || agent_id.contains('\\') {
        return Err("invalid agent id".into());
    }
    if session_id.contains("..") || session_id.contains('/') || session_id.contains('\\') {
        return Err("invalid session id".into());
    }
    let paths = resolve_paths();
    let agent_dir = paths.base_dir.join("agents").join(agent_id);
    let jsonl_name = format!("{}.jsonl", session_id);

    // Search in both sessions and sessions_archive
    let file_path = ["sessions", "sessions_archive"]
        .iter()
        .map(|dir| agent_dir.join(dir).join(&jsonl_name))
        .find(|p| p.exists());

    let file_path = match file_path {
        Some(p) => p,
        None => return Ok(Vec::new()),
    };

    let file = fs::File::open(&file_path).map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);
    let mut messages: Vec<Value> = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let obj: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if obj.get("type").and_then(Value::as_str) == Some("message") {
            let role = obj
                .pointer("/message/role")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let content = obj
                .pointer("/message/content")
                .map(|c| {
                    if let Some(arr) = c.as_array() {
                        arr.iter()
                            .filter_map(|item| item.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else if let Some(s) = c.as_str() {
                        s.to_string()
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default();
            messages.push(serde_json::json!({
                "role": role,
                "content": content,
            }));
        }
    }

    Ok(messages)
}

#[tauri::command]
pub fn list_recipes(source: Option<String>) -> Result<Vec<crate::recipe::Recipe>, String> {
    let paths = resolve_paths();
    let default_path = paths.clawpal_dir.join("recipes").join("recipes.json");
    Ok(load_recipes_with_fallback(source, &default_path))
}

#[tauri::command]
pub async fn manage_rescue_bot(
    action: String,
    profile: Option<String>,
    rescue_port: Option<u16>,
) -> Result<RescueBotManageResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let action = RescueBotAction::parse(&action)?;
        let profile = profile
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .unwrap_or("rescue")
            .to_string();

        let main_port = read_openclaw_config(&resolve_paths())
            .map(|cfg| clawpal_core::doctor::resolve_gateway_port_from_config(&cfg))
            .unwrap_or(18789);
        let (already_configured, existing_port) = resolve_local_rescue_profile_state(&profile)?;
        let should_configure = !already_configured || action == RescueBotAction::Set;
        let rescue_port = if should_configure {
            rescue_port.unwrap_or_else(|| clawpal_core::doctor::suggest_rescue_port(main_port))
        } else {
            existing_port
                .or(rescue_port)
                .unwrap_or_else(|| clawpal_core::doctor::suggest_rescue_port(main_port))
        };
        let min_recommended_port = main_port.saturating_add(20);

        if should_configure && matches!(action, RescueBotAction::Set | RescueBotAction::Activate) {
            clawpal_core::doctor::ensure_rescue_port_spacing(main_port, rescue_port)?;
        }

        if action == RescueBotAction::Status && !already_configured {
            return Ok(RescueBotManageResult {
                action: action.as_str().into(),
                profile,
                main_port,
                rescue_port,
                min_recommended_port,
                was_already_configured: false,
                commands: Vec::new(),
            });
        }

        let plan = build_rescue_bot_command_plan(action, &profile, rescue_port, should_configure);
        let mut commands = Vec::new();

        for command in plan {
            let result = run_local_rescue_bot_command(command)?;
            if result.output.exit_code != 0 {
                if action == RescueBotAction::Status {
                    commands.push(result);
                    break;
                }
                if is_rescue_cleanup_noop(action, &result.command, &result.output) {
                    commands.push(result);
                    continue;
                }
                if action == RescueBotAction::Activate
                    && is_gateway_restart_command(&result.command)
                    && is_gateway_restart_timeout(&result.output)
                {
                    commands.push(result);
                    run_local_gateway_restart_fallback(&profile, &mut commands)?;
                    continue;
                }
                return Err(command_failure_message(&result.command, &result.output));
            }
            commands.push(result);
        }

        Ok(RescueBotManageResult {
            action: action.as_str().into(),
            profile,
            main_port,
            rescue_port,
            min_recommended_port,
            was_already_configured: already_configured,
            commands,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn diagnose_primary_via_rescue(
    target_profile: Option<String>,
    rescue_profile: Option<String>,
) -> Result<RescuePrimaryDiagnosisResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let target_profile = normalize_profile_name(target_profile.as_deref(), "primary");
        let rescue_profile = normalize_profile_name(rescue_profile.as_deref(), "rescue");
        diagnose_primary_via_rescue_local(&target_profile, &rescue_profile)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn repair_primary_via_rescue(
    target_profile: Option<String>,
    rescue_profile: Option<String>,
    issue_ids: Option<Vec<String>>,
) -> Result<RescuePrimaryRepairResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let target_profile = normalize_profile_name(target_profile.as_deref(), "primary");
        let rescue_profile = normalize_profile_name(rescue_profile.as_deref(), "rescue");
        repair_primary_via_rescue_local(
            &target_profile,
            &rescue_profile,
            issue_ids.unwrap_or_default(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

fn collect_model_summary(cfg: &Value) -> ModelSummary {
    let global_default_model = cfg
        .pointer("/agents/defaults/model")
        .and_then(|value| read_model_value(value))
        .or_else(|| {
            cfg.pointer("/agents/default/model")
                .and_then(|value| read_model_value(value))
        });

    let mut agent_overrides = Vec::new();
    if let Some(agents) = cfg.pointer("/agents/list").and_then(Value::as_array) {
        for agent in agents {
            if let Some(model_value) = agent.get("model").and_then(read_model_value) {
                let should_emit = global_default_model
                    .as_ref()
                    .map(|global| global != &model_value)
                    .unwrap_or(true);
                if should_emit {
                    let id = agent.get("id").and_then(Value::as_str).unwrap_or("agent");
                    agent_overrides.push(format!("{id} => {model_value}"));
                }
            }
        }
    }
    ModelSummary {
        global_default_model,
        agent_overrides,
        channel_overrides: collect_channel_model_overrides(cfg),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RescueBotAction {
    Set,
    Activate,
    Status,
    Deactivate,
    Unset,
}

impl RescueBotAction {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "set" | "configure" => Ok(Self::Set),
            "activate" | "start" => Ok(Self::Activate),
            "status" => Ok(Self::Status),
            "deactivate" | "stop" => Ok(Self::Deactivate),
            "unset" | "remove" | "delete" => Ok(Self::Unset),
            _ => Err("action must be one of: set, activate, status, deactivate, unset".into()),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Set => "set",
            Self::Activate => "activate",
            Self::Status => "status",
            Self::Deactivate => "deactivate",
            Self::Unset => "unset",
        }
    }
}

fn normalize_profile_name(raw: Option<&str>, fallback: &str) -> String {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn build_profile_command(profile: &str, args: &[&str]) -> Vec<String> {
    let mut command = vec!["--profile".to_string(), profile.to_string()];
    command.extend(args.iter().map(|item| (*item).to_string()));
    command
}

fn command_detail(output: &OpenclawCommandOutput) -> String {
    clawpal_core::doctor::command_output_detail(&output.stderr, &output.stdout)
}

fn gateway_output_ok(output: &OpenclawCommandOutput) -> bool {
    clawpal_core::doctor::gateway_output_ok(output.exit_code, &output.stdout, &output.stderr)
}

fn gateway_output_detail(output: &OpenclawCommandOutput) -> String {
    clawpal_core::doctor::gateway_output_detail(output.exit_code, &output.stdout, &output.stderr)
        .unwrap_or_else(|| command_detail(output))
}

fn build_rescue_primary_diagnosis(
    target_profile: &str,
    rescue_profile: &str,
    rescue_configured: bool,
    rescue_port: Option<u16>,
    rescue_gateway_status: Option<&OpenclawCommandOutput>,
    primary_doctor_output: &OpenclawCommandOutput,
    primary_gateway_status: &OpenclawCommandOutput,
) -> RescuePrimaryDiagnosisResult {
    let mut checks = Vec::new();
    let mut issues: Vec<clawpal_core::doctor::DoctorIssue> = Vec::new();

    checks.push(RescuePrimaryCheckItem {
        id: "rescue.profile.configured".into(),
        title: "Rescue profile configured".into(),
        ok: rescue_configured,
        detail: if rescue_configured {
            rescue_port
                .map(|port| format!("profile={rescue_profile}, port={port}"))
                .unwrap_or_else(|| format!("profile={rescue_profile}, port unknown"))
        } else {
            format!("profile={rescue_profile} not configured")
        },
    });

    if !rescue_configured {
        issues.push(clawpal_core::doctor::DoctorIssue {
            id: "rescue.profile.missing".into(),
            code: "rescue.profile.missing".into(),
            severity: "error".into(),
            message: format!("Rescue profile \"{rescue_profile}\" is not configured"),
            auto_fixable: false,
            fix_hint: Some("Activate Rescue Bot first".into()),
            source: "rescue".into(),
        });
    }

    if let Some(output) = rescue_gateway_status {
        let ok = gateway_output_ok(output);
        checks.push(RescuePrimaryCheckItem {
            id: "rescue.gateway.status".into(),
            title: "Rescue gateway status".into(),
            ok,
            detail: gateway_output_detail(output),
        });
        if !ok {
            issues.push(clawpal_core::doctor::DoctorIssue {
                id: "rescue.gateway.unhealthy".into(),
                code: "rescue.gateway.unhealthy".into(),
                severity: "warn".into(),
                message: "Rescue gateway is not healthy".into(),
                auto_fixable: false,
                fix_hint: Some("Inspect rescue gateway logs before using failover".into()),
                source: "rescue".into(),
            });
        }
    }

    let doctor_report = clawpal_core::doctor::parse_json_loose(&primary_doctor_output.stdout)
        .or_else(|| clawpal_core::doctor::parse_json_loose(&primary_doctor_output.stderr));
    let doctor_issues = doctor_report
        .as_ref()
        .map(|report| clawpal_core::doctor::parse_doctor_issues(report, "primary"))
        .unwrap_or_default();
    let doctor_issue_count = doctor_issues.len();
    let doctor_score = doctor_report
        .as_ref()
        .and_then(|report| report.get("score"))
        .and_then(Value::as_i64);
    let doctor_ok_from_report = doctor_report
        .as_ref()
        .and_then(|report| report.get("ok"))
        .and_then(Value::as_bool)
        .unwrap_or(primary_doctor_output.exit_code == 0);
    let doctor_has_error = doctor_issues.iter().any(|issue| issue.severity == "error");
    let doctor_check_ok = doctor_ok_from_report && !doctor_has_error;

    let doctor_detail = if let Some(score) = doctor_score {
        format!("score={score}, issues={doctor_issue_count}")
    } else {
        command_detail(primary_doctor_output)
    };
    checks.push(RescuePrimaryCheckItem {
        id: "primary.doctor".into(),
        title: "Primary doctor report".into(),
        ok: doctor_check_ok,
        detail: doctor_detail,
    });

    if doctor_report.is_none() && primary_doctor_output.exit_code != 0 {
        issues.push(clawpal_core::doctor::DoctorIssue {
            id: "primary.doctor.failed".into(),
            code: "primary.doctor.failed".into(),
            severity: "error".into(),
            message: "Primary doctor command failed".into(),
            auto_fixable: false,
            fix_hint: Some(
                "Review doctor output in this check and open gateway logs for details".into(),
            ),
            source: "primary".into(),
        });
    }
    issues.extend(doctor_issues);

    let primary_gateway_ok = gateway_output_ok(primary_gateway_status);
    checks.push(RescuePrimaryCheckItem {
        id: "primary.gateway.status".into(),
        title: "Primary gateway status".into(),
        ok: primary_gateway_ok,
        detail: gateway_output_detail(primary_gateway_status),
    });
    if !primary_gateway_ok {
        issues.push(clawpal_core::doctor::DoctorIssue {
            id: "primary.gateway.unhealthy".into(),
            code: "primary.gateway.unhealthy".into(),
            severity: "error".into(),
            message: "Primary gateway is not healthy".into(),
            auto_fixable: false,
            fix_hint: Some("Inspect gateway logs and restart primary gateway".into()),
            source: "primary".into(),
        });
    }

    clawpal_core::doctor::dedupe_doctor_issues(&mut issues);
    let status = clawpal_core::doctor::classify_doctor_issue_status(&issues);
    let issues: Vec<RescuePrimaryIssue> = issues
        .into_iter()
        .map(|issue| RescuePrimaryIssue {
            id: issue.id,
            code: issue.code,
            severity: issue.severity,
            message: issue.message,
            auto_fixable: issue.auto_fixable,
            fix_hint: issue.fix_hint,
            source: issue.source,
        })
        .collect();

    RescuePrimaryDiagnosisResult {
        status,
        checked_at: format_timestamp_from_unix(unix_timestamp_secs()),
        target_profile: target_profile.to_string(),
        rescue_profile: rescue_profile.to_string(),
        rescue_configured,
        rescue_port,
        checks,
        issues,
    }
}

fn diagnose_primary_via_rescue_local(
    target_profile: &str,
    rescue_profile: &str,
) -> Result<RescuePrimaryDiagnosisResult, String> {
    let (rescue_configured, rescue_port) = resolve_local_rescue_profile_state(rescue_profile)?;
    let rescue_gateway_status = if rescue_configured {
        let command = build_profile_command(
            rescue_profile,
            &["gateway", "status", "--no-probe", "--json"],
        );
        Some(run_openclaw_dynamic(&command)?)
    } else {
        None
    };
    let primary_doctor_output = run_local_primary_doctor_with_fallback(target_profile)?;
    let primary_gateway_command = build_profile_command(
        target_profile,
        &["gateway", "status", "--no-probe", "--json"],
    );
    let primary_gateway_output = run_openclaw_dynamic(&primary_gateway_command)?;

    Ok(build_rescue_primary_diagnosis(
        target_profile,
        rescue_profile,
        rescue_configured,
        rescue_port,
        rescue_gateway_status.as_ref(),
        &primary_doctor_output,
        &primary_gateway_output,
    ))
}

async fn diagnose_primary_via_rescue_remote(
    pool: &SshConnectionPool,
    host_id: &str,
    target_profile: &str,
    rescue_profile: &str,
) -> Result<RescuePrimaryDiagnosisResult, String> {
    let (rescue_configured, rescue_port) =
        resolve_remote_rescue_profile_state(pool, host_id, rescue_profile).await?;
    let rescue_gateway_status = if rescue_configured {
        let command = build_profile_command(
            rescue_profile,
            &["gateway", "status", "--no-probe", "--json"],
        );
        Some(run_remote_openclaw_dynamic(pool, host_id, command).await?)
    } else {
        None
    };
    let primary_doctor_output =
        run_remote_primary_doctor_with_fallback(pool, host_id, target_profile).await?;
    let primary_gateway_command = build_profile_command(
        target_profile,
        &["gateway", "status", "--no-probe", "--json"],
    );
    let primary_gateway_output =
        run_remote_openclaw_dynamic(pool, host_id, primary_gateway_command).await?;

    Ok(build_rescue_primary_diagnosis(
        target_profile,
        rescue_profile,
        rescue_configured,
        rescue_port,
        rescue_gateway_status.as_ref(),
        &primary_doctor_output,
        &primary_gateway_output,
    ))
}

fn collect_safe_primary_issue_ids(
    diagnosis: &RescuePrimaryDiagnosisResult,
    requested_ids: &[String],
) -> (Vec<String>, Vec<String>) {
    let issues: Vec<clawpal_core::doctor::DoctorIssue> = diagnosis
        .issues
        .iter()
        .map(|issue| clawpal_core::doctor::DoctorIssue {
            id: issue.id.clone(),
            code: issue.code.clone(),
            severity: issue.severity.clone(),
            message: issue.message.clone(),
            auto_fixable: issue.auto_fixable,
            fix_hint: issue.fix_hint.clone(),
            source: issue.source.clone(),
        })
        .collect();
    clawpal_core::doctor::collect_safe_primary_issue_ids(&issues, requested_ids)
}

fn build_primary_issue_fix_command(
    target_profile: &str,
    issue_id: &str,
) -> Option<(String, Vec<String>)> {
    let (title, tail) = clawpal_core::doctor::build_primary_issue_fix_tail(issue_id)?;
    let tail_refs: Vec<&str> = tail.iter().map(String::as_str).collect();
    Some((title, build_profile_command(target_profile, &tail_refs)))
}

fn build_step_detail(command: &[String], output: &OpenclawCommandOutput) -> String {
    if output.exit_code == 0 {
        return command_detail(output);
    }
    command_failure_message(command, output)
}

fn run_local_profile_restart_with_fallback(
    profile: &str,
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<bool, String> {
    let restart_command = build_profile_command(profile, &["gateway", "restart"]);
    let restart_output = run_openclaw_dynamic(&restart_command)?;
    let restart_ok = restart_output.exit_code == 0;
    steps.push(RescuePrimaryRepairStep {
        id: "primary.gateway.restart".into(),
        title: "Restart primary gateway".into(),
        ok: restart_ok,
        detail: build_step_detail(&restart_command, &restart_output),
        command: Some(restart_command.clone()),
    });
    if restart_ok {
        return Ok(true);
    }

    if !is_gateway_restart_timeout(&restart_output) {
        return Ok(false);
    }

    let stop_command = build_profile_command(profile, &["gateway", "stop"]);
    let stop_output = run_openclaw_dynamic(&stop_command)?;
    steps.push(RescuePrimaryRepairStep {
        id: "primary.gateway.stop".into(),
        title: "Stop primary gateway (restart fallback)".into(),
        ok: stop_output.exit_code == 0,
        detail: build_step_detail(&stop_command, &stop_output),
        command: Some(stop_command),
    });

    let start_command = build_profile_command(profile, &["gateway", "start"]);
    let start_output = run_openclaw_dynamic(&start_command)?;
    let start_ok = start_output.exit_code == 0;
    steps.push(RescuePrimaryRepairStep {
        id: "primary.gateway.start".into(),
        title: "Start primary gateway (restart fallback)".into(),
        ok: start_ok,
        detail: build_step_detail(&start_command, &start_output),
        command: Some(start_command),
    });
    Ok(start_ok)
}

async fn run_remote_profile_restart_with_fallback(
    pool: &SshConnectionPool,
    host_id: &str,
    profile: &str,
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<bool, String> {
    let restart_command = build_profile_command(profile, &["gateway", "restart"]);
    let restart_output =
        run_remote_openclaw_dynamic(pool, host_id, restart_command.clone()).await?;
    let restart_ok = restart_output.exit_code == 0;
    steps.push(RescuePrimaryRepairStep {
        id: "primary.gateway.restart".into(),
        title: "Restart primary gateway".into(),
        ok: restart_ok,
        detail: build_step_detail(&restart_command, &restart_output),
        command: Some(restart_command.clone()),
    });
    if restart_ok {
        return Ok(true);
    }

    if !is_gateway_restart_timeout(&restart_output) {
        return Ok(false);
    }

    let stop_command = build_profile_command(profile, &["gateway", "stop"]);
    let stop_output = run_remote_openclaw_dynamic(pool, host_id, stop_command.clone()).await?;
    steps.push(RescuePrimaryRepairStep {
        id: "primary.gateway.stop".into(),
        title: "Stop primary gateway (restart fallback)".into(),
        ok: stop_output.exit_code == 0,
        detail: build_step_detail(&stop_command, &stop_output),
        command: Some(stop_command),
    });

    let start_command = build_profile_command(profile, &["gateway", "start"]);
    let start_output = run_remote_openclaw_dynamic(pool, host_id, start_command.clone()).await?;
    let start_ok = start_output.exit_code == 0;
    steps.push(RescuePrimaryRepairStep {
        id: "primary.gateway.start".into(),
        title: "Start primary gateway (restart fallback)".into(),
        ok: start_ok,
        detail: build_step_detail(&start_command, &start_output),
        command: Some(start_command),
    });
    Ok(start_ok)
}

fn repair_primary_via_rescue_local(
    target_profile: &str,
    rescue_profile: &str,
    issue_ids: Vec<String>,
) -> Result<RescuePrimaryRepairResult, String> {
    let attempted_at = format_timestamp_from_unix(unix_timestamp_secs());
    let before = diagnose_primary_via_rescue_local(target_profile, rescue_profile)?;
    let (selected_issue_ids, mut skipped_issue_ids) =
        collect_safe_primary_issue_ids(&before, &issue_ids);
    let mut applied_issue_ids = Vec::new();
    let mut failed_issue_ids = Vec::new();
    let mut steps = Vec::new();

    if !before.rescue_configured {
        steps.push(RescuePrimaryRepairStep {
            id: "precheck.rescue_configured".into(),
            title: "Rescue profile availability".into(),
            ok: false,
            detail: format!(
                "Rescue profile \"{}\" is not configured; activate it before repair",
                before.rescue_profile
            ),
            command: None,
        });
        let after = before.clone();
        return Ok(RescuePrimaryRepairResult {
            attempted_at,
            target_profile: target_profile.to_string(),
            rescue_profile: rescue_profile.to_string(),
            selected_issue_ids,
            applied_issue_ids,
            skipped_issue_ids,
            failed_issue_ids,
            steps,
            before,
            after,
        });
    }

    if selected_issue_ids.is_empty() {
        steps.push(RescuePrimaryRepairStep {
            id: "repair.noop".into(),
            title: "No safe auto-fixes available".into(),
            ok: true,
            detail: "No auto-fixable primary issues were selected".into(),
            command: None,
        });
    } else {
        for issue_id in &selected_issue_ids {
            let Some((title, command)) = build_primary_issue_fix_command(target_profile, issue_id)
            else {
                skipped_issue_ids.push(issue_id.clone());
                steps.push(RescuePrimaryRepairStep {
                    id: format!("repair.{issue_id}"),
                    title: "Skip unsupported issue fix".into(),
                    ok: false,
                    detail: format!("No safe repair mapping for issue \"{issue_id}\""),
                    command: None,
                });
                continue;
            };
            let output = run_openclaw_dynamic(&command)?;
            let ok = output.exit_code == 0;
            steps.push(RescuePrimaryRepairStep {
                id: format!("repair.{issue_id}"),
                title,
                ok,
                detail: build_step_detail(&command, &output),
                command: Some(command),
            });
            if ok {
                applied_issue_ids.push(issue_id.clone());
            } else {
                failed_issue_ids.push(issue_id.clone());
            }
        }
    }

    if !applied_issue_ids.is_empty() {
        let restart_ok = run_local_profile_restart_with_fallback(target_profile, &mut steps)?;
        if !restart_ok {
            failed_issue_ids.push("primary.gateway.restart".into());
        }
    }

    let after = diagnose_primary_via_rescue_local(target_profile, rescue_profile)?;
    Ok(RescuePrimaryRepairResult {
        attempted_at,
        target_profile: target_profile.to_string(),
        rescue_profile: rescue_profile.to_string(),
        selected_issue_ids,
        applied_issue_ids,
        skipped_issue_ids,
        failed_issue_ids,
        steps,
        before,
        after,
    })
}

async fn repair_primary_via_rescue_remote(
    pool: &SshConnectionPool,
    host_id: &str,
    target_profile: &str,
    rescue_profile: &str,
    issue_ids: Vec<String>,
) -> Result<RescuePrimaryRepairResult, String> {
    let attempted_at = format_timestamp_from_unix(unix_timestamp_secs());
    let before =
        diagnose_primary_via_rescue_remote(pool, host_id, target_profile, rescue_profile).await?;
    let (selected_issue_ids, mut skipped_issue_ids) =
        collect_safe_primary_issue_ids(&before, &issue_ids);
    let mut applied_issue_ids = Vec::new();
    let mut failed_issue_ids = Vec::new();
    let mut steps = Vec::new();

    if !before.rescue_configured {
        steps.push(RescuePrimaryRepairStep {
            id: "precheck.rescue_configured".into(),
            title: "Rescue profile availability".into(),
            ok: false,
            detail: format!(
                "Rescue profile \"{}\" is not configured; activate it before repair",
                before.rescue_profile
            ),
            command: None,
        });
        let after = before.clone();
        return Ok(RescuePrimaryRepairResult {
            attempted_at,
            target_profile: target_profile.to_string(),
            rescue_profile: rescue_profile.to_string(),
            selected_issue_ids,
            applied_issue_ids,
            skipped_issue_ids,
            failed_issue_ids,
            steps,
            before,
            after,
        });
    }

    if selected_issue_ids.is_empty() {
        steps.push(RescuePrimaryRepairStep {
            id: "repair.noop".into(),
            title: "No safe auto-fixes available".into(),
            ok: true,
            detail: "No auto-fixable primary issues were selected".into(),
            command: None,
        });
    } else {
        for issue_id in &selected_issue_ids {
            let Some((title, command)) = build_primary_issue_fix_command(target_profile, issue_id)
            else {
                skipped_issue_ids.push(issue_id.clone());
                steps.push(RescuePrimaryRepairStep {
                    id: format!("repair.{issue_id}"),
                    title: "Skip unsupported issue fix".into(),
                    ok: false,
                    detail: format!("No safe repair mapping for issue \"{issue_id}\""),
                    command: None,
                });
                continue;
            };
            let output = run_remote_openclaw_dynamic(pool, host_id, command.clone()).await?;
            let ok = output.exit_code == 0;
            steps.push(RescuePrimaryRepairStep {
                id: format!("repair.{issue_id}"),
                title,
                ok,
                detail: build_step_detail(&command, &output),
                command: Some(command),
            });
            if ok {
                applied_issue_ids.push(issue_id.clone());
            } else {
                failed_issue_ids.push(issue_id.clone());
            }
        }
    }

    if !applied_issue_ids.is_empty() {
        let restart_ok =
            run_remote_profile_restart_with_fallback(pool, host_id, target_profile, &mut steps)
                .await?;
        if !restart_ok {
            failed_issue_ids.push("primary.gateway.restart".into());
        }
    }

    let after =
        diagnose_primary_via_rescue_remote(pool, host_id, target_profile, rescue_profile).await?;
    Ok(RescuePrimaryRepairResult {
        attempted_at,
        target_profile: target_profile.to_string(),
        rescue_profile: rescue_profile.to_string(),
        selected_issue_ids,
        applied_issue_ids,
        skipped_issue_ids,
        failed_issue_ids,
        steps,
        before,
        after,
    })
}

fn resolve_local_rescue_profile_state(profile: &str) -> Result<(bool, Option<u16>), String> {
    let output = crate::cli_runner::run_openclaw(&[
        "--profile",
        profile,
        "config",
        "get",
        "gateway.port",
        "--json",
    ])?;
    if output.exit_code != 0 {
        return Ok((false, None));
    }
    let port = crate::cli_runner::parse_json_output(&output)
        .ok()
        .and_then(|value| clawpal_core::doctor::parse_rescue_port_value(&value));
    Ok((true, port))
}

fn build_rescue_bot_command_plan(
    action: RescueBotAction,
    profile: &str,
    rescue_port: u16,
    include_configure: bool,
) -> Vec<Vec<String>> {
    clawpal_core::doctor::build_rescue_bot_command_plan(
        action.as_str(),
        profile,
        rescue_port,
        include_configure,
    )
}

fn command_failure_message(command: &[String], output: &OpenclawCommandOutput) -> String {
    clawpal_core::doctor::command_failure_message(
        command,
        output.exit_code,
        &output.stderr,
        &output.stdout,
    )
}

fn is_gateway_restart_command(command: &[String]) -> bool {
    clawpal_core::doctor::is_gateway_restart_command(command)
}

fn is_gateway_restart_timeout(output: &OpenclawCommandOutput) -> bool {
    clawpal_core::doctor::gateway_restart_timeout(&output.stderr, &output.stdout)
}

fn is_rescue_cleanup_noop(
    action: RescueBotAction,
    command: &[String],
    output: &OpenclawCommandOutput,
) -> bool {
    clawpal_core::doctor::rescue_cleanup_noop(
        action.as_str(),
        command,
        output.exit_code,
        &output.stderr,
        &output.stdout,
    )
}

fn run_local_rescue_bot_command(command: Vec<String>) -> Result<RescueBotCommandResult, String> {
    let output = run_openclaw_dynamic(&command)?;
    Ok(RescueBotCommandResult { command, output })
}

fn run_local_primary_doctor_with_fallback(profile: &str) -> Result<OpenclawCommandOutput, String> {
    let json_command = build_profile_command(profile, &["doctor", "--json"]);
    let output = run_openclaw_dynamic(&json_command)?;
    if output.exit_code != 0
        && clawpal_core::doctor::doctor_json_option_unsupported(&output.stderr, &output.stdout)
    {
        let plain_command = build_profile_command(profile, &["doctor"]);
        return run_openclaw_dynamic(&plain_command);
    }
    Ok(output)
}

fn run_local_gateway_restart_fallback(
    profile: &str,
    commands: &mut Vec<RescueBotCommandResult>,
) -> Result<(), String> {
    let stop_command = vec![
        "--profile".to_string(),
        profile.to_string(),
        "gateway".to_string(),
        "stop".to_string(),
    ];
    let stop_result = run_local_rescue_bot_command(stop_command)?;
    commands.push(stop_result);

    let start_command = vec![
        "--profile".to_string(),
        profile.to_string(),
        "gateway".to_string(),
        "start".to_string(),
    ];
    let start_result = run_local_rescue_bot_command(start_command)?;
    if start_result.output.exit_code != 0 {
        return Err(command_failure_message(
            &start_result.command,
            &start_result.output,
        ));
    }
    commands.push(start_result);
    Ok(())
}

fn run_openclaw_dynamic(args: &[String]) -> Result<OpenclawCommandOutput, String> {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    crate::cli_runner::run_openclaw(&refs).map(Into::into)
}

async fn resolve_remote_rescue_profile_state(
    pool: &SshConnectionPool,
    host_id: &str,
    profile: &str,
) -> Result<(bool, Option<u16>), String> {
    let output = crate::cli_runner::run_openclaw_remote(
        pool,
        host_id,
        &[
            "--profile",
            profile,
            "config",
            "get",
            "gateway.port",
            "--json",
        ],
    )
    .await?;
    if output.exit_code != 0 {
        return Ok((false, None));
    }
    let port = crate::cli_runner::parse_json_output(&output)
        .ok()
        .and_then(|value| clawpal_core::doctor::parse_rescue_port_value(&value));
    Ok((true, port))
}

fn run_openclaw_raw(args: &[&str]) -> Result<OpenclawCommandOutput, String> {
    run_openclaw_raw_timeout(args, None)
}

fn run_openclaw_raw_timeout(
    args: &[&str],
    timeout_secs: Option<u64>,
) -> Result<OpenclawCommandOutput, String> {
    let mut command = Command::new(clawpal_core::openclaw::resolve_openclaw_bin());
    command
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(path) = crate::cli_runner::get_active_openclaw_home_override() {
        command.env("OPENCLAW_HOME", path);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to run openclaw: {error}"))?;

    if let Some(secs) = timeout_secs {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        loop {
            match child.try_wait().map_err(|e| e.to_string())? {
                Some(status) => {
                    let mut stdout_buf = Vec::new();
                    let mut stderr_buf = Vec::new();
                    if let Some(mut out) = child.stdout.take() {
                        std::io::Read::read_to_end(&mut out, &mut stdout_buf).ok();
                    }
                    if let Some(mut err) = child.stderr.take() {
                        std::io::Read::read_to_end(&mut err, &mut stderr_buf).ok();
                    }
                    let exit_code = status.code().unwrap_or(-1);
                    let result = OpenclawCommandOutput {
                        stdout: String::from_utf8_lossy(&stdout_buf).trim_end().to_string(),
                        stderr: String::from_utf8_lossy(&stderr_buf).trim_end().to_string(),
                        exit_code,
                    };
                    if exit_code != 0 {
                        let details = if !result.stderr.is_empty() {
                            result.stderr.clone()
                        } else {
                            result.stdout.clone()
                        };
                        return Err(format!("openclaw command failed ({exit_code}): {details}"));
                    }
                    return Ok(result);
                }
                None => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        return Err(format!(
                            "Command timed out after {secs}s. The gateway may still be restarting in the background."
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
        }
    } else {
        let output = child
            .wait_with_output()
            .map_err(|error| format!("failed to run openclaw: {error}"))?;
        let exit_code = output.status.code().unwrap_or(-1);
        let result = OpenclawCommandOutput {
            stdout: String::from_utf8_lossy(&output.stdout)
                .trim_end()
                .to_string(),
            stderr: String::from_utf8_lossy(&output.stderr)
                .trim_end()
                .to_string(),
            exit_code,
        };
        if exit_code != 0 {
            let details = if !result.stderr.is_empty() {
                result.stderr.clone()
            } else {
                result.stdout.clone()
            };
            return Err(format!("openclaw command failed ({exit_code}): {details}"));
        }
        Ok(result)
    }
}

#[tauri::command]
pub fn set_active_openclaw_home(path: Option<String>) -> Result<bool, String> {
    crate::cli_runner::set_active_openclaw_home_override(path)?;
    Ok(true)
}

#[tauri::command]
pub fn set_active_clawpal_data_dir(path: Option<String>) -> Result<bool, String> {
    crate::cli_runner::set_active_clawpal_data_override(path)?;
    Ok(true)
}

#[tauri::command]
pub fn local_openclaw_config_exists(openclaw_home: String) -> Result<bool, String> {
    let home = openclaw_home.trim();
    if home.is_empty() {
        return Ok(false);
    }
    let expanded = shellexpand::tilde(home).to_string();
    let config_path = PathBuf::from(expanded)
        .join(".openclaw")
        .join("openclaw.json");
    Ok(config_path.exists())
}

#[tauri::command]
pub fn delete_local_instance_home(openclaw_home: String) -> Result<bool, String> {
    let home = openclaw_home.trim();
    if home.is_empty() {
        return Err("openclaw_home is required".to_string());
    }
    let expanded = shellexpand::tilde(home).to_string();
    let target = PathBuf::from(expanded);
    if !target.exists() {
        return Ok(true);
    }

    let canonical_target = target
        .canonicalize()
        .map_err(|e| format!("failed to resolve target path: {e}"))?;
    let user_home =
        dirs::home_dir().ok_or_else(|| "failed to resolve HOME directory".to_string())?;
    let allowed_root = user_home.join(".clawpal");
    let canonical_allowed_root = allowed_root
        .canonicalize()
        .map_err(|e| format!("failed to resolve ~/.clawpal path: {e}"))?;

    if !canonical_target.starts_with(&canonical_allowed_root) {
        return Err("refuse to delete path outside ~/.clawpal".to_string());
    }
    if canonical_target == canonical_allowed_root {
        return Err("refuse to delete ~/.clawpal root".to_string());
    }

    fs::remove_dir_all(&canonical_target).map_err(|e| {
        format!(
            "failed to delete '{}': {e}",
            canonical_target.to_string_lossy()
        )
    })?;
    Ok(true)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnsureAccessResult {
    pub instance_id: String,
    pub transport: String,
    pub working_chain: Vec<String>,
    pub used_legacy_fallback: bool,
    pub profile_reused: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordInstallExperienceResult {
    pub saved: bool,
    pub total_count: usize,
}

pub async fn ensure_access_profile_impl(
    instance_id: String,
    transport: String,
) -> Result<EnsureAccessResult, String> {
    let paths = resolve_paths();
    let store = AccessDiscoveryStore::new(paths.clawpal_dir.join("access-discovery"));
    if let Some(existing) = store.load_profile(&instance_id)? {
        if !existing.working_chain.is_empty() {
            return Ok(EnsureAccessResult {
                instance_id,
                transport,
                working_chain: existing.working_chain,
                used_legacy_fallback: false,
                profile_reused: true,
            });
        }
    }

    let probe_plan = build_probe_plan_for_local();
    let probes = probe_plan
        .iter()
        .enumerate()
        .map(|(idx, cmd)| {
            run_probe_with_redaction(&format!("probe-{idx}"), cmd, "planned", true, 0)
        })
        .collect::<Vec<_>>();

    let mut profile = CapabilityProfile::example_local(&instance_id);
    profile.transport = transport.clone();
    profile.probes = probes;
    profile.verified_at = unix_timestamp_secs();

    let used_legacy_fallback = if store.save_profile(&profile).is_err() {
        true
    } else {
        false
    };

    Ok(EnsureAccessResult {
        instance_id,
        transport,
        working_chain: profile.working_chain,
        used_legacy_fallback,
        profile_reused: false,
    })
}

#[tauri::command]
pub async fn ensure_access_profile(
    instance_id: String,
    transport: String,
) -> Result<EnsureAccessResult, String> {
    ensure_access_profile_impl(instance_id, transport).await
}

pub async fn ensure_access_profile_for_test(
    instance_id: &str,
) -> Result<EnsureAccessResult, String> {
    ensure_access_profile_impl(instance_id.to_string(), "local".to_string()).await
}

fn value_array_as_strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

#[tauri::command]
pub async fn record_install_experience(
    session_id: String,
    instance_id: String,
    goal: String,
    store: State<'_, InstallSessionStore>,
) -> Result<RecordInstallExperienceResult, String> {
    let id = session_id.trim();
    if id.is_empty() {
        return Err("session_id is required".to_string());
    }
    let session = store
        .get(id)?
        .ok_or_else(|| format!("install session not found: {id}"))?;
    if !matches!(session.state, InstallState::Ready) {
        return Err(format!(
            "install session is not ready: {}",
            session.state.as_str()
        ));
    }

    let transport = session.method.as_str().to_string();
    let paths = resolve_paths();
    let discovery_store = AccessDiscoveryStore::new(paths.clawpal_dir.join("access-discovery"));
    let profile = discovery_store.load_profile(&instance_id)?;
    let successful_chain = profile.map(|p| p.working_chain).unwrap_or_default();
    let commands = value_array_as_strings(session.artifacts.get("executed_commands"));

    let experience = ExecutionExperience {
        instance_id: instance_id.clone(),
        goal,
        transport,
        method: session.method.as_str().to_string(),
        commands,
        successful_chain,
        recorded_at: unix_timestamp_secs(),
    };
    let total_count = discovery_store.save_experience(experience)?;
    Ok(RecordInstallExperienceResult {
        saved: true,
        total_count,
    })
}

/// Extract the last JSON array from CLI output that may contain ANSI codes and plugin logs.
/// Scans from the end to find the last `]`, then finds its matching `[`.
fn extract_last_json_array(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    let end = bytes.iter().rposition(|&b| b == b']')?;
    let mut depth = 0;
    for i in (0..=end).rev() {
        match bytes[i] {
            b']' => depth += 1,
            b'[' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&raw[i..=end]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse `openclaw channels resolve --json` output into a map of id -> name.
fn parse_resolve_name_map(stdout: &str) -> Option<HashMap<String, String>> {
    let json_str = extract_last_json_array(stdout)?;
    let parsed: Vec<Value> = serde_json::from_str(json_str).ok()?;
    let mut map = HashMap::new();
    for item in parsed {
        let resolved = item
            .get("resolved")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !resolved {
            continue;
        }
        if let (Some(input), Some(name)) = (
            item.get("input").and_then(Value::as_str),
            item.get("name").and_then(Value::as_str),
        ) {
            let name = name.trim().to_string();
            if !name.is_empty() {
                map.insert(input.to_string(), name);
            }
        }
    }
    Some(map)
}

/// Parse `openclaw directory groups list --json` output into channel ids.
fn parse_directory_group_channel_ids(stdout: &str) -> Vec<String> {
    let json_str = match extract_last_json_array(stdout) {
        Some(v) => v,
        None => return Vec::new(),
    };
    let parsed: Vec<Value> = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut ids = Vec::new();
    for item in parsed {
        let raw = item.get("id").and_then(Value::as_str).unwrap_or("");
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = trimmed
            .strip_prefix("channel:")
            .unwrap_or(trimmed)
            .trim()
            .to_string();
        if normalized.is_empty() || ids.contains(&normalized) {
            continue;
        }
        ids.push(normalized);
    }
    ids
}

fn collect_discord_config_guild_ids(discord_cfg: Option<&Value>) -> Vec<String> {
    let mut guild_ids = Vec::new();
    if let Some(guilds) = discord_cfg
        .and_then(|d| d.get("guilds"))
        .and_then(Value::as_object)
    {
        for guild_id in guilds.keys() {
            if !guild_ids.contains(guild_id) {
                guild_ids.push(guild_id.clone());
            }
        }
    }
    if let Some(accounts) = discord_cfg
        .and_then(|d| d.get("accounts"))
        .and_then(Value::as_object)
    {
        for account in accounts.values() {
            if let Some(guilds) = account.get("guilds").and_then(Value::as_object) {
                for guild_id in guilds.keys() {
                    if !guild_ids.contains(guild_id) {
                        guild_ids.push(guild_id.clone());
                    }
                }
            }
        }
    }
    guild_ids
}

fn collect_discord_config_guild_name_fallbacks(
    discord_cfg: Option<&Value>,
) -> HashMap<String, String> {
    let mut guild_names = HashMap::new();

    if let Some(guilds) = discord_cfg
        .and_then(|d| d.get("guilds"))
        .and_then(Value::as_object)
    {
        for (guild_id, guild_val) in guilds {
            let guild_name = guild_val
                .get("slug")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if let Some(name) = guild_name {
                guild_names.entry(guild_id.clone()).or_insert(name);
            }
        }
    }

    if let Some(accounts) = discord_cfg
        .and_then(|d| d.get("accounts"))
        .and_then(Value::as_object)
    {
        for account in accounts.values() {
            if let Some(guilds) = account.get("guilds").and_then(Value::as_object) {
                for (guild_id, guild_val) in guilds {
                    let guild_name = guild_val
                        .get("slug")
                        .and_then(Value::as_str)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    if let Some(name) = guild_name {
                        guild_names.entry(guild_id.clone()).or_insert(name);
                    }
                }
            }
        }
    }

    guild_names
}

fn collect_discord_cache_guild_name_fallbacks(
    entries: &[DiscordGuildChannel],
) -> HashMap<String, String> {
    let mut guild_names = HashMap::new();
    for entry in entries {
        let name = entry.guild_name.trim();
        if name.is_empty() || name == entry.guild_id {
            continue;
        }
        guild_names
            .entry(entry.guild_id.clone())
            .or_insert_with(|| name.to_string());
    }
    guild_names
}

fn parse_discord_cache_guild_name_fallbacks(cache_json: &str) -> HashMap<String, String> {
    let entries: Vec<DiscordGuildChannel> = serde_json::from_str(cache_json).unwrap_or_default();
    collect_discord_cache_guild_name_fallbacks(&entries)
}

#[cfg(test)]
mod discord_directory_parse_tests {
    use super::{
        parse_directory_group_channel_ids, parse_discord_cache_guild_name_fallbacks,
        DiscordGuildChannel,
    };

    #[test]
    fn parse_directory_groups_extracts_channel_ids() {
        let stdout = r#"
[plugins] example
[
  {"kind":"group","id":"channel:123"},
  {"kind":"group","id":"channel:456"},
  {"kind":"group","id":"channel:123"},
  {"kind":"group","id":"  channel:789  "}
]
"#;
        let ids = parse_directory_group_channel_ids(stdout);
        assert_eq!(ids, vec!["123", "456", "789"]);
    }

    #[test]
    fn parse_directory_groups_handles_missing_json() {
        let stdout = "not json";
        let ids = parse_directory_group_channel_ids(stdout);
        assert!(ids.is_empty());
    }

    #[test]
    fn parse_discord_cache_guild_name_fallbacks_uses_non_id_names() {
        let payload = vec![
            DiscordGuildChannel {
                guild_id: "1".into(),
                guild_name: "Guild One".into(),
                channel_id: "11".into(),
                channel_name: "chan-1".into(),
                default_agent_id: None,
            },
            DiscordGuildChannel {
                guild_id: "1".into(),
                guild_name: "1".into(),
                channel_id: "12".into(),
                channel_name: "chan-2".into(),
                default_agent_id: None,
            },
            DiscordGuildChannel {
                guild_id: "2".into(),
                guild_name: "2".into(),
                channel_id: "21".into(),
                channel_name: "chan-3".into(),
                default_agent_id: None,
            },
        ];
        let text = serde_json::to_string(&payload).expect("serialize payload");
        let fallbacks = parse_discord_cache_guild_name_fallbacks(&text);
        assert_eq!(fallbacks.get("1"), Some(&"Guild One".to_string()));
        assert!(!fallbacks.contains_key("2"));
    }
}

fn extract_version_from_text(input: &str) -> Option<String> {
    let re = regex::Regex::new(r"\d+\.\d+(?:\.\d+){1,3}(?:[-+._a-zA-Z0-9]*)?").ok()?;
    re.find(input).map(|mat| mat.as_str().to_string())
}

fn compare_semver(installed: &str, latest: Option<&str>) -> bool {
    let installed = normalize_semver_components(installed);
    let latest = latest.and_then(normalize_semver_components);
    let (mut installed, mut latest) = match (installed, latest) {
        (Some(installed), Some(latest)) => (installed, latest),
        _ => return false,
    };

    let len = installed.len().max(latest.len());
    while installed.len() < len {
        installed.push(0);
    }
    while latest.len() < len {
        latest.push(0);
    }
    installed < latest
}

fn normalize_semver_components(raw: &str) -> Option<Vec<u32>> {
    let mut parts = Vec::new();
    for bit in raw.split('.') {
        let filtered = bit.trim_start_matches(|c: char| c == 'v' || c == 'V');
        let head = filtered
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap_or("");
        if head.is_empty() {
            continue;
        }
        parts.push(head.parse::<u32>().ok()?);
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts)
}

fn unix_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |delta| delta.as_secs())
}

fn format_timestamp_from_unix(timestamp: u64) -> String {
    let Some(utc) = chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp as i64, 0) else {
        return "unknown".into();
    };
    utc.to_rfc3339()
}

fn openclaw_update_cache_path(paths: &crate::models::OpenClawPaths) -> PathBuf {
    paths.clawpal_dir.join("openclaw-update-cache.json")
}

fn read_openclaw_update_cache(path: &Path) -> Option<OpenclawUpdateCache> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str::<OpenclawUpdateCache>(&text).ok()
}

fn save_openclaw_update_cache(path: &Path, cache: &OpenclawUpdateCache) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(cache).map_err(|error| error.to_string())?;
    write_text(path, &text)
}

fn read_model_catalog_cache(path: &Path) -> Option<ModelCatalogProviderCache> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str::<ModelCatalogProviderCache>(&text).ok()
}

fn save_model_catalog_cache(path: &Path, cache: &ModelCatalogProviderCache) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(cache).map_err(|error| error.to_string())?;
    write_text(path, &text)
}

fn model_catalog_cache_path(paths: &crate::models::OpenClawPaths) -> PathBuf {
    paths.clawpal_dir.join("model-catalog-cache.json")
}

fn remote_model_catalog_cache_path(paths: &crate::models::OpenClawPaths, host_id: &str) -> PathBuf {
    let safe_host_id: String = host_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    paths
        .clawpal_dir
        .join("remote-model-catalog")
        .join(format!("{safe_host_id}.json"))
}

fn normalize_model_ref(raw: &str) -> String {
    raw.trim().to_lowercase().replace('\\', "/")
}

fn resolve_openclaw_version() -> String {
    use std::sync::OnceLock;
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION
        .get_or_init(|| match run_openclaw_raw(&["--version"]) {
            Ok(output) => {
                extract_version_from_text(&output.stdout).unwrap_or_else(|| "unknown".into())
            }
            Err(_) => "unknown".into(),
        })
        .clone()
}

fn check_openclaw_update_cached(
    paths: &crate::models::OpenClawPaths,
    force: bool,
) -> Result<OpenclawUpdateCheck, String> {
    let cache_path = openclaw_update_cache_path(paths);
    let now = unix_timestamp_secs();
    if !force {
        if let Some(cached) = read_openclaw_update_cache(&cache_path) {
            if now.saturating_sub(cached.checked_at) < cached.ttl_seconds {
                let installed_version = cached
                    .installed_version
                    .unwrap_or_else(resolve_openclaw_version);
                let upgrade_available =
                    compare_semver(&installed_version, cached.latest_version.as_deref());
                return Ok(OpenclawUpdateCheck {
                    installed_version,
                    latest_version: cached.latest_version,
                    upgrade_available,
                    channel: cached.channel,
                    details: cached.details,
                    source: cached.source,
                    checked_at: format_timestamp_from_unix(now),
                });
            }
        }
    }

    let installed_version = resolve_openclaw_version();
    let (latest_version, channel, details, source, upgrade_available) =
        detect_openclaw_update_cached(&installed_version).unwrap_or((
            None,
            None,
            Some("failed to detect update status".into()),
            "openclaw-command".into(),
            false,
        ));
    let checked_at = format_timestamp_from_unix(now);
    let cache = OpenclawUpdateCache {
        checked_at: now,
        latest_version: latest_version.clone(),
        channel,
        details: details.clone(),
        source: source.clone(),
        installed_version: Some(installed_version.clone()),
        ttl_seconds: 60 * 60 * 6,
    };
    save_openclaw_update_cache(&cache_path, &cache)?;
    let upgrade = compare_semver(&installed_version, latest_version.as_deref());
    Ok(OpenclawUpdateCheck {
        installed_version,
        latest_version,
        upgrade_available: upgrade || upgrade_available,
        channel: cache.channel,
        details,
        source,
        checked_at,
    })
}

fn detect_openclaw_update_cached(
    installed_version: &str,
) -> Option<(Option<String>, Option<String>, Option<String>, String, bool)> {
    let output = run_openclaw_raw(&["update", "status"]).ok()?;
    if let Some((latest_version, channel, details, upgrade_available)) =
        parse_openclaw_update_json(&output.stdout, installed_version)
    {
        return Some((
            latest_version,
            Some(channel),
            Some(details),
            "openclaw update status --json".into(),
            upgrade_available,
        ));
    }
    let parsed = parse_openclaw_update_text(&output.stdout);
    if let Some((latest_version, channel, details)) = parsed {
        let source = "openclaw update status".into();
        let available = latest_version
            .as_ref()
            .is_some_and(|latest| compare_semver(installed_version, Some(latest)));
        return Some((
            latest_version,
            Some(channel),
            Some(details),
            source,
            available,
        ));
    }
    let latest_version = query_openclaw_latest_npm().ok().flatten();
    let details = latest_version
        .as_ref()
        .map(|value| format!("npm latest {value}"))
        .unwrap_or_else(|| "update status not available".into());
    let upgrade = latest_version
        .as_ref()
        .is_some_and(|latest| compare_semver(installed_version, Some(latest.as_str())));
    Some((latest_version, None, Some(details), "npm".into(), upgrade))
}

fn parse_openclaw_update_json(
    raw: &str,
    installed_version: &str,
) -> Option<(Option<String>, String, String, bool)> {
    let json_str = clawpal_core::doctor::extract_json_from_output(raw)?;
    let payload: Value = serde_json::from_str(json_str).ok()?;
    let channel = payload
        .pointer("/channel/value")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    let latest_from_update = payload
        .pointer("/update/registry/latestVersion")
        .and_then(Value::as_str)
        .map(|value| value.to_string());
    let latest = payload
        .pointer("/availability/latestVersion")
        .and_then(Value::as_str)
        .map(|value| value.to_string())
        .or(latest_from_update);
    let has_update = payload
        .pointer("/availability/available")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let details = payload
        .pointer("/availability/latestVersion")
        .and_then(Value::as_str)
        .map(|value| format!("npm latest {value}"))
        .or_else(|| {
            if has_update {
                Some("update available".into())
            } else {
                Some("up to date".into())
            }
        })
        .unwrap_or_else(|| "update status unavailable".into());

    let upgrade_available = if let Some(latest_version) = latest.as_deref() {
        compare_semver(installed_version, Some(latest_version))
    } else {
        has_update
    };

    Some((latest, channel, details, upgrade_available))
}

fn parse_openclaw_update_text(raw: &str) -> Option<(Option<String>, String, String)> {
    let mut channel = String::from("unknown");
    for line in raw.lines() {
        if line.contains("Channel") {
            let right = line.split('│').last().or_else(|| line.split('|').last())?;
            channel = right.trim().to_string();
        }
        if line.to_lowercase().contains("update") && line.contains("npm latest") {
            if let Some(token) = extract_version_from_text(line) {
                return Some((Some(token), channel, line.trim().to_string()));
            }
            return Some((None, channel, line.trim().to_string()));
        }
        if line.to_lowercase().contains("update") && line.contains("unknown") {
            return Some((None, channel, line.trim().to_string()));
        }
    }
    None
}

fn query_openclaw_latest_npm() -> Result<Option<String>, String> {
    // Query npm registry directly via HTTP — no local npm CLI needed
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;
    let resp = client
        .get("https://registry.npmjs.org/openclaw/latest")
        .header("Accept", "application/json")
        .send()
        .map_err(|e| format!("npm registry request failed: {e}"))?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let body: Value = resp
        .json()
        .map_err(|e| format!("npm registry parse failed: {e}"))?;
    let version = body
        .get("version")
        .and_then(Value::as_str)
        .map(String::from);
    Ok(version)
}

const DISCORD_REST_USER_AGENT: &str = "DiscordBot (https://openclaw.ai, 1.0)";

/// Fetch a Discord guild name via the Discord REST API using a bot token.
fn fetch_discord_guild_name(bot_token: &str, guild_id: &str) -> Result<String, String> {
    let url = format!("https://discord.com/api/v10/guilds/{guild_id}");
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .user_agent(DISCORD_REST_USER_AGENT)
        .build()
        .map_err(|e| format!("Discord HTTP client error: {e}"))?;
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bot {bot_token}"))
        .send()
        .map_err(|e| format!("Discord API request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Discord API returned status {}", resp.status()));
    }
    let body: Value = resp
        .json()
        .map_err(|e| format!("Failed to parse Discord response: {e}"))?;
    body.get("name")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| "No name field in Discord guild response".to_string())
}

/// Fetch Discord channels for a guild via REST API using a bot token.
fn fetch_discord_guild_channels(
    bot_token: &str,
    guild_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let url = format!("https://discord.com/api/v10/guilds/{guild_id}/channels");
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .user_agent(DISCORD_REST_USER_AGENT)
        .build()
        .map_err(|e| format!("Discord HTTP client error: {e}"))?;
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bot {bot_token}"))
        .send()
        .map_err(|e| format!("Discord API request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Discord API returned status {}", resp.status()));
    }
    let body: Value = resp
        .json()
        .map_err(|e| format!("Failed to parse Discord response: {e}"))?;
    let arr = body
        .as_array()
        .ok_or_else(|| "Discord response is not an array".to_string())?;
    let mut out = Vec::new();
    for item in arr {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        // Filter out categories (type 4), voice channels (type 2), and stage channels (type 13)
        let channel_type = item.get("type").and_then(Value::as_u64).unwrap_or(0);
        if channel_type == 4 || channel_type == 2 || channel_type == 13 {
            continue;
        }
        if let (Some(id), Some(name)) = (id, name) {
            if !out.iter().any(|(existing_id, _)| *existing_id == id) {
                out.push((id, name));
            }
        }
    }
    Ok(out)
}

fn collect_channel_summary(cfg: &Value) -> ChannelSummary {
    let examples = collect_channel_model_overrides_list(cfg);
    let configured_channels = cfg
        .get("channels")
        .and_then(|v| v.as_object())
        .map(|channels| channels.len())
        .unwrap_or(0);

    ChannelSummary {
        configured_channels,
        channel_model_overrides: examples.len(),
        channel_examples: examples,
    }
}

fn read_model_value(value: &Value) -> Option<String> {
    if let Some(value) = value.as_str() {
        return Some(value.to_string());
    }

    if let Some(model_obj) = value.as_object() {
        if let Some(primary) = model_obj.get("primary").and_then(Value::as_str) {
            return Some(primary.to_string());
        }
        if let Some(name) = model_obj.get("name").and_then(Value::as_str) {
            return Some(name.to_string());
        }
        if let Some(model) = model_obj.get("model").and_then(Value::as_str) {
            return Some(model.to_string());
        }
        if let Some(model) = model_obj.get("default").and_then(Value::as_str) {
            return Some(model.to_string());
        }
        if let Some(v) = model_obj.get("provider").and_then(Value::as_str) {
            if let Some(inner) = model_obj.get("id").and_then(Value::as_str) {
                return Some(format!("{v}/{inner}"));
            }
        }
    }
    None
}

fn collect_channel_model_overrides(cfg: &Value) -> Vec<String> {
    collect_channel_model_overrides_list(cfg)
}

fn collect_channel_model_overrides_list(cfg: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(channels) = cfg.get("channels").and_then(Value::as_object) {
        for (name, entry) in channels {
            let mut branch = Vec::new();
            collect_channel_paths(name, entry, &mut branch);
            out.extend(branch);
        }
    }
    out
}

fn collect_channel_paths(prefix: &str, node: &Value, out: &mut Vec<String>) {
    if let Some(obj) = node.as_object() {
        if let Some(model) = obj.get("model").and_then(read_model_value) {
            out.push(format!("{prefix} => {model}"));
        }
        for (key, child) in obj {
            if key == "model" {
                continue;
            }
            let next = format!("{prefix}.{key}");
            collect_channel_paths(&next, child, out);
        }
    }
}

fn collect_memory_overview(base_dir: &Path) -> MemorySummary {
    let memory_root = base_dir.join("memory");
    collect_file_inventory(&memory_root, Some(80))
}

fn collect_file_inventory(path: &Path, max_files: Option<usize>) -> MemorySummary {
    let mut queue = VecDeque::new();
    let mut file_count = 0usize;
    let mut total_bytes = 0u64;
    let mut files = Vec::new();

    if !path.exists() {
        return MemorySummary {
            file_count: 0,
            total_bytes: 0,
            files,
        };
    }

    queue.push_back(path.to_path_buf());
    while let Some(current) = queue.pop_front() {
        let entries = match fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_dir() {
                    queue.push_back(entry_path);
                    continue;
                }
                if metadata.is_file() {
                    file_count += 1;
                    total_bytes = total_bytes.saturating_add(metadata.len());
                    if max_files.is_none_or(|limit| files.len() < limit) {
                        files.push(MemoryFileSummary {
                            path: entry_path.to_string_lossy().to_string(),
                            size_bytes: metadata.len(),
                        });
                    }
                }
            }
        }
    }

    files.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    MemorySummary {
        file_count,
        total_bytes,
        files,
    }
}

fn collect_session_overview(base_dir: &Path) -> SessionSummary {
    let agents_dir = base_dir.join("agents");
    let mut by_agent = Vec::new();
    let mut total_session_files = 0usize;
    let mut total_archive_files = 0usize;
    let mut total_bytes = 0u64;

    if !agents_dir.exists() {
        return SessionSummary {
            total_session_files,
            total_archive_files,
            total_bytes,
            by_agent,
        };
    }

    if let Ok(entries) = fs::read_dir(agents_dir) {
        for entry in entries.flatten() {
            let agent_path = entry.path();
            if !agent_path.is_dir() {
                continue;
            }
            let agent = entry.file_name().to_string_lossy().to_string();
            let sessions_dir = agent_path.join("sessions");
            let archive_dir = agent_path.join("sessions_archive");

            let session_info = collect_file_inventory_with_limit(&sessions_dir);
            let archive_info = collect_file_inventory_with_limit(&archive_dir);

            if session_info.files > 0 || archive_info.files > 0 {
                by_agent.push(AgentSessionSummary {
                    agent: agent.clone(),
                    session_files: session_info.files,
                    archive_files: archive_info.files,
                    total_bytes: session_info
                        .total_bytes
                        .saturating_add(archive_info.total_bytes),
                });
            }

            total_session_files = total_session_files.saturating_add(session_info.files);
            total_archive_files = total_archive_files.saturating_add(archive_info.files);
            total_bytes = total_bytes
                .saturating_add(session_info.total_bytes)
                .saturating_add(archive_info.total_bytes);
        }
    }

    by_agent.sort_by(|a, b| b.total_bytes.cmp(&a.total_bytes));
    SessionSummary {
        total_session_files,
        total_archive_files,
        total_bytes,
        by_agent,
    }
}

struct InventorySummary {
    files: usize,
    total_bytes: u64,
}

fn collect_file_inventory_with_limit(path: &Path) -> InventorySummary {
    if !path.exists() {
        return InventorySummary {
            files: 0,
            total_bytes: 0,
        };
    }
    let mut queue = VecDeque::new();
    let mut files = 0usize;
    let mut total_bytes = 0u64;
    queue.push_back(path.to_path_buf());
    while let Some(current) = queue.pop_front() {
        let entries = match fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                let p = entry.path();
                if metadata.is_dir() {
                    queue.push_back(p);
                } else if metadata.is_file() {
                    files += 1;
                    total_bytes = total_bytes.saturating_add(metadata.len());
                }
            }
        }
    }
    InventorySummary { files, total_bytes }
}

fn list_session_files_detailed(base_dir: &Path) -> Result<Vec<SessionFile>, String> {
    let agents_root = base_dir.join("agents");
    if !agents_root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let entries = fs::read_dir(&agents_root).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }
        let agent = entry.file_name().to_string_lossy().to_string();
        let sessions_root = entry_path.join("sessions");
        let archive_root = entry_path.join("sessions_archive");

        collect_session_files_in_scope(&sessions_root, &agent, "sessions", base_dir, &mut out)?;
        collect_session_files_in_scope(&archive_root, &agent, "archive", base_dir, &mut out)?;
    }
    out.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(out)
}

fn collect_session_files_in_scope(
    scope_root: &Path,
    agent: &str,
    kind: &str,
    base_dir: &Path,
    out: &mut Vec<SessionFile>,
) -> Result<(), String> {
    if !scope_root.exists() {
        return Ok(());
    }
    let mut queue = VecDeque::new();
    queue.push_back(scope_root.to_path_buf());
    while let Some(current) = queue.pop_front() {
        let entries = match fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let entry_path = entry.path();
            let metadata = match entry.metadata() {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            if metadata.is_dir() {
                queue.push_back(entry_path);
                continue;
            }
            if metadata.is_file() {
                let relative_path = entry_path
                    .strip_prefix(base_dir)
                    .unwrap_or(&entry_path)
                    .to_string_lossy()
                    .to_string();
                out.push(SessionFile {
                    path: entry_path.to_string_lossy().to_string(),
                    relative_path,
                    agent: agent.to_string(),
                    kind: kind.to_string(),
                    size_bytes: metadata.len(),
                });
            }
        }
    }
    Ok(())
}

fn clear_agent_and_global_sessions(
    agents_root: &Path,
    agent_id: Option<&str>,
) -> Result<usize, String> {
    if !agents_root.exists() {
        return Ok(0);
    }
    let mut total = 0usize;
    let mut targets = Vec::new();

    match agent_id {
        Some(agent) => targets.push(agents_root.join(agent)),
        None => {
            for entry in fs::read_dir(agents_root).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                    targets.push(entry.path());
                }
            }
        }
    }

    for agent_path in targets {
        let sessions = agent_path.join("sessions");
        let archive = agent_path.join("sessions_archive");
        total = total.saturating_add(clear_directory_contents(&sessions)?);
        total = total.saturating_add(clear_directory_contents(&archive)?);
        fs::create_dir_all(&sessions).map_err(|e| e.to_string())?;
        fs::create_dir_all(&archive).map_err(|e| e.to_string())?;
    }
    Ok(total)
}

fn clear_directory_contents(target: &Path) -> Result<usize, String> {
    if !target.exists() {
        return Ok(0);
    }
    let mut total = 0usize;
    let entries = fs::read_dir(target).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let metadata = entry.metadata().map_err(|e| e.to_string())?;
        if metadata.is_dir() {
            total = total.saturating_add(clear_directory_contents(&path)?);
            fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
            continue;
        }
        if metadata.is_file() || metadata.is_symlink() {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
            total = total.saturating_add(1);
        }
    }
    Ok(total)
}

fn model_profiles_path(paths: &crate::models::OpenClawPaths) -> std::path::PathBuf {
    paths.clawpal_dir.join("model-profiles.json")
}

fn profile_to_model_value(profile: &ModelProfile) -> String {
    let provider = profile.provider.trim();
    let model = profile.model.trim();
    if provider.is_empty() {
        return model.to_string();
    }
    if model.is_empty() {
        return format!("{provider}/");
    }
    let normalized_prefix = format!("{}/", provider.to_lowercase());
    if model.to_lowercase().starts_with(&normalized_prefix) {
        model.to_string()
    } else {
        format!("{provider}/{model}")
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedApiKey {
    pub profile_id: String,
    pub masked_key: String,
}

fn truncate_error_text(input: &str, max_chars: usize) -> String {
    if let Some((i, _)) = input.char_indices().nth(max_chars) {
        format!("{}...", &input[..i])
    } else {
        input.to_string()
    }
}

const MAX_ERROR_SNIPPET_CHARS: usize = 280;

fn default_base_url_for_provider(provider: &str) -> Option<&'static str> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "openai" => Some("https://api.openai.com/v1"),
        "openrouter" => Some("https://openrouter.ai/api/v1"),
        "groq" => Some("https://api.groq.com/openai/v1"),
        "deepseek" => Some("https://api.deepseek.com/v1"),
        "xai" | "grok" => Some("https://api.x.ai/v1"),
        "together" => Some("https://api.together.xyz/v1"),
        "mistral" => Some("https://api.mistral.ai/v1"),
        "anthropic" => Some("https://api.anthropic.com/v1"),
        _ => None,
    }
}

fn run_provider_probe(
    provider: String,
    model: String,
    base_url: Option<String>,
    api_key: String,
) -> Result<(), String> {
    let provider_trimmed = provider.trim().to_string();
    let mut model_trimmed = model.trim().to_string();
    if provider_trimmed.is_empty() || model_trimmed.is_empty() {
        return Err("provider and model are required".into());
    }
    let provider_prefix = format!("{}/", provider_trimmed.to_ascii_lowercase());
    if model_trimmed
        .to_ascii_lowercase()
        .starts_with(&provider_prefix)
    {
        model_trimmed = model_trimmed[provider_prefix.len()..].to_string();
        if model_trimmed.trim().is_empty() {
            return Err("model is empty after provider prefix normalization".into());
        }
    }
    if api_key.trim().is_empty() {
        return Err("API key is not configured for this profile".into());
    }

    let resolved_base = base_url
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.trim_end_matches('/').to_string())
        .or_else(|| default_base_url_for_provider(&provider_trimmed).map(str::to_string))
        .ok_or_else(|| format!("No base URL configured for provider '{}'", provider_trimmed))?;

    // Use stream:true so the provider returns HTTP headers immediately once
    // the request is accepted, rather than waiting for the full completion.
    // We only need the status code to verify auth + model access.
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let lower = provider_trimmed.to_ascii_lowercase();
    let auth_kind = infer_auth_kind(&provider_trimmed, api_key.trim(), InternalAuthKind::ApiKey);
    let response = if lower == "anthropic" {
        let url = format!("{}/messages", resolved_base);
        let mut req = client
            .post(&url)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");
        req = match auth_kind {
            InternalAuthKind::Authorization => {
                req.header("Authorization", format!("Bearer {}", api_key.trim()))
            }
            InternalAuthKind::ApiKey => req.header("x-api-key", api_key.trim()),
        };
        req.json(&serde_json::json!({
            "model": model_trimmed,
            "max_tokens": 1,
            "stream": true,
            "messages": [{"role": "user", "content": "ping"}]
        }))
        .send()
        .map_err(|e| format!("Provider request failed: {e}"))?
    } else {
        let url = format!("{}/chat/completions", resolved_base);
        let mut req = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key.trim()))
            .header("content-type", "application/json")
            .json(&serde_json::json!({
                "model": model_trimmed,
                "messages": [{"role": "user", "content": "ping"}],
                "max_tokens": 1,
                "stream": true
            }));
        if lower == "openrouter" {
            req = req
                .header("HTTP-Referer", "https://clawpal.zhixian.io")
                .header("X-Title", "ClawPal");
        }
        req.send()
            .map_err(|e| format!("Provider request failed: {e}"))?
    };

    if response.status().is_success() {
        return Ok(());
    }

    let status = response.status().as_u16();
    let body = response
        .text()
        .unwrap_or_else(|e| format!("(could not read response body: {e})"));
    let snippet = truncate_error_text(body.trim(), MAX_ERROR_SNIPPET_CHARS);
    if snippet.is_empty() {
        Err(format!("Provider rejected credentials (HTTP {status})"))
    } else {
        Err(format!(
            "Provider rejected credentials (HTTP {status}): {snippet}"
        ))
    }
}

fn resolve_profile_api_key_with_priority(
    profile: &ModelProfile,
    base_dir: &Path,
) -> Option<(String, u8)> {
    resolve_profile_credential_with_priority(profile, base_dir)
        .map(|(credential, priority)| (credential.secret, priority))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InternalAuthKind {
    ApiKey,
    Authorization,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InternalProviderCredential {
    pub secret: String,
    pub kind: InternalAuthKind,
}

fn infer_auth_kind(provider: &str, secret: &str, fallback: InternalAuthKind) -> InternalAuthKind {
    if provider.trim().eq_ignore_ascii_case("anthropic") {
        let lower = secret.trim().to_ascii_lowercase();
        if lower.starts_with("sk-ant-oat") || lower.starts_with("oauth_") {
            return InternalAuthKind::Authorization;
        }
    }
    fallback
}

fn resolve_profile_credential_with_priority(
    profile: &ModelProfile,
    base_dir: &Path,
) -> Option<(InternalProviderCredential, u8)> {
    // 1. Try auth_ref as env var name directly (e.g. "OPENAI_API_KEY")
    let auth_ref = profile.auth_ref.trim();
    if !auth_ref.is_empty() {
        if let Ok(val) = std::env::var(auth_ref) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                let kind = infer_auth_kind(&profile.provider, trimmed, InternalAuthKind::ApiKey);
                return Some((
                    InternalProviderCredential {
                        secret: trimmed.to_string(),
                        kind,
                    },
                    40,
                ));
            }
        }
    }

    // 2. Look up auth_ref in agent-level auth store files
    //    Keys are stored at: {base_dir}/agents/{agent}/agent/{auth-profiles.json|auth.json}
    if !auth_ref.is_empty() {
        if let Some(credential) = resolve_credential_from_agent_auth_profiles(base_dir, auth_ref) {
            return Some((credential, 30));
        }
    }

    // 3. Direct api_key field (legacy/manual ClawPal input)
    if let Some(ref key) = profile.api_key {
        let trimmed = key.trim();
        if !trimmed.is_empty() {
            let kind = infer_auth_kind(&profile.provider, trimmed, InternalAuthKind::ApiKey);
            return Some((
                InternalProviderCredential {
                    secret: trimmed.to_string(),
                    kind,
                },
                20,
            ));
        }
    }

    // 4. Try common env var naming conventions based on provider
    let provider = profile.provider.trim().to_uppercase().replace('-', "_");
    if !provider.is_empty() {
        for suffix in ["_API_KEY", "_KEY", "_TOKEN"] {
            let env_name = format!("{provider}{suffix}");
            if let Ok(val) = std::env::var(&env_name) {
                let trimmed = val.trim();
                if !trimmed.is_empty() {
                    let fallback_kind = if suffix == "_TOKEN" {
                        InternalAuthKind::Authorization
                    } else {
                        InternalAuthKind::ApiKey
                    };
                    let kind = infer_auth_kind(&profile.provider, trimmed, fallback_kind);
                    return Some((
                        InternalProviderCredential {
                            secret: trimmed.to_string(),
                            kind,
                        },
                        10,
                    ));
                }
            }
        }
    }

    None
}

fn resolve_profile_api_key(profile: &ModelProfile, base_dir: &Path) -> String {
    resolve_profile_api_key_with_priority(profile, base_dir)
        .map(|(key, _)| key)
        .unwrap_or_default()
}

pub(crate) fn collect_provider_credentials_for_internal(
) -> HashMap<String, InternalProviderCredential> {
    let paths = resolve_paths();
    collect_provider_credentials_from_paths(&paths)
}

pub(crate) fn collect_provider_credentials_from_paths(
    paths: &crate::models::OpenClawPaths,
) -> HashMap<String, InternalProviderCredential> {
    let profiles = load_model_profiles(&paths);
    collect_provider_credentials_from_profiles(&profiles, &paths.base_dir)
}

fn collect_provider_credentials_from_profiles(
    profiles: &[ModelProfile],
    base_dir: &Path,
) -> HashMap<String, InternalProviderCredential> {
    let mut out = HashMap::<String, (InternalProviderCredential, u8)>::new();
    for profile in profiles.iter().filter(|p| p.enabled) {
        let Some((credential, priority)) =
            resolve_profile_credential_with_priority(profile, base_dir)
        else {
            continue;
        };
        let provider = profile.provider.trim().to_lowercase();
        match out.get_mut(&provider) {
            Some((existing_credential, existing_priority)) => {
                if priority > *existing_priority {
                    *existing_credential = credential;
                    *existing_priority = priority;
                }
            }
            None => {
                out.insert(provider, (credential, priority));
            }
        }
    }
    out.into_iter().map(|(k, (v, _))| (k, v)).collect()
}

fn resolve_credential_from_agent_auth_profiles(
    base_dir: &Path,
    auth_ref: &str,
) -> Option<InternalProviderCredential> {
    for root in local_openclaw_roots(base_dir) {
        let agents_dir = root.join("agents");
        if !agents_dir.exists() {
            continue;
        }
        let entries = match fs::read_dir(&agents_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let agent_dir = entry.path().join("agent");
            if let Some(credential) =
                resolve_credential_from_local_auth_store_dir(&agent_dir, auth_ref)
            {
                return Some(credential);
            }
        }
    }
    None
}

fn resolve_credential_from_local_auth_store_dir(
    agent_dir: &Path,
    auth_ref: &str,
) -> Option<InternalProviderCredential> {
    for file_name in ["auth-profiles.json", "auth.json"] {
        let auth_file = agent_dir.join(file_name);
        if !auth_file.exists() {
            continue;
        }
        let text = fs::read_to_string(&auth_file).ok()?;
        let data: Value = serde_json::from_str(&text).ok()?;
        if let Some(credential) = resolve_credential_from_auth_store_json(&data, auth_ref) {
            return Some(credential);
        }
    }
    None
}

fn local_openclaw_roots(base_dir: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::<PathBuf>::new();
    let mut seen = std::collections::BTreeSet::<PathBuf>::new();
    let push_root = |roots: &mut Vec<PathBuf>,
                     seen: &mut std::collections::BTreeSet<PathBuf>,
                     root: PathBuf| {
        if seen.insert(root.clone()) {
            roots.push(root);
        }
    };
    push_root(&mut roots, &mut seen, base_dir.to_path_buf());
    let home = dirs::home_dir();
    if let Some(home) = home {
        if let Ok(entries) = fs::read_dir(&home) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if name.starts_with(".openclaw") {
                    push_root(&mut roots, &mut seen, path);
                }
            }
        }
    }
    roots
}

fn auth_ref_lookup_keys(auth_ref: &str) -> Vec<String> {
    let mut out = Vec::new();
    let trimmed = auth_ref.trim();
    if trimmed.is_empty() {
        return out;
    }
    out.push(trimmed.to_string());
    if let Some((provider, _)) = trimmed.split_once(':') {
        if !provider.trim().is_empty() {
            out.push(provider.trim().to_string());
        }
    }
    out
}

fn resolve_key_from_auth_store_json(data: &Value, auth_ref: &str) -> Option<String> {
    resolve_credential_from_auth_store_json(data, auth_ref).map(|credential| credential.secret)
}

fn resolve_credential_from_auth_store_json(
    data: &Value,
    auth_ref: &str,
) -> Option<InternalProviderCredential> {
    let keys = auth_ref_lookup_keys(auth_ref);
    if keys.is_empty() {
        return None;
    }

    if let Some(profiles) = data.get("profiles").and_then(Value::as_object) {
        for key in &keys {
            if let Some(auth_entry) = profiles.get(key) {
                if let Some(credential) = extract_credential_from_auth_entry(auth_entry) {
                    return Some(credential);
                }
            }
        }
    }

    if let Some(root_obj) = data.as_object() {
        for key in &keys {
            if let Some(auth_entry) = root_obj.get(key) {
                if let Some(credential) = extract_credential_from_auth_entry(auth_entry) {
                    return Some(credential);
                }
            }
        }
    }

    None
}

/// Extract the actual key/token from an agent auth-profiles entry.
/// Handles different auth types: token, api_key, oauth.
fn extract_credential_from_auth_entry(entry: &Value) -> Option<InternalProviderCredential> {
    let auth_type = entry
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let provider = entry
        .get("provider")
        .or_else(|| entry.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let kind_from_type = match auth_type.as_str() {
        "oauth" | "token" | "authorization" => Some(InternalAuthKind::Authorization),
        "api_key" | "api-key" | "apikey" => Some(InternalAuthKind::ApiKey),
        _ => None,
    };
    // "token" type → "token" field (e.g. anthropic)
    // "api_key" type → "key" field (e.g. kimi-coding)
    // "oauth" type → "access" field (e.g. minimax-portal, openai-codex)
    for field in ["token", "key", "apiKey", "api_key", "access"] {
        if let Some(val) = entry.get(field).and_then(Value::as_str) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                let fallback_kind = match field {
                    "token" | "access" => InternalAuthKind::Authorization,
                    _ => InternalAuthKind::ApiKey,
                };
                let kind =
                    infer_auth_kind(provider, trimmed, kind_from_type.unwrap_or(fallback_kind));
                return Some(InternalProviderCredential {
                    secret: trimmed.to_string(),
                    kind,
                });
            }
        }
    }
    None
}

fn mask_api_key(key: &str) -> String {
    let key = key.trim();
    if key.is_empty() {
        return "not set".to_string();
    }
    if key.len() <= 8 {
        return "***".to_string();
    }
    let prefix = &key[..4.min(key.len())];
    let suffix = &key[key.len().saturating_sub(4)..];
    format!("{prefix}...{suffix}")
}

fn load_model_profiles(paths: &crate::models::OpenClawPaths) -> Vec<ModelProfile> {
    let path = model_profiles_path(paths);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| r#"{"profiles":[]}"#.to_string());
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Storage {
        Wrapped {
            #[serde(default)]
            profiles: Vec<ModelProfile>,
        },
        Plain(Vec<ModelProfile>),
    }
    match serde_json::from_str::<Storage>(&text).unwrap_or(Storage::Wrapped {
        profiles: Vec::new(),
    }) {
        Storage::Wrapped { profiles } => profiles,
        Storage::Plain(profiles) => profiles,
    }
}

fn save_model_profiles(
    paths: &crate::models::OpenClawPaths,
    profiles: &[ModelProfile],
) -> Result<(), String> {
    let path = model_profiles_path(paths);
    #[derive(serde::Serialize)]
    struct Storage<'a> {
        profiles: &'a [ModelProfile],
        #[serde(rename = "version")]
        version: u8,
    }
    let payload = Storage {
        profiles,
        version: 1,
    };
    let text = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
    crate::config_io::write_text(&path, &text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn sync_profile_auth_to_main_agent_with_source(
    paths: &crate::models::OpenClawPaths,
    profile: &ModelProfile,
    source_base_dir: &Path,
) -> Result<(), String> {
    let resolved_key = resolve_profile_api_key(profile, source_base_dir);
    let api_key = resolved_key.trim();
    if api_key.is_empty() {
        return Ok(());
    }

    let provider = profile.provider.trim();
    if provider.is_empty() {
        return Ok(());
    }
    let auth_ref = profile.auth_ref.trim().to_string();
    let auth_ref = if auth_ref.is_empty() {
        format!("{provider}:default")
    } else {
        auth_ref
    };

    let auth_file = paths
        .base_dir
        .join("agents")
        .join("main")
        .join("agent")
        .join("auth-profiles.json");
    if let Some(parent) = auth_file.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let mut root = fs::read_to_string(&auth_file)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or_else(|| serde_json::json!({ "version": 1 }));

    if !root.is_object() {
        root = serde_json::json!({ "version": 1 });
    }
    let Some(root_obj) = root.as_object_mut() else {
        return Err("failed to prepare auth profile root object".to_string());
    };

    if !root_obj.contains_key("version") {
        root_obj.insert("version".into(), Value::from(1_u64));
    }

    let profiles_val = root_obj
        .entry("profiles".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !profiles_val.is_object() {
        *profiles_val = Value::Object(Map::new());
    }
    if let Some(profiles_map) = profiles_val.as_object_mut() {
        profiles_map.insert(
            auth_ref.clone(),
            serde_json::json!({
                "type": "api_key",
                "provider": provider,
                "key": api_key,
            }),
        );
    }

    let last_good_val = root_obj
        .entry("lastGood".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !last_good_val.is_object() {
        *last_good_val = Value::Object(Map::new());
    }
    if let Some(last_good_map) = last_good_val.as_object_mut() {
        last_good_map.insert(provider.to_string(), Value::String(auth_ref));
    }

    let serialized = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    write_text(&auth_file, &serialized)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&auth_file, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn maybe_sync_main_auth_for_model_value(
    paths: &crate::models::OpenClawPaths,
    model_value: Option<String>,
) -> Result<(), String> {
    let source_base_dir = paths.base_dir.clone();
    maybe_sync_main_auth_for_model_value_with_source(paths, model_value, &source_base_dir)
}

fn maybe_sync_main_auth_for_model_value_with_source(
    paths: &crate::models::OpenClawPaths,
    model_value: Option<String>,
    source_base_dir: &Path,
) -> Result<(), String> {
    let Some(model_value) = model_value else {
        return Ok(());
    };
    let normalized = model_value.trim().to_lowercase();
    if normalized.is_empty() {
        return Ok(());
    }
    let profiles = load_model_profiles(paths);
    for profile in &profiles {
        let profile_model = profile_to_model_value(profile);
        if profile_model.trim().to_lowercase() == normalized {
            return sync_profile_auth_to_main_agent_with_source(paths, profile, source_base_dir);
        }
    }
    Ok(())
}

fn collect_main_auth_model_candidates(cfg: &Value) -> Vec<String> {
    let mut models = Vec::new();
    if let Some(model) = cfg
        .pointer("/agents/defaults/model")
        .and_then(read_model_value)
    {
        models.push(model);
    }
    if let Some(agents) = cfg.pointer("/agents/list").and_then(Value::as_array) {
        for agent in agents {
            let is_main = agent
                .get("id")
                .and_then(Value::as_str)
                .map(|id| id.eq_ignore_ascii_case("main"))
                .unwrap_or(false);
            if !is_main {
                continue;
            }
            if let Some(model) = agent.get("model").and_then(read_model_value) {
                models.push(model);
            }
        }
    }
    models
}

fn sync_main_auth_for_config(
    paths: &crate::models::OpenClawPaths,
    cfg: &Value,
) -> Result<(), String> {
    let source_base_dir = paths.base_dir.clone();
    let mut seen = HashSet::new();
    for model in collect_main_auth_model_candidates(cfg) {
        let normalized = model.trim().to_lowercase();
        if normalized.is_empty() || !seen.insert(normalized) {
            continue;
        }
        maybe_sync_main_auth_for_model_value_with_source(paths, Some(model), &source_base_dir)?;
    }
    Ok(())
}

fn sync_main_auth_for_active_config(paths: &crate::models::OpenClawPaths) -> Result<(), String> {
    let cfg = read_openclaw_config(paths)?;
    sync_main_auth_for_config(paths, &cfg)
}

fn write_config_with_snapshot(
    paths: &crate::models::OpenClawPaths,
    current_text: &str,
    next: &Value,
    source: &str,
) -> Result<(), String> {
    let _ = add_snapshot(
        &paths.history_dir,
        &paths.metadata_path,
        Some(source.to_string()),
        source,
        true,
        current_text,
        None,
    )?;
    write_json(&paths.config_path, next)
}

fn set_nested_value(root: &mut Value, path: &str, value: Option<Value>) -> Result<(), String> {
    let path = path.trim().trim_matches('.');
    if path.is_empty() {
        return Err("invalid path".into());
    }
    let mut cur = root;
    let mut parts = path.split('.').peekable();
    while let Some(part) = parts.next() {
        let is_last = parts.peek().is_none();
        let obj = cur
            .as_object_mut()
            .ok_or_else(|| "path must point to object".to_string())?;
        if is_last {
            if let Some(v) = value {
                obj.insert(part.to_string(), v);
            } else {
                obj.remove(part);
            }
            return Ok(());
        }
        let child = obj
            .entry(part.to_string())
            .or_insert_with(|| Value::Object(Default::default()));
        if !child.is_object() {
            *child = Value::Object(Default::default());
        }
        cur = child;
    }
    unreachable!("path should have at least one segment");
}

fn set_agent_model_value(
    root: &mut Value,
    agent_id: &str,
    model: Option<String>,
) -> Result<(), String> {
    if let Some(agents) = root.pointer_mut("/agents").and_then(Value::as_object_mut) {
        if let Some(list) = agents.get_mut("list").and_then(Value::as_array_mut) {
            for agent in list {
                if agent.get("id").and_then(Value::as_str) == Some(agent_id) {
                    if let Some(agent_obj) = agent.as_object_mut() {
                        match model {
                            Some(v) => {
                                // If existing model is an object, update "primary" inside it
                                if let Some(existing) = agent_obj.get_mut("model") {
                                    if let Some(model_obj) = existing.as_object_mut() {
                                        model_obj.insert("primary".into(), Value::String(v));
                                        return Ok(());
                                    }
                                }
                                agent_obj.insert("model".into(), Value::String(v));
                            }
                            None => {
                                agent_obj.remove("model");
                            }
                        }
                    }
                    return Ok(());
                }
            }
        }
    }
    Err(format!("agent not found: {agent_id}"))
}

fn load_model_catalog(
    paths: &crate::models::OpenClawPaths,
) -> Result<Vec<ModelCatalogProvider>, String> {
    let cache_path = model_catalog_cache_path(paths);
    let current_version = resolve_openclaw_version();
    let cached = read_model_catalog_cache(&cache_path);
    if let Some(selected) = select_catalog_from_cache(cached.as_ref(), &current_version) {
        return Ok(selected);
    }

    if let Some(catalog) = extract_model_catalog_from_cli(paths) {
        if !catalog.is_empty() {
            return Ok(catalog);
        }
    }

    if let Some(previous) = cached {
        if !previous.providers.is_empty() && previous.error.is_none() {
            return Ok(previous.providers);
        }
    }

    Err("Failed to load model catalog from openclaw CLI".into())
}

fn select_catalog_from_cache(
    cached: Option<&ModelCatalogProviderCache>,
    current_version: &str,
) -> Option<Vec<ModelCatalogProvider>> {
    let cache = cached?;
    if cache.cli_version != current_version {
        return None;
    }
    if cache.error.is_some() || cache.providers.is_empty() {
        return None;
    }
    Some(cache.providers.clone())
}

/// Parse CLI output from `openclaw models list --all --json` into grouped providers.
/// Handles various output formats: flat arrays, {models: [...]}, {items: [...]}, {data: [...]}.
/// Strips prefix junk (plugin log lines) before the JSON.
fn parse_model_catalog_from_cli_output(raw: &str) -> Option<Vec<ModelCatalogProvider>> {
    let json_str = clawpal_core::doctor::extract_json_from_output(raw)?;
    let response: Value = serde_json::from_str(json_str).ok()?;
    let models: Vec<Value> = response
        .as_array()
        .map(|values| values.to_vec())
        .or_else(|| {
            response
                .get("models")
                .and_then(Value::as_array)
                .map(|values| values.to_vec())
        })
        .or_else(|| {
            response
                .get("items")
                .and_then(Value::as_array)
                .map(|values| values.to_vec())
        })
        .or_else(|| {
            response
                .get("data")
                .and_then(Value::as_array)
                .map(|values| values.to_vec())
        })
        .unwrap_or_default();
    if models.is_empty() {
        return None;
    }
    let mut providers: BTreeMap<String, ModelCatalogProvider> = BTreeMap::new();
    for model in &models {
        let key = model
            .get("key")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                let provider = model.get("provider").and_then(Value::as_str)?;
                let model_id = model.get("id").and_then(Value::as_str)?;
                Some(format!("{provider}/{model_id}"))
            });
        let key = match key {
            Some(k) => k,
            None => continue,
        };
        let mut parts = key.splitn(2, '/');
        let provider = match parts.next() {
            Some(p) if !p.trim().is_empty() => p.trim().to_lowercase(),
            _ => continue,
        };
        let id = parts.next().unwrap_or("").trim().to_string();
        if id.is_empty() {
            continue;
        }
        let name = model
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| model.get("model").and_then(Value::as_str))
            .or_else(|| model.get("title").and_then(Value::as_str))
            .map(str::to_string);
        let base_url = model
            .get("baseUrl")
            .or_else(|| model.get("base_url"))
            .or_else(|| model.get("apiBase"))
            .or_else(|| model.get("api_base"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                response
                    .get("providers")
                    .and_then(Value::as_object)
                    .and_then(|providers| providers.get(&provider))
                    .and_then(Value::as_object)
                    .and_then(|provider_cfg| {
                        provider_cfg
                            .get("baseUrl")
                            .or_else(|| provider_cfg.get("base_url"))
                            .or_else(|| provider_cfg.get("apiBase"))
                            .or_else(|| provider_cfg.get("api_base"))
                            .and_then(Value::as_str)
                    })
                    .map(str::to_string)
            });
        let entry = providers
            .entry(provider.clone())
            .or_insert(ModelCatalogProvider {
                provider: provider.clone(),
                base_url,
                models: Vec::new(),
            });
        if !entry.models.iter().any(|existing| existing.id == id) {
            entry.models.push(ModelCatalogModel {
                id: id.clone(),
                name: name.clone(),
            });
        }
    }

    if providers.is_empty() {
        return None;
    }

    let mut out: Vec<ModelCatalogProvider> = providers.into_values().collect();
    for provider in &mut out {
        provider.models.sort_by(|a, b| a.id.cmp(&b.id));
    }
    out.sort_by(|a, b| a.provider.cmp(&b.provider));
    Some(out)
}

fn extract_model_catalog_from_cli(
    paths: &crate::models::OpenClawPaths,
) -> Option<Vec<ModelCatalogProvider>> {
    let output = run_openclaw_raw(&["models", "list", "--all", "--json", "--no-color"]).ok()?;
    if output.stdout.trim().is_empty() {
        return None;
    }

    let out = parse_model_catalog_from_cli_output(&output.stdout)?;
    let _ = cache_model_catalog(paths, out.clone());
    Some(out)
}

fn cache_model_catalog(
    paths: &crate::models::OpenClawPaths,
    providers: Vec<ModelCatalogProvider>,
) -> Option<()> {
    let cache_path = model_catalog_cache_path(paths);
    let now = unix_timestamp_secs();
    let cache = ModelCatalogProviderCache {
        cli_version: resolve_openclaw_version(),
        updated_at: now,
        providers,
        source: "openclaw models list --all --json".into(),
        error: None,
    };
    let _ = save_model_catalog_cache(&cache_path, &cache);
    Some(())
}

#[cfg(test)]
mod model_catalog_cache_tests {
    use super::*;

    #[test]
    fn test_select_cached_catalog_same_version() {
        let cached = ModelCatalogProviderCache {
            cli_version: "1.2.3".into(),
            updated_at: 123,
            providers: vec![ModelCatalogProvider {
                provider: "openrouter".into(),
                base_url: None,
                models: vec![ModelCatalogModel {
                    id: "moonshotai/kimi-k2.5".into(),
                    name: Some("Kimi".into()),
                }],
            }],
            source: "openclaw models list --all --json".into(),
            error: None,
        };
        let selected = select_catalog_from_cache(Some(&cached), "1.2.3");
        assert!(selected.is_some(), "same version should use cache");
    }

    #[test]
    fn test_select_cached_catalog_version_mismatch_requires_refresh() {
        let cached = ModelCatalogProviderCache {
            cli_version: "1.2.2".into(),
            updated_at: 123,
            providers: vec![ModelCatalogProvider {
                provider: "openrouter".into(),
                base_url: None,
                models: vec![ModelCatalogModel {
                    id: "moonshotai/kimi-k2.5".into(),
                    name: Some("Kimi".into()),
                }],
            }],
            source: "openclaw models list --all --json".into(),
            error: None,
        };
        let selected = select_catalog_from_cache(Some(&cached), "1.2.3");
        assert!(
            selected.is_none(),
            "version mismatch must force CLI refresh"
        );
    }
}

#[cfg(test)]
mod model_value_tests {
    use super::*;

    fn profile(provider: &str, model: &str) -> ModelProfile {
        ModelProfile {
            id: "p1".into(),
            name: "p".into(),
            provider: provider.into(),
            model: model.into(),
            auth_ref: "".into(),
            api_key: None,
            base_url: None,
            description: None,
            enabled: true,
        }
    }

    #[test]
    fn test_profile_to_model_value_keeps_provider_prefix_for_nested_model_id() {
        let p = profile("openrouter", "moonshotai/kimi-k2.5");
        assert_eq!(
            profile_to_model_value(&p),
            "openrouter/moonshotai/kimi-k2.5",
        );
    }
}

#[cfg(test)]
mod rescue_bot_tests {
    use super::*;

    #[test]
    fn test_suggest_rescue_port_prefers_large_gap() {
        assert_eq!(clawpal_core::doctor::suggest_rescue_port(18789), 19789);
    }

    #[test]
    fn test_ensure_rescue_port_spacing_rejects_small_gap() {
        let err = clawpal_core::doctor::ensure_rescue_port_spacing(18789, 18800).unwrap_err();
        assert!(err.contains(">= +20"));
    }

    #[test]
    fn test_build_rescue_bot_command_plan_for_activate() {
        let commands =
            build_rescue_bot_command_plan(RescueBotAction::Activate, "rescue", 19789, true);
        let expected = vec![
            vec!["--profile", "rescue", "setup"],
            vec![
                "--profile",
                "rescue",
                "config",
                "set",
                "gateway.port",
                "19789",
                "--json",
            ],
            vec!["--profile", "rescue", "gateway", "install"],
            vec!["--profile", "rescue", "gateway", "restart"],
            vec![
                "--profile",
                "rescue",
                "gateway",
                "status",
                "--no-probe",
                "--json",
            ],
        ]
        .into_iter()
        .map(|items| items.into_iter().map(String::from).collect::<Vec<_>>())
        .collect::<Vec<_>>();
        assert_eq!(commands, expected);
    }

    #[test]
    fn test_build_rescue_bot_command_plan_for_activate_without_reconfigure() {
        let commands =
            build_rescue_bot_command_plan(RescueBotAction::Activate, "rescue", 19789, false);
        let expected = vec![
            vec!["--profile", "rescue", "gateway", "install"],
            vec!["--profile", "rescue", "gateway", "restart"],
            vec![
                "--profile",
                "rescue",
                "gateway",
                "status",
                "--no-probe",
                "--json",
            ],
        ]
        .into_iter()
        .map(|items| items.into_iter().map(String::from).collect::<Vec<_>>())
        .collect::<Vec<_>>();
        assert_eq!(commands, expected);
    }

    #[test]
    fn test_build_rescue_bot_command_plan_for_unset() {
        let commands =
            build_rescue_bot_command_plan(RescueBotAction::Unset, "rescue", 19789, false);
        let expected = vec![
            vec!["--profile", "rescue", "gateway", "stop"],
            vec!["--profile", "rescue", "gateway", "uninstall"],
            vec!["--profile", "rescue", "config", "unset", "gateway.port"],
        ]
        .into_iter()
        .map(|items| items.into_iter().map(String::from).collect::<Vec<_>>())
        .collect::<Vec<_>>();
        assert_eq!(commands, expected);
    }

    #[test]
    fn test_parse_rescue_bot_action_unset_aliases() {
        assert_eq!(
            RescueBotAction::parse("unset").unwrap(),
            RescueBotAction::Unset
        );
        assert_eq!(
            RescueBotAction::parse("remove").unwrap(),
            RescueBotAction::Unset
        );
        assert_eq!(
            RescueBotAction::parse("delete").unwrap(),
            RescueBotAction::Unset
        );
    }

    #[test]
    fn test_is_rescue_cleanup_noop_matches_stop_not_running() {
        let output = OpenclawCommandOutput {
            stdout: String::new(),
            stderr: "Gateway is not running".into(),
            exit_code: 1,
        };
        let command = vec![
            "--profile".to_string(),
            "rescue".to_string(),
            "gateway".to_string(),
            "stop".to_string(),
        ];
        assert!(is_rescue_cleanup_noop(
            RescueBotAction::Deactivate,
            &command,
            &output
        ));
    }

    #[test]
    fn test_is_rescue_cleanup_noop_matches_unset_missing_key() {
        let output = OpenclawCommandOutput {
            stdout: String::new(),
            stderr: "config key gateway.port not found".into(),
            exit_code: 1,
        };
        let command = vec![
            "--profile".to_string(),
            "rescue".to_string(),
            "config".to_string(),
            "unset".to_string(),
            "gateway.port".to_string(),
        ];
        assert!(is_rescue_cleanup_noop(
            RescueBotAction::Unset,
            &command,
            &output
        ));
    }

    #[test]
    fn test_is_gateway_restart_timeout_matches_health_check_timeout() {
        let output = OpenclawCommandOutput {
            stdout: String::new(),
            stderr: "Gateway restart timed out after 60s waiting for health checks.".into(),
            exit_code: 1,
        };
        assert!(clawpal_core::doctor::gateway_restart_timeout(
            &output.stderr,
            &output.stdout
        ));
    }

    #[test]
    fn test_is_gateway_restart_timeout_ignores_other_errors() {
        let output = OpenclawCommandOutput {
            stdout: String::new(),
            stderr: "gateway start failed: address already in use".into(),
            exit_code: 1,
        };
        assert!(!clawpal_core::doctor::gateway_restart_timeout(
            &output.stderr,
            &output.stdout
        ));
    }

    #[test]
    fn test_doctor_json_option_unsupported_matches_unknown_option() {
        let output = OpenclawCommandOutput {
            stdout: String::new(),
            stderr: "error: unknown option '--json'".into(),
            exit_code: 1,
        };
        assert!(clawpal_core::doctor::doctor_json_option_unsupported(
            &output.stderr,
            &output.stdout
        ));
    }

    #[test]
    fn test_doctor_json_option_unsupported_ignores_other_failures() {
        let output = OpenclawCommandOutput {
            stdout: String::new(),
            stderr: "doctor command failed to connect".into(),
            exit_code: 1,
        };
        assert!(!clawpal_core::doctor::doctor_json_option_unsupported(
            &output.stderr,
            &output.stdout
        ));
    }

    #[test]
    fn test_parse_doctor_issues_reads_camel_case_fields() {
        let report = serde_json::json!({
            "issues": [
                {
                    "id": "primary.test",
                    "code": "primary.test",
                    "severity": "warn",
                    "message": "test issue",
                    "autoFixable": true,
                    "fixHint": "do thing"
                }
            ]
        });
        let issues = clawpal_core::doctor::parse_doctor_issues(&report, "primary");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, "primary.test");
        assert_eq!(issues[0].severity, "warn");
        assert!(issues[0].auto_fixable);
        assert_eq!(issues[0].fix_hint.as_deref(), Some("do thing"));
    }

    #[test]
    fn test_extract_json_from_output_uses_trailing_balanced_payload() {
        let raw = "[plugins] warmup cache\n[warn] using fallback transport\n{\"ok\":false,\"issues\":[{\"id\":\"x\"}]}";
        let json = clawpal_core::doctor::extract_json_from_output(raw).unwrap();
        assert_eq!(json, "{\"ok\":false,\"issues\":[{\"id\":\"x\"}]}");
    }

    #[test]
    fn test_parse_json_loose_handles_leading_bracketed_logs() {
        let raw = "[plugins] warmup cache\n[warn] using fallback transport\n{\"running\":false,\"healthy\":false}";
        let parsed =
            clawpal_core::doctor::parse_json_loose(raw).expect("expected trailing JSON payload");
        assert_eq!(parsed.get("running").and_then(Value::as_bool), Some(false));
        assert_eq!(parsed.get("healthy").and_then(Value::as_bool), Some(false));
    }

    #[test]
    fn test_classify_doctor_issue_status_prioritizes_error() {
        let issues = vec![
            RescuePrimaryIssue {
                id: "a".into(),
                code: "a".into(),
                severity: "warn".into(),
                message: "warn".into(),
                auto_fixable: false,
                fix_hint: None,
                source: "primary".into(),
            },
            RescuePrimaryIssue {
                id: "b".into(),
                code: "b".into(),
                severity: "error".into(),
                message: "error".into(),
                auto_fixable: false,
                fix_hint: None,
                source: "primary".into(),
            },
        ];
        let core: Vec<clawpal_core::doctor::DoctorIssue> = issues
            .into_iter()
            .map(|issue| clawpal_core::doctor::DoctorIssue {
                id: issue.id,
                code: issue.code,
                severity: issue.severity,
                message: issue.message,
                auto_fixable: issue.auto_fixable,
                fix_hint: issue.fix_hint,
                source: issue.source,
            })
            .collect();
        assert_eq!(
            clawpal_core::doctor::classify_doctor_issue_status(&core),
            "broken"
        );
    }

    #[test]
    fn test_collect_safe_primary_issue_ids_filters_non_primary_and_non_fixable() {
        let diagnosis = RescuePrimaryDiagnosisResult {
            status: "degraded".into(),
            checked_at: "2026-02-25T00:00:00Z".into(),
            target_profile: "primary".into(),
            rescue_profile: "rescue".into(),
            rescue_configured: true,
            rescue_port: Some(19789),
            checks: Vec::new(),
            issues: vec![
                RescuePrimaryIssue {
                    id: "field.agents".into(),
                    code: "required.field".into(),
                    severity: "warn".into(),
                    message: "missing agents".into(),
                    auto_fixable: true,
                    fix_hint: None,
                    source: "primary".into(),
                },
                RescuePrimaryIssue {
                    id: "field.port".into(),
                    code: "invalid.port".into(),
                    severity: "error".into(),
                    message: "port invalid".into(),
                    auto_fixable: false,
                    fix_hint: None,
                    source: "primary".into(),
                },
                RescuePrimaryIssue {
                    id: "rescue.gateway.unhealthy".into(),
                    code: "rescue.gateway.unhealthy".into(),
                    severity: "warn".into(),
                    message: "rescue unhealthy".into(),
                    auto_fixable: true,
                    fix_hint: None,
                    source: "rescue".into(),
                },
            ],
        };

        let (selected, skipped) = collect_safe_primary_issue_ids(
            &diagnosis,
            &[
                "field.agents".into(),
                "field.port".into(),
                "rescue.gateway.unhealthy".into(),
            ],
        );
        assert_eq!(selected, vec!["field.agents"]);
        assert_eq!(skipped, vec!["field.port", "rescue.gateway.unhealthy"]);
    }

    #[test]
    fn test_build_primary_issue_fix_command_for_field_port() {
        let (_, command) = build_primary_issue_fix_command("primary", "field.port")
            .expect("field.port should have safe fix command");
        assert_eq!(
            command,
            vec![
                "--profile",
                "primary",
                "config",
                "set",
                "gateway.port",
                "18789",
                "--json"
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod model_profile_upsert_tests {
    use super::*;
    use std::path::PathBuf;

    fn mk_profile(
        id: &str,
        provider: &str,
        model: &str,
        auth_ref: &str,
        api_key: Option<&str>,
    ) -> ModelProfile {
        ModelProfile {
            id: id.to_string(),
            name: format!("{provider}/{model}"),
            provider: provider.to_string(),
            model: model.to_string(),
            auth_ref: auth_ref.to_string(),
            api_key: api_key.map(str::to_string),
            base_url: None,
            description: None,
            enabled: true,
        }
    }

    fn mk_paths(base_dir: PathBuf, clawpal_dir: PathBuf) -> crate::models::OpenClawPaths {
        crate::models::OpenClawPaths {
            openclaw_dir: base_dir.clone(),
            config_path: base_dir.join("openclaw.json"),
            base_dir,
            history_dir: clawpal_dir.join("history"),
            metadata_path: clawpal_dir.join("metadata.json"),
            clawpal_dir,
        }
    }

    #[test]
    fn preserve_existing_auth_fields_on_edit_when_payload_is_blank() {
        let profiles = vec![mk_profile(
            "p-1",
            "kimi-coding",
            "k2p5",
            "kimi-coding:default",
            Some("sk-old"),
        )];
        let incoming = mk_profile("p-1", "kimi-coding", "k2.5", "", None);
        let content = serde_json::json!({ "profiles": profiles, "version": 1 }).to_string();
        let (persisted, next_json) =
            clawpal_core::profile::upsert_profile_in_storage_json(&content, incoming)
                .expect("upsert");
        assert_eq!(persisted.api_key.as_deref(), Some("sk-old"));
        assert_eq!(persisted.auth_ref, "kimi-coding:default");
        let next_profiles = clawpal_core::profile::list_profiles_from_storage_json(&next_json);
        assert_eq!(next_profiles[0].model, "k2.5");
    }

    #[test]
    fn reuse_provider_credentials_for_new_profile_when_missing() {
        let donor = mk_profile(
            "p-donor",
            "openrouter",
            "model-a",
            "openrouter:default",
            Some("sk-donor"),
        );
        let incoming = mk_profile("", "openrouter", "model-b", "", None);
        let content = serde_json::json!({ "profiles": [donor], "version": 1 }).to_string();
        let (saved, _) = clawpal_core::profile::upsert_profile_in_storage_json(&content, incoming)
            .expect("upsert");
        assert_eq!(saved.auth_ref, "openrouter:default");
        assert_eq!(saved.api_key.as_deref(), Some("sk-donor"));
    }

    #[test]
    fn sync_auth_can_copy_key_from_auth_ref_source_store() {
        let tmp_root =
            std::env::temp_dir().join(format!("clawpal-auth-sync-{}", uuid::Uuid::new_v4()));
        let source_base = tmp_root.join("source-openclaw");
        let target_base = tmp_root.join("target-openclaw");
        let clawpal_dir = tmp_root.join("clawpal");
        let source_auth_file = source_base
            .join("agents")
            .join("main")
            .join("agent")
            .join("auth-profiles.json");
        let target_auth_file = target_base
            .join("agents")
            .join("main")
            .join("agent")
            .join("auth-profiles.json");

        fs::create_dir_all(source_auth_file.parent().unwrap()).expect("create source auth dir");
        let source_payload = serde_json::json!({
            "version": 1,
            "profiles": {
                "kimi-coding:default": {
                    "type": "api_key",
                    "provider": "kimi-coding",
                    "key": "sk-from-source-store"
                }
            }
        });
        write_text(
            &source_auth_file,
            &serde_json::to_string_pretty(&source_payload).expect("serialize source payload"),
        )
        .expect("write source auth");

        let paths = mk_paths(target_base, clawpal_dir);
        let profile = mk_profile("p1", "kimi-coding", "k2p5", "kimi-coding:default", None);
        sync_profile_auth_to_main_agent_with_source(&paths, &profile, &source_base)
            .expect("sync auth");

        let target_text = fs::read_to_string(target_auth_file).expect("read target auth");
        let target_json: Value = serde_json::from_str(&target_text).expect("parse target auth");
        let key = target_json
            .pointer("/profiles/kimi-coding:default/key")
            .and_then(Value::as_str);
        assert_eq!(key, Some("sk-from-source-store"));

        let _ = fs::remove_dir_all(tmp_root);
    }

    #[test]
    fn resolve_key_from_auth_store_json_supports_wrapped_and_legacy_formats() {
        let wrapped = serde_json::json!({
            "version": 1,
            "profiles": {
                "kimi-coding:default": {
                    "type": "api_key",
                    "provider": "kimi-coding",
                    "key": "sk-wrapped"
                }
            }
        });
        assert_eq!(
            resolve_key_from_auth_store_json(&wrapped, "kimi-coding:default"),
            Some("sk-wrapped".to_string())
        );

        let legacy = serde_json::json!({
            "kimi-coding": {
                "type": "api_key",
                "provider": "kimi-coding",
                "key": "sk-legacy"
            }
        });
        assert_eq!(
            resolve_key_from_auth_store_json(&legacy, "kimi-coding:default"),
            Some("sk-legacy".to_string())
        );
    }

    #[test]
    fn resolve_key_from_local_auth_store_dir_reads_auth_json_when_profiles_file_missing() {
        let tmp_root =
            std::env::temp_dir().join(format!("clawpal-auth-store-test-{}", uuid::Uuid::new_v4()));
        let agent_dir = tmp_root.join("agents").join("main").join("agent");
        fs::create_dir_all(&agent_dir).expect("create agent dir");
        let legacy_auth = serde_json::json!({
            "openai": {
                "type": "api_key",
                "provider": "openai",
                "key": "sk-openai-legacy"
            }
        });
        write_text(
            &agent_dir.join("auth.json"),
            &serde_json::to_string_pretty(&legacy_auth).expect("serialize legacy auth"),
        )
        .expect("write auth.json");

        let resolved = resolve_credential_from_local_auth_store_dir(&agent_dir, "openai:default");
        assert_eq!(
            resolved.map(|credential| credential.secret),
            Some("sk-openai-legacy".to_string())
        );
        let _ = fs::remove_dir_all(tmp_root);
    }

    #[test]
    fn resolve_profile_api_key_prefers_auth_ref_store_over_direct_api_key() {
        let tmp_root =
            std::env::temp_dir().join(format!("clawpal-auth-priority-{}", uuid::Uuid::new_v4()));
        let base_dir = tmp_root.join("openclaw");
        let auth_file = base_dir
            .join("agents")
            .join("main")
            .join("agent")
            .join("auth-profiles.json");
        fs::create_dir_all(auth_file.parent().expect("auth parent")).expect("create auth dir");
        let payload = serde_json::json!({
            "version": 1,
            "profiles": {
                "anthropic:default": {
                    "type": "token",
                    "provider": "anthropic",
                    "token": "sk-anthropic-from-store"
                }
            }
        });
        write_text(
            &auth_file,
            &serde_json::to_string_pretty(&payload).expect("serialize payload"),
        )
        .expect("write auth payload");

        let profile = mk_profile(
            "p-anthropic",
            "anthropic",
            "claude-opus-4-5",
            "anthropic:default",
            Some("sk-stale-direct"),
        );
        let resolved = resolve_profile_api_key(&profile, &base_dir);
        assert_eq!(resolved, "sk-anthropic-from-store");
        let _ = fs::remove_dir_all(tmp_root);
    }

    #[test]
    fn collect_provider_api_keys_prefers_higher_priority_source_for_same_provider() {
        let tmp_root = std::env::temp_dir().join(format!(
            "clawpal-provider-key-priority-{}",
            uuid::Uuid::new_v4()
        ));
        let base_dir = tmp_root.join("openclaw");
        let auth_file = base_dir
            .join("agents")
            .join("main")
            .join("agent")
            .join("auth-profiles.json");
        fs::create_dir_all(auth_file.parent().expect("auth parent")).expect("create auth dir");
        let payload = serde_json::json!({
            "version": 1,
            "profiles": {
                "anthropic:default": {
                    "type": "token",
                    "provider": "anthropic",
                    "token": "sk-anthropic-good"
                }
            }
        });
        write_text(
            &auth_file,
            &serde_json::to_string_pretty(&payload).expect("serialize payload"),
        )
        .expect("write auth payload");
        let stale = mk_profile(
            "anthropic-stale",
            "anthropic",
            "claude-opus-4-5",
            "",
            Some("sk-anthropic-stale"),
        );
        let preferred = mk_profile(
            "anthropic-ref",
            "anthropic",
            "claude-opus-4-6",
            "anthropic:default",
            None,
        );
        let creds = collect_provider_credentials_from_profiles(
            &[stale.clone(), preferred.clone()],
            &base_dir,
        );
        let anthropic = creds
            .get("anthropic")
            .expect("anthropic credential should exist");
        assert_eq!(anthropic.secret, "sk-anthropic-good");
        assert_eq!(anthropic.kind, InternalAuthKind::Authorization);
        let _ = fs::remove_dir_all(tmp_root);
    }

    #[test]
    fn collect_main_auth_candidates_prefers_defaults_and_main_agent() {
        let cfg = serde_json::json!({
            "agents": {
                "defaults": {
                    "model": { "primary": "kimi-coding/k2p5" }
                },
                "list": [
                    { "id": "main", "model": "anthropic/claude-opus-4-6" },
                    { "id": "worker", "model": "openai/gpt-4.1" }
                ]
            }
        });
        let models = collect_main_auth_model_candidates(&cfg);
        assert_eq!(
            models,
            vec![
                "kimi-coding/k2p5".to_string(),
                "anthropic/claude-opus-4-6".to_string(),
            ]
        );
    }
}

fn collect_channel_nodes(cfg: &Value) -> Vec<ChannelNode> {
    let mut out = Vec::new();
    if let Some(channels) = cfg.get("channels") {
        walk_channel_nodes("channels", channels, &mut out);
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn walk_channel_nodes(prefix: &str, node: &Value, out: &mut Vec<ChannelNode>) {
    let Some(obj) = node.as_object() else {
        return;
    };

    if is_channel_like_node(prefix, obj) {
        let channel_type = resolve_channel_type(prefix, obj);
        let mode = resolve_channel_mode(obj);
        let allowlist = collect_channel_allowlist(obj);
        let has_model_field = obj.contains_key("model");
        let model = obj.get("model").and_then(read_model_value);
        out.push(ChannelNode {
            path: prefix.to_string(),
            channel_type,
            mode,
            allowlist,
            model,
            has_model_field,
            display_name: None,
            name_status: None,
        });
    }

    for (key, child) in obj {
        if key == "allowlist" || key == "model" || key == "mode" {
            continue;
        }
        if let Value::Object(_) = child {
            walk_channel_nodes(&format!("{prefix}.{key}"), child, out);
        }
    }
}

fn enrich_channel_display_names(
    paths: &crate::models::OpenClawPaths,
    cfg: &Value,
    nodes: &mut [ChannelNode],
) -> Result<(), String> {
    let mut grouped: BTreeMap<String, Vec<(usize, String, String)>> = BTreeMap::new();
    let mut local_names: Vec<(usize, String)> = Vec::new();

    for (index, node) in nodes.iter().enumerate() {
        if let Some((plugin, identifier, kind)) = resolve_channel_node_identity(cfg, node) {
            grouped
                .entry(plugin)
                .or_default()
                .push((index, identifier, kind));
        }
        if node.display_name.is_none() {
            if let Some(local_name) = channel_node_local_name(cfg, &node.path) {
                local_names.push((index, local_name));
            }
        }
    }
    for (index, local_name) in local_names {
        if let Some(node) = nodes.get_mut(index) {
            node.display_name = Some(local_name);
            node.name_status = Some("local".into());
        }
    }

    let cache_file = paths.clawpal_dir.join("channel-name-cache.json");
    if nodes.is_empty() {
        if cache_file.exists() {
            let _ = fs::remove_file(&cache_file);
        }
        return Ok(());
    }

    for (plugin, entries) in grouped {
        if entries.is_empty() {
            continue;
        }
        let ids: Vec<String> = entries
            .iter()
            .map(|(_, identifier, _)| identifier.clone())
            .collect();
        let kind = &entries[0].2;
        let mut args = vec![
            "channels".to_string(),
            "resolve".to_string(),
            "--json".to_string(),
            "--channel".to_string(),
            plugin.clone(),
            "--kind".to_string(),
            kind.clone(),
        ];
        for entry in &ids {
            args.push(entry.clone());
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = match run_openclaw_raw(&args) {
            Ok(output) => output,
            Err(_) => {
                for (index, _, _) in entries {
                    nodes[index].name_status = Some("resolve failed".into());
                }
                continue;
            }
        };
        if output.stdout.trim().is_empty() {
            for (index, _, _) in entries {
                nodes[index].name_status = Some("unresolved".into());
            }
            continue;
        }
        let json_str =
            clawpal_core::doctor::extract_json_from_output(&output.stdout).unwrap_or("[]");
        let parsed: Vec<Value> = serde_json::from_str(json_str).unwrap_or_default();
        let mut name_map = HashMap::new();
        for item in parsed {
            let input = item
                .get("input")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let resolved = item
                .get("resolved")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let note = item
                .get("note")
                .and_then(Value::as_str)
                .map(|value| value.to_string());
            if !input.is_empty() {
                name_map.insert(input, (resolved, name, note));
            }
        }

        for (index, identifier, _) in entries {
            if let Some((resolved, name, note)) = name_map.get(&identifier) {
                if *resolved {
                    if let Some(name) = name {
                        nodes[index].display_name = Some(name.clone());
                        nodes[index].name_status = Some("resolved".into());
                    } else {
                        nodes[index].name_status = Some("resolved".into());
                    }
                } else if let Some(note) = note {
                    nodes[index].name_status = Some(note.clone());
                } else {
                    nodes[index].name_status = Some("unresolved".into());
                }
            } else {
                nodes[index].name_status = Some("unresolved".into());
            }
        }
    }

    let _ = save_json_cache(&cache_file, nodes);
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct ChannelNameCacheEntry {
    path: String,
    display_name: Option<String>,
    name_status: Option<String>,
}

fn save_json_cache(cache_file: &Path, nodes: &[ChannelNode]) -> Result<(), String> {
    let payload: Vec<ChannelNameCacheEntry> = nodes
        .iter()
        .map(|node| ChannelNameCacheEntry {
            path: node.path.clone(),
            display_name: node.display_name.clone(),
            name_status: node.name_status.clone(),
        })
        .collect();
    write_text(
        cache_file,
        &serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?,
    )
}

fn resolve_channel_node_identity(
    cfg: &Value,
    node: &ChannelNode,
) -> Option<(String, String, String)> {
    let parts: Vec<&str> = node.path.split('.').collect();
    if parts.len() < 2 || parts[0] != "channels" {
        return None;
    }
    let plugin = parts[1].to_string();
    let identifier = channel_last_segment(node.path.as_str())?;
    let config_node = channel_lookup_node(cfg, &node.path);
    let kind = if node.channel_type.as_deref() == Some("dm") || node.path.ends_with(".dm") {
        "user".to_string()
    } else if config_node
        .and_then(|value| {
            value
                .get("users")
                .or(value.get("members"))
                .or_else(|| value.get("peerIds"))
        })
        .is_some()
    {
        "user".to_string()
    } else {
        "group".to_string()
    };
    Some((plugin, identifier, kind))
}

fn channel_last_segment(path: &str) -> Option<String> {
    path.split('.').next_back().map(|value| value.to_string())
}

fn channel_node_local_name(cfg: &Value, path: &str) -> Option<String> {
    channel_lookup_node(cfg, path).and_then(|node| {
        if let Some(slug) = node.get("slug").and_then(Value::as_str) {
            let trimmed = slug.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        if let Some(name) = node.get("name").and_then(Value::as_str) {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        None
    })
}

fn channel_lookup_node<'a>(cfg: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = cfg;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn is_channel_like_node(prefix: &str, obj: &serde_json::Map<String, Value>) -> bool {
    if prefix == "channels" {
        return false;
    }
    if obj.contains_key("model")
        || obj.contains_key("type")
        || obj.contains_key("mode")
        || obj.contains_key("policy")
        || obj.contains_key("allowlist")
        || obj.contains_key("allowFrom")
        || obj.contains_key("groupAllowFrom")
        || obj.contains_key("dmPolicy")
        || obj.contains_key("groupPolicy")
        || obj.contains_key("guilds")
        || obj.contains_key("accounts")
        || obj.contains_key("dm")
        || obj.contains_key("users")
        || obj.contains_key("enabled")
        || obj.contains_key("token")
        || obj.contains_key("botToken")
    {
        return true;
    }
    if prefix.contains(".accounts.") || prefix.contains(".guilds.") || prefix.contains(".channels.")
    {
        return true;
    }
    if prefix.ends_with(".dm") || prefix.ends_with(".default") {
        return true;
    }
    false
}

fn resolve_channel_type(prefix: &str, obj: &serde_json::Map<String, Value>) -> Option<String> {
    obj.get("type")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            if prefix.ends_with(".dm") {
                Some("dm".into())
            } else if prefix.contains(".accounts.") {
                Some("account".into())
            } else if prefix.contains(".channels.") && prefix.contains(".guilds.") {
                Some("channel".into())
            } else if prefix.contains(".guilds.") {
                Some("guild".into())
            } else if obj.contains_key("guilds") {
                Some("platform".into())
            } else if obj.contains_key("accounts") {
                Some("platform".into())
            } else {
                None
            }
        })
}

fn resolve_channel_mode(obj: &serde_json::Map<String, Value>) -> Option<String> {
    let mut modes: Vec<String> = Vec::new();
    if let Some(v) = obj.get("mode").and_then(Value::as_str) {
        modes.push(v.to_string());
    }
    if let Some(v) = obj.get("policy").and_then(Value::as_str) {
        if !modes.iter().any(|m| m == v) {
            modes.push(v.to_string());
        }
    }
    if let Some(v) = obj.get("dmPolicy").and_then(Value::as_str) {
        if !modes.iter().any(|m| m == v) {
            modes.push(v.to_string());
        }
    }
    if let Some(v) = obj.get("groupPolicy").and_then(Value::as_str) {
        if !modes.iter().any(|m| m == v) {
            modes.push(v.to_string());
        }
    }
    if modes.is_empty() {
        None
    } else {
        Some(modes.join(" / "))
    }
}

fn collect_channel_allowlist(obj: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut uniq = HashSet::<String>::new();
    for key in ["allowlist", "allowFrom", "groupAllowFrom"] {
        if let Some(values) = obj.get(key).and_then(Value::as_array) {
            for value in values.iter().filter_map(Value::as_str) {
                let next = value.to_string();
                if uniq.insert(next.clone()) {
                    out.push(next);
                }
            }
        }
    }
    if let Some(values) = obj.get("users").and_then(Value::as_array) {
        for value in values.iter().filter_map(Value::as_str) {
            let next = value.to_string();
            if uniq.insert(next.clone()) {
                out.push(next);
            }
        }
    }
    out
}

fn collect_agent_ids(cfg: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(agents) = cfg
        .get("agents")
        .and_then(|v| v.get("list"))
        .and_then(Value::as_array)
    {
        for agent in agents {
            if let Some(id) = agent.get("id").and_then(Value::as_str) {
                ids.push(id.to_string());
            }
        }
    }
    // Implicit "main" agent when no agents.list
    if ids.is_empty() {
        ids.push("main".into());
    }
    ids
}

fn collect_model_bindings(cfg: &Value, profiles: &[ModelProfile]) -> Vec<ModelBinding> {
    let mut out = Vec::new();
    let global = cfg
        .pointer("/agents/defaults/model")
        .or_else(|| cfg.pointer("/agents/default/model"))
        .and_then(read_model_value);
    out.push(ModelBinding {
        scope: "global".into(),
        scope_id: "global".into(),
        model_profile_id: find_profile_by_model(profiles, global.as_deref()),
        model_value: global,
        path: Some("agents.defaults.model".into()),
    });

    if let Some(agents) = cfg
        .get("agents")
        .and_then(|v| v.get("list"))
        .and_then(Value::as_array)
    {
        for agent in agents {
            let id = agent.get("id").and_then(Value::as_str).unwrap_or("agent");
            let model = agent.get("model").and_then(read_model_value);
            out.push(ModelBinding {
                scope: "agent".into(),
                scope_id: id.to_string(),
                model_profile_id: find_profile_by_model(profiles, model.as_deref()),
                model_value: model,
                path: Some(format!("agents.list.{id}.model")),
            });
        }
    }

    fn walk_channel_binding(
        prefix: &str,
        node: &Value,
        out: &mut Vec<ModelBinding>,
        profiles: &[ModelProfile],
    ) {
        if let Some(obj) = node.as_object() {
            if let Some(model) = obj.get("model").and_then(read_model_value) {
                out.push(ModelBinding {
                    scope: "channel".into(),
                    scope_id: prefix.to_string(),
                    model_profile_id: find_profile_by_model(profiles, Some(&model)),
                    model_value: Some(model),
                    path: Some(format!("{}.model", prefix)),
                });
            }
            for (k, child) in obj {
                if let Value::Object(_) = child {
                    walk_channel_binding(&format!("{}.{}", prefix, k), child, out, profiles);
                }
            }
        }
    }

    if let Some(channels) = cfg.get("channels") {
        walk_channel_binding("channels", channels, &mut out, profiles);
    }

    out
}

fn find_profile_by_model(profiles: &[ModelProfile], value: Option<&str>) -> Option<String> {
    let value = value?;
    let normalized = normalize_model_ref(value);
    for profile in profiles {
        if normalize_model_ref(&profile_to_model_value(profile)) == normalized
            || normalize_model_ref(&profile.model) == normalized
        {
            return Some(profile.id.clone());
        }
    }
    None
}

fn resolve_auth_ref_for_provider(cfg: &Value, provider: &str) -> Option<String> {
    let provider = provider.trim().to_lowercase();
    if provider.is_empty() {
        return None;
    }
    if let Some(auth_profiles) = cfg.pointer("/auth/profiles").and_then(Value::as_object) {
        let mut fallback = None;
        for (profile_id, profile) in auth_profiles {
            let entry_provider = profile.get("provider").or_else(|| profile.get("name"));
            if let Some(entry_provider) = entry_provider.and_then(Value::as_str) {
                if entry_provider.trim().eq_ignore_ascii_case(&provider) {
                    if profile_id.ends_with(":default") {
                        return Some(profile_id.clone());
                    }
                    if fallback.is_none() {
                        fallback = Some(profile_id.clone());
                    }
                }
            }
        }
        if fallback.is_some() {
            return fallback;
        }
    }
    None
}

// resolve_full_api_key is intentionally not exposed as a Tauri command.
// It returns raw API keys which should never be sent to the frontend.
#[allow(dead_code)]
fn resolve_full_api_key(profile_id: String) -> Result<String, String> {
    let paths = resolve_paths();
    let profiles = load_model_profiles(&paths);
    let profile = profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| "Profile not found".to_string())?;
    let key = resolve_profile_api_key(profile, &paths.base_dir);
    if key.is_empty() {
        return Err("No API key configured for this profile".to_string());
    }
    Ok(key)
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("URL is required".into());
    }
    // Allow http(s) URLs and local paths within user home directory
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        // For local paths, ensure they don't execute apps
        let path = std::path::Path::new(trimmed);
        if path
            .extension()
            .map_or(false, |ext| ext == "app" || ext == "exe")
        {
            return Err("Cannot open application files".into());
        }
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/c", "start", &url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---- Backup / Restore ----

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub name: String,
    pub path: String,
    pub created_at: String,
    pub size_bytes: u64,
}

fn copy_dir_recursive(
    src: &Path,
    dst: &Path,
    skip_dirs: &HashSet<&str>,
    total: &mut u64,
) -> Result<(), String> {
    let entries =
        fs::read_dir(src).map_err(|e| format!("Failed to read dir {}: {e}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip the config file (already copied separately) and skip dirs
        if name_str == "openclaw.json" {
            continue;
        }

        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        let dest = dst.join(&name);

        if file_type.is_dir() {
            if skip_dirs.contains(name_str.as_ref()) {
                continue;
            }
            fs::create_dir_all(&dest)
                .map_err(|e| format!("Failed to create dir {}: {e}", dest.display()))?;
            copy_dir_recursive(&entry.path(), &dest, skip_dirs, total)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &dest)
                .map_err(|e| format!("Failed to copy {}: {e}", name_str))?;
            *total += fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        }
    }
    Ok(())
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                total += dir_size(&entry.path());
            } else {
                total += fs::metadata(entry.path()).map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}

fn restore_dir_recursive(src: &Path, dst: &Path, skip_dirs: &HashSet<&str>) -> Result<(), String> {
    let entries = fs::read_dir(src).map_err(|e| format!("Failed to read backup dir: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str == "openclaw.json" {
            continue; // Already restored separately
        }

        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        let dest = dst.join(&name);

        if file_type.is_dir() {
            if skip_dirs.contains(name_str.as_ref()) {
                continue;
            }
            fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            restore_dir_recursive(&entry.path(), &dest, skip_dirs)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &dest)
                .map_err(|e| format!("Failed to restore {}: {e}", name_str))?;
        }
    }
    Ok(())
}

// ---- Remote Backup / Restore (via SSH) ----

fn resolve_model_provider_base_url(cfg: &Value, provider: &str) -> Option<String> {
    let provider = provider.trim();
    if provider.is_empty() {
        return None;
    }
    cfg.pointer("/models/providers")
        .and_then(Value::as_object)
        .and_then(|providers| providers.get(provider))
        .and_then(Value::as_object)
        .and_then(|provider_cfg| {
            provider_cfg
                .get("baseUrl")
                .or_else(|| provider_cfg.get("base_url"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    provider_cfg
                        .get("apiBase")
                        .or_else(|| provider_cfg.get("api_base"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
        })
}

#[tauri::command]
pub fn list_registered_instances() -> Result<Vec<clawpal_core::instance::Instance>, String> {
    let registry = clawpal_core::instance::InstanceRegistry::load().map_err(|e| e.to_string())?;
    // Best-effort self-heal: persist normalized instance ids (e.g., legacy empty SSH ids).
    let _ = registry.save();
    Ok(registry.list())
}

#[tauri::command]
pub fn delete_registered_instance(instance_id: String) -> Result<bool, String> {
    let id = instance_id.trim();
    if id.is_empty() || id == "local" {
        return Ok(false);
    }
    let mut registry =
        clawpal_core::instance::InstanceRegistry::load().map_err(|e| e.to_string())?;
    let removed = registry.remove(id).is_some();
    if removed {
        registry.save().map_err(|e| e.to_string())?;
    }
    Ok(removed)
}

#[tauri::command]
pub async fn connect_docker_instance(
    home: String,
    label: Option<String>,
    instance_id: Option<String>,
) -> Result<clawpal_core::instance::Instance, String> {
    clawpal_core::connect::connect_docker(&home, label.as_deref(), instance_id.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn connect_local_instance(
    home: String,
    label: Option<String>,
    instance_id: Option<String>,
) -> Result<clawpal_core::instance::Instance, String> {
    clawpal_core::connect::connect_local(&home, label.as_deref(), instance_id.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn connect_ssh_instance(
    host_id: String,
) -> Result<clawpal_core::instance::Instance, String> {
    let hosts = read_hosts_from_registry()?;
    let host = hosts
        .into_iter()
        .find(|h| h.id == host_id)
        .ok_or_else(|| format!("No SSH host config with id: {host_id}"))?;
    // Register the SSH host as an instance in the instance registry
    // (skip the actual SSH connectivity probe — the caller already connected)
    let instance = clawpal_core::instance::Instance {
        id: host.id.clone(),
        instance_type: clawpal_core::instance::InstanceType::RemoteSsh,
        label: host.label.clone(),
        openclaw_home: None,
        clawpal_data_dir: None,
        ssh_host_config: Some(host),
    };
    let mut registry =
        clawpal_core::instance::InstanceRegistry::load().map_err(|e| e.to_string())?;
    let _ = registry.remove(&instance.id);
    registry.add(instance.clone()).map_err(|e| e.to_string())?;
    registry.save().map_err(|e| e.to_string())?;
    Ok(instance)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyDockerInstance {
    pub id: String,
    pub label: String,
    pub openclaw_home: Option<String>,
    pub clawpal_data_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMigrationResult {
    pub imported_ssh_hosts: usize,
    pub imported_docker_instances: usize,
    pub imported_open_tab_instances: usize,
    pub total_instances: usize,
}

fn fallback_label_from_instance_id(instance_id: &str) -> String {
    if instance_id == "local" {
        return "Local".to_string();
    }
    if let Some(suffix) = instance_id.strip_prefix("docker:") {
        if suffix.is_empty() {
            return "docker-local".to_string();
        }
        if suffix.starts_with("docker-") {
            return suffix.to_string();
        }
        return format!("docker-{suffix}");
    }
    if let Some(suffix) = instance_id.strip_prefix("ssh:") {
        return if suffix.is_empty() {
            "SSH".to_string()
        } else {
            suffix.to_string()
        };
    }
    instance_id.to_string()
}

fn upsert_registry_instance(
    registry: &mut clawpal_core::instance::InstanceRegistry,
    instance: clawpal_core::instance::Instance,
) -> Result<(), String> {
    let _ = registry.remove(&instance.id);
    registry.add(instance).map_err(|e| e.to_string())
}

fn migrate_legacy_ssh_file(
    paths: &crate::models::OpenClawPaths,
    registry: &mut clawpal_core::instance::InstanceRegistry,
) -> Result<usize, String> {
    let legacy_path = paths.clawpal_dir.join("remote-instances.json");
    if !legacy_path.exists() {
        return Ok(0);
    }
    let text = fs::read_to_string(&legacy_path).map_err(|e| e.to_string())?;
    let hosts: Vec<SshHostConfig> = serde_json::from_str(&text).unwrap_or_default();
    let mut count = 0usize;
    for host in hosts {
        let instance = clawpal_core::instance::Instance {
            id: host.id.clone(),
            instance_type: clawpal_core::instance::InstanceType::RemoteSsh,
            label: if host.label.trim().is_empty() {
                host.host.clone()
            } else {
                host.label.clone()
            },
            openclaw_home: None,
            clawpal_data_dir: None,
            ssh_host_config: Some(host),
        };
        upsert_registry_instance(registry, instance)?;
        count += 1;
    }
    // Remove legacy file after successful migration so it doesn't
    // re-add deleted hosts on subsequent page loads.
    if count > 0 {
        let _ = fs::remove_file(&legacy_path);
    }
    Ok(count)
}

#[tauri::command]
pub fn migrate_legacy_instances(
    legacy_docker_instances: Vec<LegacyDockerInstance>,
    legacy_open_tab_ids: Vec<String>,
) -> Result<LegacyMigrationResult, String> {
    let paths = resolve_paths();
    let mut registry =
        clawpal_core::instance::InstanceRegistry::load().map_err(|e| e.to_string())?;

    // Ensure local instance exists for old users.
    if registry.get("local").is_none() {
        upsert_registry_instance(
            &mut registry,
            clawpal_core::instance::Instance {
                id: "local".to_string(),
                instance_type: clawpal_core::instance::InstanceType::Local,
                label: "Local".to_string(),
                openclaw_home: None,
                clawpal_data_dir: None,
                ssh_host_config: None,
            },
        )?;
    }

    let imported_ssh_hosts = migrate_legacy_ssh_file(&paths, &mut registry)?;

    let mut imported_docker_instances = 0usize;
    for docker in legacy_docker_instances {
        let id = docker.id.trim();
        if id.is_empty() {
            continue;
        }
        let label = if docker.label.trim().is_empty() {
            fallback_label_from_instance_id(id)
        } else {
            docker.label.clone()
        };
        upsert_registry_instance(
            &mut registry,
            clawpal_core::instance::Instance {
                id: id.to_string(),
                instance_type: clawpal_core::instance::InstanceType::Docker,
                label,
                openclaw_home: docker.openclaw_home.clone(),
                clawpal_data_dir: docker.clawpal_data_dir.clone(),
                ssh_host_config: None,
            },
        )?;
        imported_docker_instances += 1;
    }

    let mut imported_open_tab_instances = 0usize;
    for tab_id in legacy_open_tab_ids {
        let id = tab_id.trim();
        if id.is_empty() {
            continue;
        }
        if registry.get(id).is_some() {
            continue;
        }
        if id == "local" {
            continue;
        }
        if id.starts_with("docker:") {
            upsert_registry_instance(
                &mut registry,
                clawpal_core::instance::Instance {
                    id: id.to_string(),
                    instance_type: clawpal_core::instance::InstanceType::Docker,
                    label: fallback_label_from_instance_id(id),
                    openclaw_home: None,
                    clawpal_data_dir: None,
                    ssh_host_config: None,
                },
            )?;
            imported_open_tab_instances += 1;
            continue;
        }
        if id.starts_with("ssh:") {
            let host_alias = id.strip_prefix("ssh:").unwrap_or("").to_string();
            upsert_registry_instance(
                &mut registry,
                clawpal_core::instance::Instance {
                    id: id.to_string(),
                    instance_type: clawpal_core::instance::InstanceType::RemoteSsh,
                    label: fallback_label_from_instance_id(id),
                    openclaw_home: None,
                    clawpal_data_dir: None,
                    ssh_host_config: Some(clawpal_core::instance::SshHostConfig {
                        id: id.to_string(),
                        label: fallback_label_from_instance_id(id),
                        host: host_alias,
                        port: 22,
                        username: String::new(),
                        auth_method: "ssh_config".to_string(),
                        key_path: None,
                        password: None,
                        passphrase: None,
                    }),
                },
            )?;
            imported_open_tab_instances += 1;
        }
    }

    registry.save().map_err(|e| e.to_string())?;
    let total_instances = registry.list().len();
    Ok(LegacyMigrationResult {
        imported_ssh_hosts,
        imported_docker_instances,
        imported_open_tab_instances,
        total_instances,
    })
}

// ---------------------------------------------------------------------------
// Task 3: Remote instance config CRUD
// ---------------------------------------------------------------------------

pub type SshConfigHostSuggestion = clawpal_core::ssh::config::SshConfigHostSuggestion;

fn ssh_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh").join("config"))
}

fn read_hosts_from_registry() -> Result<Vec<SshHostConfig>, String> {
    clawpal_core::ssh::registry::list_ssh_hosts()
}

#[tauri::command]
pub fn list_ssh_hosts() -> Result<Vec<SshHostConfig>, String> {
    read_hosts_from_registry()
}

#[tauri::command]
pub fn list_ssh_config_hosts() -> Result<Vec<SshConfigHostSuggestion>, String> {
    let Some(path) = ssh_config_path() else {
        return Ok(Vec::new());
    };
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data =
        fs::read_to_string(&path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    Ok(clawpal_core::ssh::config::parse_ssh_config_hosts(&data))
}

#[tauri::command]
pub fn upsert_ssh_host(host: SshHostConfig) -> Result<SshHostConfig, String> {
    clawpal_core::ssh::registry::upsert_ssh_host(host)
}

#[tauri::command]
pub fn delete_ssh_host(host_id: String) -> Result<bool, String> {
    clawpal_core::ssh::registry::delete_ssh_host(&host_id)
}

// ---------------------------------------------------------------------------
// Task 4: SSH connect / disconnect / status
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn ssh_connect(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<bool, String> {
    crate::commands::logs::log_dev(format!("[dev][ssh_connect] begin host_id={host_id}"));
    // If already connected and handle is alive, reuse
    if pool.is_connected(&host_id).await {
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_connect] reuse existing connection host_id={host_id}"
        ));
        return Ok(true);
    }
    let hosts = read_hosts_from_registry()?;
    if hosts.is_empty() {
        crate::commands::logs::log_dev("[dev][ssh_connect] host registry is empty");
    }
    let host = hosts.into_iter().find(|h| h.id == host_id).ok_or_else(|| {
        let mut ids = Vec::new();
        for h in read_hosts_from_registry().unwrap_or_default() {
            ids.push(h.id);
        }
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_connect] no host found host_id={host_id} known={ids:?}"
        ));
        format!("No SSH host config with id: {host_id}")
    })?;
    // If the host has a stored passphrase, use it directly
    let connect_result = if let Some(ref pp) = host.passphrase {
        if !pp.is_empty() {
            crate::commands::logs::log_dev(format!(
                "[dev][ssh_connect] using stored passphrase for host_id={host_id}"
            ));
            pool.connect_with_passphrase(&host, Some(pp.as_str())).await
        } else {
            pool.connect(&host).await
        }
    } else {
        pool.connect(&host).await
    };
    if let Err(error) = connect_result {
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_connect] failed host_id={} host={} user={} port={} auth_method={} error={}",
            host_id, host.host, host.username, host.port, host.auth_method, error
        ));
        return Err(format!("ssh connect failed: {error}"));
    }
    crate::commands::logs::log_dev(format!("[dev][ssh_connect] success host_id={host_id}"));
    Ok(true)
}

#[tauri::command]
pub async fn ssh_connect_with_passphrase(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    passphrase: String,
) -> Result<bool, String> {
    crate::commands::logs::log_dev(format!(
        "[dev][ssh_connect_with_passphrase] begin host_id={host_id}"
    ));
    if pool.is_connected(&host_id).await {
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_connect_with_passphrase] reuse existing connection host_id={host_id}"
        ));
        return Ok(true);
    }
    let hosts = read_hosts_from_registry()?;
    if hosts.is_empty() {
        crate::commands::logs::log_dev("[dev][ssh_connect_with_passphrase] host registry is empty");
    }
    let host = hosts.into_iter().find(|h| h.id == host_id).ok_or_else(|| {
        let mut ids = Vec::new();
        for h in read_hosts_from_registry().unwrap_or_default() {
            ids.push(h.id);
        }
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_connect_with_passphrase] no host found host_id={host_id} known={ids:?}"
        ));
        format!("No SSH host config with id: {host_id}")
    })?;
    if let Err(error) = pool
        .connect_with_passphrase(&host, Some(passphrase.as_str()))
        .await
    {
        crate::commands::logs::log_dev(format!(
            "[dev][ssh_connect_with_passphrase] failed host_id={} host={} user={} port={} auth_method={} error={}",
            host_id,
            host.host,
            host.username,
            host.port,
            host.auth_method,
            error
        ));
        return Err(format!("ssh connect failed: {error}"));
    }
    crate::commands::logs::log_dev(format!(
        "[dev][ssh_connect_with_passphrase] success host_id={host_id}"
    ));
    Ok(true)
}

#[tauri::command]
pub async fn ssh_disconnect(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<bool, String> {
    pool.disconnect(&host_id).await?;
    Ok(true)
}

#[tauri::command]
pub async fn ssh_status(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<String, String> {
    if pool.is_connected(&host_id).await {
        Ok("connected".to_string())
    } else {
        Ok("disconnected".to_string())
    }
}

// ---------------------------------------------------------------------------
// Task 5: SSH exec and SFTP Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn ssh_exec(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    command: String,
) -> Result<SshExecResult, String> {
    pool.exec(&host_id, &command).await
}

#[tauri::command]
pub async fn sftp_read_file(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    path: String,
) -> Result<String, String> {
    pool.sftp_read(&host_id, &path).await
}

#[tauri::command]
pub async fn sftp_write_file(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    path: String,
    content: String,
) -> Result<bool, String> {
    pool.sftp_write(&host_id, &path, &content).await?;
    Ok(true)
}

#[tauri::command]
pub async fn sftp_list_dir(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    path: String,
) -> Result<Vec<SftpEntry>, String> {
    pool.sftp_list(&host_id, &path).await
}

#[tauri::command]
pub async fn sftp_remove_file(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    path: String,
) -> Result<bool, String> {
    pool.sftp_remove(&host_id, &path).await?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Task 6: Remote business commands
// ---------------------------------------------------------------------------

fn is_owner_display_parse_error(text: &str) -> bool {
    clawpal_core::doctor::owner_display_parse_error(text)
}

async fn run_openclaw_remote_with_autofix(
    pool: &SshConnectionPool,
    host_id: &str,
    args: &[&str],
) -> Result<crate::cli_runner::CliOutput, String> {
    let first = crate::cli_runner::run_openclaw_remote(pool, host_id, args).await?;
    if first.exit_code == 0 {
        return Ok(first);
    }
    let combined = format!("{}\n{}", first.stderr, first.stdout);
    if !is_owner_display_parse_error(&combined) {
        return Ok(first);
    }
    let _ = crate::cli_runner::run_openclaw_remote(pool, host_id, &["doctor", "--fix"]).await;
    crate::cli_runner::run_openclaw_remote(pool, host_id, args).await
}

/// Tier 2: slow, optional — openclaw version + duplicate detection (2 SSH calls in parallel).
/// Called once on mount and on-demand (e.g., after upgrade), not in poll loop.
// ---------------------------------------------------------------------------
// Remote config mutation helpers & commands
// ---------------------------------------------------------------------------

/// Private helper: snapshot current config then write new config on remote.
async fn remote_write_config_with_snapshot(
    pool: &SshConnectionPool,
    host_id: &str,
    config_path: &str,
    current_text: &str,
    next: &Value,
    source: &str,
) -> Result<(), String> {
    // Use core function to prepare config write
    let (new_text, snapshot_text) =
        clawpal_core::config::prepare_config_write(current_text, next, source)?;

    // Create snapshot dir
    pool.exec(host_id, "mkdir -p ~/.clawpal/snapshots").await?;

    // Generate snapshot filename
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let snapshot_path = clawpal_core::config::snapshot_filename(ts, source);
    let snapshot_full_path = format!("~/.clawpal/snapshots/{snapshot_path}");

    // Write snapshot and new config via SFTP
    pool.sftp_write(host_id, &snapshot_full_path, &snapshot_text)
        .await?;
    pool.sftp_write(host_id, config_path, &new_text).await?;
    Ok(())
}

async fn remote_resolve_openclaw_config_path(
    pool: &SshConnectionPool,
    host_id: &str,
) -> Result<String, String> {
    let result = pool
        .exec_login(
            host_id,
            clawpal_core::doctor::remote_openclaw_config_path_probe_script(),
        )
        .await?;
    if result.exit_code != 0 {
        let details = format!("{}\n{}", result.stderr.trim(), result.stdout.trim());
        return Err(format!(
            "Failed to resolve remote openclaw config path ({}): {}",
            result.exit_code,
            details.trim()
        ));
    }
    let path = result.stdout.trim();
    if path.is_empty() {
        return Err("Remote openclaw config path probe returned empty output".into());
    }
    Ok(path.to_string())
}

async fn remote_read_openclaw_config_text_and_json(
    pool: &SshConnectionPool,
    host_id: &str,
) -> Result<(String, String, Value), String> {
    let config_path = remote_resolve_openclaw_config_path(pool, host_id).await?;
    let raw = pool.sftp_read(host_id, &config_path).await?;
    let (parsed, normalized) = clawpal_core::config::parse_and_normalize_config(&raw)
        .map_err(|e| format!("Failed to parse remote config: {e}"))?;
    Ok((config_path, normalized, parsed))
}

async fn run_remote_rescue_bot_command(
    pool: &SshConnectionPool,
    host_id: &str,
    command: Vec<String>,
) -> Result<RescueBotCommandResult, String> {
    let mut remote_cmd = String::from("openclaw");
    for arg in &command {
        remote_cmd.push(' ');
        remote_cmd.push_str(&shell_escape(arg));
    }
    let raw = pool.exec_login(host_id, &remote_cmd).await?;
    Ok(RescueBotCommandResult {
        command,
        output: OpenclawCommandOutput {
            stdout: raw.stdout,
            stderr: raw.stderr,
            exit_code: raw.exit_code as i32,
        },
    })
}

async fn run_remote_openclaw_dynamic(
    pool: &SshConnectionPool,
    host_id: &str,
    command: Vec<String>,
) -> Result<OpenclawCommandOutput, String> {
    Ok(run_remote_rescue_bot_command(pool, host_id, command)
        .await?
        .output)
}

async fn run_remote_primary_doctor_with_fallback(
    pool: &SshConnectionPool,
    host_id: &str,
    profile: &str,
) -> Result<OpenclawCommandOutput, String> {
    let json_command = build_profile_command(profile, &["doctor", "--json"]);
    let output = run_remote_openclaw_dynamic(pool, host_id, json_command).await?;
    if output.exit_code != 0
        && clawpal_core::doctor::doctor_json_option_unsupported(&output.stderr, &output.stdout)
    {
        let plain_command = build_profile_command(profile, &["doctor"]);
        return run_remote_openclaw_dynamic(pool, host_id, plain_command).await;
    }
    Ok(output)
}

async fn run_remote_gateway_restart_fallback(
    pool: &SshConnectionPool,
    host_id: &str,
    profile: &str,
    commands: &mut Vec<RescueBotCommandResult>,
) -> Result<(), String> {
    let stop_command = vec![
        "--profile".to_string(),
        profile.to_string(),
        "gateway".to_string(),
        "stop".to_string(),
    ];
    let stop_result = run_remote_rescue_bot_command(pool, host_id, stop_command).await?;
    commands.push(stop_result);

    let start_command = vec![
        "--profile".to_string(),
        profile.to_string(),
        "gateway".to_string(),
        "start".to_string(),
    ];
    let start_result = run_remote_rescue_bot_command(pool, host_id, start_command).await?;
    if start_result.output.exit_code != 0 {
        return Err(command_failure_message(
            &start_result.command,
            &start_result.output,
        ));
    }
    commands.push(start_result);
    Ok(())
}

fn is_remote_missing_path_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("no such file")
        || lower.contains("no such file or directory")
        || lower.contains("not found")
        || lower.contains("cannot open")
}

fn is_valid_env_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

async fn read_remote_env_var(
    pool: &SshConnectionPool,
    host_id: &str,
    name: &str,
) -> Result<Option<String>, String> {
    if !is_valid_env_var_name(name) {
        return Err(format!("Invalid environment variable name: {name}"));
    }

    let cmd = format!("printenv -- {name}");
    let out = pool
        .exec_login(host_id, &cmd)
        .await
        .map_err(|e| format!("Failed to read remote env var {name}: {e}"))?;

    if out.exit_code != 0 {
        return Ok(None);
    }

    let value = out.stdout.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value.to_string()))
    }
}

async fn resolve_remote_key_from_agent_auth_profiles(
    pool: &SshConnectionPool,
    host_id: &str,
    auth_ref: &str,
) -> Result<Option<String>, String> {
    let roots = resolve_remote_openclaw_roots(pool, host_id).await?;

    for root in roots {
        let agents_path = format!("{}/agents", root.trim_end_matches('/'));
        let entries = match pool.sftp_list(host_id, &agents_path).await {
            Ok(entries) => entries,
            Err(e) if is_remote_missing_path_error(&e) => continue,
            Err(e) => {
                return Err(format!(
                    "Failed to list remote agents directory at {agents_path}: {e}"
                ))
            }
        };

        for agent in entries.into_iter().filter(|entry| entry.is_dir) {
            let agent_dir = format!("{}/agents/{}/agent", root.trim_end_matches('/'), agent.name);
            for file_name in ["auth-profiles.json", "auth.json"] {
                let auth_file = format!("{agent_dir}/{file_name}");
                let text = match pool.sftp_read(host_id, &auth_file).await {
                    Ok(text) => text,
                    Err(e) if is_remote_missing_path_error(&e) => continue,
                    Err(e) => {
                        return Err(format!(
                            "Failed to read remote auth store at {auth_file}: {e}"
                        ))
                    }
                };
                let data: Value = serde_json::from_str(&text).map_err(|e| {
                    format!("Failed to parse remote auth store at {auth_file}: {e}")
                })?;
                if let Some(key) = resolve_key_from_auth_store_json(&data, auth_ref) {
                    return Ok(Some(key));
                }
            }
        }
    }

    Ok(None)
}

async fn resolve_remote_openclaw_roots(
    pool: &SshConnectionPool,
    host_id: &str,
) -> Result<Vec<String>, String> {
    let mut roots = Vec::<String>::new();
    let primary = pool
        .exec_login(
            host_id,
            clawpal_core::doctor::remote_openclaw_root_probe_script(),
        )
        .await?;
    let primary_trimmed = primary.stdout.trim();
    if !primary_trimmed.is_empty() {
        roots.push(primary_trimmed.to_string());
    }

    let discover = pool
        .exec_login(
            host_id,
            "for d in \"$HOME\"/.openclaw*; do [ -d \"$d\" ] && printf '%s\\n' \"$d\"; done",
        )
        .await?;
    for line in discover.stdout.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            roots.push(trimmed.to_string());
        }
    }
    let mut deduped = Vec::<String>::new();
    let mut seen = std::collections::BTreeSet::<String>::new();
    for root in roots {
        if seen.insert(root.clone()) {
            deduped.push(root);
        }
    }
    roots = deduped;
    Ok(roots)
}

async fn resolve_remote_profile_base_url(
    pool: &SshConnectionPool,
    host_id: &str,
    profile: &ModelProfile,
) -> Result<Option<String>, String> {
    if let Some(base) = profile
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return Ok(Some(base.to_string()));
    }

    let config_path = match remote_resolve_openclaw_config_path(pool, host_id).await {
        Ok(path) => path,
        Err(_) => return Ok(None),
    };
    let raw = match pool.sftp_read(host_id, &config_path).await {
        Ok(raw) => raw,
        Err(e) if is_remote_missing_path_error(&e) => return Ok(None),
        Err(e) => {
            return Err(format!(
                "Failed to read remote config for base URL resolution: {e}"
            ))
        }
    };
    let cfg = match clawpal_core::config::parse_and_normalize_config(&raw) {
        Ok((parsed, _)) => parsed,
        Err(e) => {
            return Err(format!(
                "Failed to parse remote config for base URL resolution: {e}"
            ))
        }
    };
    Ok(resolve_model_provider_base_url(&cfg, &profile.provider))
}

async fn resolve_remote_profile_api_key(
    pool: &SshConnectionPool,
    host_id: &str,
    profile: &ModelProfile,
) -> Result<String, String> {
    let mut auth_refs = Vec::<String>::new();
    let auth_ref = profile.auth_ref.trim();
    if !auth_ref.is_empty() {
        auth_refs.push(auth_ref.to_string());
    }
    let provider = profile.provider.trim().to_lowercase();
    if !provider.is_empty() {
        let fallback = format!("{provider}:default");
        if !auth_refs.iter().any(|candidate| candidate == &fallback) {
            auth_refs.push(fallback);
        }
    }

    for auth_ref in &auth_refs {
        // Try auth_ref as remote env var name directly (e.g. OPENAI_API_KEY)
        if auth_ref
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            if let Some(key) = read_remote_env_var(pool, host_id, auth_ref).await? {
                return Ok(key);
            }
        }

        // Try auth_ref from remote agent auth-profiles.json
        if let Some(key) =
            resolve_remote_key_from_agent_auth_profiles(pool, host_id, auth_ref).await?
        {
            return Ok(key);
        }
    }

    // Try provider-based env conventions as fallback
    let provider_env = profile.provider.trim().to_uppercase().replace('-', "_");
    if !provider_env.is_empty() {
        for suffix in ["_API_KEY", "_KEY", "_TOKEN"] {
            let env_name = format!("{provider_env}{suffix}");
            if let Some(key) = read_remote_env_var(pool, host_id, &env_name).await? {
                return Ok(key);
            }
        }
    }

    // Fallback to direct apiKey only when no authoritative source is resolved.
    if let Some(key) = &profile.api_key {
        let trimmed_key = key.trim();
        if !trimmed_key.is_empty() {
            return Ok(trimmed_key.to_string());
        }
    }

    Ok(String::new())
}

#[tauri::command]
pub async fn run_openclaw_upgrade() -> Result<String, String> {
    let output = Command::new("bash")
        .args(["-c", "curl -fsSL https://openclaw.ai/install.sh | bash"])
        .output()
        .map_err(|e| format!("Failed to run upgrade: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = if stderr.is_empty() {
        stdout
    } else {
        format!("{stdout}\n{stderr}")
    };
    if output.status.success() {
        clear_openclaw_version_cache();
        Ok(combined)
    } else {
        Err(combined)
    }
}

// ---------------------------------------------------------------------------
// Cron jobs
// ---------------------------------------------------------------------------

fn parse_cron_jobs(text: &str) -> Value {
    let jobs = clawpal_core::cron::parse_cron_jobs(text).unwrap_or_default();
    Value::Array(jobs)
}

// ---------------------------------------------------------------------------
// Remote cron jobs
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Watchdog management
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_watchdog_status() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let paths = resolve_paths();
        let wd_dir = paths.clawpal_dir.join("watchdog");
        let status_path = wd_dir.join("status.json");
        let pid_path = wd_dir.join("watchdog.pid");

        let mut status = if status_path.exists() {
            let text = std::fs::read_to_string(&status_path).map_err(|e| e.to_string())?;
            serde_json::from_str::<Value>(&text).unwrap_or(Value::Null)
        } else {
            Value::Null
        };

        let alive = if pid_path.exists() {
            let pid_str = std::fs::read_to_string(&pid_path).unwrap_or_default();
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                std::process::Command::new("kill")
                    .args(["-0", &pid.to_string()])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
            } else {
                false
            }
        } else {
            false
        };

        if let Value::Object(ref mut map) = status {
            map.insert("alive".into(), Value::Bool(alive));
            map.insert(
                "deployed".into(),
                Value::Bool(wd_dir.join("watchdog.js").exists()),
            );
        } else {
            let mut map = serde_json::Map::new();
            map.insert("alive".into(), Value::Bool(alive));
            map.insert(
                "deployed".into(),
                Value::Bool(wd_dir.join("watchdog.js").exists()),
            );
            status = Value::Object(map);
        }

        Ok(status)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn deploy_watchdog(app_handle: tauri::AppHandle) -> Result<bool, String> {
    let paths = resolve_paths();
    let wd_dir = paths.clawpal_dir.join("watchdog");
    std::fs::create_dir_all(&wd_dir).map_err(|e| e.to_string())?;

    let resource_path = app_handle
        .path()
        .resolve(
            "resources/watchdog.js",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| format!("Failed to resolve watchdog resource: {e}"))?;

    let content = std::fs::read_to_string(&resource_path)
        .map_err(|e| format!("Failed to read watchdog resource: {e}"))?;

    std::fs::write(wd_dir.join("watchdog.js"), content).map_err(|e| e.to_string())?;
    crate::logging::log_info("Watchdog deployed");
    Ok(true)
}

#[tauri::command]
pub fn start_watchdog() -> Result<bool, String> {
    let paths = resolve_paths();
    let wd_dir = paths.clawpal_dir.join("watchdog");
    let script = wd_dir.join("watchdog.js");
    let pid_path = wd_dir.join("watchdog.pid");
    let log_path = wd_dir.join("watchdog.log");

    if !script.exists() {
        return Err("Watchdog not deployed. Deploy first.".into());
    }

    if pid_path.exists() {
        let pid_str = std::fs::read_to_string(&pid_path).unwrap_or_default();
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            let alive = std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if alive {
                return Ok(true);
            }
        }
    }

    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| e.to_string())?;
    let log_err = log_file.try_clone().map_err(|e| e.to_string())?;

    let _child = std::process::Command::new("node")
        .arg(&script)
        .current_dir(&wd_dir)
        .env("CLAWPAL_WATCHDOG_DIR", &wd_dir)
        .stdout(log_file)
        .stderr(log_err)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start watchdog: {e}"))?;

    // PID file is written by watchdog.js itself via acquirePidFile()
    crate::logging::log_info("Watchdog started");
    Ok(true)
}

#[tauri::command]
pub fn stop_watchdog() -> Result<bool, String> {
    let paths = resolve_paths();
    let pid_path = paths.clawpal_dir.join("watchdog").join("watchdog.pid");

    if !pid_path.exists() {
        return Ok(true);
    }

    let pid_str = std::fs::read_to_string(&pid_path).unwrap_or_default();
    if let Ok(pid) = pid_str.trim().parse::<u32>() {
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .output();
    }

    let _ = std::fs::remove_file(&pid_path);
    crate::logging::log_info("Watchdog stopped");
    Ok(true)
}

#[tauri::command]
pub fn uninstall_watchdog() -> Result<bool, String> {
    let paths = resolve_paths();
    let wd_dir = paths.clawpal_dir.join("watchdog");

    // Stop first if running
    let pid_path = wd_dir.join("watchdog.pid");
    if pid_path.exists() {
        let pid_str = std::fs::read_to_string(&pid_path).unwrap_or_default();
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            let _ = std::process::Command::new("kill")
                .arg(pid.to_string())
                .output();
        }
    }

    // Remove entire watchdog directory
    if wd_dir.exists() {
        std::fs::remove_dir_all(&wd_dir).map_err(|e| e.to_string())?;
    }
    crate::logging::log_info("Watchdog uninstalled");
    Ok(true)
}

// ---------------------------------------------------------------------------
// Log reading commands
// ---------------------------------------------------------------------------
const MAX_LOG_TAIL_LINES: usize = 400;

fn clamp_log_lines(lines: Option<usize>) -> usize {
    let requested = lines.unwrap_or(200);
    requested.clamp(1, MAX_LOG_TAIL_LINES)
}

#[tauri::command]
pub fn read_app_log(lines: Option<usize>) -> Result<String, String> {
    crate::logging::read_log_tail("app.log", clamp_log_lines(lines))
}

#[tauri::command]
pub fn read_error_log(lines: Option<usize>) -> Result<String, String> {
    crate::logging::read_log_tail("error.log", clamp_log_lines(lines))
}

#[tauri::command]
pub fn log_app_event(message: String) -> Result<bool, String> {
    let trimmed = message.trim();
    if !trimmed.is_empty() {
        crate::logging::log_info(trimmed);
    }
    Ok(true)
}

#[tauri::command]
pub fn read_gateway_log(lines: Option<usize>) -> Result<String, String> {
    let paths = crate::models::resolve_paths();
    let path = paths.openclaw_dir.join("logs/gateway.log");
    if !path.exists() {
        return Ok(String::new());
    }
    crate::logging::read_path_tail(&path, clamp_log_lines(lines))
}

#[tauri::command]
pub fn read_gateway_error_log(lines: Option<usize>) -> Result<String, String> {
    let paths = crate::models::resolve_paths();
    let path = paths.openclaw_dir.join("logs/gateway.err.log");
    if !path.exists() {
        return Ok(String::new());
    }
    crate::logging::read_path_tail(&path, clamp_log_lines(lines))
}

// ---------------------------------------------------------------------------
// Remote watchdog management
// ---------------------------------------------------------------------------
