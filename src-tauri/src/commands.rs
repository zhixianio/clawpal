use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::{fs, process::Command, time::{SystemTime, UNIX_EPOCH}};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use tauri::{Manager, State};

use crate::config_io::{ensure_dirs, read_openclaw_config, write_json, write_text};
use crate::doctor::{apply_auto_fixes, run_doctor, DoctorReport};
use crate::history::{add_snapshot, list_snapshots, read_snapshot};
use crate::models::resolve_paths;
use crate::ssh::{SshConnectionPool, SshHostConfig, SshExecResult, SftpEntry};

/// Escape a string for safe inclusion in a single-quoted shell argument.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Resolve the `openclaw` binary path, with fallback probing for common locations.
/// `fix_path_env::fix()` in main.rs patches PATH from the user's login shell, but
/// it silently fails on some setups. This function caches the resolved path for the
/// lifetime of the process.
pub(crate) fn resolve_openclaw_bin() -> &'static str {
    use std::sync::OnceLock;
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        // First: check if "openclaw" is already in PATH (fix_path_env worked)
        if find_in_path("openclaw") {
            return "openclaw".to_string();
        }
        // Fallback: probe well-known locations
        let home = std::env::var("HOME").unwrap_or_default();
        let candidates = [
            "/opt/homebrew/bin/openclaw".to_string(),
            "/usr/local/bin/openclaw".to_string(),
            format!("{home}/.npm-global/bin/openclaw"),
            format!("{home}/.local/bin/openclaw"),
        ];
        // Also probe nvm directories (any node version)
        let nvm_dir = std::env::var("NVM_DIR")
            .unwrap_or_else(|_| format!("{home}/.nvm"));
        let nvm_pattern = format!("{nvm_dir}/versions/node");
        let mut nvm_candidates = Vec::new();
        if let Ok(entries) = fs::read_dir(&nvm_pattern) {
            for entry in entries.flatten() {
                let p = entry.path().join("bin/openclaw");
                if p.exists() {
                    nvm_candidates.push(p.to_string_lossy().to_string());
                }
            }
        }
        for candidate in candidates.iter().chain(nvm_candidates.iter()) {
            if Path::new(candidate).is_file() {
                // Prepend its directory to PATH so child processes benefit.
                // Called exactly once via OnceLock, before other threads read PATH.
                if let Some(dir) = Path::new(candidate).parent() {
                    if let Ok(current_path) = std::env::var("PATH") {
                        let dir_str = dir.to_string_lossy();
                        let already_in_path = std::env::split_paths(&current_path)
                            .any(|p| p == Path::new(dir_str.as_ref()));
                        if !already_in_path {
                            std::env::set_var("PATH", format!("{dir_str}:{current_path}"));
                        }
                    }
                }
                return candidate.clone();
            }
        }
        // Last resort: return bare name and let the OS error propagate
        "openclaw".to_string()
    })
}

/// Check if a binary exists in PATH without executing it.
fn find_in_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
        .unwrap_or(false)
}

use crate::recipe::{
    load_recipes_with_fallback,
    collect_change_paths,
    build_candidate_config_from_template,
    format_diff,
    ApplyResult,
    PreviewResult,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenclawCommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfile {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub auth_ref: String,
    #[serde(default)]
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub description: Option<String>,
    pub enabled: bool,
}

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

static OPENCLAW_VERSION_CACHE: std::sync::Mutex<Option<Option<String>>> = std::sync::Mutex::new(None);

/// Fast status: reads config + quick TCP probe of gateway port.
#[tauri::command]
pub fn get_status_light() -> Result<StatusLight, String> {

    let paths = resolve_paths();
    let cfg = read_openclaw_config(&paths)?;
    let explicit_count = cfg
        .get("agents")
        .and_then(|a| a.get("list"))
        .and_then(|a| a.as_array())
        .map(|a| a.len() as u32)
        .unwrap_or(0);
    // At least 1 agent (implicit "main") when agents section exists
    let active_agents = if explicit_count == 0 && cfg.get("agents").is_some() { 1 } else { explicit_count };
    let global_default_model = cfg
        .pointer("/agents/defaults/model")
        .and_then(read_model_value)
        .or_else(|| cfg.pointer("/agents/default/model").and_then(read_model_value));

    let fallback_models = cfg
        .pointer("/agents/defaults/model/fallbacks")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default();

    // Quick gateway health: TCP connect to gateway port
    let gateway_port = cfg.pointer("/gateway/port")
        .and_then(Value::as_u64)
        .unwrap_or(18789) as u16;
    let healthy = std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], gateway_port)),
        std::time::Duration::from_millis(200),
    ).is_ok();

    Ok(StatusLight {
        healthy,
        active_agents,
        global_default_model,
        fallback_models,
    })
}

/// Local status extra: openclaw version (cached) + no duplicate detection needed locally.
#[tauri::command]
pub fn get_status_extra() -> Result<StatusExtra, String> {
    let openclaw_version = {
        let mut cache = OPENCLAW_VERSION_CACHE.lock().unwrap();
        if cache.is_none() {
            *cache = Some(
                std::process::Command::new(resolve_openclaw_bin())
                    .arg("--version")
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()),
            );
        }
        cache.as_ref().unwrap().clone()
    };
    Ok(StatusExtra {
        openclaw_version,
        duplicate_installs: Vec::new(),
    })
}

/// Returns cached catalog instantly without calling CLI. Returns empty if no cache.
#[tauri::command]
pub fn get_cached_model_catalog() -> Result<Vec<ModelCatalogProvider>, String> {
    let paths = resolve_paths();
    let cache_path = model_catalog_cache_path(&paths);
    if let Some(cached) = read_model_catalog_cache(&cache_path) {
        if cached.error.is_none() && !cached.providers.is_empty() {
            return Ok(cached.providers);
        }
    }
    Ok(Vec::new())
}

/// Refresh catalog from CLI and update cache. Returns the fresh catalog.
#[tauri::command]
pub fn refresh_model_catalog() -> Result<Vec<ModelCatalogProvider>, String> {
    let paths = resolve_paths();
    let cfg = read_openclaw_config(&paths)?;
    load_model_catalog(&paths, &cfg)
}

#[tauri::command]
pub fn get_system_status() -> Result<SystemStatus, String> {
    let paths = resolve_paths();
    ensure_dirs(&paths)?;
    let cfg = read_openclaw_config(&paths)?;
    let active_agents = cfg
        .get("agents")
        .and_then(|a| a.get("list"))
        .and_then(|a| a.as_array())
        .map(|a| a.len() as u32)
        .unwrap_or(0);
    let snapshots = list_snapshots(&paths.metadata_path).unwrap_or_default().items.len();
    let model_summary = collect_model_summary(&cfg);
    let channel_summary = collect_channel_summary(&cfg);
    let memory = collect_memory_overview(&paths.base_dir);
    let sessions = collect_session_overview(&paths.base_dir);
    let openclaw_version = resolve_openclaw_version();
    let openclaw_update = check_openclaw_update_cached(&paths, false).unwrap_or_else(|_| OpenclawUpdateCheck {
        installed_version: openclaw_version.clone(),
        latest_version: None,
        upgrade_available: false,
        channel: None,
        details: Some("update status unavailable".into()),
        source: "unknown".into(),
        checked_at: format_timestamp_from_unix(unix_timestamp_secs()),
    });
    Ok(SystemStatus {
        healthy: true,
        config_path: paths.config_path.to_string_lossy().to_string(),
        openclaw_dir: paths.openclaw_dir.to_string_lossy().to_string(),
        clawpal_dir: paths.clawpal_dir.to_string_lossy().to_string(),
        openclaw_version,
        active_agents,
        snapshots,
        channels: channel_summary,
        models: model_summary,
        memory,
        sessions,
        openclaw_update,
    })
}

#[tauri::command]
pub fn list_model_profiles() -> Result<Vec<ModelProfile>, String> {
    let paths = resolve_paths();
    Ok(load_model_profiles(&paths))
}

#[tauri::command]
pub fn check_openclaw_update() -> Result<OpenclawUpdateCheck, String> {
    let paths = resolve_paths();
    check_openclaw_update_cached(&paths, true)
}

#[tauri::command]
pub fn extract_model_profiles_from_config() -> Result<ExtractModelProfilesResult, String> {
    let paths = resolve_paths();
    let cfg = read_openclaw_config(&paths)?;
    let profiles = load_model_profiles(&paths);
    let bindings = collect_model_bindings(&cfg, &profiles);
    let mut created = 0usize;
    let mut reused = 0usize;
    let mut skipped_invalid = 0usize;
    let mut seen = HashSet::new();

    let mut next_profiles = profiles;
    let mut model_profile_map: HashMap<String, String> = HashMap::new();
    for profile in &next_profiles {
        model_profile_map.insert(normalize_model_ref(&profile_to_model_value(profile)), profile.id.clone());
    }

    for binding in bindings {
        let scope_label = match binding.scope.as_str() {
            "global" => "global".to_string(),
            "agent" => format!("agent:{}", binding.scope_id),
            "channel" => format!("channel:{}", binding.scope_id),
            _ => binding.scope_id,
        };
        let Some(model_ref) = binding.model_value else {
            continue;
        };
        let model_ref = normalize_model_ref(&model_ref);
        if model_ref.trim().is_empty() {
            continue;
        }
        if model_profile_map.contains_key(&model_ref) || seen.contains(&model_ref) {
            reused += 1;
            continue;
        }
        let mut parts = model_ref.splitn(2, '/');
        let provider = parts.next().unwrap_or("").trim();
        let model = parts.next().unwrap_or("").trim();
        if provider.is_empty() || model.is_empty() {
            skipped_invalid += 1;
            continue;
        }
        let auth_ref = resolve_auth_ref_for_provider(&cfg, provider)
            .unwrap_or_else(|| format!("{provider}:default"));
        let base_url = resolve_model_provider_base_url(&cfg, provider);
        let profile = ModelProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name: format!("{scope_label} model profile"),
            provider: provider.to_string(),
            model: model.to_string(),
            auth_ref,
            api_key: None,
            base_url,
            description: Some(format!("Extracted from config ({scope_label})")),
            enabled: true,
        };
        let key = profile_to_model_value(&profile);
        model_profile_map.insert(normalize_model_ref(&key), profile.id.clone());
        next_profiles.push(profile);
        seen.insert(model_ref);
        created += 1;
    }

    if created > 0 {
        save_model_profiles(&paths, &next_profiles)?;
    }

    Ok(ExtractModelProfilesResult {
        created,
        reused,
        skipped_invalid,
    })
}

#[tauri::command]
pub fn upsert_model_profile(mut profile: ModelProfile) -> Result<ModelProfile, String> {
    if profile.provider.trim().is_empty() || profile.model.trim().is_empty() {
        return Err("provider and model are required".into());
    }
    if profile.name.trim().is_empty() {
        profile.name = format!("{}/{}", profile.provider, profile.model);
    }
    let has_api_key = profile.api_key.as_ref().is_some_and(|k| !k.trim().is_empty());
    if profile.auth_ref.trim().is_empty() && !has_api_key {
        // Auto-resolve auth ref from openclaw config or env vars
        let paths_tmp = resolve_paths();
        if let Ok(cfg) = read_openclaw_config(&paths_tmp) {
            if let Some(auth_ref) = resolve_auth_ref_for_provider(&cfg, &profile.provider) {
                profile.auth_ref = auth_ref;
            }
        }
        if profile.auth_ref.trim().is_empty() {
            // Try env var convention
            let provider_upper = profile.provider.trim().to_uppercase().replace('-', "_");
            for suffix in ["_API_KEY", "_KEY", "_TOKEN"] {
                let env_name = format!("{provider_upper}{suffix}");
                if std::env::var(&env_name).map(|v| !v.trim().is_empty()).unwrap_or(false) {
                    profile.auth_ref = env_name;
                    break;
                }
            }
        }
        if profile.auth_ref.trim().is_empty() {
            return Err("API key or auth env var is required".into());
        }
    }
    let paths = resolve_paths();
    let mut profiles = load_model_profiles(&paths);
    if profile.id.trim().is_empty() {
        profile.id = uuid::Uuid::new_v4().to_string();
    }
    let id = profile.id.clone();
    if let Some(existing) = profiles.iter_mut().find(|p| p.id == id) {
        *existing = profile.clone();
    } else {
        profiles.push(profile.clone());
    }
    save_model_profiles(&paths, &profiles)?;
    Ok(profile)
}

#[tauri::command]
pub fn delete_model_profile(profile_id: String) -> Result<bool, String> {
    let paths = resolve_paths();
    let mut profiles = load_model_profiles(&paths);
    let before = profiles.len();
    profiles.retain(|p| p.id != profile_id);
    if profiles.len() == before {
        return Ok(false);
    }
    save_model_profiles(&paths, &profiles)?;
    Ok(true)
}

#[tauri::command]
pub fn resolve_provider_auth(provider: String) -> Result<ProviderAuthSuggestion, String> {
    let provider_trimmed = provider.trim();
    if provider_trimmed.is_empty() {
        return Ok(ProviderAuthSuggestion { auth_ref: None, has_key: false, source: String::new() });
    }
    let paths = resolve_paths();
    let cfg = read_openclaw_config(&paths)?;

    // 1. Check openclaw config auth profiles
    if let Some(auth_ref) = resolve_auth_ref_for_provider(&cfg, provider_trimmed) {
        return Ok(ProviderAuthSuggestion {
            auth_ref: Some(auth_ref),
            has_key: true,
            source: "openclaw auth profile".into(),
        });
    }

    // 2. Check env vars
    let provider_upper = provider_trimmed.to_uppercase().replace('-', "_");
    for suffix in ["_API_KEY", "_KEY", "_TOKEN"] {
        let env_name = format!("{provider_upper}{suffix}");
        if std::env::var(&env_name).map(|v| !v.trim().is_empty()).unwrap_or(false) {
            return Ok(ProviderAuthSuggestion {
                auth_ref: Some(env_name),
                has_key: true,
                source: "environment variable".into(),
            });
        }
    }

    // 3. Check existing model profiles for this provider
    let profiles = load_model_profiles(&paths);
    for p in &profiles {
        if p.provider.eq_ignore_ascii_case(provider_trimmed) {
            let key = resolve_profile_api_key(p, &paths.base_dir);
            if !key.is_empty() {
                let auth_ref = if !p.auth_ref.trim().is_empty() {
                    Some(p.auth_ref.clone())
                } else {
                    None
                };
                return Ok(ProviderAuthSuggestion {
                    auth_ref,
                    has_key: true,
                    source: format!("existing profile {}/{}", p.provider, p.model),
                });
            }
        }
    }

    Ok(ProviderAuthSuggestion { auth_ref: None, has_key: false, source: String::new() })
}

#[tauri::command]
pub async fn list_channels() -> Result<Vec<ChannelNode>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let paths = resolve_paths();
        let cfg = read_openclaw_config(&paths)?;
        let mut nodes = collect_channel_nodes(&cfg);
        enrich_channel_display_names(&paths, &cfg, &mut nodes)?;
        Ok(nodes)
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn list_channels_minimal() -> Result<Vec<ChannelNode>, String> {
    let output = crate::cli_runner::run_openclaw(&["config", "get", "channels", "--json"])
        .map_err(|e| format!("Failed to run openclaw: {e}"))?;
    if output.exit_code != 0 {
        let msg = format!("{} {}", output.stderr, output.stdout).to_lowercase();
        if msg.contains("not found") {
            return Ok(Vec::new());
        }
        // Fallback: direct read
        let paths = resolve_paths();
        let cfg = read_openclaw_config(&paths)?;
        return Ok(collect_channel_nodes(&cfg));
    }
    let channels_val = crate::cli_runner::parse_json_output(&output).unwrap_or(Value::Null);
    let cfg = serde_json::json!({ "channels": channels_val });
    Ok(collect_channel_nodes(&cfg))
}

/// Read Discord guild/channels from persistent cache. Fast, no subprocess.
#[tauri::command]
pub fn list_discord_guild_channels() -> Result<Vec<DiscordGuildChannel>, String> {
    let paths = resolve_paths();
    let cache_file = paths.clawpal_dir.join("discord-guild-channels.json");
    if cache_file.exists() {
        let text = fs::read_to_string(&cache_file).map_err(|e| e.to_string())?;
        let entries: Vec<DiscordGuildChannel> = serde_json::from_str(&text).unwrap_or_default();
        return Ok(entries);
    }
    Ok(Vec::new())
}

