use std::path::{Path, PathBuf};
use std::process::Command;
use std::{
    collections::hash_map::DefaultHasher,
    collections::HashSet,
    hash::{Hash, Hasher},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::models::{resolve_paths, OpenClawPaths};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt};

use super::sanitize::{sanitize_line, sanitize_output};

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
        "openai" | "openai-codex" | "github-copilot" | "copilot" => Some("openai"),
        "anthropic" => Some("anthropic"),
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

fn provider_available(env_pairs: &[(String, String)], provider: &str) -> bool {
    match provider {
        "openrouter" => env_pairs.iter().any(|(k, _)| k == "OPENROUTER_API_KEY"),
        "openai" => env_pairs.iter().any(|(k, _)| k == "OPENAI_API_KEY"),
        "anthropic" => env_pairs
            .iter()
            .any(|(k, _)| k == "ANTHROPIC_API_KEY" || k == "ANTHROPIC_OAUTH_TOKEN"),
        "gemini" => env_pairs.iter().any(|(k, _)| k == "GEMINI_API_KEY"),
        "moonshot" | "kimi-code" => env_pairs.iter().any(|(k, _)| k == "MOONSHOT_API_KEY"),
        _ => false,
    }
}

fn normalize_zeroclaw_provider(provider: &str) -> Option<&'static str> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "openrouter" => Some("openrouter"),
        "openai" | "openai-codex" | "github-copilot" | "copilot" => Some("openai"),
        "anthropic" => Some("anthropic"),
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
) -> Vec<&'static str> {
    let mut provider_order: Vec<&'static str> = Vec::new();
    if let Some(model) = preferred_model {
        if let Some(provider) = preferred_provider_from_model_value(model) {
            if provider_available(env_pairs, provider) {
                provider_order.push(provider);
            }
        }
    }
    for provider in ["openrouter", "openai", "anthropic", "gemini", "moonshot"] {
        if provider_available(env_pairs, provider) && !provider_order.contains(&provider) {
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
    let provider_order = provider_order_for_runtime(&env_pairs, preferred_model.as_deref());
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
        "anthropic" => Some("claude-3-7-sonnet-latest"),
        "openai" => Some("gpt-4o-mini"),
        "openrouter" => Some("anthropic/claude-3.5-sonnet"),
        "gemini" => Some("gemini-2.0-flash"),
        "moonshot" => Some("kimi-k2.5"),
        _ => None,
    }
}

