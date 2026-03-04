use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::{
    collections::hash_map::DefaultHasher,
    collections::HashSet,
    hash::{Hash, Hasher},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::models::{resolve_paths, OpenClawPaths};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::sanitize::sanitize_output;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ZeroclawUsageStats {
    pub total_calls: u64,
    pub usage_calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub last_updated_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ZeroclawRuntimeTarget {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub source: String,
    pub preferred_model: Option<String>,
    pub provider_order: Vec<String>,
}

fn usage_store() -> &'static Mutex<ZeroclawUsageStats> {
    static STORE: OnceLock<Mutex<ZeroclawUsageStats>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(ZeroclawUsageStats::default()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

const ZEROCLAW_MESSAGE_MAX_BYTES: usize = 24 * 1024;
const ZEROCLAW_EXEC_TIMEOUT_SECS: u64 = 90;
const ZEROCLAW_LOCAL_PROVIDER_TIMEOUT_SECS: u64 = 45;

fn truncate_utf8_tail(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }
    let mut start = input.len().saturating_sub(max_bytes);
    while start < input.len() && !input.is_char_boundary(start) {
        start += 1;
    }
    input[start..].to_string()
}

fn clamp_message_for_cli(message: &str) -> String {
    if message.len() <= ZEROCLAW_MESSAGE_MAX_BYTES {
        return message.to_string();
    }
    let marker = "[clawpal notice] earlier context truncated to fit runtime argument limits.\n";
    let keep = ZEROCLAW_MESSAGE_MAX_BYTES.saturating_sub(marker.len());
    let tail = truncate_utf8_tail(message, keep);
    format!("{marker}{tail}")
}

fn run_zeroclaw_with_timeout(
    cmd: &Path,
    args: &[String],
    env_pairs: &[(String, String)],
    timeout_secs: u64,
) -> Result<Output, String> {
    let mut child = Command::new(cmd)
        .envs(env_pairs.iter().cloned())
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run zeroclaw sidecar: {e}"))?;

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|e| format!("failed to collect zeroclaw sidecar output: {e}"));
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let output = child.wait_with_output().ok();
                    let stderr = output
                        .as_ref()
                        .map(|out| String::from_utf8_lossy(&out.stderr).trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_default();
                    let detail = if stderr.is_empty() {
                        String::new()
                    } else {
                        format!(" stderr: {stderr}")
                    };
                    return Err(format!(
                        "zeroclaw sidecar timed out after {}s.{}",
                        timeout_secs, detail
                    ));
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                return Err(format!("failed to poll zeroclaw sidecar process: {e}"));
            }
        }
    }
}

fn as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(num) => num.as_u64(),
        Value::String(raw) => raw.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn parse_usage_from_value(value: &Value) -> Option<(u64, u64, u64)> {
    if let Value::Object(obj) = value {
        if let Some(usage) = obj.get("usage") {
            if let Some(tokens) = parse_usage_from_value(usage) {
                return Some(tokens);
            }
        }
        let prompt = obj
            .get("prompt_tokens")
            .and_then(as_u64)
            .or_else(|| obj.get("input_tokens").and_then(as_u64))
            .unwrap_or(0);
        let completion = obj
            .get("completion_tokens")
            .and_then(as_u64)
            .or_else(|| obj.get("output_tokens").and_then(as_u64))
            .unwrap_or(0);
        let total = obj
            .get("total_tokens")
            .and_then(as_u64)
            .unwrap_or(prompt.saturating_add(completion));
        if prompt > 0 || completion > 0 || total > 0 {
            return Some((prompt, completion, total));
        }
        for child in obj.values() {
            if let Some(tokens) = parse_usage_from_value(child) {
                return Some(tokens);
            }
        }
    }
    if let Value::Array(arr) = value {
        for child in arr {
            if let Some(tokens) = parse_usage_from_value(child) {
                return Some(tokens);
            }
        }
    }
    None
}

fn parse_usage_from_text(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Some(tokens) = parse_usage_from_value(&value) {
            return Some(tokens);
        }
    }
    for candidate in crate::json_util::extract_json_objects(trimmed) {
        if let Ok(value) = serde_json::from_str::<Value>(&candidate) {
            if let Some(tokens) = parse_usage_from_value(&value) {
                return Some(tokens);
            }
        }
    }
    let lowered = trimmed.to_ascii_lowercase();
    let parse_labeled = |labels: &[&str]| -> Option<u64> {
        for label in labels {
            if let Some(pos) = lowered.find(label) {
                let start = pos + label.len();
                let mut found_digit = false;
                let mut digits = String::new();
                for ch in lowered[start..].chars() {
                    if ch.is_ascii_digit() {
                        found_digit = true;
                        digits.push(ch);
                        continue;
                    }
                    if found_digit {
                        break;
                    }
                    if ch.is_ascii_whitespace() || ch == ':' || ch == '=' || ch == '"' || ch == '\''
                    {
                        continue;
                    }
                    break;
                }
                if let Ok(value) = digits.parse::<u64>() {
                    return Some(value);
                }
            }
        }
        None
    };
    let prompt = parse_labeled(&[
        "prompt_tokens",
        "prompt tokens",
        "input_tokens",
        "input tokens",
    ])
    .unwrap_or(0);
    let completion = parse_labeled(&[
        "completion_tokens",
        "completion tokens",
        "output_tokens",
        "output tokens",
    ])
    .unwrap_or(0);
    let total = parse_labeled(&["total_tokens", "total tokens"])
        .unwrap_or(prompt.saturating_add(completion));
    if prompt > 0 || completion > 0 || total > 0 {
        return Some((prompt, completion, total));
    }
    None
}

fn record_zeroclaw_usage(stdout: &str, stderr: &str) {
    if let Ok(mut stats) = usage_store().lock() {
        stats.total_calls = stats.total_calls.saturating_add(1);
        stats.last_updated_ms = now_ms();
        if let Some((prompt, completion, total)) =
            parse_usage_from_text(stdout).or_else(|| parse_usage_from_text(stderr))
        {
            stats.usage_calls = stats.usage_calls.saturating_add(1);
            stats.prompt_tokens = stats.prompt_tokens.saturating_add(prompt);
            stats.completion_tokens = stats.completion_tokens.saturating_add(completion);
            stats.total_tokens = stats.total_tokens.saturating_add(total);
        }
    }
}