/// Resolve Discord guild/channel names via openclaw CLI and persist to cache.
#[tauri::command]
pub async fn refresh_discord_guild_channels() -> Result<Vec<DiscordGuildChannel>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let paths = resolve_paths();
        ensure_dirs(&paths)?;
        let cfg = read_openclaw_config(&paths)?;

        let discord_cfg = cfg
            .get("channels")
            .and_then(|c| c.get("discord"));

        // Extract bot token: top-level first, then fall back to first account token
        let bot_token = discord_cfg
            .and_then(|d| d.get("botToken").or_else(|| d.get("token")))
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .or_else(|| {
                discord_cfg
                    .and_then(|d| d.get("accounts"))
                    .and_then(Value::as_object)
                    .and_then(|accounts| {
                        accounts.values().find_map(|acct| {
                            acct.get("token").and_then(Value::as_str).filter(|s| !s.is_empty()).map(|s| s.to_string())
                        })
                    })
            });

        let mut entries: Vec<DiscordGuildChannel> = Vec::new();
        let mut channel_ids: Vec<String> = Vec::new();
        let mut unresolved_guild_ids: Vec<String> = Vec::new();

        // Helper: collect guilds from a guilds object
        let mut collect_guilds = |guilds: &serde_json::Map<String, Value>| {
            for (guild_id, guild_val) in guilds {
                let guild_name = guild_val
                    .get("slug")
                    .or_else(|| guild_val.get("name"))
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| guild_id.clone());

                if guild_name == *guild_id && !unresolved_guild_ids.contains(guild_id) {
                    unresolved_guild_ids.push(guild_id.clone());
                }

                if let Some(channels) = guild_val.get("channels").and_then(Value::as_object) {
                    for (channel_id, _channel_val) in channels {
                        if entries.iter().any(|e| e.guild_id == *guild_id && e.channel_id == *channel_id) {
                            continue;
                        }
                        channel_ids.push(channel_id.clone());
                        entries.push(DiscordGuildChannel {
                            guild_id: guild_id.clone(),
                            guild_name: guild_name.clone(),
                            channel_id: channel_id.clone(),
                            channel_name: channel_id.clone(),
                        });
                    }
                }
            }
        };

        // Collect from channels.discord.guilds (top-level structured config)
        if let Some(guilds) = discord_cfg.and_then(|d| d.get("guilds")).and_then(Value::as_object) {
            collect_guilds(guilds);
        }

        // Collect from channels.discord.accounts.<accountId>.guilds (multi-account config)
        if let Some(accounts) = discord_cfg.and_then(|d| d.get("accounts")).and_then(Value::as_object) {
            for (_account_id, account_val) in accounts {
                if let Some(guilds) = account_val.get("guilds").and_then(Value::as_object) {
                    collect_guilds(guilds);
                }
            }
        }

        drop(collect_guilds); // Release mutable borrows before bindings section

        // Also collect from bindings array (users may only have bindings, no guilds map)
        if let Some(bindings) = cfg.get("bindings").and_then(Value::as_array) {
            for b in bindings {
                let m = match b.get("match") {
                    Some(m) => m,
                    None => continue,
                };
                if m.get("channel").and_then(Value::as_str) != Some("discord") {
                    continue;
                }
                let guild_id = match m.get("guildId") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Number(n)) => n.to_string(),
                    _ => continue,
                };
                let channel_id = match m.pointer("/peer/id") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Number(n)) => n.to_string(),
                    _ => continue,
                };
                // Skip if already collected from guilds map
                if entries.iter().any(|e| e.guild_id == guild_id && e.channel_id == channel_id) {
                    continue;
                }
                if !unresolved_guild_ids.contains(&guild_id) {
                    unresolved_guild_ids.push(guild_id.clone());
                }
                channel_ids.push(channel_id.clone());
                entries.push(DiscordGuildChannel {
                    guild_id: guild_id.clone(),
                    guild_name: guild_id.clone(),
                    channel_id: channel_id.clone(),
                    channel_name: channel_id.clone(),
                });
            }
        }

        if entries.is_empty() {
            return Ok(Vec::new());
        }

        // Resolve channel names via openclaw CLI
        if !channel_ids.is_empty() {
            let mut args = vec![
                "channels", "resolve", "--json",
                "--channel", "discord",
                "--kind", "auto",
            ];
            let id_refs: Vec<&str> = channel_ids.iter().map(String::as_str).collect();
            args.extend_from_slice(&id_refs);

            if let Ok(output) = run_openclaw_raw(&args) {
                if let Some(name_map) = parse_resolve_name_map(&output.stdout) {
                    for entry in &mut entries {
                        if let Some(name) = name_map.get(&entry.channel_id) {
                            entry.channel_name = name.clone();
                        }
                    }
                }
            }
        }

        // Resolve guild names via Discord REST API
        if let Some(token) = &bot_token {
            if !unresolved_guild_ids.is_empty() {
                let mut guild_name_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
                for gid in &unresolved_guild_ids {
                    if let Ok(name) = fetch_discord_guild_name(token, gid) {
                        guild_name_map.insert(gid.clone(), name);
                    }
                }
                for entry in &mut entries {
                    if let Some(name) = guild_name_map.get(&entry.guild_id) {
                        entry.guild_name = name.clone();
                    }
                }
            }
        }

        // Persist to cache
        let cache_file = paths.clawpal_dir.join("discord-guild-channels.json");
        let json = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
        write_text(&cache_file, &json)?;

        Ok(entries)
    }).await.map_err(|e| e.to_string())?
}

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
    set_nested_value(&mut cfg, &format!("{path}.type"), channel_type.map(Value::String))?;
    set_nested_value(&mut cfg, &format!("{path}.mode"), mode.map(Value::String))?;
    let allowlist_values = allowlist
        .into_iter()
        .map(Value::String)
        .collect::<Vec<_>>();
    set_nested_value(&mut cfg, &format!("{path}.allowlist"), Some(Value::Array(allowlist_values)))?;
    set_nested_value(&mut cfg, &format!("{path}.model"), model.map(Value::String))?;
    write_config_with_snapshot(&paths, &current, &cfg, "update-channel")?;
    Ok(true)
}

/// List current channel→agent bindings from config.
#[tauri::command]
pub async fn list_bindings(
    cache: tauri::State<'_, crate::cli_runner::CliCache>,
) -> Result<Vec<Value>, String> {
    let cache_key = "local:bindings";
    if let Some(cached) = cache.get(cache_key, None) {
        return serde_json::from_str(&cached).map_err(|e| e.to_string());
    }
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let output = crate::cli_runner::run_openclaw(&["config", "get", "bindings", "--json"])?;
        // "bindings" may not exist yet — treat "not found" as empty
        if output.exit_code != 0 {
            let msg = format!("{} {}", output.stderr, output.stdout).to_lowercase();
            if msg.contains("not found") {
                return Ok(Vec::new());
            }
        }
        let json = crate::cli_runner::parse_json_output(&output)?;
        let result = json.as_array().cloned().unwrap_or_default();
        if let Ok(serialized) = serde_json::to_string(&result) {
            cache.set(cache_key.to_string(), serialized);
        }
        Ok(result)
    }).await.map_err(|e| e.to_string())?
}

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
    let model = model_value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    // If existing model is an object (has fallbacks etc.), only update "primary" inside it
    if let Some(existing) = cfg.pointer_mut("/agents/defaults/model") {
        if let Some(model_obj) = existing.as_object_mut() {
            match model {
                Some(v) => { model_obj.insert("primary".into(), Value::String(v)); }
                None => { model_obj.remove("primary"); }
            }
            write_config_with_snapshot(&paths, &current, &cfg, "set-global-model")?;
            return Ok(true);
        }
    }
    // Fallback: plain string or missing — set the whole value
    set_nested_value(
        &mut cfg,
        "agents.defaults.model",
        model.map(Value::String),
    )?;
    write_config_with_snapshot(&paths, &current, &cfg, "set-global-model")?;
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
    let value = model_value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
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
    let value = model_value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
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

#[tauri::command]
pub async fn list_agents_overview(
    cache: tauri::State<'_, crate::cli_runner::CliCache>,
) -> Result<Vec<AgentOverview>, String> {
    let cache_key = "local:agents-list";
    if let Some(cached) = cache.get(cache_key, None) {
        return serde_json::from_str(&cached).map_err(|e| e.to_string());
    }
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let output = crate::cli_runner::run_openclaw(&["agents", "list", "--json"])?;
        let json = crate::cli_runner::parse_json_output(&output)?;
        let result = parse_agents_cli_output(&json, None)?;
        if let Ok(serialized) = serde_json::to_string(&result) {
            cache.set(cache_key.to_string(), serialized);
        }
        Ok(result)
    }).await.map_err(|e| e.to_string())?
}

/// Check if an agent has active sessions by examining sessions/sessions.json.
/// Returns true if the file exists and is larger than 2 bytes (i.e. not just "{}").
fn agent_has_sessions(base_dir: &std::path::Path, agent_id: &str) -> bool {
    let sessions_file = base_dir.join("agents").join(agent_id).join("sessions").join("sessions.json");
    match std::fs::metadata(&sessions_file) {
        Ok(m) => m.len() > 2, // "{}" is 2 bytes = empty
        Err(_) => false,
    }
}

/// Parse the JSON output of `openclaw agents list --json` into Vec<AgentOverview>.
/// `online_set`: if Some, use it to determine online status; if None, check local sessions.
fn parse_agents_cli_output(json: &Value, online_set: Option<&std::collections::HashSet<String>>) -> Result<Vec<AgentOverview>, String> {
    let arr = json.as_array().ok_or("agents list output is not an array")?;
    let paths = if online_set.is_none() { Some(resolve_paths()) } else { None };
    let mut agents = Vec::new();
    for entry in arr {
        let id = entry.get("id").and_then(Value::as_str).unwrap_or("main").to_string();
        let name = entry.get("identityName").and_then(Value::as_str).map(|s| s.to_string());
        let emoji = entry.get("identityEmoji").and_then(Value::as_str).map(|s| s.to_string());
        let model = entry.get("model").and_then(Value::as_str).map(|s| s.to_string());
        let workspace = entry.get("workspace").and_then(Value::as_str).map(|s| s.to_string());
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

#[tauri::command]
pub fn create_agent(
    agent_id: String,
    model_value: Option<String>,
    independent: Option<bool>,
) -> Result<AgentOverview, String> {
    let agent_id = agent_id.trim().to_string();
    if agent_id.is_empty() {
        return Err("Agent ID is required".into());
    }
    if !agent_id.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err("Agent ID may only contain letters, numbers, hyphens, and underscores".into());
    }

    let paths = resolve_paths();
    let mut cfg = read_openclaw_config(&paths)?;
    let current = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;

    let existing_ids = collect_agent_ids(&cfg);
    if existing_ids.iter().any(|id| id.eq_ignore_ascii_case(&agent_id)) {
        return Err(format!("Agent '{}' already exists", agent_id));
    }

    let model_display = model_value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());

    // If independent, create a dedicated workspace directory;
    // otherwise inherit the default workspace so the gateway doesn't auto-create one.
    let workspace = if independent.unwrap_or(false) {
        let ws_dir = paths.base_dir.join("workspaces").join(&agent_id);
        fs::create_dir_all(&ws_dir).map_err(|e| e.to_string())?;
        let ws_path = ws_dir.to_string_lossy().to_string();
        Some(ws_path)
    } else {
        cfg.pointer("/agents/defaults/workspace")
            .or_else(|| cfg.pointer("/agents/default/workspace"))
            .and_then(Value::as_str)
            .map(|s| s.to_string())
    };

    // Build agent entry
    let mut agent_obj = serde_json::Map::new();
    agent_obj.insert("id".into(), Value::String(agent_id.clone()));
    if let Some(ref model_str) = model_display {
        agent_obj.insert("model".into(), Value::String(model_str.clone()));
    }
    if let Some(ref ws) = workspace {
        agent_obj.insert("workspace".into(), Value::String(ws.clone()));
    }

    let agents = cfg
        .as_object_mut()
        .ok_or("config is not an object")?
        .entry("agents")
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .ok_or("agents is not an object")?;
    let list = agents
        .entry("list")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("agents.list is not an array")?;
    list.push(Value::Object(agent_obj));

    write_config_with_snapshot(&paths, &current, &cfg, "create-agent")?;
    Ok(AgentOverview {
        id: agent_id,
        name: None,
        emoji: None,
        model: model_display,
        channels: vec![],
        online: false,
        workspace,
    })
}

#[tauri::command]
pub fn delete_agent(agent_id: String) -> Result<bool, String> {
    let agent_id = agent_id.trim().to_string();
    if agent_id.is_empty() {
        return Err("Agent ID is required".into());
    }
    if agent_id == "main" {
        return Err("Cannot delete the main agent".into());
    }

    let paths = resolve_paths();
    let mut cfg = read_openclaw_config(&paths)?;
    let current = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;

    let list = cfg
        .pointer_mut("/agents/list")
        .and_then(Value::as_array_mut)
        .ok_or("agents.list not found")?;

    let before = list.len();
    list.retain(|agent| {
        agent.get("id").and_then(Value::as_str) != Some(&agent_id)
    });

    if list.len() == before {
        return Err(format!("Agent '{}' not found", agent_id));
    }

    // Reset any bindings that reference this agent back to "main" (default)
    // so the channel doesn't lose its binding entry entirely.
    if let Some(bindings) = cfg.pointer_mut("/bindings").and_then(Value::as_array_mut) {
        for b in bindings.iter_mut() {
            if b.get("agentId").and_then(Value::as_str) == Some(&agent_id) {
                if let Some(obj) = b.as_object_mut() {
                    obj.insert("agentId".into(), Value::String("main".into()));
                }
            }
        }
    }

    write_config_with_snapshot(&paths, &current, &cfg, "delete-agent")?;
    Ok(true)
}

#[tauri::command]
pub fn setup_agent_identity(
    agent_id: String,
    name: String,
    emoji: Option<String>,
) -> Result<bool, String> {
    let agent_id = agent_id.trim().to_string();
    let name = name.trim().to_string();
    if agent_id.is_empty() {
        return Err("Agent ID is required".into());
    }
    if name.is_empty() {
        return Err("Name is required".into());
    }

    let paths = resolve_paths();
    let cfg = read_openclaw_config(&paths)?;

    // Find the agent's workspace
    let agents_list = cfg.pointer("/agents/list")
        .and_then(Value::as_array)
        .ok_or("agents.list not found")?;

    let agent = agents_list.iter()
        .find(|a| a.get("id").and_then(Value::as_str) == Some(&agent_id))
        .ok_or_else(|| format!("Agent '{}' not found", agent_id))?;

    let default_workspace = cfg.pointer("/agents/defaults/workspace")
        .or_else(|| cfg.pointer("/agents/default/workspace"))
        .and_then(Value::as_str)
        .map(|s| expand_tilde(s));

    let workspace = agent.get("workspace")
        .and_then(Value::as_str)
        .map(|s| expand_tilde(s))
        .or(default_workspace)
        .ok_or_else(|| format!("Agent '{}' has no workspace configured", agent_id))?;

    // Build IDENTITY.md content
    let mut content = format!("- Name: {}\n", name);
    if let Some(ref e) = emoji {
        let e = e.trim();
        if !e.is_empty() {
            content.push_str(&format!("- Emoji: {}\n", e));
        }
    }

    let ws_path = std::path::Path::new(&workspace);
    fs::create_dir_all(ws_path).map_err(|e| format!("Failed to create workspace dir: {}", e))?;
    let identity_path = ws_path.join("IDENTITY.md");
    fs::write(&identity_path, &content)
        .map_err(|e| format!("Failed to write IDENTITY.md: {}", e))?;

    Ok(true)
}