fn candidate_models_for_provider(provider: &str) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let provider_aliases: &[&str] = match provider {
        "openai" => &["openai", "openai-codex", "github-copilot", "copilot"],
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
            let alias_match = current == "openai" && raw == "openai-codex";
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

async fn run_zeroclaw_retry<T, Fut>(
    base_args: &[String],
    provider_order: &[&str],
    preferred_model: Option<String>,
    mut run_once: T,
) -> Result<String, String>
where
    T: FnMut(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    let mut attempt_errors = Vec::<String>::new();
    for provider in provider_order {
        let provider = *provider;
        let mut provider_base_args = base_args.to_vec();
        provider_base_args.push("-p".to_string());
        provider_base_args.push(provider.to_string());

        let mut model_candidates = candidate_models_for_provider(provider);
        prepend_preferred_model_candidate(
            &mut model_candidates,
            preferred_model.clone(),
            Some(provider),
        );
        if model_candidates.is_empty() {
            match run_once(provider_base_args.clone()).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    attempt_errors.push(format!("provider={provider} no-model: {e}"));
                    continue;
                }
            }
        }

        let mut auth_break = false;
        for model in &model_candidates {
            let mut args = provider_base_args.clone();
            args.push("--model".to_string());
            args.push(model.clone());
            match run_once(args).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    attempt_errors.push(format!("provider={provider} model={model}: {e}"));
                    let lower = e.to_ascii_lowercase();
                    if lower.contains("authentication_error")
                        || lower.contains("unauthorized")
                        || lower.contains("invalid x-api-key")
                        || lower.contains("invalid api key")
                    {
                        auth_break = true;
                        break;
                    }
                }
            }
        }
        if !auth_break {
            if let Ok(v) = run_once(provider_base_args).await {
                return Ok(v);
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

fn run_zeroclaw_once(
    cmd: &Path,
    cfg: &Path,
    env_pairs: &[(String, String)],
    args: &[String],
) -> Result<String, String> {
    let output = Command::new(cmd)
        .envs(env_pairs.iter().cloned())
        .args(args)
        .output()
        .map_err(|e| format!("failed to run zeroclaw sidecar: {e}"))?;
    let stdout = sanitize_output(&String::from_utf8_lossy(&output.stdout));
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    record_zeroclaw_usage(&stdout, &stderr);
    if parse_usage_from_text(&stdout).is_none() && parse_usage_from_text(&stderr).is_none() {
        if let Ok(mut stats) = usage_store().lock() {
            if let Some((prompt, completion, total)) =
                read_usage_from_builtin_traces(cmd, cfg, env_pairs)
            {
                stats.usage_calls = stats.usage_calls.saturating_add(1);
                stats.prompt_tokens = stats.prompt_tokens.saturating_add(prompt);
                stats.completion_tokens = stats.completion_tokens.saturating_add(completion);
                stats.total_tokens = stats.total_tokens.saturating_add(total);
                stats.last_updated_ms = now_ms();
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
}

pub fn run_zeroclaw_message(
    message: &str,
    instance_id: &str,
    session_scope: &str,
) -> Result<String, String> {
    let cmd = resolve_zeroclaw_command_path()
        .ok_or_else(|| "zeroclaw binary not found in bundled resources".to_string())?;
    let cfg = doctor_sidecar_config_dir(instance_id, session_scope)?;
    ensure_runtime_trace_mode(&cfg);
    let env_pairs = zeroclaw_env_pairs_from_clawpal();
    if env_pairs.is_empty() {
        return Err(
            "No compatible API key found in ClawPal model profiles for zeroclaw.".to_string(),
        );
    }
    let cfg_arg = cfg.to_string_lossy().to_string();
    let message = clamp_message_for_cli(message);
    let base_args = vec![
        "--config-dir".to_string(),
        cfg_arg,
        "agent".to_string(),
        "-m".to_string(),
        message,
    ];
    let preferred_model = crate::commands::load_zeroclaw_model_preference();
    let provider_order = provider_order_for_runtime(&env_pairs, preferred_model.as_deref());
    if provider_order.is_empty() {
        return Err(
            "No supported zeroclaw provider is available from current profiles.".to_string(),
        );
    }
    let runtime =
        tokio::runtime::Runtime::new().map_err(|e| format!("failed to initialize runtime: {e}"))?;
    runtime.block_on(run_zeroclaw_retry(
        &base_args,
        &provider_order,
        preferred_model,
        |args| {
            let cmd = cmd.clone();
            let cfg = cfg.clone();
            let env_pairs = env_pairs.clone();
            let args = args;
            async move {
                run_zeroclaw_once(&cmd, &cfg, &env_pairs, &args)
            }
        },
    ))
}

async fn stream_once(
    cmd: &Path,
    cfg: &Path,
    env_pairs: &[(String, String)],
    args: &[String],
    on_delta: &(dyn Fn(&str) + Send + Sync),
) -> Result<String, String> {
    let mut child = tokio::process::Command::new(cmd)
        .envs(env_pairs.iter().cloned())
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn zeroclaw sidecar: {e}"))?;

    let stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| "failed to capture zeroclaw stdout".to_string())?;
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| "failed to capture zeroclaw stderr".to_string())?;

    let mut reader = tokio::io::BufReader::new(stdout_pipe).lines();
    let stderr_task = tokio::spawn(async move {
        let mut stderr_reader = tokio::io::BufReader::new(stderr_pipe);
        let mut stderr = String::new();
        let _ = stderr_reader.read_to_string(&mut stderr).await;
        stderr
    });
    let mut accumulated = String::new();

    while let Some(line) = reader
        .next_line()
        .await
        .map_err(|e| format!("error reading zeroclaw stdout: {e}"))?
    {
        if let Some(sanitized) = sanitize_line(&line) {
            if !accumulated.is_empty() {
                accumulated.push('\n');
            }
            accumulated.push_str(&sanitized);
            on_delta(&accumulated);
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("failed to wait for zeroclaw sidecar: {e}"))?;
    let stderr = stderr_task
        .await
        .unwrap_or_default()
        .trim()
        .to_string();
    record_zeroclaw_usage(&accumulated, &stderr);

    if parse_usage_from_text(&accumulated).is_none() && parse_usage_from_text(&stderr).is_none() {
        if let Ok(mut stats) = usage_store().lock() {
            if let Some((prompt, completion, total)) =
                read_usage_from_builtin_traces(cmd, cfg, env_pairs)
            {
                stats.usage_calls = stats.usage_calls.saturating_add(1);
                stats.prompt_tokens = stats.prompt_tokens.saturating_add(prompt);
                stats.completion_tokens = stats.completion_tokens.saturating_add(completion);
                stats.total_tokens = stats.total_tokens.saturating_add(total);
                stats.last_updated_ms = now_ms();
            }
        }
    }

    if !status.success() {
        let msg = if !stderr.is_empty() {
            stderr
        } else {
            accumulated.clone()
        };
        return Err(format!("zeroclaw sidecar failed: {msg}"));
    }

    if !accumulated.is_empty() {
        return Ok(accumulated);
    }
    Ok("(zeroclaw returned no output)".to_string())
}

pub async fn run_zeroclaw_message_streaming<F>(
    message: &str,
    instance_id: &str,
    session_scope: &str,
    on_delta: F,
) -> Result<String, String>
where
    F: Fn(&str) + Send + Sync + 'static,
{
    let cmd = resolve_zeroclaw_command_path()
        .ok_or_else(|| "zeroclaw binary not found in bundled resources".to_string())?;
    let cfg = doctor_sidecar_config_dir(instance_id, session_scope)?;
    ensure_runtime_trace_mode(&cfg);
    let env_pairs = zeroclaw_env_pairs_from_clawpal();
    if env_pairs.is_empty() {
        return Err(
            "No compatible API key found in ClawPal model profiles for zeroclaw.".to_string(),
        );
    }
    let cfg_arg = cfg.to_string_lossy().to_string();
    let message = clamp_message_for_cli(message);
    let base_args = vec![
        "--config-dir".to_string(),
        cfg_arg,
        "agent".to_string(),
        "-m".to_string(),
        message,
    ];
    let preferred_model = crate::commands::load_zeroclaw_model_preference();
    let provider_order = provider_order_for_runtime(&env_pairs, preferred_model.as_deref());
    if provider_order.is_empty() {
        return Err(
            "No supported zeroclaw provider is available from current profiles.".to_string(),
        );
    }

    let on_delta: std::sync::Arc<dyn Fn(&str) + Send + Sync> =
        std::sync::Arc::new(on_delta);
    (on_delta.as_ref())("");
    run_zeroclaw_retry(
        &base_args,
        &provider_order,
        preferred_model,
        |args| {
            let cmd = cmd.clone();
            let cfg = cfg.clone();
            let env_pairs = env_pairs.clone();
            let on_delta = std::sync::Arc::clone(&on_delta);
            let args = args;
            async move {
                stream_once(&cmd, &cfg, &env_pairs, &args, on_delta.as_ref()).await
            }
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{
        credential_clearly_mismatched_for_provider, normalize_model_for_provider,
        parse_usage_from_text, parse_usage_from_value, prepend_preferred_model_candidate,
        sanitize_instance_namespace, zeroclaw_command_candidates,
    };
    use serde_json::json;
    use std::path::PathBuf;

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
            normalize_model_for_provider("anthropic/claude-3.7-sonnet", Some("openrouter"));
        assert_eq!(normalized, "anthropic/claude-3.7-sonnet");
    }

    #[test]
    fn preferred_model_strips_openrouter_prefix_for_openrouter_provider() {
        let normalized = normalize_model_for_provider(
            "openrouter/anthropic/claude-3.7-sonnet",
            Some("openrouter"),
        );
        assert_eq!(normalized, "anthropic/claude-3.7-sonnet");
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
        let mut candidates = vec!["claude-3-5-sonnet-latest".to_string()];
        prepend_preferred_model_candidate(
            &mut candidates,
            Some("kimi-coding/k2p5".to_string()),
            Some("anthropic"),
        );
        assert_eq!(candidates, vec!["claude-3-5-sonnet-latest".to_string()]);
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
}