fn ensure_runtime_trace_mode(config_dir: &std::path::Path) {
    let path = config_dir.join("config.toml");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return;
    };
    let mut lines: Vec<String> = raw.lines().map(|line| line.to_string()).collect();
    let mut replaced = false;
    for line in &mut lines {
        if line.trim_start().starts_with("runtime_trace_mode") {
            *line = "runtime_trace_mode = \"rolling\"".to_string();
            replaced = true;
            break;
        }
    }
    if !replaced {
        if let Some(obs_idx) = lines
            .iter()
            .position(|line| line.trim() == "[observability]")
        {
            lines.insert(obs_idx + 1, "runtime_trace_mode = \"rolling\"".to_string());
        } else {
            lines.push(String::new());
            lines.push("[observability]".to_string());
            lines.push("runtime_trace_mode = \"rolling\"".to_string());
        }
    }
    let updated = format!("{}\n", lines.join("\n"));
    if updated != raw {
        let _ = std::fs::write(&path, updated);
    }
}

fn read_usage_from_builtin_traces(
    cmd: &std::path::Path,
    config_dir: &std::path::Path,
    env_pairs: &[(String, String)],
) -> Option<(u64, u64, u64)> {
    let cfg_arg = config_dir.to_string_lossy().to_string();
    let output = Command::new(cmd)
        .envs(env_pairs.iter().cloned())
        .args([
            "--config-dir",
            cfg_arg.as_str(),
            "doctor",
            "traces",
            "--event",
            "model_reply",
            "--limit",
            "1",
        ])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    parse_usage_from_text(stdout.as_ref()).or_else(|| parse_usage_from_text(stderr.as_ref()))
}

pub fn get_zeroclaw_usage_stats() -> ZeroclawUsageStats {
    usage_store().lock().map(|stats| *stats).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Per-session usage tracking
// ---------------------------------------------------------------------------

fn session_usage_store() -> &'static Mutex<std::collections::HashMap<String, ZeroclawUsageStats>> {
    static STORE: OnceLock<Mutex<std::collections::HashMap<String, ZeroclawUsageStats>>> =
        OnceLock::new();
    STORE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

pub fn record_session_usage(session_id: &str, prompt_tokens: u64, completion_tokens: u64) {
    if session_id.is_empty() {
        return;
    }
    if let Ok(mut map) = session_usage_store().lock() {
        let stats = map
            .entry(session_id.to_string())
            .or_insert_with(ZeroclawUsageStats::default);
        stats.total_calls = stats.total_calls.saturating_add(1);
        stats.usage_calls = stats.usage_calls.saturating_add(1);
        stats.prompt_tokens = stats.prompt_tokens.saturating_add(prompt_tokens);
        stats.completion_tokens = stats.completion_tokens.saturating_add(completion_tokens);
        stats.total_tokens = stats
            .total_tokens
            .saturating_add(prompt_tokens.saturating_add(completion_tokens));
        stats.last_updated_ms = now_ms();
    }
}

pub fn get_session_usage(session_id: &str) -> ZeroclawUsageStats {
    session_usage_store()
        .lock()
        .ok()
        .and_then(|map| map.get(session_id).copied())
        .unwrap_or_default()
}

fn sanitize_instance_namespace(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "unknown-0000000000000000".to_string();
    }

    let mut normalized = String::with_capacity(trimmed.len());
    let mut last_underscore = false;
    for ch in trimmed.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else if ch == '-' || ch == '_' {
            ch
        } else {
            '_'
        };
        if mapped == '_' {
            if !last_underscore {
                normalized.push('_');
                last_underscore = true;
            }
        } else {
            normalized.push(mapped);
            last_underscore = false;
        }
    }

    let mut base = normalized.trim_matches('_').to_string();
    if base.is_empty() {
        base = "unknown".to_string();
    }
    if base.len() > 48 {
        base.truncate(48);
    }

    let mut hasher = DefaultHasher::new();
    trimmed.hash(&mut hasher);
    let suffix = hasher.finish();
    format!("{base}-{suffix:016x}")
}

fn doctor_sidecar_config_dir(instance_id: &str, session_scope: &str) -> Result<PathBuf, String> {
    let bucket = sanitize_instance_namespace(&format!("{instance_id}::{session_scope}"));
    let dir = resolve_paths()
        .clawpal_dir
        .join("zeroclaw-sidecar")
        .join("instances")
        .join(bucket);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create zeroclaw config dir: {e}"))?;
    Ok(dir)
}

fn platform_sidecar_dir_name() -> &'static str {
    if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "darwin-aarch64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "darwin-x64"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "linux-x64"
    } else if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        "windows-x64"
    } else {
        "unknown"
    }
}

fn zeroclaw_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "zeroclaw.exe"
    } else {
        "zeroclaw"
    }
}

fn push_candidate(candidates: &mut Vec<PathBuf>, path: PathBuf) {
    if !candidates.iter().any(|p| p == &path) {
        candidates.push(path);
    }
}

fn push_relative_candidate(
    candidates: &mut Vec<PathBuf>,
    base: &Path,
    rel: &[&str],
    bin_name: &str,
) {
    let mut path = base.to_path_buf();
    for seg in rel {
        path = path.join(seg);
    }
    push_candidate(candidates, path.join(bin_name));
}