#[tauri::command]
pub async fn remote_setup_agent_identity(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    agent_id: String,
    name: String,
    emoji: Option<String>,
) -> Result<bool, String> {
    let agent_id = agent_id.trim().to_string();
    let name = name.trim().to_string();
    if agent_id.is_empty() {
        return Err("Agent ID is required".into());
    }
    if name.is_empty() {
        return Err("Name is required".into());
    }

    // Read remote config to find agent workspace
    let raw = pool.sftp_read(&host_id, "~/.openclaw/openclaw.json").await?;
    let cfg: Value = serde_json::from_str(&raw).map_err(|e| format!("Failed to parse config: {e}"))?;

    let agents_list = cfg.pointer("/agents/list")
        .and_then(Value::as_array)
        .ok_or("agents.list not found")?;

    let agent = agents_list.iter()
        .find(|a| a.get("id").and_then(Value::as_str) == Some(&agent_id))
        .ok_or_else(|| format!("Agent '{}' not found", agent_id))?;

    let default_workspace = cfg.pointer("/agents/defaults/workspace")
        .or_else(|| cfg.pointer("/agents/default/workspace"))
        .and_then(Value::as_str)
        .unwrap_or("~/.openclaw/agents");

    let workspace = agent.get("workspace")
        .and_then(Value::as_str)
        .unwrap_or(default_workspace);

    // Build IDENTITY.md content
    let mut content = format!("- Name: {}\n", name);
    if let Some(ref e) = emoji {
        let e = e.trim();
        if !e.is_empty() {
            content.push_str(&format!("- Emoji: {}\n", e));
        }
    }

    // Write via SSH
    let ws = if workspace.starts_with("~/") { workspace.to_string() } else { format!("~/{workspace}") };
    pool.exec(&host_id, &format!("mkdir -p {}", shell_escape(&ws))).await?;
    let identity_path = format!("{}/IDENTITY.md", ws);
    pool.sftp_write(&host_id, &identity_path, &content).await?;

    Ok(true)
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        if let Some(home) = std::env::var("HOME").ok() {
            return format!("{}{}", home, &path[1..]);
        }
    }
    path.to_string()
}

#[tauri::command]
pub fn list_session_files() -> Result<Vec<SessionFile>, String> {
    let paths = resolve_paths();
    list_session_files_detailed(&paths.base_dir)
}

#[tauri::command]
pub fn clear_all_sessions() -> Result<usize, String> {
    let paths = resolve_paths();
    clear_agent_and_global_sessions(&paths.base_dir.join("agents"), None)
}

#[tauri::command]
pub async fn analyze_sessions() -> Result<Vec<AgentSessionAnalysis>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        analyze_sessions_sync()
    })
    .await
    .map_err(|e| e.to_string())?
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
                    metadata.modified().ok()
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
            cat_order(&a.category).cmp(&cat_order(&b.category))
                .then(b.age_days.partial_cmp(&a.age_days).unwrap_or(std::cmp::Ordering::Equal))
        });

        let total_files = agent_sessions.len();
        let total_size_bytes = agent_sessions.iter().map(|s| s.size_bytes).sum();
        let empty_count = agent_sessions.iter().filter(|s| s.category == "empty").count();
        let low_value_count = agent_sessions.iter().filter(|s| s.category == "low_value").count();
        let valuable_count = agent_sessions.iter().filter(|s| s.category == "valuable").count();

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

#[tauri::command]
pub async fn delete_sessions_by_ids(agent_id: String, session_ids: Vec<String>) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || {
        delete_sessions_by_ids_sync(&agent_id, &session_ids)
    })
    .await
    .map_err(|e| e.to_string())?
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
                let _ = fs::write(&sessions_json_path, serde_json::to_string(&data).unwrap_or_default());
            }
        }
    }

    Ok(deleted)
}

#[tauri::command]
pub async fn preview_session(agent_id: String, session_id: String) -> Result<Vec<Value>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        preview_session_sync(&agent_id, &session_id)
    })
    .await
    .map_err(|e| e.to_string())?
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
            let role = obj.pointer("/message/role").and_then(Value::as_str).unwrap_or("unknown");
            let content = obj.pointer("/message/content")
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
pub fn apply_config_patch(
    patch_template: String,
    params: Map<String, Value>,
) -> Result<ApplyResult, String> {
    let paths = resolve_paths();
    ensure_dirs(&paths)?;
    let current = read_openclaw_config(&paths)?;
    let current_text = serde_json::to_string_pretty(&current).map_err(|e| e.to_string())?;
    let snapshot = add_snapshot(
        &paths.history_dir,
        &paths.metadata_path,
        Some("config-patch".into()),
        "apply",
        true,
        &current_text,
        None,
    )?;
    let (candidate, _changes) = build_candidate_config_from_template(&current, &patch_template, &params)?;
    write_json(&paths.config_path, &candidate)?;
    Ok(ApplyResult {
        ok: true,
        snapshot_id: Some(snapshot.id),
        config_path: paths.config_path.to_string_lossy().to_string(),
        backup_path: Some(snapshot.config_path),
        warnings: Vec::new(),
        errors: Vec::new(),
    })
}

#[tauri::command]
pub async fn restart_gateway() -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_openclaw_raw(&["gateway", "restart"])?;
        Ok(true)
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn list_history(limit: usize, offset: usize) -> Result<HistoryPage, String> {
    let paths = resolve_paths();
    let index = list_snapshots(&paths.metadata_path)?;
    let items = index
        .items
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|item| HistoryItem {
            id: item.id,
            recipe_id: item.recipe_id,
            created_at: item.created_at,
            source: item.source,
            can_rollback: item.can_rollback,
            rollback_of: item.rollback_of,
        })
        .collect();
    Ok(HistoryPage { items })
}

#[tauri::command]
pub fn preview_rollback(snapshot_id: String) -> Result<PreviewResult, String> {
    let paths = resolve_paths();
    let index = list_snapshots(&paths.metadata_path)?;
    let target = index
        .items
        .into_iter()
        .find(|s| s.id == snapshot_id)
        .ok_or_else(|| "snapshot not found".to_string())?;
    if !target.can_rollback {
        return Err("snapshot is not rollbackable".to_string());
    }

    let current = read_openclaw_config(&paths)?;
    let target_text = read_snapshot(&target.config_path)?;
    let target_json: Value = json5::from_str(&target_text).unwrap_or(Value::Object(Default::default()));
    let before_text = serde_json::to_string_pretty(&current).unwrap_or_else(|_| "{}".into());
    let after_text = serde_json::to_string_pretty(&target_json).unwrap_or_else(|_| "{}".into());
    Ok(PreviewResult {
        recipe_id: "rollback".into(),
        diff: format_diff(&current, &target_json),
        config_before: before_text,
        config_after: after_text,
        changes: collect_change_paths(&current, &target_json),
        overwrites_existing: true,
        can_rollback: true,
        impact_level: "medium".into(),
        warnings: vec!["Rollback will replace current configuration".into()],
    })
}

#[tauri::command]
pub fn rollback(snapshot_id: String) -> Result<ApplyResult, String> {
    let paths = resolve_paths();
    ensure_dirs(&paths)?;
    let index = list_snapshots(&paths.metadata_path)?;
    let target = index
        .items
        .into_iter()
        .find(|s| s.id == snapshot_id)
        .ok_or_else(|| "snapshot not found".to_string())?;
    if !target.can_rollback {
        return Err("snapshot is not rollbackable".to_string());
    }
    let target_text = read_snapshot(&target.config_path)?;
    let backup = read_openclaw_config(&paths)?;
    let backup_text = serde_json::to_string_pretty(&backup).map_err(|e| e.to_string())?;
    let _ = add_snapshot(
        &paths.history_dir,
        &paths.metadata_path,
        target.recipe_id.clone(),
        "rollback",
        true,
        &backup_text,
        Some(target.id.clone()),
    )?;
    write_text(&paths.config_path, &target_text)?;
    Ok(ApplyResult {
        ok: true,
        snapshot_id: Some(target.id),
        config_path: paths.config_path.to_string_lossy().to_string(),
        backup_path: None,
        warnings: vec!["rolled back".into()],
        errors: Vec::new(),
    })
}

#[tauri::command]
pub fn run_doctor_command() -> Result<DoctorReport, String> {
    let paths = resolve_paths();
    Ok(run_doctor(&paths))
}

#[tauri::command]
pub fn fix_issues(ids: Vec<String>) -> Result<FixResult, String> {
    let paths = resolve_paths();
    let issues = run_doctor(&paths);
    let mut fixable = Vec::new();
    for issue in issues.issues {
        if ids.contains(&issue.id) && issue.auto_fixable {
            fixable.push(issue.id);
        }
    }
    let auto_applied = apply_auto_fixes(&paths, &fixable);
    let mut remaining = Vec::new();
    let mut applied = Vec::new();
    for id in ids {
        if fixable.contains(&id) && auto_applied.iter().any(|x| x == &id) {
            applied.push(id);
        } else {
            remaining.push(id);
        }
    }
    Ok(FixResult {
        ok: true,
        applied,
        remaining_issues: remaining,
    })
}

#[tauri::command]
pub async fn remote_fix_issues(pool: State<'_, SshConnectionPool>, host_id: String, ids: Vec<String>) -> Result<FixResult, String> {
    let raw = pool.sftp_read(&host_id, "~/.openclaw/openclaw.json").await?;
    let mut cfg: Value = json5::from_str(&raw).unwrap_or_else(|_| Value::Object(Default::default()));
    let mut applied = Vec::new();

    for id in &ids {
        match id.as_str() {
            "field.agents" if cfg.get("agents").is_none() => {
                let mut agents = serde_json::Map::new();
                let mut defaults = serde_json::Map::new();
                defaults.insert("model".into(), Value::String("anthropic/claude-sonnet-4-5".into()));
                agents.insert("defaults".into(), Value::Object(defaults));
                if let Value::Object(map) = &mut cfg {
                    map.insert("agents".into(), Value::Object(agents));
                }
                applied.push(id.clone());
            }
            "json.syntax" => {
                // If we got here, json5 already parsed it or fell back to empty object
                applied.push(id.clone());
            }
            "field.port" => {
                let mut gateway = cfg
                    .get("gateway")
                    .and_then(|v| v.as_object())
                    .cloned()
                    .unwrap_or_default();
                gateway.insert("port".into(), Value::Number(serde_json::Number::from(18789_u64)));
                if let Value::Object(map) = &mut cfg {
                    map.insert("gateway".into(), Value::Object(gateway));
                }
                applied.push(id.clone());
            }
            _ => {}
        }
    }

    if !applied.is_empty() {
        let new_text = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
        remote_write_config_with_snapshot(&pool, &host_id, &raw, &cfg, "doctor-fix").await?;
        let _ = new_text; // written by remote_write_config_with_snapshot
    }

    let remaining: Vec<String> = ids.into_iter().filter(|id| !applied.contains(id)).collect();
    Ok(FixResult {
        ok: true,
        applied,
        remaining_issues: remaining,
    })
}