fn zeroclaw_command_candidates(exe: &Path, cwd: &Path, bin_name: &str) -> Vec<PathBuf> {
    let platform_dir = platform_sidecar_dir_name();
    let exe_dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut search_bases: Vec<&Path> = vec![cwd];
    if let Some(parent) = cwd.parent() {
        search_bases.push(parent);
        if let Some(grand) = parent.parent() {
            search_bases.push(grand);
        }
    }

    for base in search_bases {
        push_relative_candidate(
            &mut candidates,
            base,
            &["src-tauri", "resources", "zeroclaw", platform_dir],
            bin_name,
        );
        push_relative_candidate(
            &mut candidates,
            base,
            &["resources", "zeroclaw", platform_dir],
            bin_name,
        );
    }

    push_relative_candidate(
        &mut candidates,
        exe_dir,
        &["..", "Resources", "zeroclaw", platform_dir],
        bin_name,
    );
    push_relative_candidate(
        &mut candidates,
        exe_dir,
        &["..", "Resources", "resources", "zeroclaw", platform_dir],
        bin_name,
    );
    push_relative_candidate(
        &mut candidates,
        exe_dir,
        &["resources", "zeroclaw", platform_dir],
        bin_name,
    );

    if let Some(parent) = exe_dir.parent() {
        push_relative_candidate(
            &mut candidates,
            parent,
            &["src-tauri", "resources", "zeroclaw", platform_dir],
            bin_name,
        );
        if let Some(grand) = parent.parent() {
            push_relative_candidate(
                &mut candidates,
                grand,
                &["src-tauri", "resources", "zeroclaw", platform_dir],
                bin_name,
            );
            if let Some(rootish) = grand.parent() {
                push_relative_candidate(
                    &mut candidates,
                    rootish,
                    &["src-tauri", "resources", "zeroclaw", platform_dir],
                    bin_name,
                );
            }
        }
    }

    push_candidate(&mut candidates, exe_dir.join(bin_name));
    candidates
}

fn resolve_zeroclaw_command_path() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("CLAWPAL_ZEROCLAW_BIN") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if p.exists() {
                return Some(p);
            }
        }
    }

    let exe = std::env::current_exe().ok()?;
    let cwd = std::env::current_dir().ok()?;
    let bin_name = zeroclaw_file_name();
    let candidates = zeroclaw_command_candidates(&exe, &cwd, bin_name);
    candidates.into_iter().find(|p| p.exists())
}

pub(crate) fn resolve_zeroclaw_command_path_for_internal() -> Option<PathBuf> {
    resolve_zeroclaw_command_path()
}

fn zeroclaw_oauth_root_dir() -> PathBuf {
    resolve_paths()
        .clawpal_dir
        .join("zeroclaw-sidecar")
        .join("oauth")
}

fn zeroclaw_oauth_config_dir_path_for_instance(instance_id: &str) -> PathBuf {
    let bucket = sanitize_instance_namespace(instance_id);
    zeroclaw_oauth_root_dir().join(bucket)
}

pub(crate) fn zeroclaw_oauth_config_dir_for_instance(instance_id: &str) -> Result<PathBuf, String> {
    let dir = zeroclaw_oauth_config_dir_path_for_instance(instance_id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create zeroclaw oauth config dir: {e}"))?;
    Ok(dir)
}

fn oauth_auth_profiles_file_exists(config_dir: &Path) -> bool {
    config_dir.join("auth-profiles.json").is_file()
}

fn pick_oauth_auth_store_source_dir(
    preferred_dir: &Path,
    local_dir: &Path,
    oauth_root: &Path,
) -> Option<PathBuf> {
    if oauth_auth_profiles_file_exists(preferred_dir) {
        return Some(preferred_dir.to_path_buf());
    }
    if local_dir != preferred_dir && oauth_auth_profiles_file_exists(local_dir) {
        return Some(local_dir.to_path_buf());
    }
    let Ok(entries) = std::fs::read_dir(oauth_root) else {
        return None;
    };
    let mut fallback_dirs = Vec::<PathBuf>::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || path == preferred_dir || path == local_dir {
            continue;
        }
        if oauth_auth_profiles_file_exists(&path) {
            fallback_dirs.push(path);
        }
    }
    fallback_dirs.sort();
    fallback_dirs.into_iter().next()
}

fn resolve_oauth_auth_store_source_dir(instance_id: &str) -> Option<PathBuf> {
    let preferred_dir = zeroclaw_oauth_config_dir_path_for_instance(instance_id);
    let local_dir = zeroclaw_oauth_config_dir_path_for_instance("local");
    let oauth_root = zeroclaw_oauth_root_dir();
    pick_oauth_auth_store_source_dir(&preferred_dir, &local_dir, &oauth_root)
}

fn collect_provider_credentials_for_doctor(
) -> std::collections::HashMap<String, crate::commands::InternalProviderCredential> {
    let credentials = crate::commands::collect_provider_credentials_for_internal();
    if !credentials.is_empty() {
        return credentials;
    }

    // Fallback for docker-local and other overridden contexts:
    // if instance-specific data has no profiles yet, reuse host default profiles.
    let current = resolve_paths();
    let Some(home) = dirs::home_dir() else {
        return credentials;
    };
    let default_clawpal = home.join(".clawpal");
    let default_openclaw = home.join(".openclaw");
    if current.clawpal_dir == default_clawpal {
        return credentials;
    }
    let fallback = OpenClawPaths {
        openclaw_dir: default_openclaw.clone(),
        config_path: default_openclaw.join("openclaw.json"),
        base_dir: default_openclaw,
        clawpal_dir: default_clawpal.clone(),
        history_dir: default_clawpal.join("history"),
        metadata_path: default_clawpal.join("metadata.json"),
    };
    crate::commands::collect_provider_credentials_from_paths(&fallback)
}

fn normalize_profile_provider_for_zeroclaw(provider: &str) -> Option<&'static str> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "openrouter" => Some("openrouter"),
        "openai" => Some("openai"),
        "openai-codex" | "github-copilot" | "copilot" => Some("openai-codex"),
        "anthropic" => Some("anthropic"),
        "ollama" => Some("ollama"),
        "lmstudio" | "lm-studio" => Some("lmstudio"),
        "llamacpp" | "llama.cpp" => Some("llamacpp"),
        "vllm" => Some("vllm"),
        "google" | "gemini" | "google-vertex" | "google-gemini-cli" | "google-antigravity" => {
            Some("gemini")
        }
        "kimi-coding" | "kimi-code" => Some("kimi-code"),
        "moonshot" => Some("moonshot"),
        _ => None,
    }
}

fn credential_clearly_mismatched_for_provider(provider: &str, secret: &str) -> bool {
    let key = secret.trim().to_ascii_lowercase();
    if key.is_empty() {
        return true;
    }
    match provider {
        // OpenRouter keys should not be Moonshot/Kimi keys.
        "openrouter" => key.starts_with("sk-kimi-"),
        // Moonshot keys should not be OpenRouter keys.
        "moonshot" | "kimi-code" => key.starts_with("sk-or-"),
        // Anthropic key path should not receive Moonshot keys.
        "anthropic" => key.starts_with("sk-kimi-"),
        _ => false,
    }
}

fn zeroclaw_env_pairs_from_clawpal() -> Vec<(String, String)> {
    let provider_credentials = collect_provider_credentials_for_doctor();
    let mut out = Vec::<(String, String)>::new();
    let mut seen = HashSet::<String>::new();
    for (provider, credential) in provider_credentials {
        let Some(mapped) = normalize_profile_provider_for_zeroclaw(&provider) else {
            continue;
        };
        if credential_clearly_mismatched_for_provider(mapped, &credential.secret) {
            continue;
        }
        let Some(env_name) = (match mapped {
            "openrouter" => Some("OPENROUTER_API_KEY"),
            "openai" => Some("OPENAI_API_KEY"),
            "openai-codex" => Some(match credential.kind {
                crate::commands::InternalAuthKind::Authorization => "OPENAI_CODEX_TOKEN",
                crate::commands::InternalAuthKind::ApiKey => "OPENAI_API_KEY",
            }),
            "anthropic" => Some(match credential.kind {
                crate::commands::InternalAuthKind::Authorization => "ANTHROPIC_OAUTH_TOKEN",
                crate::commands::InternalAuthKind::ApiKey => "ANTHROPIC_API_KEY",
            }),
            "gemini" => Some("GEMINI_API_KEY"),
            "kimi-code" => Some("MOONSHOT_API_KEY"),
            "moonshot" => Some("MOONSHOT_API_KEY"),
            _ => None,
        }) else {
            continue;
        };
        if seen.insert(env_name.to_string()) {
            out.push((env_name.to_string(), credential.secret));
        }
    }
    out
}

fn pick_zeroclaw_provider(env_pairs: &[(String, String)]) -> Option<&'static str> {
    if env_pairs.iter().any(|(k, _)| k == "OPENROUTER_API_KEY") {
        return Some("openrouter");
    }
    if env_pairs.iter().any(|(k, _)| k == "OPENAI_CODEX_TOKEN") {
        return Some("openai-codex");
    }
    if env_pairs.iter().any(|(k, _)| k == "OPENAI_API_KEY") {
        return Some("openai");
    }
    if env_pairs
        .iter()
        .any(|(k, _)| k == "ANTHROPIC_API_KEY" || k == "ANTHROPIC_OAUTH_TOKEN")
    {
        return Some("anthropic");
    }
    if env_pairs.iter().any(|(k, _)| k == "GEMINI_API_KEY") {
        return Some("gemini");
    }
    if env_pairs.iter().any(|(k, _)| k == "MOONSHOT_API_KEY") {
        return Some("moonshot");
    }
    None
}

fn collect_profile_providers_for_zeroclaw() -> HashSet<&'static str> {
    let mut out = HashSet::new();
    if let Ok(profiles) = crate::commands::list_model_profiles() {
        for provider in profiles
            .into_iter()
            .filter(|p| p.enabled)
            .filter_map(|p| normalize_zeroclaw_provider(&p.provider))
        {
            out.insert(provider);
        }
    }
    out
}

fn collect_oauth_providers_from_auth_store(config_dir: &Path) -> HashSet<&'static str> {
    let mut out = HashSet::new();
    let auth_file = config_dir.join("auth-profiles.json");
    let Ok(raw) = std::fs::read_to_string(&auth_file) else {
        return out;
    };
    let Ok(json) = serde_json::from_str::<Value>(&raw) else {
        return out;
    };

    if let Some(active_profiles) = json.get("active_profiles").and_then(Value::as_object) {
        for (provider, active_profile) in active_profiles {
            if let Some(normalized) = normalize_zeroclaw_provider(provider) {
                out.insert(normalized);
                continue;
            }
            if let Some(active) = active_profile.as_str() {
                if let Some((prefix, _)) = active.split_once(':') {
                    if let Some(normalized) = normalize_zeroclaw_provider(prefix) {
                        out.insert(normalized);
                    }
                }
            }
        }
    }

    if let Some(profiles) = json.get("profiles").and_then(Value::as_object) {
        for (profile_id, entry) in profiles {
            if let Some(provider) = entry.get("provider").and_then(Value::as_str) {
                if let Some(normalized) = normalize_zeroclaw_provider(provider) {
                    out.insert(normalized);
                    let kind = entry
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase();
                    if kind == "oauth" && normalized == "openai" {
                        out.insert("openai-codex");
                    }
                    continue;
                }
            }
            if let Some((prefix, _)) = profile_id.split_once(':') {
                if let Some(normalized) = normalize_zeroclaw_provider(prefix) {
                    out.insert(normalized);
                    let kind = entry
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase();
                    if kind == "oauth" && normalized == "openai" {
                        out.insert("openai-codex");
                    }
                }
            }
        }
    }

    out
}

fn sync_instance_oauth_auth_store_into_runtime_cfg(instance_id: &str, runtime_cfg_dir: &Path) {
    let Some(oauth_cfg_dir) = resolve_oauth_auth_store_source_dir(instance_id) else {
        return;
    };
    for file_name in [".secret_key", "auth-profiles.json"] {
        let source = oauth_cfg_dir.join(file_name);
        if !source.is_file() {
            continue;
        }
        let target = runtime_cfg_dir.join(file_name);
        let _ = std::fs::copy(&source, &target);
    }
}