fn collect_model_summary(cfg: &Value) -> ModelSummary {
    let global_default_model = cfg
        .pointer("/agents/defaults/model")
        .and_then(|value| read_model_value(value))
        .or_else(|| cfg.pointer("/agents/default/model").and_then(|value| read_model_value(value)));

    let mut agent_overrides = Vec::new();
    if let Some(agents) = cfg.pointer("/agents/list").and_then(Value::as_array) {
        for agent in agents {
            if let Some(model_value) = agent.get("model").and_then(read_model_value) {
                let should_emit = global_default_model
                    .as_ref()
                    .map(|global| global != &model_value)
                    .unwrap_or(true);
                if should_emit {
                    let id = agent
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("agent");
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


fn run_openclaw_raw(args: &[&str]) -> Result<OpenclawCommandOutput, String> {
    run_openclaw_raw_timeout(args, None)
}

fn run_openclaw_raw_timeout(args: &[&str], timeout_secs: Option<u64>) -> Result<OpenclawCommandOutput, String> {
    let mut child = Command::new(resolve_openclaw_bin())
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
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
            stdout: String::from_utf8_lossy(&output.stdout).trim_end().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim_end().to_string(),
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

/// Strip leading non-JSON lines from CLI output (plugin logs, ANSI codes, etc.)
fn extract_json_from_output(raw: &str) -> Option<&str> {
    let start = raw.find('{').or_else(|| raw.find('['))?;
    Some(&raw[start..])
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
        let resolved = item.get("resolved").and_then(Value::as_bool).unwrap_or(false);
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
        let head = filtered.split(|c: char| !c.is_ascii_digit()).next().unwrap_or("");
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

fn read_openclaw_update_cache(
    path: &Path,
) -> Option<OpenclawUpdateCache> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str::<OpenclawUpdateCache>(&text).ok()
}

fn save_openclaw_update_cache(
    path: &Path,
    cache: &OpenclawUpdateCache,
) -> Result<(), String> {
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

fn save_model_catalog_cache(
    path: &Path,
    cache: &ModelCatalogProviderCache,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(cache).map_err(|error| error.to_string())?;
    write_text(path, &text)
}

fn model_catalog_cache_path(paths: &crate::models::OpenClawPaths) -> PathBuf {
    paths.clawpal_dir.join("model-catalog-cache.json")
}

fn normalize_model_ref(raw: &str) -> String {
    raw.trim().to_lowercase().replace('\\', "/")
}

fn resolve_openclaw_version() -> String {
    use std::sync::OnceLock;
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        match run_openclaw_raw(&["--version"]) {
            Ok(output) => extract_version_from_text(&output.stdout).unwrap_or_else(|| "unknown".into()),
            Err(_) => "unknown".into(),
        }
    }).clone()
}

fn check_openclaw_update_cached(paths: &crate::models::OpenClawPaths, force: bool) -> Result<OpenclawUpdateCheck, String> {
    let cache_path = openclaw_update_cache_path(paths);
    let now = unix_timestamp_secs();
    if !force {
        if let Some(cached) = read_openclaw_update_cache(&cache_path) {
            if now.saturating_sub(cached.checked_at) < cached.ttl_seconds {
                let installed_version = cached.installed_version.unwrap_or_else(resolve_openclaw_version);
                let upgrade_available = compare_semver(&installed_version, cached.latest_version.as_deref());
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
    let (latest_version, channel, details, source, upgrade_available) = detect_openclaw_update_cached(&installed_version)
        .unwrap_or((None, None, Some("failed to detect update status".into()), "openclaw-command".into(), false));
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

fn detect_openclaw_update_cached(installed_version: &str) -> Option<(Option<String>, Option<String>, Option<String>, String, bool)> {
    let output = run_openclaw_raw(&["update", "status"]).ok()?;
    if let Some((latest_version, channel, details, upgrade_available)) =
        parse_openclaw_update_json(&output.stdout, installed_version)
    {
        return Some((latest_version, Some(channel), Some(details), "openclaw update status --json".into(), upgrade_available));
    }
    let parsed = parse_openclaw_update_text(&output.stdout);
    if let Some((latest_version, channel, details)) = parsed {
        let source = "openclaw update status".into();
        let available = latest_version
            .as_ref()
            .is_some_and(|latest| compare_semver(installed_version, Some(latest)));
        return Some((latest_version, Some(channel), Some(details), source, available));
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

fn parse_openclaw_update_json(raw: &str, installed_version: &str) -> Option<(Option<String>, String, String, bool)> {
    let json_str = extract_json_from_output(raw)?;
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
    let body: Value = resp.json().map_err(|e| format!("npm registry parse failed: {e}"))?;
    let version = body.get("version").and_then(Value::as_str).map(String::from);
    Ok(version)
}

/// Fetch a Discord guild name via the Discord REST API using a bot token.
fn fetch_discord_guild_name(bot_token: &str, guild_id: &str) -> Result<String, String> {
    let url = format!("https://discord.com/api/v10/guilds/{guild_id}");
    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bot {bot_token}"))
        .send()
        .map_err(|e| format!("Discord API request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Discord API returned status {}", resp.status()));
    }
    let body: Value = resp.json().map_err(|e| format!("Failed to parse Discord response: {e}"))?;
    body.get("name")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| "No name field in Discord guild response".to_string())
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
                    total_bytes: session_info.total_bytes.saturating_add(archive_info.total_bytes),
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
    InventorySummary {
        files,
        total_bytes,
    }
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

fn clear_agent_and_global_sessions(agents_root: &Path, agent_id: Option<&str>) -> Result<usize, String> {
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
                if entry
                    .file_type()
                    .map_err(|e| e.to_string())?
                    .is_dir()
                {
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
        let metadata = entry
            .metadata()
            .map_err(|e| e.to_string())?;
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
    if profile.model.contains('/') {
        profile.model.clone()
    } else {
        format!("{}/{}", profile.provider, profile.model)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedApiKey {
    pub profile_id: String,
    pub masked_key: String,
}

#[tauri::command]
pub fn resolve_api_keys() -> Result<Vec<ResolvedApiKey>, String> {
    let paths = resolve_paths();
    let profiles = load_model_profiles(&paths);
    let mut out = Vec::new();
    for profile in &profiles {
        let key = resolve_profile_api_key(profile, &paths.base_dir);
        let masked = mask_api_key(&key);
        out.push(ResolvedApiKey {
            profile_id: profile.id.clone(),
            masked_key: masked,
        });
    }
    Ok(out)
}

fn resolve_profile_api_key(profile: &ModelProfile, base_dir: &Path) -> String {
    // 1. Direct api_key field (user entered key directly in ClawPal)
    if let Some(ref key) = profile.api_key {
        let trimmed = key.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    // 2. Try auth_ref as env var name directly (e.g. "OPENAI_API_KEY")
    let auth_ref = profile.auth_ref.trim();
    if !auth_ref.is_empty() {
        if let Ok(val) = std::env::var(auth_ref) {
            if !val.trim().is_empty() {
                return val;
            }
        }
    }

    // 3. Look up auth_ref in agent-level auth-profiles.json files
    //    Keys are stored at: {base_dir}/agents/{agent}/agent/auth-profiles.json
    if !auth_ref.is_empty() {
        if let Some(key) = resolve_key_from_agent_auth_profiles(base_dir, auth_ref) {
            return key;
        }
    }

    // 4. Try common env var naming conventions based on provider
    let provider = profile.provider.trim().to_uppercase().replace('-', "_");
    if !provider.is_empty() {
        for suffix in ["_API_KEY", "_KEY", "_TOKEN"] {
            let env_name = format!("{provider}{suffix}");
            if let Ok(val) = std::env::var(&env_name) {
                if !val.trim().is_empty() {
                    return val;
                }
            }
        }
    }

    String::new()
}

/// Reads agent-level auth-profiles.json to find the actual API key/token.
/// Scans all agents and returns the first match.
fn resolve_key_from_agent_auth_profiles(base_dir: &Path, auth_ref: &str) -> Option<String> {
    let agents_dir = base_dir.join("agents");
    if !agents_dir.exists() {
        return None;
    }
    let entries = fs::read_dir(&agents_dir).ok()?;
    for entry in entries.flatten() {
        let auth_file = entry.path().join("agent").join("auth-profiles.json");
        if !auth_file.exists() {
            continue;
        }
        let text = fs::read_to_string(&auth_file).ok()?;
        let data: Value = serde_json::from_str(&text).ok()?;
        if let Some(profiles) = data.get("profiles").and_then(Value::as_object) {
            if let Some(auth_entry) = profiles.get(auth_ref) {
                if let Some(key) = extract_token_from_auth_entry(auth_entry) {
                    return Some(key);
                }
            }
        }
    }
    None
}

/// Extract the actual key/token from an agent auth-profiles entry.
/// Handles different auth types: token, api_key, oauth.
fn extract_token_from_auth_entry(entry: &Value) -> Option<String> {
    // "token" type → "token" field (e.g. anthropic)
    // "api_key" type → "key" field (e.g. kimi-coding)
    // "oauth" type → "access" field (e.g. minimax-portal, openai-codex)
    for field in ["token", "key", "apiKey", "api_key", "access"] {
        if let Some(val) = entry.get(field).and_then(Value::as_str) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
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
    struct Storage {
        #[serde(default)]
        profiles: Vec<ModelProfile>,
    }
    let parsed = serde_json::from_str::<Storage>(&text).unwrap_or(Storage {
        profiles: Vec::new(),
    });
    parsed.profiles
}

fn save_model_profiles(paths: &crate::models::OpenClawPaths, profiles: &[ModelProfile]) -> Result<(), String> {
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
    cfg: &Value,
) -> Result<Vec<ModelCatalogProvider>, String> {
    let now = unix_timestamp_secs();
    let cache_path = model_catalog_cache_path(paths);
    let current_version = resolve_openclaw_version();
    let ttl_seconds = 60 * 60 * 12;
    if let Some(cached) = read_model_catalog_cache(&cache_path)
        .filter(|cache| cache.cli_version == current_version)
    {
        if now.saturating_sub(cached.updated_at) < ttl_seconds && cached.error.is_none() {
            return Ok(cached.providers);
        }
        if cached.error.is_none() {
            if let Some(fresh) = extract_model_catalog_from_cli(paths) {
                if !fresh.is_empty() {
                    return Ok(fresh);
                }
            }
            if !cached.providers.is_empty() {
                return Ok(cached.providers);
            }
        }
    }

    if let Some(catalog) = extract_model_catalog_from_cli(paths) {
        if !catalog.is_empty() {
            let cache = ModelCatalogProviderCache {
                cli_version: current_version,
                updated_at: now,
                providers: catalog.clone(),
                source: "openclaw models list --all --json".into(),
                error: None,
            };
            let _ = save_model_catalog_cache(&cache_path, &cache);
            return Ok(catalog);
        }
    }

    let fallback = collect_model_catalog(cfg);
    if let Some(cached) = read_model_catalog_cache(&cache_path) {
        if !cached.providers.is_empty() {
            let catalog = if fallback.is_empty() {
                cached.providers
            } else {
                fallback
            };
            return Ok(catalog);
        }
    }
    Ok(fallback)
}

/// Parse CLI output from `openclaw models list --all --json` into grouped providers.
/// Handles various output formats: flat arrays, {models: [...]}, {items: [...]}, {data: [...]}.
/// Strips prefix junk (plugin log lines) before the JSON.
fn parse_model_catalog_from_cli_output(raw: &str) -> Option<Vec<ModelCatalogProvider>> {
    let json_str = extract_json_from_output(raw)?;
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
        let entry = providers.entry(provider.clone()).or_insert(ModelCatalogProvider {
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

fn cache_model_catalog(paths: &crate::models::OpenClawPaths, providers: Vec<ModelCatalogProvider>) -> Option<()> {
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

fn collect_model_catalog(cfg: &Value) -> Vec<ModelCatalogProvider> {
    let mut providers: BTreeMap<String, ModelCatalogProvider> = BTreeMap::new();

    if let Some(configured) = cfg.pointer("/models/providers").and_then(Value::as_object) {
        for (provider_name, provider_cfg) in configured {
            let provider_model_map = extract_catalog_models(provider_cfg).unwrap_or_default();
            let base_url = provider_cfg
                .get("baseUrl")
                .or_else(|| provider_cfg.get("base_url"))
                .and_then(Value::as_str)
                .map(str::to_string);

            providers.entry(provider_name.clone())
                .and_modify(|entry| {
                    if entry.base_url.is_none() {
                        entry.base_url = base_url.clone();
                    }
                    for model in provider_model_map.iter() {
                        if !entry.models.iter().any(|item| item.id == model.id) {
                            entry.models.push(model.clone());
                        }
                    }
                })
                .or_insert(ModelCatalogProvider {
                    provider: provider_name.clone(),
                    base_url,
                    models: provider_model_map,
                });
        }
    }

    if let Some(auth_profiles) = cfg.pointer("/auth/profiles").and_then(Value::as_object) {
        for profile in auth_profiles.values() {
            if let Some(provider_name) = profile
                .get("provider")
                .or_else(|| profile.get("name"))
                .and_then(Value::as_str)
            {
                providers.entry(provider_name.to_string())
                    .or_insert(ModelCatalogProvider {
                        provider: provider_name.to_string(),
                        base_url: None,
                        models: Vec::new(),
                    });
            }
        }
    }

    providers.into_values().collect()
}

fn extract_catalog_models(provider_cfg: &Value) -> Option<Vec<ModelCatalogModel>> {
    let mut model_items: BTreeMap<String, String> = BTreeMap::new();
    let raw_models = provider_cfg.get("models")?.as_array()?;
    for model in raw_models {
        let id = model.as_str().map(str::to_string).or_else(|| {
            model
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        if let Some(id) = id {
            let name = model
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string);
            model_items.entry(id).or_insert(name.unwrap_or_default());
        }
    }
    if model_items.is_empty() {
        return None;
    }
    let models = model_items
        .into_iter()
        .map(|(id, name)| ModelCatalogModel {
            id,
            name: (!name.is_empty()).then_some(name),
        })
        .collect();
    Some(models)
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
        let ids: Vec<String> = entries.iter().map(|(_, identifier, _)| identifier.clone()).collect();
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
        let json_str = extract_json_from_output(&output.stdout).unwrap_or("[]");
        let parsed: Vec<Value> = serde_json::from_str(json_str).unwrap_or_default();
        let mut name_map = HashMap::new();
        for item in parsed {
            let input = item.get("input").and_then(Value::as_str).unwrap_or_default().to_string();
            let resolved = item.get("resolved").and_then(Value::as_bool).unwrap_or(false);
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let note = item.get("note").and_then(Value::as_str).map(|value| value.to_string());
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
    write_text(cache_file, &serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?)
}

fn resolve_channel_node_identity(cfg: &Value, node: &ChannelNode) -> Option<(String, String, String)> {
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
        .and_then(|value| value.get("users").or(value.get("members")).or_else(|| value.get("peerIds")))
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
    if prefix.contains(".accounts.") || prefix.contains(".guilds.") || prefix.contains(".channels.") {
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
    if let Some(agents) = cfg.get("agents").and_then(|v| v.get("list")).and_then(Value::as_array) {
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

    if let Some(agents) = cfg.get("agents").and_then(|v| v.get("list")).and_then(Value::as_array) {
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

    fn walk_channel_binding(prefix: &str, node: &Value, out: &mut Vec<ModelBinding>, profiles: &[ModelProfile]) {
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

#[tauri::command]
pub fn read_raw_config() -> Result<String, String> {
    let paths = resolve_paths();
    let cfg = read_openclaw_config(&paths)?;
    serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())
}

// resolve_full_api_key is intentionally not exposed as a Tauri command.
// It returns raw API keys which should never be sent to the frontend.
#[allow(dead_code)]
fn resolve_full_api_key(profile_id: String) -> Result<String, String> {
    let paths = resolve_paths();
    let profiles = load_model_profiles(&paths);
    let profile = profiles.iter().find(|p| p.id == profile_id)
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
        if path.extension().map_or(false, |ext| ext == "app" || ext == "exe") {
            return Err("Cannot open application files".into());
        }
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(&url).spawn().map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open").arg(&url).spawn().map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd").args(["/c", "start", &url]).spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn chat_via_openclaw(agent_id: String, message: String, session_id: Option<String>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut args = vec![
            "agent".to_string(),
            "--local".to_string(),
            "--agent".to_string(),
            agent_id,
            "--message".to_string(),
            message,
            "--json".to_string(),
            "--no-color".to_string(),
        ];
        if let Some(sid) = session_id {
            args.push("--session-id".to_string());
            args.push(sid);
        }

        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let output = run_openclaw_raw(&arg_refs)?;
        let json_str = extract_json_from_output(&output.stdout)
            .ok_or_else(|| format!("No JSON in openclaw output: {}", output.stdout))?;
        serde_json::from_str(json_str)
            .map_err(|e| format!("Parse openclaw response failed: {}", e))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
}

#[tauri::command]
pub async fn remote_chat_via_openclaw(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    agent_id: String,
    message: String,
    session_id: Option<String>,
) -> Result<Value, String> {
    let escaped_msg = message.replace('\'', "'\\''");
    let escaped_agent = agent_id.replace('\'', "'\\''");
    let mut cmd = format!(
        "openclaw agent --local --agent '{}' --message '{}' --json --no-color",
        escaped_agent, escaped_msg
    );
    if let Some(sid) = session_id {
        let escaped_sid = sid.replace('\'', "'\\''");
        cmd.push_str(&format!(" --session-id '{}'", escaped_sid));
    }
    let result = pool.exec_login(&host_id, &cmd).await?;
    if result.exit_code != 0 {
        return Err(format!(
            "Remote chat failed (exit {}): {}",
            result.exit_code, result.stderr
        ));
    }
    let json_str = extract_json_from_output(&result.stdout)
        .ok_or_else(|| format!("No JSON in remote openclaw output: {}", result.stdout))?;
    serde_json::from_str(json_str)
        .map_err(|e| format!("Failed to parse remote chat response: {e}"))
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

#[tauri::command]
pub fn backup_before_upgrade() -> Result<BackupInfo, String> {
    let paths = resolve_paths();
    let backups_dir = paths.clawpal_dir.join("backups");
    fs::create_dir_all(&backups_dir).map_err(|e| format!("Failed to create backups dir: {e}"))?;

    let now_secs = unix_timestamp_secs();
    let now_dt = chrono::DateTime::<chrono::Utc>::from_timestamp(now_secs as i64, 0);
    let name = now_dt
        .map(|dt| dt.format("%Y-%m-%d_%H%M%S").to_string())
        .unwrap_or_else(|| format!("{now_secs}"));
    let backup_dir = backups_dir.join(&name);
    fs::create_dir_all(&backup_dir).map_err(|e| format!("Failed to create backup dir: {e}"))?;

    let mut total_bytes = 0u64;

    // Copy config file
    if paths.config_path.exists() {
        let dest = backup_dir.join("openclaw.json");
        fs::copy(&paths.config_path, &dest).map_err(|e| format!("Failed to copy config: {e}"))?;
        total_bytes += fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    }

    // Copy directories, excluding sessions and archive
    let skip_dirs: HashSet<&str> = ["sessions", "archive", ".clawpal"].iter().copied().collect();
    copy_dir_recursive(&paths.base_dir, &backup_dir, &skip_dirs, &mut total_bytes)?;

    Ok(BackupInfo {
        name: name.clone(),
        path: backup_dir.to_string_lossy().to_string(),
        created_at: format_timestamp_from_unix(now_secs),
        size_bytes: total_bytes,
    })
}

fn copy_dir_recursive(src: &Path, dst: &Path, skip_dirs: &HashSet<&str>, total: &mut u64) -> Result<(), String> {
    let entries = fs::read_dir(src).map_err(|e| format!("Failed to read dir {}: {e}", src.display()))?;
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
            fs::create_dir_all(&dest).map_err(|e| format!("Failed to create dir {}: {e}", dest.display()))?;
            copy_dir_recursive(&entry.path(), &dest, skip_dirs, total)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &dest).map_err(|e| format!("Failed to copy {}: {e}", name_str))?;
            *total += fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn list_backups() -> Result<Vec<BackupInfo>, String> {
    let paths = resolve_paths();
    let backups_dir = paths.clawpal_dir.join("backups");
    if !backups_dir.exists() {
        return Ok(Vec::new());
    }
    let mut backups = Vec::new();
    let entries = fs::read_dir(&backups_dir).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let size = dir_size(&path);
        let created_at = fs::metadata(&path)
            .and_then(|m| m.created())
            .map(|t| {
                let secs = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                format_timestamp_from_unix(secs)
            })
            .unwrap_or_else(|_| name.clone());
        backups.push(BackupInfo {
            name,
            path: path.to_string_lossy().to_string(),
            created_at,
            size_bytes: size,
        });
    }
    backups.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(backups)
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

#[tauri::command]
pub fn restore_from_backup(backup_name: String) -> Result<String, String> {
    let paths = resolve_paths();
    let backup_dir = paths.clawpal_dir.join("backups").join(&backup_name);
    if !backup_dir.exists() {
        return Err(format!("Backup '{}' not found", backup_name));
    }

    // Restore config file
    let backup_config = backup_dir.join("openclaw.json");
    if backup_config.exists() {
        fs::copy(&backup_config, &paths.config_path)
            .map_err(|e| format!("Failed to restore config: {e}"))?;
    }

    // Restore other directories (agents except sessions/archive, memory, etc.)
    let skip_dirs: HashSet<&str> = ["sessions", "archive", ".clawpal"].iter().copied().collect();
    restore_dir_recursive(&backup_dir, &paths.base_dir, &skip_dirs)?;

    Ok(format!("Restored from backup '{}'", backup_name))
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
            fs::copy(entry.path(), &dest).map_err(|e| format!("Failed to restore {}: {e}", name_str))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn delete_backup(backup_name: String) -> Result<bool, String> {
    let paths = resolve_paths();
    let backup_dir = paths.clawpal_dir.join("backups").join(&backup_name);
    if !backup_dir.exists() {
        return Ok(false);
    }
    fs::remove_dir_all(&backup_dir).map_err(|e| format!("Failed to delete backup: {e}"))?;
    Ok(true)
}

// ---- Remote Backup / Restore (via SSH) ----

#[tauri::command]
pub async fn remote_backup_before_upgrade(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<BackupInfo, String> {
    let now_secs = unix_timestamp_secs();
    let now_dt = chrono::DateTime::<chrono::Utc>::from_timestamp(now_secs as i64, 0);
    let name = now_dt
        .map(|dt| dt.format("%Y-%m-%d_%H%M%S").to_string())
        .unwrap_or_else(|| format!("{now_secs}"));

    let escaped_name = shell_escape(&name);
    let cmd = format!(
        concat!(
            "set -e; ",
            "BDIR=\"$HOME/.clawpal/backups/\"{name}; ",
            "mkdir -p \"$BDIR\"; ",
            "cp \"$HOME/.openclaw/openclaw.json\" \"$BDIR/\" 2>/dev/null || true; ",
            "cp -r \"$HOME/.openclaw/agents\" \"$BDIR/\" 2>/dev/null || true; ",
            "cp -r \"$HOME/.openclaw/memory\" \"$BDIR/\" 2>/dev/null || true; ",
            "du -sk \"$BDIR\" 2>/dev/null | awk '{{print $1 * 1024}}' || echo 0"
        ),
        name = escaped_name
    );

    let result = pool.exec_login(&host_id, &cmd).await?;
    if result.exit_code != 0 {
        return Err(format!("Remote backup failed (exit {}): {}", result.exit_code, result.stderr));
    }

    let size_bytes: u64 = result.stdout.trim().lines().last()
        .and_then(|l| l.trim().parse().ok())
        .unwrap_or(0);

    Ok(BackupInfo {
        name,
        path: String::new(),
        created_at: format_timestamp_from_unix(now_secs),
        size_bytes,
    })
}

#[tauri::command]
pub async fn remote_list_backups(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<BackupInfo>, String> {
    // Migrate remote data from legacy path ~/.openclaw/.clawpal → ~/.clawpal
    let _ = pool.exec_login(&host_id, concat!(
        "if [ -d \"$HOME/.openclaw/.clawpal\" ]; then ",
            "mkdir -p \"$HOME/.clawpal\"; ",
            "cp -a \"$HOME/.openclaw/.clawpal/.\" \"$HOME/.clawpal/\" 2>/dev/null; ",
            "rm -rf \"$HOME/.openclaw/.clawpal\"; ",
        "fi"
    )).await;

    // List backup directory names
    let list_result = pool
        .exec_login(&host_id, "ls -1d \"$HOME/.clawpal/backups\"/*/  2>/dev/null || true")
        .await?;

    let dirs: Vec<String> = list_result
        .stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().trim_end_matches('/').to_string())
        .collect();

    if dirs.is_empty() {
        return Ok(Vec::new());
    }

    // Build a single command to get sizes for all backup dirs (du -sk is POSIX portable)
    let du_parts: Vec<String> = dirs
        .iter()
        .map(|d| format!("du -sk '{}' 2>/dev/null || echo '0\t{}'", d, d))
        .collect();
    let du_cmd = du_parts.join("; ");
    let du_result = pool.exec_login(&host_id, &du_cmd).await?;

    let mut size_map = std::collections::HashMap::new();
    for line in du_result.stdout.lines() {
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        if parts.len() == 2 {
            let size_kb: u64 = parts[0].trim().parse().unwrap_or(0);
            let path = parts[1].trim().trim_end_matches('/');
            size_map.insert(path.to_string(), size_kb * 1024);
        }
    }

    let mut backups: Vec<BackupInfo> = dirs
        .iter()
        .map(|d| {
            let name = d.rsplit('/').next().unwrap_or(d).to_string();
            let size_bytes = size_map.get(d.trim_end_matches('/')).copied().unwrap_or(0);
            BackupInfo {
                name: name.clone(),
                path: d.clone(),
                created_at: name.clone(), // Name is the timestamp
                size_bytes,
            }
        })
        .collect();

    backups.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(backups)
}

#[tauri::command]
pub async fn remote_restore_from_backup(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    backup_name: String,
) -> Result<String, String> {
    let escaped_name = shell_escape(&backup_name);
    let cmd = format!(
        concat!(
            "set -e; ",
            "BDIR=\"$HOME/.clawpal/backups/\"{name}; ",
            "[ -d \"$BDIR\" ] || {{ echo 'Backup not found'; exit 1; }}; ",
            "cp \"$BDIR/openclaw.json\" \"$HOME/.openclaw/openclaw.json\" 2>/dev/null || true; ",
            "[ -d \"$BDIR/agents\" ] && cp -r \"$BDIR/agents\" \"$HOME/.openclaw/\" 2>/dev/null || true; ",
            "[ -d \"$BDIR/memory\" ] && cp -r \"$BDIR/memory\" \"$HOME/.openclaw/\" 2>/dev/null || true; ",
            "echo 'Restored from backup '{name}"
        ),
        name = escaped_name
    );

    let result = pool.exec_login(&host_id, &cmd).await?;
    if result.exit_code != 0 {
        return Err(format!("Remote restore failed: {}", result.stderr));
    }

    Ok(format!("Restored from backup '{}'", backup_name))
}

#[tauri::command]
pub async fn remote_delete_backup(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    backup_name: String,
) -> Result<bool, String> {
    let escaped_name = shell_escape(&backup_name);
    let cmd = format!(
        "BDIR=\"$HOME/.clawpal/backups/\"{name}; [ -d \"$BDIR\" ] && rm -rf \"$BDIR\" && echo 'deleted' || echo 'not_found'",
        name = escaped_name
    );

    let result = pool.exec_login(&host_id, &cmd).await?;
    Ok(result.stdout.trim() == "deleted")
}

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

// ---------------------------------------------------------------------------
// Task 3: Remote instance config CRUD
// ---------------------------------------------------------------------------

fn remote_instances_path() -> PathBuf {
    resolve_paths().clawpal_dir.join("remote-instances.json")
}

fn read_hosts_from_disk() -> Result<Vec<SshHostConfig>, String> {
    let path = remote_instances_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = fs::read_to_string(&path).map_err(|e| format!("Failed to read remote-instances.json: {e}"))?;
    serde_json::from_str(&data).map_err(|e| format!("Failed to parse remote-instances.json: {e}"))
}

fn write_hosts_to_disk(hosts: &[SshHostConfig]) -> Result<(), String> {
    let path = remote_instances_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(hosts).map_err(|e| format!("Failed to serialize hosts: {e}"))?;
    fs::write(&path, &json).map_err(|e| format!("Failed to write remote-instances.json: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[tauri::command]
pub fn list_ssh_hosts() -> Result<Vec<SshHostConfig>, String> {
    read_hosts_from_disk()
}

#[tauri::command]
pub fn upsert_ssh_host(host: SshHostConfig) -> Result<SshHostConfig, String> {
    let mut hosts = read_hosts_from_disk()?;
    if let Some(existing) = hosts.iter_mut().find(|h| h.id == host.id) {
        *existing = host.clone();
    } else {
        hosts.push(host.clone());
    }
    write_hosts_to_disk(&hosts)?;
    Ok(host)
}

#[tauri::command]
pub fn delete_ssh_host(host_id: String) -> Result<bool, String> {
    let mut hosts = read_hosts_from_disk()?;
    let before = hosts.len();
    hosts.retain(|h| h.id != host_id);
    let removed = hosts.len() < before;
    write_hosts_to_disk(&hosts)?;
    Ok(removed)
}

// ---------------------------------------------------------------------------
// Task 4: SSH connect / disconnect / status
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn ssh_connect(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<bool, String> {
    // If already connected and handle is alive, reuse
    if pool.is_connected(&host_id).await {
        return Ok(true);
    }
    let hosts = read_hosts_from_disk()?;
    let host = hosts.into_iter().find(|h| h.id == host_id)
        .ok_or_else(|| format!("No SSH host config with id: {host_id}"))?;
    pool.connect(&host).await?;
    Ok(true)
}

#[tauri::command]
pub async fn ssh_disconnect(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<bool, String> {
    pool.disconnect(&host_id).await?;
    Ok(true)
}

#[tauri::command]
pub async fn ssh_status(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<String, String> {
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
pub async fn ssh_exec(pool: State<'_, SshConnectionPool>, host_id: String, command: String) -> Result<SshExecResult, String> {
    pool.exec(&host_id, &command).await
}

#[tauri::command]
pub async fn sftp_read_file(pool: State<'_, SshConnectionPool>, host_id: String, path: String) -> Result<String, String> {
    pool.sftp_read(&host_id, &path).await
}

#[tauri::command]
pub async fn sftp_write_file(pool: State<'_, SshConnectionPool>, host_id: String, path: String, content: String) -> Result<bool, String> {
    pool.sftp_write(&host_id, &path, &content).await?;
    Ok(true)
}

#[tauri::command]
pub async fn sftp_list_dir(pool: State<'_, SshConnectionPool>, host_id: String, path: String) -> Result<Vec<SftpEntry>, String> {
    pool.sftp_list(&host_id, &path).await
}

#[tauri::command]
pub async fn sftp_remove_file(pool: State<'_, SshConnectionPool>, host_id: String, path: String) -> Result<bool, String> {
    pool.sftp_remove(&host_id, &path).await?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Task 6: Remote business commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn remote_read_raw_config(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<String, String> {
    // openclaw config get requires a path — there's no way to dump the full config via CLI.
    // Use sftp_read directly since this function's purpose is returning the entire raw config.
    pool.sftp_read(&host_id, "~/.openclaw/openclaw.json").await
}

#[tauri::command]
pub async fn remote_get_system_status(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<StatusLight, String> {
    // Tier 1: fast, essential — health check + agents config (2 SSH calls in parallel)
    let (config_res, pgrep_res) = tokio::join!(
        crate::cli_runner::run_openclaw_remote(&pool, &host_id, &["config", "get", "agents", "--json"]),
        pool.exec(&host_id, "pgrep -f '[o]penclaw-gateway' >/dev/null 2>&1"),
    );

    let config_ok = matches!(&config_res, Ok(output) if output.exit_code == 0);

    let (active_agents, global_default_model, fallback_models) = match config_res {
        Ok(ref output) if output.exit_code == 0 => {
            let cfg: Value = crate::cli_runner::parse_json_output(output).unwrap_or(Value::Null);
            let explicit = cfg.pointer("/list")
                .and_then(Value::as_array)
                .map(|a| a.len() as u32)
                .unwrap_or(0);
            let agents = if explicit == 0 { 1 } else { explicit };
            let model = cfg.pointer("/defaults/model")
                .and_then(|v| read_model_value(v))
                .or_else(|| cfg.pointer("/default/model").and_then(|v| read_model_value(v)));
            let fallbacks = cfg.pointer("/defaults/model/fallbacks")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).map(String::from).collect())
                .unwrap_or_default();
            (agents, model, fallbacks)
        }
        _ => (0, None, Vec::new()),
    };

    // Avoid false negatives from transient SSH exec failures:
    // if health probe fails but config fetch in the same cycle succeeded,
    // keep health as true instead of flipping to unhealthy.
    let healthy = match pgrep_res {
        Ok(r) => r.exit_code == 0,
        Err(_) if config_ok => true,
        Err(_) => false,
    };

    Ok(StatusLight {
        healthy,
        active_agents,
        global_default_model,
        fallback_models,
    })
}

/// Tier 2: slow, optional — openclaw version + duplicate detection (2 SSH calls in parallel).
/// Called once on mount and on-demand (e.g., after upgrade), not in poll loop.
#[tauri::command]
pub async fn remote_get_status_extra(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<StatusExtra, String> {
    let detect_duplicates_script = concat!(
        "seen=''; for p in $(which -a openclaw 2>/dev/null) ",
        "\"$HOME/.npm-global/bin/openclaw\" \"/usr/local/bin/openclaw\" \"/opt/homebrew/bin/openclaw\"; do ",
        "[ -x \"$p\" ] || continue; ",
        "rp=$(readlink -f \"$p\" 2>/dev/null || echo \"$p\"); ",
        "echo \"$seen\" | grep -qF \"$rp\" && continue; ",
        "seen=\"$seen $rp\"; ",
        "v=$($p --version 2>/dev/null || echo 'unknown'); ",
        "echo \"$p: $v\"; ",
        "done"
    );

    let (version_res, dup_res) = tokio::join!(
        pool.exec_login(&host_id, "openclaw --version"),
        pool.exec_login(&host_id, detect_duplicates_script),
    );

    let openclaw_version = match version_res {
        Ok(r) if r.exit_code == 0 => Some(r.stdout.trim().to_string()),
        Ok(r) => {
            let trimmed = r.stdout.trim().to_string();
            if trimmed.is_empty() { None } else { Some(trimmed) }
        }
        Err(_) => None,
    };

    let duplicate_installs = match dup_res {
        Ok(r) => {
            let entries: Vec<String> = r.stdout.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            if entries.len() > 1 { entries } else { Vec::new() }
        }
        Err(_) => Vec::new(),
    };

    Ok(StatusExtra {
        openclaw_version,
        duplicate_installs,
    })
}

#[tauri::command]
pub async fn remote_check_openclaw_update(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Value, String> {
    // Get installed version and extract clean semver — don't fail if binary not found
    let installed_version = match pool.exec_login(&host_id, "openclaw --version").await {
        Ok(r) => extract_version_from_text(r.stdout.trim())
            .unwrap_or_else(|| r.stdout.trim().to_string()),
        Err(_) => String::new(),
    };

    // Try `openclaw update status --json` first (may not exist on older versions)
    let update_result = pool.exec_login(&host_id, "openclaw update status --json --no-color 2>/dev/null").await;
    if let Ok(r) = update_result {
        if r.exit_code == 0 && !r.stdout.trim().is_empty() {
            if let Some((latest, _channel, _details, upgrade)) =
                parse_openclaw_update_json(&r.stdout, &installed_version)
            {
                return Ok(serde_json::json!({
                    "upgradeAvailable": upgrade,
                    "latestVersion": latest,
                    "installedVersion": installed_version,
                }));
            }
        }
    }

    // Fallback: query npm registry directly from Tauri (no remote CLI dependency)
    // Must use spawn_blocking because reqwest::blocking panics in async context
    let latest_version = tokio::task::spawn_blocking(|| {
        query_openclaw_latest_npm().ok().flatten()
    }).await.unwrap_or(None);
    let upgrade = latest_version
        .as_ref()
        .is_some_and(|latest| compare_semver(&installed_version, Some(latest.as_str())));
    Ok(serde_json::json!({
        "upgradeAvailable": upgrade,
        "latestVersion": latest_version,
        "installedVersion": installed_version,
    }))
}

#[tauri::command]
pub async fn remote_list_agents_overview(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<Vec<AgentOverview>, String> {
    let output = crate::cli_runner::run_openclaw_remote(&pool, &host_id, &["agents", "list", "--json"]).await?;
    let json = crate::cli_runner::parse_json_output(&output)?;
    // Check which agents have sessions remotely (single command, batch check)
    // Lists agents whose sessions.json is larger than 2 bytes (not just "{}")
    let online_set = match pool.exec_login(
        &host_id,
        "for d in ~/.openclaw/agents/*/sessions/sessions.json; do [ -f \"$d\" ] && [ $(wc -c < \"$d\") -gt 2 ] && basename $(dirname $(dirname \"$d\")); done",
    ).await {
        Ok(result) => {
            result.stdout.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect::<std::collections::HashSet<String>>()
        }
        Err(_) => std::collections::HashSet::new(), // fallback: all offline
    };
    parse_agents_cli_output(&json, Some(&online_set))
}

#[tauri::command]
pub async fn remote_list_channels_minimal(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<Vec<ChannelNode>, String> {
    let output = crate::cli_runner::run_openclaw_remote(&pool, &host_id, &["config", "get", "channels", "--json"]).await?;
    // channels key might not exist yet
    if output.exit_code != 0 {
        let msg = format!("{} {}", output.stderr, output.stdout).to_lowercase();
        if msg.contains("not found") {
            return Ok(Vec::new());
        }
        return Err(format!("openclaw config get channels failed: {}", output.stderr));
    }
    let channels_val = crate::cli_runner::parse_json_output(&output).unwrap_or(Value::Null);
    // Wrap in top-level object with "channels" key so collect_channel_nodes works
    let cfg = serde_json::json!({ "channels": channels_val });
    Ok(collect_channel_nodes(&cfg))
}

#[tauri::command]
pub async fn remote_list_bindings(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<Vec<Value>, String> {
    let output = crate::cli_runner::run_openclaw_remote(&pool, &host_id, &["config", "get", "bindings", "--json"]).await?;
    // "bindings" may not exist yet — treat non-zero exit with "not found" as empty
    if output.exit_code != 0 {
        let msg = format!("{} {}", output.stderr, output.stdout).to_lowercase();
        if msg.contains("not found") {
            return Ok(Vec::new());
        }
    }
    let json = crate::cli_runner::parse_json_output(&output)?;
    Ok(json.as_array().cloned().unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Remote config mutation helpers & commands
// ---------------------------------------------------------------------------

/// Private helper: snapshot current config then write new config on remote.
async fn remote_write_config_with_snapshot(
    pool: &SshConnectionPool,
    host_id: &str,
    current_text: &str,
    next: &Value,
    source: &str,
) -> Result<(), String> {
    // Create snapshot dir
    pool.exec(host_id, "mkdir -p ~/.clawpal/snapshots").await?;
    // Write snapshot (use chrono-free timestamp from SystemTime)
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let snapshot_path = format!("~/.clawpal/snapshots/{ts}-{source}.json");
    pool.sftp_write(host_id, &snapshot_path, current_text).await?;
    // Write new config
    let new_text = serde_json::to_string_pretty(next).map_err(|e| e.to_string())?;
    pool.sftp_write(host_id, "~/.openclaw/openclaw.json", &new_text).await?;
    Ok(())
}

#[tauri::command]
pub async fn remote_restart_gateway(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<bool, String> {
    pool.exec_login(&host_id, "openclaw gateway restart").await?;
    Ok(true)
}


#[tauri::command]
pub async fn remote_apply_config_patch(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    patch_template: String,
    params: Map<String, Value>,
) -> Result<ApplyResult, String> {
    let raw = pool.sftp_read(&host_id, "~/.openclaw/openclaw.json").await?;
    let current: Value =
        serde_json::from_str(&raw).map_err(|e| format!("Failed to parse remote config: {e}"))?;
    let current_text = serde_json::to_string_pretty(&current).map_err(|e| e.to_string())?;
    let (candidate, _changes) =
        build_candidate_config_from_template(&current, &patch_template, &params)?;
    remote_write_config_with_snapshot(&pool, &host_id, &current_text, &candidate, "config-patch")
        .await?;
    Ok(ApplyResult {
        ok: true,
        snapshot_id: None,
        config_path: "~/.openclaw/openclaw.json".to_string(),
        backup_path: None,
        warnings: Vec::new(),
        errors: Vec::new(),
    })
}

#[tauri::command]
pub async fn remote_run_doctor(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Value, String> {
    let result = pool
        .exec_login(
            &host_id,
            "openclaw doctor --json 2>/dev/null || openclaw doctor 2>&1",
        )
        .await?;
    // Try to parse as JSON first
    if let Ok(json) = serde_json::from_str::<Value>(&result.stdout) {
        return Ok(json);
    }
    // Fallback: return raw output as a simple report
    Ok(serde_json::json!({
        "ok": result.exit_code == 0,
        "score": if result.exit_code == 0 { 100 } else { 0 },
        "issues": [],
        "rawOutput": result.stdout,
    }))
}

#[tauri::command]
pub async fn remote_list_history(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Value, String> {
    // Ensure dir exists
    pool.exec(&host_id, "mkdir -p ~/.clawpal/snapshots").await?;
    let entries = pool.sftp_list(&host_id, "~/.clawpal/snapshots").await?;
    let mut items: Vec<Value> = Vec::new();
    for entry in entries {
        if entry.name.starts_with('.') || entry.is_dir {
            continue;
        }
        // Parse filename: {unix_ts}-{source}-{summary}.json
        let stem = entry.name.trim_end_matches(".json");
        let parts: Vec<&str> = stem.splitn(3, '-').collect();
        let ts_str = parts.first().unwrap_or(&"0");
        let source = parts.get(1).unwrap_or(&"unknown");
        let recipe_id = parts.get(2).map(|s| s.to_string());
        let created_at = ts_str.parse::<i64>().unwrap_or(0);
        // Convert Unix timestamp to ISO 8601 format for frontend compatibility
        let created_at_iso = chrono::DateTime::from_timestamp(created_at, 0)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| created_at.to_string());
        let is_rollback = *source == "rollback";
        items.push(serde_json::json!({
            "id": entry.name,
            "recipeId": recipe_id,
            "createdAt": created_at_iso,
            "source": source,
            "canRollback": !is_rollback,
        }));
    }
    // Sort newest first
    items.sort_by(|a, b| {
        let ta = a["createdAt"].as_str().unwrap_or("");
        let tb = b["createdAt"].as_str().unwrap_or("");
        tb.cmp(ta)
    });
    Ok(serde_json::json!({ "items": items }))
}

#[tauri::command]
pub async fn remote_preview_rollback(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    snapshot_id: String,
) -> Result<PreviewResult, String> {
    let snapshot_path = format!("~/.clawpal/snapshots/{snapshot_id}");
    let snapshot_text = pool.sftp_read(&host_id, &snapshot_path).await?;
    let target: Value = serde_json::from_str(&snapshot_text)
        .map_err(|e| format!("Failed to parse snapshot: {e}"))?;

    let current_text = pool.sftp_read(&host_id, "~/.openclaw/openclaw.json").await?;
    let current: Value = serde_json::from_str(&current_text)
        .map_err(|e| format!("Failed to parse config: {e}"))?;

    let before = serde_json::to_string_pretty(&current).unwrap_or_else(|_| "{}".into());
    let after = serde_json::to_string_pretty(&target).unwrap_or_else(|_| "{}".into());
    Ok(PreviewResult {
        recipe_id: "rollback".into(),
        diff: format_diff(&current, &target),
        config_before: before,
        config_after: after,
        changes: collect_change_paths(&current, &target),
        overwrites_existing: true,
        can_rollback: true,
        impact_level: "medium".into(),
        warnings: vec!["Rollback will replace current configuration".into()],
    })
}

#[tauri::command]
pub async fn remote_rollback(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    snapshot_id: String,
) -> Result<ApplyResult, String> {
    let snapshot_path = format!("~/.clawpal/snapshots/{snapshot_id}");
    let target_text = pool.sftp_read(&host_id, &snapshot_path).await?;
    let target: Value = serde_json::from_str(&target_text)
        .map_err(|e| format!("Failed to parse snapshot: {e}"))?;

    let current_text = pool.sftp_read(&host_id, "~/.openclaw/openclaw.json").await?;
    remote_write_config_with_snapshot(&pool, &host_id, &current_text, &target, "rollback").await?;

    Ok(ApplyResult {
        ok: true,
        snapshot_id: Some(snapshot_id),
        config_path: "~/.openclaw/openclaw.json".into(),
        backup_path: None,
        warnings: vec!["rolled back".into()],
        errors: Vec::new(),
    })
}

#[tauri::command]
pub async fn remote_list_discord_guild_channels(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<DiscordGuildChannel>, String> {
    let output = crate::cli_runner::run_openclaw_remote(&pool, &host_id, &["config", "get", "channels.discord", "--json"]).await?;
    let discord_section = if output.exit_code == 0 {
        crate::cli_runner::parse_json_output(&output).unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    // Wrap to match existing code expectations (rest of function uses cfg.get("channels").and_then(|c| c.get("discord")))
    let cfg = serde_json::json!({ "channels": { "discord": discord_section } });

    let discord_cfg = cfg
        .get("channels")
        .and_then(|c| c.get("discord"));

    // Extract bot token: top-level first, then fall back to first account token
    let bot_token = discord_cfg
        .and_then(|d| d.get("botToken").or_else(|| d.get("token")))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .or_else(|| {
            discord_cfg
                .and_then(|d| d.get("accounts"))
                .and_then(Value::as_object)
                .and_then(|accounts| {
                    accounts.values().find_map(|acct| {
                        acct.get("token").and_then(Value::as_str).filter(|s| !s.is_empty()).map(|s| s.to_string())
                    })
                })
        });

    let mut entries: Vec<DiscordGuildChannel> = Vec::new();
    let mut channel_ids: Vec<String> = Vec::new();
    let mut unresolved_guild_ids: Vec<String> = Vec::new();

    // Helper: collect guilds from a guilds object
    let collect_guilds = |guilds: &serde_json::Map<String, Value>,
                               entries: &mut Vec<DiscordGuildChannel>,
                               channel_ids: &mut Vec<String>,
                               unresolved_guild_ids: &mut Vec<String>| {
        for (guild_id, guild_val) in guilds {
            let guild_name = guild_val
                .get("slug")
                .or_else(|| guild_val.get("name"))
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| guild_id.clone());

            if guild_name == *guild_id && !unresolved_guild_ids.contains(guild_id) {
                unresolved_guild_ids.push(guild_id.clone());
            }

            if let Some(channels) = guild_val.get("channels").and_then(Value::as_object) {
                for (channel_id, _) in channels {
                    if entries.iter().any(|e| e.guild_id == *guild_id && e.channel_id == *channel_id) {
                        continue;
                    }
                    channel_ids.push(channel_id.clone());
                    entries.push(DiscordGuildChannel {
                        guild_id: guild_id.clone(),
                        guild_name: guild_name.clone(),
                        channel_id: channel_id.clone(),
                        channel_name: channel_id.clone(),
                    });
                }
            }
        }
    };

    // Collect from channels.discord.guilds (top-level structured config)
    if let Some(guilds) = discord_cfg.and_then(|d| d.get("guilds")).and_then(Value::as_object) {
        collect_guilds(guilds, &mut entries, &mut channel_ids, &mut unresolved_guild_ids);
    }

    // Collect from channels.discord.accounts.<accountId>.guilds (multi-account config)
    if let Some(accounts) = discord_cfg.and_then(|d| d.get("accounts")).and_then(Value::as_object) {
        for (_account_id, account_val) in accounts {
            if let Some(guilds) = account_val.get("guilds").and_then(Value::as_object) {
                collect_guilds(guilds, &mut entries, &mut channel_ids, &mut unresolved_guild_ids);
            }
        }
    }

    // Also collect from bindings array (users may only have bindings, no guilds map)
    if let Some(bindings) = cfg.get("bindings").and_then(Value::as_array) {
        for b in bindings {
            let m = match b.get("match") {
                Some(m) => m,
                None => continue,
            };
            if m.get("channel").and_then(Value::as_str) != Some("discord") {
                continue;
            }
            let guild_id = match m.get("guildId") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                _ => continue,
            };
            let channel_id = match m.pointer("/peer/id") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                _ => continue,
            };
            if entries.iter().any(|e| e.guild_id == guild_id && e.channel_id == channel_id) {
                continue;
            }
            if !unresolved_guild_ids.contains(&guild_id) {
                unresolved_guild_ids.push(guild_id.clone());
            }
            channel_ids.push(channel_id.clone());
            entries.push(DiscordGuildChannel {
                guild_id: guild_id.clone(),
                guild_name: guild_id.clone(),
                channel_id: channel_id.clone(),
                channel_name: channel_id.clone(),
            });
        }
    }

    // Resolve channel names via openclaw CLI on remote
    if !channel_ids.is_empty() {
        let ids_arg = channel_ids.join(" ");
        let cmd = format!("openclaw channels resolve --json --channel discord --kind auto {}", ids_arg);
        if let Ok(r) = pool.exec_login(&host_id, &cmd).await {
            if r.exit_code == 0 && !r.stdout.trim().is_empty() {
                if let Some(name_map) = parse_resolve_name_map(&r.stdout) {
                    for entry in &mut entries {
                        if let Some(name) = name_map.get(&entry.channel_id) {
                            entry.channel_name = name.clone();
                        }
                    }
                }
            }
        }
    }

    // Resolve guild names via Discord REST API (guild names can't be resolved by openclaw CLI)
    // Must use spawn_blocking because reqwest::blocking panics in async context
    if let Some(token) = bot_token {
        if !unresolved_guild_ids.is_empty() {
            let guild_name_map = tokio::task::spawn_blocking(move || {
                let mut map = std::collections::HashMap::new();
                for gid in &unresolved_guild_ids {
                    if let Ok(name) = fetch_discord_guild_name(&token, gid) {
                        map.insert(gid.clone(), name);
                    }
                }
                map
            }).await.unwrap_or_default();
            for entry in &mut entries {
                if let Some(name) = guild_name_map.get(&entry.guild_id) {
                    entry.guild_name = name.clone();
                }
            }
        }
    }

    // Persist to remote cache
    if !entries.is_empty() {
        let json = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
        let _ = pool.sftp_write(&host_id, "~/.clawpal/discord-guild-channels.json", &json).await;
    }

    Ok(entries)
}

#[tauri::command]
pub async fn remote_write_raw_config(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    content: String,
) -> Result<bool, String> {
    // Validate it's valid JSON
    let next: Value =
        serde_json::from_str(&content).map_err(|e| format!("Invalid JSON: {e}"))?;
    // Read current for snapshot
    let current = pool
        .sftp_read(&host_id, "~/.openclaw/openclaw.json")
        .await
        .unwrap_or_default();
    remote_write_config_with_snapshot(&pool, &host_id, &current, &next, "raw-edit").await?;
    Ok(true)
}

#[tauri::command]
pub async fn remote_analyze_sessions(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<AgentSessionAnalysis>, String> {
    // Run a shell script via SSH that scans session files and outputs JSON.
    // This is MUCH faster than doing per-file SFTP reads.
    let script = r#"
setopt nonomatch 2>/dev/null; shopt -s nullglob 2>/dev/null
cd ~/.openclaw/agents 2>/dev/null || { echo '[]'; exit 0; }
now=$(date +%s)
sep=""
echo "["
for agent_dir in */; do
  [ -d "$agent_dir" ] || continue
  agent="${agent_dir%/}"
  # Sanitize agent name for JSON (escape backslash then double-quote)
  safe_agent=$(printf '%s' "$agent" | sed 's/\\/\\\\/g; s/"/\\"/g')
  for kind in sessions sessions_archive; do
    dir="$agent_dir$kind"
    [ -d "$dir" ] || continue
    for f in "$dir"/*.jsonl; do
      [ -f "$f" ] || continue
      fname=$(basename "$f" .jsonl)
      safe_fname=$(printf '%s' "$fname" | sed 's/\\/\\\\/g; s/"/\\"/g')
      size=$(wc -c < "$f" 2>/dev/null | tr -d ' ')
      msgs=$(grep -c '"type":"message"' "$f" 2>/dev/null || true)
      [ -z "$msgs" ] && msgs=0
      user_msgs=$(grep -c '"role":"user"' "$f" 2>/dev/null || true)
      [ -z "$user_msgs" ] && user_msgs=0
      asst_msgs=$(grep -c '"role":"assistant"' "$f" 2>/dev/null || true)
      [ -z "$asst_msgs" ] && asst_msgs=0
      mtime=$(stat -c %Y "$f" 2>/dev/null || stat -f %m "$f" 2>/dev/null || echo 0)
      age_days=$(( (now - mtime) / 86400 ))
      printf '%s{"agent":"%s","sessionId":"%s","sizeBytes":%s,"messageCount":%s,"userMessageCount":%s,"assistantMessageCount":%s,"ageDays":%s,"kind":"%s"}' \
        "$sep" "$safe_agent" "$safe_fname" "$size" "$msgs" "$user_msgs" "$asst_msgs" "$age_days" "$kind"
      sep=","
    done
  done
done
echo "]"
"#;

    let result = pool.exec(&host_id, script).await?;
    if result.exit_code != 0 && result.stdout.trim().is_empty() {
        // No agents directory — return empty
        return Ok(Vec::new());
    }

    // Parse the JSON output
    let raw_sessions: Vec<Value> = serde_json::from_str(result.stdout.trim())
        .map_err(|e| format!("Failed to parse remote session data: {e}\nOutput: {}", &result.stdout[..result.stdout.len().min(500)]))?;

    // Group by agent and classify
    let mut agent_map: std::collections::BTreeMap<String, Vec<SessionAnalysis>> = std::collections::BTreeMap::new();

    for val in &raw_sessions {
        let agent = val.get("agent").and_then(Value::as_str).unwrap_or("unknown").to_string();
        let session_id = val.get("sessionId").and_then(Value::as_str).unwrap_or("").to_string();
        let size_bytes = val.get("sizeBytes").and_then(Value::as_u64).unwrap_or(0);
        let message_count = val.get("messageCount").and_then(Value::as_u64).unwrap_or(0) as usize;
        let user_message_count = val.get("userMessageCount").and_then(Value::as_u64).unwrap_or(0) as usize;
        let assistant_message_count = val.get("assistantMessageCount").and_then(Value::as_u64).unwrap_or(0) as usize;
        let age_days = val.get("ageDays").and_then(Value::as_f64).unwrap_or(0.0);
        let kind = val.get("kind").and_then(Value::as_str).unwrap_or("sessions").to_string();

        let category = if size_bytes < 500 || message_count == 0 {
            "empty"
        } else if user_message_count <= 1 && age_days > 7.0 {
            "low_value"
        } else {
            "valuable"
        };

        agent_map.entry(agent.clone()).or_default().push(SessionAnalysis {
            agent: agent.clone(),
            session_id,
            file_path: String::new(),
            size_bytes,
            message_count,
            user_message_count,
            assistant_message_count,
            last_activity: None,
            age_days,
            total_tokens: 0,
            model: None,
            category: category.to_string(),
            kind,
        });
    }

    let mut results: Vec<AgentSessionAnalysis> = Vec::new();
    for (agent, mut sessions) in agent_map {
        sessions.sort_by(|a, b| {
            let cat_order = |c: &str| match c { "empty" => 0, "low_value" => 1, _ => 2 };
            cat_order(&a.category).cmp(&cat_order(&b.category))
                .then(b.age_days.partial_cmp(&a.age_days).unwrap_or(std::cmp::Ordering::Equal))
        });
        let total_files = sessions.len();
        let total_size_bytes = sessions.iter().map(|s| s.size_bytes).sum();
        let empty_count = sessions.iter().filter(|s| s.category == "empty").count();
        let low_value_count = sessions.iter().filter(|s| s.category == "low_value").count();
        let valuable_count = sessions.iter().filter(|s| s.category == "valuable").count();

        results.push(AgentSessionAnalysis {
            agent,
            total_files,
            total_size_bytes,
            empty_count,
            low_value_count,
            valuable_count,
            sessions,
        });
    }
    Ok(results)
}

#[tauri::command]
pub async fn remote_delete_sessions_by_ids(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    agent_id: String,
    session_ids: Vec<String>,
) -> Result<usize, String> {
    if agent_id.trim().is_empty() || agent_id.contains("..") || agent_id.contains('/') {
        return Err("invalid agent id".into());
    }

    let mut deleted = 0usize;
    for sid in &session_ids {
        if sid.contains("..") || sid.contains('/') || sid.contains('\\') {
            continue;
        }
        // Delete from both sessions and sessions_archive
        let cmd = format!(
            "rm -f ~/.openclaw/agents/{agent}/sessions/{sid}.jsonl ~/.openclaw/agents/{agent}/sessions/{sid}-topic-*.jsonl ~/.openclaw/agents/{agent}/sessions_archive/{sid}.jsonl ~/.openclaw/agents/{agent}/sessions_archive/{sid}-topic-*.jsonl 2>/dev/null; echo ok",
            agent = agent_id, sid = sid
        );
        if let Ok(r) = pool.exec(&host_id, &cmd).await {
            if r.stdout.trim() == "ok" {
                deleted += 1;
            }
        }
    }

    // Clean up sessions.json
    let sessions_json_path = format!("~/.openclaw/agents/{}/sessions/sessions.json", agent_id);
    if let Ok(content) = pool.sftp_read(&host_id, &sessions_json_path).await {
        if let Ok(mut data) = serde_json::from_str::<serde_json::Map<String, Value>>(&content) {
            let id_set: HashSet<&str> = session_ids.iter().map(String::as_str).collect();
            data.retain(|_key, val| {
                let sid = val.get("sessionId").and_then(Value::as_str).unwrap_or("");
                !id_set.contains(sid)
            });
            let updated = serde_json::to_string(&data).unwrap_or_default();
            let _ = pool.sftp_write(&host_id, &sessions_json_path, &updated).await;
        }
    }

    Ok(deleted)
}

#[tauri::command]
pub async fn remote_list_session_files(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<SessionFile>, String> {
    let script = r#"
setopt nonomatch 2>/dev/null; shopt -s nullglob 2>/dev/null
cd ~/.openclaw/agents 2>/dev/null || { echo "[]"; exit 0; }
sep=""
echo "["
for agent_dir in */; do
  [ -d "$agent_dir" ] || continue
  agent="${agent_dir%/}"
  safe_agent=$(printf '%s' "$agent" | sed 's/\\/\\\\/g; s/"/\\"/g')
  for kind in sessions sessions_archive; do
    dir="$agent_dir$kind"
    [ -d "$dir" ] || continue
    for f in "$dir"/*.jsonl; do
      [ -f "$f" ] || continue
      size=$(wc -c < "$f" 2>/dev/null | tr -d ' ')
      safe_path=$(printf '%s' "$f" | sed 's/\\/\\\\/g; s/"/\\"/g')
      printf '%s{"agent":"%s","kind":"%s","path":"%s","sizeBytes":%s}' "$sep" "$safe_agent" "$kind" "$safe_path" "$size"
      sep=","
    done
  done
done
echo "]"
"#;
    let result = pool.exec(&host_id, script).await?;
    let raw: Vec<Value> = serde_json::from_str(result.stdout.trim())
        .unwrap_or_default();

    let mut out = Vec::new();
    for val in &raw {
        let agent = val.get("agent").and_then(Value::as_str).unwrap_or("").to_string();
        let kind = val.get("kind").and_then(Value::as_str).unwrap_or("sessions").to_string();
        let path = val.get("path").and_then(Value::as_str).unwrap_or("").to_string();
        let size_bytes = val.get("sizeBytes").and_then(Value::as_u64).unwrap_or(0);
        out.push(SessionFile {
            relative_path: path.clone(),
            path,
            agent,
            kind,
            size_bytes,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn remote_clear_all_sessions(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<usize, String> {
    let script = r#"
setopt nonomatch 2>/dev/null; shopt -s nullglob 2>/dev/null
count=0
cd ~/.openclaw/agents 2>/dev/null || { echo "0"; exit 0; }
for agent_dir in */; do
  for kind in sessions sessions_archive; do
    dir="$agent_dir$kind"
    [ -d "$dir" ] || continue
    for f in "$dir"/*; do
      [ -f "$f" ] || continue
      rm -f "$f" && count=$((count + 1))
    done
  done
done
echo "$count"
"#;
    let result = pool.exec(&host_id, script).await?;
    let count: usize = result.stdout.trim().parse().unwrap_or(0);
    Ok(count)
}

#[tauri::command]
pub async fn remote_preview_session(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    agent_id: String,
    session_id: String,
) -> Result<Vec<Value>, String> {
    if agent_id.contains("..") || agent_id.contains('/') || session_id.contains("..") || session_id.contains('/') {
        return Err("invalid id".into());
    }
    let jsonl_name = format!("{}.jsonl", session_id);

    // Try sessions dir first, then archive
    let paths = [
        format!("~/.openclaw/agents/{}/sessions/{}", agent_id, jsonl_name),
        format!("~/.openclaw/agents/{}/sessions_archive/{}", agent_id, jsonl_name),
    ];

    let mut content = String::new();
    for path in &paths {
        if let Ok(c) = pool.sftp_read(&host_id, path).await {
            content = c;
            break;
        }
    }
    if content.is_empty() {
        return Ok(Vec::new());
    }

    let mut messages: Vec<Value> = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let obj: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if obj.get("type").and_then(Value::as_str) == Some("message") {
            let role = obj.pointer("/message/role").and_then(Value::as_str).unwrap_or("unknown");
            let content_val = obj.pointer("/message/content")
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
                "content": content_val,
            }));
        }
    }
    Ok(messages)
}

#[tauri::command]
pub async fn remote_list_model_profiles(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<ModelProfile>, String> {
    let content = pool.sftp_read(&host_id, "~/.clawpal/model-profiles.json").await
        .unwrap_or_else(|_| r#"{"profiles":[]}"#.to_string());
    #[derive(serde::Deserialize)]
    struct Storage {
        #[serde(default)]
        profiles: Vec<ModelProfile>,
    }
    let parsed: Storage = serde_json::from_str(&content).unwrap_or(Storage { profiles: Vec::new() });
    Ok(parsed.profiles)
}

#[tauri::command]
pub async fn remote_upsert_model_profile(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    mut profile: ModelProfile,
) -> Result<ModelProfile, String> {
    if profile.provider.trim().is_empty() || profile.model.trim().is_empty() {
        return Err("provider and model are required".into());
    }
    if profile.name.trim().is_empty() {
        profile.name = format!("{}/{}", profile.provider, profile.model);
    }

    // Load existing profiles
    let content = pool.sftp_read(&host_id, "~/.clawpal/model-profiles.json").await
        .unwrap_or_else(|_| r#"{"profiles":[]}"#.to_string());
    #[derive(serde::Deserialize, serde::Serialize)]
    struct Storage {
        #[serde(default)]
        profiles: Vec<ModelProfile>,
        #[serde(default = "default_version")]
        version: u8,
    }
    fn default_version() -> u8 { 1 }
    let mut storage: Storage = serde_json::from_str(&content).unwrap_or(Storage { profiles: Vec::new(), version: 1 });

    if profile.id.trim().is_empty() {
        profile.id = uuid::Uuid::new_v4().to_string();
    }
    let id = profile.id.clone();
    if let Some(existing) = storage.profiles.iter_mut().find(|p| p.id == id) {
        // Preserve existing API key if new one is empty
        if profile.api_key.as_ref().map_or(true, |k| k.trim().is_empty()) {
            profile.api_key = existing.api_key.clone();
        }
        *existing = profile.clone();
    } else {
        // New profile: if no API key provided, try to reuse from same-provider profile
        if profile.api_key.as_ref().map_or(true, |k| k.trim().is_empty()) {
            if let Some(donor) = storage.profiles.iter().find(|p| {
                p.provider == profile.provider
                    && p.api_key.as_ref().is_some_and(|k| !k.trim().is_empty())
            }) {
                profile.api_key = donor.api_key.clone();
            }
        }
        storage.profiles.push(profile.clone());
    }

    // Ensure .clawpal dir exists
    let _ = pool.exec(&host_id, "mkdir -p ~/.clawpal").await;
    let text = serde_json::to_string_pretty(&storage).map_err(|e| e.to_string())?;
    pool.sftp_write(&host_id, "~/.clawpal/model-profiles.json", &text).await?;
    Ok(profile)
}

#[tauri::command]
pub async fn remote_delete_model_profile(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    profile_id: String,
) -> Result<bool, String> {
    let content = pool.sftp_read(&host_id, "~/.clawpal/model-profiles.json").await
        .unwrap_or_else(|_| r#"{"profiles":[]}"#.to_string());
    #[derive(serde::Deserialize, serde::Serialize)]
    struct Storage {
        #[serde(default)]
        profiles: Vec<ModelProfile>,
        #[serde(default = "default_version")]
        version: u8,
    }
    fn default_version() -> u8 { 1 }
    let mut storage: Storage = serde_json::from_str(&content).unwrap_or(Storage { profiles: Vec::new(), version: 1 });
    let before = storage.profiles.len();
    storage.profiles.retain(|p| p.id != profile_id);
    if storage.profiles.len() == before {
        return Ok(false);
    }
    let text = serde_json::to_string_pretty(&storage).map_err(|e| e.to_string())?;
    pool.sftp_write(&host_id, "~/.clawpal/model-profiles.json", &text).await?;
    Ok(true)
}

#[tauri::command]
pub async fn remote_resolve_api_keys(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<ResolvedApiKey>, String> {
    let content = pool.sftp_read(&host_id, "~/.clawpal/model-profiles.json").await
        .unwrap_or_else(|_| r#"{"profiles":[]}"#.to_string());
    #[derive(serde::Deserialize)]
    struct Storage {
        #[serde(default)]
        profiles: Vec<ModelProfile>,
    }
    let storage: Storage = serde_json::from_str(&content).unwrap_or(Storage { profiles: Vec::new() });
    let mut out = Vec::new();
    for profile in &storage.profiles {
        let masked = if let Some(ref key) = profile.api_key {
            if key.len() > 8 {
                format!("{}...{}", &key[..4], &key[key.len()-4..])
            } else if !key.is_empty() {
                "****".to_string()
            } else if !profile.auth_ref.is_empty() {
                format!("via {}", profile.auth_ref)
            } else {
                "not set".to_string()
            }
        } else if !profile.auth_ref.is_empty() {
            format!("via {}", profile.auth_ref)
        } else {
            "not set".to_string()
        };
        out.push(ResolvedApiKey {
            profile_id: profile.id.clone(),
            masked_key: masked,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn remote_extract_model_profiles_from_config(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<ExtractModelProfilesResult, String> {
    let raw = pool.sftp_read(&host_id, "~/.openclaw/openclaw.json").await?;
    let cfg: Value = serde_json::from_str(&raw).map_err(|e| format!("Failed to parse remote config: {e}"))?;

    let profiles_raw = pool.sftp_read(&host_id, "~/.clawpal/model-profiles.json").await
        .unwrap_or_else(|_| r#"{"profiles":[]}"#.to_string());
    #[derive(serde::Deserialize)]
    struct StorageIn {
        #[serde(default)]
        profiles: Vec<ModelProfile>,
    }
    let existing: StorageIn = serde_json::from_str(&profiles_raw).unwrap_or(StorageIn { profiles: Vec::new() });
    let profiles = existing.profiles;

    let bindings = collect_model_bindings(&cfg, &profiles);
    let mut created = 0usize;
    let mut reused = 0usize;
    let mut skipped_invalid = 0usize;
    let mut seen = HashSet::new();

    let mut next_profiles = profiles;
    let mut model_profile_map: HashMap<String, String> = HashMap::new();
    for profile in &next_profiles {
        model_profile_map.insert(normalize_model_ref(&profile_to_model_value(profile)), profile.id.clone());
    }

    for binding in bindings {
        let scope_label = match binding.scope.as_str() {
            "global" => "global".to_string(),
            "agent" => format!("agent:{}", binding.scope_id),
            "channel" => format!("channel:{}", binding.scope_id),
            _ => binding.scope_id,
        };
        let Some(model_ref) = binding.model_value else { continue };
        let model_ref = normalize_model_ref(&model_ref);
        if model_ref.trim().is_empty() { continue }
        if model_profile_map.contains_key(&model_ref) || seen.contains(&model_ref) {
            reused += 1;
            continue;
        }
        let mut parts = model_ref.splitn(2, '/');
        let provider = parts.next().unwrap_or("").trim();
        let model = parts.next().unwrap_or("").trim();
        if provider.is_empty() || model.is_empty() {
            skipped_invalid += 1;
            continue;
        }
        let auth_ref = resolve_auth_ref_for_provider(&cfg, provider)
            .unwrap_or_else(|| format!("{provider}:default"));
        let base_url = resolve_model_provider_base_url(&cfg, provider);
        let new_profile = ModelProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name: format!("{scope_label} model profile"),
            provider: provider.to_string(),
            model: model.to_string(),
            auth_ref,
            api_key: None,
            base_url,
            description: Some(format!("Extracted from config ({scope_label})")),
            enabled: true,
        };
        let key = profile_to_model_value(&new_profile);
        model_profile_map.insert(normalize_model_ref(&key), new_profile.id.clone());
        next_profiles.push(new_profile);
        seen.insert(model_ref);
        created += 1;
    }

    if created > 0 {
        #[derive(serde::Serialize)]
        struct StorageOut<'a> {
            profiles: &'a [ModelProfile],
            version: u8,
        }
        let payload = StorageOut { profiles: &next_profiles, version: 1 };
        let text = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
        let _ = pool.exec(&host_id, "mkdir -p ~/.clawpal").await;
        pool.sftp_write(&host_id, "~/.clawpal/model-profiles.json", &text).await?;
    }

    Ok(ExtractModelProfilesResult { created, reused, skipped_invalid })
}

#[tauri::command]
pub async fn remote_refresh_model_catalog(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<ModelCatalogProvider>, String> {
    let result = pool.exec_login(&host_id, "openclaw models list --all --json --no-color").await;
    if let Ok(r) = result {
        if r.exit_code == 0 && !r.stdout.trim().is_empty() {
            if let Some(catalog) = parse_model_catalog_from_cli_output(&r.stdout) {
                return Ok(catalog);
            }
        }
    }

    // Fallback: extract from remote config
    let raw = pool.sftp_read(&host_id, "~/.openclaw/openclaw.json").await?;
    let cfg: Value = serde_json::from_str(&raw).map_err(|e| format!("Failed to parse remote config: {e}"))?;
    Ok(collect_model_catalog(&cfg))
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

#[tauri::command]
pub async fn remote_run_openclaw_upgrade(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<String, String> {
    // Use the official install script with --no-prompt for non-interactive SSH.
    // The script handles npm prefix/permissions, bin links, and PATH fixups
    // that plain `npm install -g` misses (e.g. stale /usr/bin/openclaw symlinks).
    let version_before = pool.exec_login(&host_id, "openclaw --version 2>/dev/null || true").await
        .map(|r| r.stdout.trim().to_string()).unwrap_or_default();

    let install_cmd = "curl -fsSL --proto '=https' --tlsv1.2 https://openclaw.ai/install.sh | bash -s -- --no-prompt --no-onboard 2>&1";
    let result = pool.exec_login(&host_id, install_cmd).await?;
    let combined = if result.stderr.is_empty() {
        result.stdout.clone()
    } else {
        format!("{}\n{}", result.stdout, result.stderr)
    };

    if result.exit_code != 0 {
        return Err(combined);
    }

    // Restart gateway after successful upgrade (best-effort)
    let _ = pool.exec_login(&host_id, "openclaw gateway restart 2>/dev/null || true").await;

    // Verify version actually changed
    let version_after = pool.exec_login(&host_id, "openclaw --version 2>/dev/null || true").await
        .map(|r| r.stdout.trim().to_string()).unwrap_or_default();
    if !version_before.is_empty() && !version_after.is_empty() && version_before == version_after {
        return Err(format!("{combined}\n\nWarning: version unchanged after upgrade ({version_before}). Check PATH or npm prefix."));
    }

    Ok(combined)
}

// ---------------------------------------------------------------------------
// Cron jobs
// ---------------------------------------------------------------------------

/// Strip Doctor warning banners from CLI output to show only meaningful errors.
/// Doctor banners look like: ╭─ ... ─╮ ... ╰─ ... ─╯
fn strip_doctor_banner(text: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let mut in_banner = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.contains("Doctor warnings") && trimmed.contains('╮') {
            in_banner = true;
            continue;
        }
        if in_banner {
            if trimmed.contains('╯') {
                in_banner = false;
            }
            continue;
        }
        if !trimmed.is_empty() {
            lines.push(line);
        }
    }
    let result = lines.join("\n").trim().to_string();
    if result.is_empty() { "Command failed".into() } else { result }
}

fn parse_cron_jobs(text: &str) -> Value {
    let parsed: Value = serde_json::from_str(text).unwrap_or(Value::Array(vec![]));
    // Handle { "version": N, "jobs": [...] } wrapper
    let jobs = if let Some(arr) = parsed.pointer("/jobs") {
        arr.clone()
    } else {
        parsed
    };
    match jobs {
        Value::Array(arr) => {
            let mapped: Vec<Value> = arr.into_iter().map(|mut v| {
                // Map "id" → "jobId" for frontend compatibility
                if let Value::Object(ref mut obj) = v {
                    if let Some(id) = obj.get("id").cloned() {
                        obj.entry("jobId".to_string()).or_insert(id);
                    }
                }
                v
            }).collect();
            Value::Array(mapped)
        }
        Value::Object(map) => {
            let arr: Vec<Value> = map.into_iter().map(|(k, mut v)| {
                if let Value::Object(ref mut obj) = v {
                    obj.entry("jobId".to_string()).or_insert(Value::String(k.clone()));
                    obj.entry("id".to_string()).or_insert(Value::String(k));
                }
                v
            }).collect();
            Value::Array(arr)
        }
        _ => Value::Array(vec![]),
    }
}

#[tauri::command]
pub fn list_cron_jobs() -> Result<Value, String> {
    let paths = resolve_paths();
    let jobs_path = paths.base_dir.join("cron").join("jobs.json");
    if !jobs_path.exists() {
        return Ok(Value::Array(vec![]));
    }
    let text = std::fs::read_to_string(&jobs_path).map_err(|e| e.to_string())?;
    Ok(parse_cron_jobs(&text))
}

#[tauri::command]
pub fn get_cron_runs(job_id: String, limit: Option<usize>) -> Result<Vec<Value>, String> {
    let paths = resolve_paths();
    let runs_path = paths.base_dir.join("cron").join("runs").join(format!("{}.jsonl", job_id));
    if !runs_path.exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(&runs_path).map_err(|e| e.to_string())?;
    let mut runs: Vec<Value> = text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    runs.reverse();
    let limit = limit.unwrap_or(10);
    runs.truncate(limit);
    Ok(runs)
}

#[tauri::command]
pub async fn trigger_cron_job(job_id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let output = std::process::Command::new(resolve_openclaw_bin())
            .args(["cron", "run", &job_id])
            .output()
            .map_err(|e| format!("Failed to run openclaw: {e}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if output.status.success() {
            Ok(stdout)
        } else {
            // Extract meaningful error lines, skip Doctor warning banners
            let error_msg = strip_doctor_banner(&format!("{stdout}\n{stderr}"));
            Err(error_msg)
        }
    }).await.map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub fn delete_cron_job(job_id: String) -> Result<String, String> {
    let output = std::process::Command::new(resolve_openclaw_bin())
        .args(["cron", "remove", &job_id])
        .output()
        .map_err(|e| format!("Failed to run openclaw: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        Err(format!("{stdout}\n{stderr}"))
    }
}

// ---------------------------------------------------------------------------
// Remote cron jobs
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn remote_list_cron_jobs(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<Value, String> {
    let raw = pool.sftp_read(&host_id, "~/.openclaw/cron/jobs.json").await;
    match raw {
        Ok(text) => Ok(parse_cron_jobs(&text)),
        Err(_) => Ok(Value::Array(vec![])),
    }
}

#[tauri::command]
pub async fn remote_get_cron_runs(pool: State<'_, SshConnectionPool>, host_id: String, job_id: String, limit: Option<usize>) -> Result<Vec<Value>, String> {
    let path = format!("~/.openclaw/cron/runs/{}.jsonl", job_id);
    let raw = pool.sftp_read(&host_id, &path).await;
    match raw {
        Ok(text) => {
            let mut runs: Vec<Value> = text.lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            runs.reverse();
            let limit = limit.unwrap_or(10);
            runs.truncate(limit);
            Ok(runs)
        }
        Err(_) => Ok(vec![]),
    }
}

#[tauri::command]
pub async fn remote_trigger_cron_job(pool: State<'_, SshConnectionPool>, host_id: String, job_id: String) -> Result<String, String> {
    let result = pool.exec_login(&host_id, &format!("openclaw cron run {}", shell_escape(&job_id))).await?;
    if result.exit_code == 0 {
        Ok(result.stdout)
    } else {
        Err(format!("{}\n{}", result.stdout, result.stderr))
    }
}

#[tauri::command]
pub async fn remote_delete_cron_job(pool: State<'_, SshConnectionPool>, host_id: String, job_id: String) -> Result<String, String> {
    let result = pool.exec_login(&host_id, &format!("openclaw cron remove {}", shell_escape(&job_id))).await?;
    if result.exit_code == 0 {
        Ok(result.stdout)
    } else {
        Err(format!("{}\n{}", result.stdout, result.stderr))
    }
}

// ---------------------------------------------------------------------------
// Watchdog management
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_watchdog_status() -> Result<Value, String> {
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
        map.insert("deployed".into(), Value::Bool(wd_dir.join("watchdog.js").exists()));
    } else {
        let mut map = serde_json::Map::new();
        map.insert("alive".into(), Value::Bool(alive));
        map.insert("deployed".into(), Value::Bool(wd_dir.join("watchdog.js").exists()));
        status = Value::Object(map);
    }

    Ok(status)
}

#[tauri::command]
pub fn deploy_watchdog(app_handle: tauri::AppHandle) -> Result<bool, String> {
    let paths = resolve_paths();
    let wd_dir = paths.clawpal_dir.join("watchdog");
    std::fs::create_dir_all(&wd_dir).map_err(|e| e.to_string())?;

    let resource_path = app_handle.path()
        .resolve("resources/watchdog.js", tauri::path::BaseDirectory::Resource)
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
        .create(true).append(true)
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
            let _ = std::process::Command::new("kill").arg(pid.to_string()).output();
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

#[tauri::command]
pub fn read_app_log(lines: Option<usize>) -> Result<String, String> {
    crate::logging::read_log_tail("app.log", lines.unwrap_or(200))
}

#[tauri::command]
pub fn read_error_log(lines: Option<usize>) -> Result<String, String> {
    crate::logging::read_log_tail("error.log", lines.unwrap_or(200))
}

#[tauri::command]
pub fn read_gateway_log(lines: Option<usize>) -> Result<String, String> {
    let paths = crate::models::resolve_paths();
    let path = paths.openclaw_dir.join("logs/gateway.log");
    if !path.exists() {
        return Ok(String::new());
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let all_lines: Vec<&str> = content.lines().collect();
    let n = lines.unwrap_or(200);
    let start = all_lines.len().saturating_sub(n);
    Ok(all_lines[start..].join("\n"))
}

#[tauri::command]
pub fn read_gateway_error_log(lines: Option<usize>) -> Result<String, String> {
    let paths = crate::models::resolve_paths();
    let path = paths.openclaw_dir.join("logs/gateway.err.log");
    if !path.exists() {
        return Ok(String::new());
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let all_lines: Vec<&str> = content.lines().collect();
    let n = lines.unwrap_or(200);
    let start = all_lines.len().saturating_sub(n);
    Ok(all_lines[start..].join("\n"))
}

#[tauri::command]
pub async fn remote_read_app_log(pool: State<'_, SshConnectionPool>, host_id: String, lines: Option<usize>) -> Result<String, String> {
    let n = lines.unwrap_or(200);
    let cmd = format!("tail -n {n} ~/.clawpal/logs/app.log 2>/dev/null || echo ''");
    let result = pool.exec(&host_id, &cmd).await?;
    Ok(result.stdout)
}

#[tauri::command]
pub async fn remote_read_error_log(pool: State<'_, SshConnectionPool>, host_id: String, lines: Option<usize>) -> Result<String, String> {
    let n = lines.unwrap_or(200);
    let cmd = format!("tail -n {n} ~/.clawpal/logs/error.log 2>/dev/null || echo ''");
    let result = pool.exec(&host_id, &cmd).await?;
    Ok(result.stdout)
}

#[tauri::command]
pub async fn remote_read_gateway_log(pool: State<'_, SshConnectionPool>, host_id: String, lines: Option<usize>) -> Result<String, String> {
    let n = lines.unwrap_or(200);
    let cmd = format!("tail -n {n} ~/.openclaw/logs/gateway.log 2>/dev/null || echo ''");
    let result = pool.exec(&host_id, &cmd).await?;
    Ok(result.stdout)
}

#[tauri::command]
pub async fn remote_read_gateway_error_log(pool: State<'_, SshConnectionPool>, host_id: String, lines: Option<usize>) -> Result<String, String> {
    let n = lines.unwrap_or(200);
    let cmd = format!("tail -n {n} ~/.openclaw/logs/gateway.err.log 2>/dev/null || echo ''");
    let result = pool.exec(&host_id, &cmd).await?;
    Ok(result.stdout)
}

// ---------------------------------------------------------------------------
// Remote watchdog management
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn remote_get_watchdog_status(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<Value, String> {
    let status_raw = pool.sftp_read(&host_id, "~/.clawpal/watchdog/status.json").await;
    let mut status = match status_raw {
        Ok(text) => serde_json::from_str::<Value>(&text).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    };

    let pid_raw = pool.sftp_read(&host_id, "~/.clawpal/watchdog/watchdog.pid").await;
    let alive = match pid_raw {
        Ok(pid_str) => {
            let cmd = format!("kill -0 {} 2>/dev/null && echo alive || echo dead", pid_str.trim());
            pool.exec(&host_id, &cmd).await
                .map(|r| r.stdout.trim() == "alive")
                .unwrap_or(false)
        }
        Err(_) => false,
    };

    let deployed = pool.sftp_read(&host_id, "~/.clawpal/watchdog/watchdog.js").await.is_ok();

    if let Value::Object(ref mut map) = status {
        map.insert("alive".into(), Value::Bool(alive));
        map.insert("deployed".into(), Value::Bool(deployed));
    } else {
        let mut map = serde_json::Map::new();
        map.insert("alive".into(), Value::Bool(alive));
        map.insert("deployed".into(), Value::Bool(deployed));
        status = Value::Object(map);
    }

    Ok(status)
}

#[tauri::command]
pub async fn remote_deploy_watchdog(app_handle: tauri::AppHandle, pool: State<'_, SshConnectionPool>, host_id: String) -> Result<bool, String> {
    let resource_path = app_handle.path()
        .resolve("resources/watchdog.js", tauri::path::BaseDirectory::Resource)
        .map_err(|e| format!("Failed to resolve watchdog resource: {e}"))?;
    let content = std::fs::read_to_string(&resource_path)
        .map_err(|e| format!("Failed to read watchdog resource: {e}"))?;

    pool.exec(&host_id, "mkdir -p ~/.clawpal/watchdog").await?;
    pool.sftp_write(&host_id, "~/.clawpal/watchdog/watchdog.js", &content).await?;
    Ok(true)
}

#[tauri::command]
pub async fn remote_start_watchdog(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<bool, String> {
    let pid_raw = pool.sftp_read(&host_id, "~/.clawpal/watchdog/watchdog.pid").await;
    if let Ok(pid_str) = pid_raw {
        let cmd = format!("kill -0 {} 2>/dev/null && echo alive || echo dead", pid_str.trim());
        if let Ok(r) = pool.exec(&host_id, &cmd).await {
            if r.stdout.trim() == "alive" {
                return Ok(true);
            }
        }
    }

    let cmd = "cd ~/.clawpal/watchdog && nohup node watchdog.js >> watchdog.log 2>&1 &";
    pool.exec(&host_id, cmd).await?;
    // watchdog.js writes its own PID file to ~/.clawpal/watchdog/
    Ok(true)
}

#[tauri::command]
pub async fn remote_stop_watchdog(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<bool, String> {
    let pid_raw = pool.sftp_read(&host_id, "~/.clawpal/watchdog/watchdog.pid").await;
    if let Ok(pid_str) = pid_raw {
        let _ = pool.exec(&host_id, &format!("kill {} 2>/dev/null", pid_str.trim())).await;
    }
    let _ = pool.exec(&host_id, "rm -f ~/.clawpal/watchdog/watchdog.pid").await;
    Ok(true)
}

#[tauri::command]
pub async fn remote_uninstall_watchdog(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<bool, String> {
    // Stop first
    let pid_raw = pool.sftp_read(&host_id, "~/.clawpal/watchdog/watchdog.pid").await;
    if let Ok(pid_str) = pid_raw {
        let _ = pool.exec(&host_id, &format!("kill {} 2>/dev/null", pid_str.trim())).await;
    }
    // Remove entire directory
    let _ = pool.exec(&host_id, "rm -rf ~/.clawpal/watchdog").await;
    Ok(true)
}