fn provider_available(
    env_pairs: &[(String, String)],
    profile_providers: &HashSet<&'static str>,
    oauth_providers: &HashSet<&'static str>,
    provider: &str,
) -> bool {
    match provider {
        "openrouter" => env_pairs.iter().any(|(k, _)| k == "OPENROUTER_API_KEY"),
        "openai-codex" => {
            env_pairs
                .iter()
                .any(|(k, _)| k == "OPENAI_CODEX_TOKEN" || k == "OPENAI_API_KEY")
                || oauth_providers.contains("openai-codex")
        }
        "openai" => env_pairs.iter().any(|(k, _)| k == "OPENAI_API_KEY"),
        "anthropic" => {
            env_pairs
                .iter()
                .any(|(k, _)| k == "ANTHROPIC_API_KEY" || k == "ANTHROPIC_OAUTH_TOKEN")
                || oauth_providers.contains("anthropic")
        }
        "gemini" => {
            env_pairs.iter().any(|(k, _)| k == "GEMINI_API_KEY")
                || oauth_providers.contains("gemini")
        }
        "moonshot" | "kimi-code" => env_pairs.iter().any(|(k, _)| k == "MOONSHOT_API_KEY"),
        "ollama" | "lmstudio" | "llamacpp" | "vllm" => profile_providers.contains(provider),
        _ => false,
    }
}

fn is_keyless_local_provider(provider: &str) -> bool {
    matches!(provider, "ollama" | "lmstudio" | "llamacpp" | "vllm")
}

fn provider_exec_timeout_secs(provider: &str) -> u64 {
    if is_keyless_local_provider(provider) {
        ZEROCLAW_LOCAL_PROVIDER_TIMEOUT_SECS
    } else {
        ZEROCLAW_EXEC_TIMEOUT_SECS
    }
}

fn should_try_no_model_fallback(provider: &str) -> bool {
    !is_keyless_local_provider(provider)
}

fn normalize_zeroclaw_provider(provider: &str) -> Option<&'static str> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "openrouter" => Some("openrouter"),
        "openai" => Some("openai"),
        "openai-codex" | "github-copilot" | "copilot" => Some("openai-codex"),
        "anthropic" => Some("anthropic"),
        "ollama" => Some("ollama"),
        "lmstudio" | "lm-studio" => Some("lmstudio"),
        "llamacpp" | "llama.cpp" => Some("llamacpp"),
        "vllm" => Some("vllm"),
        "gemini" | "google" | "google-vertex" | "google-gemini-cli" | "google-antigravity" => {
            Some("gemini")
        }
        "moonshot" | "kimi-code" | "kimi-coding" => Some("moonshot"),
        _ => None,
    }
}

fn preferred_provider_from_model_value(model: &str) -> Option<&'static str> {
    let (provider, _) = model.trim().split_once('/')?;
    normalize_zeroclaw_provider(provider)
}

fn provider_order_for_runtime(
    env_pairs: &[(String, String)],
    preferred_model: Option<&str>,
    profile_providers: &HashSet<&'static str>,
    oauth_providers: &HashSet<&'static str>,
) -> Vec<&'static str> {
    let mut provider_order: Vec<&'static str> = Vec::new();
    if let Some(model) = preferred_model {
        if let Some(provider) = preferred_provider_from_model_value(model) {
            if provider_available(env_pairs, profile_providers, oauth_providers, provider) {
                provider_order.push(provider);
            }
        }
    }
    for provider in [
        "openrouter",
        "openai-codex",
        "openai",
        "anthropic",
        "gemini",
        "moonshot",
        "ollama",
        "lmstudio",
        "llamacpp",
        "vllm",
    ] {
        if provider_available(env_pairs, profile_providers, oauth_providers, provider)
            && !provider_order.contains(&provider)
        {
            provider_order.push(provider);
        }
    }
    if provider_order.is_empty() {
        if let Some(provider) = pick_zeroclaw_provider(env_pairs) {
            provider_order.push(provider);
        }
    }
    provider_order
}

pub fn get_zeroclaw_runtime_target() -> ZeroclawRuntimeTarget {
    let env_pairs = zeroclaw_env_pairs_from_clawpal();
    let preferred_model = crate::commands::load_zeroclaw_model_preference();
    let profile_providers = collect_profile_providers_for_zeroclaw();
    let oauth_providers = resolve_oauth_auth_store_source_dir("local")
        .map(|dir| collect_oauth_providers_from_auth_store(dir.as_path()))
        .unwrap_or_default();
    let provider_order = provider_order_for_runtime(
        &env_pairs,
        preferred_model.as_deref(),
        &profile_providers,
        &oauth_providers,
    );
    let provider_order_text: Vec<String> =
        provider_order.iter().map(|p| (*p).to_string()).collect();
    let Some(provider) = provider_order.first().copied() else {
        return ZeroclawRuntimeTarget {
            provider: None,
            model: None,
            source: "unavailable".to_string(),
            preferred_model,
            provider_order: provider_order_text,
        };
    };

    let mut model_candidates = candidate_models_for_provider(provider);
    prepend_preferred_model_candidate(
        &mut model_candidates,
        preferred_model.clone(),
        Some(provider),
    );
    let selected = model_candidates.first().cloned();
    let preferred_applied = preferred_model
        .as_deref()
        .map(|raw| normalize_model_for_provider(raw, Some(provider)))
        .filter(|normalized| !normalized.is_empty())
        .is_some_and(|normalized| {
            selected
                .as_deref()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(&normalized))
        });

    let source = if selected.is_none() {
        "provider_only"
    } else if preferred_applied {
        "preferred"
    } else {
        "auto"
    };

    ZeroclawRuntimeTarget {
        provider: Some(provider.to_string()),
        model: selected,
        source: source.to_string(),
        preferred_model,
        provider_order: provider_order_text,
    }
}

fn default_model_for_provider(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" => Some("claude-sonnet-4-5"),
        "openai" => Some("gpt-4o-mini"),
        "openrouter" => Some("anthropic/claude-sonnet-4-5"),
        "gemini" => Some("gemini-2.0-flash"),
        "moonshot" => Some("kimi-k2.5"),
        _ => None,
    }
}

fn candidate_models_for_provider(provider: &str) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let provider_aliases: &[&str] = match provider {
        "openai" => &["openai"],
        "openai-codex" => &["openai-codex", "github-copilot", "copilot"],
        "gemini" => &[
            "gemini",
            "google",
            "google-vertex",
            "google-gemini-cli",
            "google-antigravity",
        ],
        "moonshot" => &["moonshot", "kimi-code", "kimi-coding"],
        _ => &[provider],
    };
    if let Ok(profiles) = crate::commands::list_model_profiles() {
        for p in profiles.into_iter().filter(|p| {
            p.enabled
                && provider_aliases
                    .iter()
                    .any(|alias| p.provider.trim().eq_ignore_ascii_case(alias))
        }) {
            let mut model = p.model.trim().to_string();
            if model.is_empty() {
                continue;
            }
            if provider != "openrouter" && provider != "moonshot" {
                if let Some((_, tail)) = model.split_once('/') {
                    model = tail.to_string();
                }
            }
            if !out.contains(&model) {
                out.push(model);
            }
        }
    }
    if let Some(default_model) = default_model_for_provider(provider) {
        let d = default_model.to_string();
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

fn normalize_model_for_provider(model: &str, provider: Option<&str>) -> String {
    let mut normalized = model.trim().to_string();
    if normalized.is_empty() {
        return normalized;
    }
    if let Some(provider_name) = provider {
        if provider_name == "openrouter" {
            let provider_prefix = "openrouter/";
            if normalized.to_ascii_lowercase().starts_with(provider_prefix) {
                normalized = normalized[provider_prefix.len()..].to_string();
            }
        } else {
            let provider_prefix = format!("{provider_name}/");
            if normalized
                .to_ascii_lowercase()
                .starts_with(&provider_prefix)
            {
                normalized = normalized[provider_prefix.len()..].to_string();
            }
        }
    }
    normalized
}

fn prepend_preferred_model_candidate(
    candidates: &mut Vec<String>,
    preferred_model: Option<String>,
    provider: Option<&str>,
) {
    let Some(model) = preferred_model else {
        return;
    };
    if let Some(provider_name) = provider {
        if let Some((raw_provider, _)) = model.split_once('/') {
            let raw = raw_provider.trim().to_ascii_lowercase();
            let current = provider_name.trim().to_ascii_lowercase();
            let alias_match = (current == "openai" && raw == "openai-codex")
                || (current == "openai-codex" && raw == "openai");
            if raw != current && !alias_match {
                return;
            }
        } else if let Some(preferred_provider) = preferred_provider_from_model_value(&model) {
            if preferred_provider != provider_name {
                return;
            }
        }
    }
    let normalized = normalize_model_for_provider(&model, provider);
    if normalized.is_empty() {
        return;
    }
    if candidates
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(&normalized))
    {
        return;
    }
    candidates.insert(0, normalized);
}

pub fn run_zeroclaw_message(
    message: &str,
    instance_id: &str,
    session_scope: &str,
) -> Result<String, String> {
    let cmd = resolve_zeroclaw_command_path()
        .ok_or_else(|| "zeroclaw binary not found in bundled resources".to_string())?;
    let cfg = doctor_sidecar_config_dir(instance_id, session_scope)?;
    sync_instance_oauth_auth_store_into_runtime_cfg(instance_id, &cfg);
    ensure_runtime_trace_mode(&cfg);
    let env_pairs = zeroclaw_env_pairs_from_clawpal();
    let profile_providers = collect_profile_providers_for_zeroclaw();
    let oauth_providers = collect_oauth_providers_from_auth_store(&cfg);
    let cfg_arg = cfg.to_string_lossy().to_string();
    let message = clamp_message_for_cli(message);
    let base_args = vec![
        "--config-dir".to_string(),
        cfg_arg,
        "agent".to_string(),
        "-m".to_string(),
        message,
    ];
    // Per-session model override takes priority over global preference.
    let preferred_model = crate::commands::preferences::lookup_session_model_override(instance_id)
        .or_else(|| crate::commands::load_zeroclaw_model_preference());
    let provider_order = provider_order_for_runtime(
        &env_pairs,
        preferred_model.as_deref(),
        &profile_providers,
        &oauth_providers,
    );
    if provider_order.is_empty() {
        if env_pairs.is_empty() {
            return Err(
                "No compatible API key found in ClawPal model profiles for zeroclaw.".to_string(),
            );
        }
        return Err(
            "No supported zeroclaw provider is available from current profiles.".to_string(),
        );
    }
    let mut attempt_errors = Vec::<String>::new();
    let try_once = |args: Vec<String>, timeout_secs: u64| -> Result<String, String> {
        let output = run_zeroclaw_with_timeout(&cmd, &args, &env_pairs, timeout_secs)?;
        let stdout = sanitize_output(&String::from_utf8_lossy(&output.stdout));
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        record_zeroclaw_usage(&stdout, &stderr);
        // Also record per-session usage.
        let session_usage =
            parse_usage_from_text(&stdout).or_else(|| parse_usage_from_text(&stderr));
        if let Some((prompt, completion, _total)) = session_usage {
            record_session_usage(instance_id, prompt, completion);
        }
        if session_usage.is_none() {
            if let Ok(mut stats) = usage_store().lock() {
                if let Some((prompt, completion, total)) =
                    read_usage_from_builtin_traces(&cmd, &cfg, &env_pairs)
                {
                    stats.usage_calls = stats.usage_calls.saturating_add(1);
                    stats.prompt_tokens = stats.prompt_tokens.saturating_add(prompt);
                    stats.completion_tokens = stats.completion_tokens.saturating_add(completion);
                    stats.total_tokens = stats.total_tokens.saturating_add(total);
                    stats.last_updated_ms = now_ms();
                    // Record per-session usage from traces as well.
                    record_session_usage(instance_id, prompt, completion);
                }
            }
        }
        if !output.status.success() {
            let msg = if !stderr.is_empty() { stderr } else { stdout };
            return Err(format!("zeroclaw sidecar failed: {msg}"));
        }
        if !stdout.is_empty() {
            return Ok(stdout);
        }
        Ok("(zeroclaw returned no output)".to_string())
    };
    for provider in provider_order {
        let timeout_secs = provider_exec_timeout_secs(provider);
        let mut provider_base_args = base_args.clone();
        provider_base_args.push("-p".to_string());
        provider_base_args.push(provider.to_string());

        let mut model_candidates = candidate_models_for_provider(provider);
        prepend_preferred_model_candidate(
            &mut model_candidates,
            preferred_model.clone(),
            Some(provider),
        );
        if model_candidates.is_empty() {
            match try_once(provider_base_args.clone(), timeout_secs) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    attempt_errors.push(format!("provider={provider} no-model: {e}"));
                    continue;
                }
            }
        }

        for model in model_candidates {
            let mut args = provider_base_args.clone();
            args.push("--model".to_string());
            args.push(model.clone());
            match try_once(args, timeout_secs) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    attempt_errors.push(format!("provider={provider} model={model}: {e}"));
                    let lower = e.to_ascii_lowercase();
                    if lower.contains("authentication_error")
                        || lower.contains("unauthorized")
                        || lower.contains("invalid x-api-key")
                        || lower.contains("invalid api key")
                    {
                        break;
                    }
                    continue;
                }
            }
        }
        if should_try_no_model_fallback(provider) {
            match try_once(provider_base_args.clone(), timeout_secs) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    attempt_errors.push(format!("provider={provider} fallback: {e}"));
                }
            }
        }
    }

    if attempt_errors.is_empty() {
        Err("zeroclaw sidecar failed with no actionable error details.".to_string())
    } else {
        Err(format!(
            "All providers/models failed. Attempts: {}",
            attempt_errors.join(" | ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        credential_clearly_mismatched_for_provider, normalize_model_for_provider,
        normalize_zeroclaw_provider, parse_usage_from_text, parse_usage_from_value,
        pick_oauth_auth_store_source_dir, prepend_preferred_model_candidate,
        provider_exec_timeout_secs, provider_order_for_runtime, sanitize_instance_namespace,
        should_try_no_model_fallback, zeroclaw_command_candidates, ZEROCLAW_EXEC_TIMEOUT_SECS,
        ZEROCLAW_LOCAL_PROVIDER_TIMEOUT_SECS,
    };
    use serde_json::json;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("clawpal-{prefix}-{unique}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_auth_profiles(dir: &Path) {
        std::fs::write(
            dir.join("auth-profiles.json"),
            r#"{"profiles":{"openai:default":{"provider":"openai","profile_name":"default"}}}"#,
        )
        .expect("write auth-profiles");
    }

    #[test]
    fn instance_namespace_is_stable_for_same_instance() {
        let a = sanitize_instance_namespace("docker:local");
        let b = sanitize_instance_namespace("docker:local");
        assert_eq!(a, b);
    }

    #[test]
    fn instance_namespace_is_isolated_across_instances() {
        let local = sanitize_instance_namespace("local");
        let docker = sanitize_instance_namespace("docker:local");
        assert_ne!(local, docker);
        assert!(!docker.contains(':'));
        assert!(!docker.contains('/'));
    }

    #[test]
    fn instance_namespace_is_isolated_across_sessions() {
        let a = sanitize_instance_namespace("vm1::session-a");
        let b = sanitize_instance_namespace("vm1::session-b");
        assert_ne!(a, b);
    }

    #[test]
    fn preferred_model_is_normalized_for_non_openrouter_provider() {
        let normalized = normalize_model_for_provider("openai/gpt-4.1", Some("openai"));
        assert_eq!(normalized, "gpt-4.1");
    }

    #[test]
    fn preferred_model_preserves_prefix_for_openrouter() {
        let normalized =
            normalize_model_for_provider("anthropic/claude-sonnet-4-5", Some("openrouter"));
        assert_eq!(normalized, "anthropic/claude-sonnet-4-5");
    }

    #[test]
    fn preferred_model_strips_openrouter_prefix_for_openrouter_provider() {
        let normalized = normalize_model_for_provider(
            "openrouter/anthropic/claude-sonnet-4-5",
            Some("openrouter"),
        );
        assert_eq!(normalized, "anthropic/claude-sonnet-4-5");
    }

    #[test]
    fn preferred_model_is_prepended_without_duplicates() {
        let mut candidates = vec!["gpt-4o-mini".to_string(), "gpt-4.1".to_string()];
        prepend_preferred_model_candidate(
            &mut candidates,
            Some("openai/gpt-4.1".to_string()),
            Some("openai"),
        );
        assert_eq!(
            candidates,
            vec!["gpt-4o-mini".to_string(), "gpt-4.1".to_string()]
        );

        prepend_preferred_model_candidate(
            &mut candidates,
            Some("openai/gpt-4.5".to_string()),
            Some("openai"),
        );
        assert_eq!(
            candidates,
            vec![
                "gpt-4.5".to_string(),
                "gpt-4o-mini".to_string(),
                "gpt-4.1".to_string()
            ]
        );
    }

    #[test]
    fn preferred_model_is_ignored_when_provider_mismatches() {
        let mut candidates = vec!["claude-sonnet-4-5".to_string()];
        prepend_preferred_model_candidate(
            &mut candidates,
            Some("kimi-coding/k2p5".to_string()),
            Some("anthropic"),
        );
        assert_eq!(candidates, vec!["claude-sonnet-4-5".to_string()]);
    }

    #[test]
    fn parse_usage_from_value_supports_usage_object() {
        let value = json!({
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 3,
                "total_tokens": 15
            }
        });
        assert_eq!(parse_usage_from_value(&value), Some((12, 3, 15)));
    }

    #[test]
    fn parse_usage_from_text_supports_embedded_json() {
        let raw = r#"trace...
{"result":"ok","usage":{"input_tokens":9,"output_tokens":4}}
done"#;
        assert_eq!(parse_usage_from_text(raw), Some((9, 4, 13)));
    }

    #[test]
    fn parse_usage_from_text_supports_plain_token_labels() {
        let raw = "prompt_tokens: 111 completion_tokens=22 total_tokens 133";
        assert_eq!(parse_usage_from_text(raw), Some((111, 22, 133)));
    }

    #[test]
    fn zeroclaw_candidates_cover_repo_src_tauri_from_target_debug() {
        let exe = PathBuf::from("/repo/target/debug/clawpal.exe");
        let cwd = PathBuf::from("/repo/target/debug");
        let candidates = zeroclaw_command_candidates(&exe, &cwd, "zeroclaw.exe");
        let expected = PathBuf::from(format!(
            "/repo/src-tauri/resources/zeroclaw/{}/zeroclaw.exe",
            super::platform_sidecar_dir_name()
        ));
        assert!(
            candidates.iter().any(|p| p == &expected),
            "missing expected candidate: {expected:?}\nactual={candidates:?}"
        );
    }

    #[test]
    fn credential_guard_skips_obvious_provider_key_mismatch() {
        assert!(credential_clearly_mismatched_for_provider(
            "openrouter",
            "sk-kimi-abcdef"
        ));
        assert!(credential_clearly_mismatched_for_provider(
            "moonshot",
            "sk-or-v1-abcdef"
        ));
        assert!(!credential_clearly_mismatched_for_provider(
            "openrouter",
            "sk-or-v1-abcdef"
        ));
    }

    #[test]
    fn normalize_provider_supports_ollama_family() {
        assert_eq!(normalize_zeroclaw_provider("ollama"), Some("ollama"));
        assert_eq!(normalize_zeroclaw_provider("lm-studio"), Some("lmstudio"));
        assert_eq!(normalize_zeroclaw_provider("llama.cpp"), Some("llamacpp"));
        assert_eq!(normalize_zeroclaw_provider("vllm"), Some("vllm"));
    }

    #[test]
    fn provider_order_supports_keyless_local_provider_from_profiles() {
        let env_pairs: Vec<(String, String)> = Vec::new();
        let mut profile_providers = HashSet::new();
        profile_providers.insert("ollama");
        let oauth_providers = HashSet::new();
        let ordered = provider_order_for_runtime(
            &env_pairs,
            Some("ollama/qwen3.5:27b"),
            &profile_providers,
            &oauth_providers,
        );
        assert_eq!(ordered.first().copied(), Some("ollama"));
    }

    #[test]
    fn provider_order_keeps_keyed_provider_unavailable_without_credential() {
        let env_pairs: Vec<(String, String)> = Vec::new();
        let mut profile_providers = HashSet::new();
        profile_providers.insert("openrouter");
        let oauth_providers = HashSet::new();
        let ordered = provider_order_for_runtime(
            &env_pairs,
            Some("openrouter/anthropic/claude-sonnet-4-5"),
            &profile_providers,
            &oauth_providers,
        );
        assert!(ordered.is_empty());
    }

    #[test]
    fn provider_order_accepts_openai_codex_from_oauth_store_without_env_key() {
        let env_pairs: Vec<(String, String)> = Vec::new();
        let profile_providers = HashSet::new();
        let mut oauth_providers = HashSet::new();
        oauth_providers.insert("openai-codex");
        let ordered = provider_order_for_runtime(
            &env_pairs,
            Some("openai-codex/gpt-5"),
            &profile_providers,
            &oauth_providers,
        );
        assert_eq!(ordered.first().copied(), Some("openai-codex"));
    }

    #[test]
    fn oauth_auth_store_prefers_instance_specific_dir_when_present() {
        let root = make_temp_dir("oauth-source-preferred");
        let preferred = root.join("preferred");
        let local = root.join("local");
        let other = root.join("other");
        std::fs::create_dir_all(&preferred).expect("create preferred");
        std::fs::create_dir_all(&local).expect("create local");
        std::fs::create_dir_all(&other).expect("create other");
        write_auth_profiles(&preferred);
        write_auth_profiles(&local);
        write_auth_profiles(&other);

        let selected = pick_oauth_auth_store_source_dir(&preferred, &local, &root);
        assert_eq!(selected.as_deref(), Some(preferred.as_path()));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn oauth_auth_store_falls_back_to_local_dir() {
        let root = make_temp_dir("oauth-source-local");
        let preferred = root.join("preferred");
        let local = root.join("local");
        let other = root.join("other");
        std::fs::create_dir_all(&preferred).expect("create preferred");
        std::fs::create_dir_all(&local).expect("create local");
        std::fs::create_dir_all(&other).expect("create other");
        write_auth_profiles(&local);
        write_auth_profiles(&other);

        let selected = pick_oauth_auth_store_source_dir(&preferred, &local, &root);
        assert_eq!(selected.as_deref(), Some(local.as_path()));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn oauth_auth_store_falls_back_to_any_instance_when_local_missing() {
        let root = make_temp_dir("oauth-source-any");
        let preferred = root.join("preferred");
        let local = root.join("local");
        let z_dir = root.join("zzz");
        let a_dir = root.join("aaa");
        std::fs::create_dir_all(&preferred).expect("create preferred");
        std::fs::create_dir_all(&local).expect("create local");
        std::fs::create_dir_all(&z_dir).expect("create zzz");
        std::fs::create_dir_all(&a_dir).expect("create aaa");
        write_auth_profiles(&z_dir);
        write_auth_profiles(&a_dir);

        let selected = pick_oauth_auth_store_source_dir(&preferred, &local, &root);
        assert_eq!(selected.as_deref(), Some(a_dir.as_path()));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn local_provider_uses_shorter_timeout() {
        assert_eq!(
            provider_exec_timeout_secs("ollama"),
            ZEROCLAW_LOCAL_PROVIDER_TIMEOUT_SECS
        );
        assert_eq!(
            provider_exec_timeout_secs("lmstudio"),
            ZEROCLAW_LOCAL_PROVIDER_TIMEOUT_SECS
        );
        assert_eq!(
            provider_exec_timeout_secs("openrouter"),
            ZEROCLAW_EXEC_TIMEOUT_SECS
        );
    }

    #[test]
    fn no_model_fallback_is_skipped_for_local_provider() {
        assert!(!should_try_no_model_fallback("ollama"));
        assert!(!should_try_no_model_fallback("llamacpp"));
        assert!(should_try_no_model_fallback("openrouter"));
    }
}
