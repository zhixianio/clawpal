use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::config_io::{read_json, write_json};
use crate::models::{resolve_paths, OpenClawPaths};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppPreferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zeroclaw_model: Option<String>,
    #[serde(default)]
    pub show_zeroclaw_doctor_ui: bool,
    #[serde(default)]
    pub show_rescue_bot_ui: bool,
}

fn app_preferences_path(paths: &OpenClawPaths) -> std::path::PathBuf {
    paths.clawpal_dir.join("app-preferences.json")
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

fn sanitize_zeroclaw_model_preference(value: Option<String>) -> Option<String> {
    let normalized_value = normalize_optional_string(value)?;
    let normalized_ref = super::normalize_model_ref(&normalized_value);
    if normalized_ref.is_empty() {
        return None;
    }

    let Ok(profiles) = crate::commands::list_model_profiles() else {
        // If profiles cannot be loaded, keep the current preference to avoid
        // dropping user intent due to transient IO issues.
        return Some(normalized_value);
    };

    let mut valid_models = HashSet::<String>::new();
    for profile in profiles.into_iter().filter(|p| p.enabled) {
        let key = super::normalize_model_ref(&super::profile_to_model_value(&profile));
        if !key.is_empty() {
            valid_models.insert(key);
        }
    }

    if valid_models.contains(&normalized_ref) {
        Some(normalized_value)
    } else {
        None
    }
}

pub fn load_app_preferences_from_paths(paths: &OpenClawPaths) -> AppPreferences {
    let path = app_preferences_path(paths);
    let mut prefs = read_json::<AppPreferences>(&path).unwrap_or_default();
    prefs.zeroclaw_model = sanitize_zeroclaw_model_preference(prefs.zeroclaw_model);
    prefs
}

fn save_app_preferences_from_paths(
    paths: &OpenClawPaths,
    prefs: &AppPreferences,
) -> Result<(), String> {
    let path = app_preferences_path(paths);
    write_json(&path, prefs)
}

pub fn load_zeroclaw_model_preference() -> Option<String> {
    let paths = resolve_paths();
    load_app_preferences_from_paths(&paths).zeroclaw_model
}

#[tauri::command]
pub fn get_app_preferences() -> Result<AppPreferences, String> {
    let paths = resolve_paths();
    Ok(load_app_preferences_from_paths(&paths))
}

#[tauri::command]
pub fn set_zeroclaw_model_preference(model: Option<String>) -> Result<AppPreferences, String> {
    let paths = resolve_paths();
    let mut prefs = load_app_preferences_from_paths(&paths);
    prefs.zeroclaw_model = normalize_optional_string(model);
    save_app_preferences_from_paths(&paths, &prefs)?;
    Ok(prefs)
}

#[tauri::command]
pub fn set_zeroclaw_doctor_ui_preference(show_ui: bool) -> Result<AppPreferences, String> {
    let paths = resolve_paths();
    let mut prefs = load_app_preferences_from_paths(&paths);
    prefs.show_zeroclaw_doctor_ui = show_ui;
    save_app_preferences_from_paths(&paths, &prefs)?;
    Ok(prefs)
}

#[tauri::command]
pub fn set_rescue_bot_ui_preference(show_ui: bool) -> Result<AppPreferences, String> {
    let paths = resolve_paths();
    let mut prefs = load_app_preferences_from_paths(&paths);
    prefs.show_rescue_bot_ui = show_ui;
    save_app_preferences_from_paths(&paths, &prefs)?;
    Ok(prefs)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ZeroclawUsageStatsResponse {
    pub total_calls: u64,
    pub usage_calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub last_updated_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ZeroclawRuntimeTargetResponse {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub source: String,
    pub preferred_model: Option<String>,
    pub provider_order: Vec<String>,
}

#[tauri::command]
pub fn get_zeroclaw_usage_stats() -> Result<ZeroclawUsageStatsResponse, String> {
    let stats = crate::runtime::zeroclaw::process::get_zeroclaw_usage_stats();
    Ok(ZeroclawUsageStatsResponse {
        total_calls: stats.total_calls,
        usage_calls: stats.usage_calls,
        prompt_tokens: stats.prompt_tokens,
        completion_tokens: stats.completion_tokens,
        total_tokens: stats.total_tokens,
        last_updated_ms: stats.last_updated_ms,
    })
}

#[tauri::command]
pub fn get_session_usage_stats(session_id: String) -> Result<ZeroclawUsageStatsResponse, String> {
    let stats = crate::runtime::zeroclaw::process::get_session_usage(&session_id);
    Ok(ZeroclawUsageStatsResponse {
        total_calls: stats.total_calls,
        usage_calls: stats.usage_calls,
        prompt_tokens: stats.prompt_tokens,
        completion_tokens: stats.completion_tokens,
        total_tokens: stats.total_tokens,
        last_updated_ms: stats.last_updated_ms,
    })
}

#[tauri::command]
pub fn get_zeroclaw_runtime_target() -> Result<ZeroclawRuntimeTargetResponse, String> {
    let target = crate::runtime::zeroclaw::process::get_zeroclaw_runtime_target();
    Ok(ZeroclawRuntimeTargetResponse {
        provider: target.provider,
        model: target.model,
        source: target.source,
        preferred_model: target.preferred_model,
        provider_order: target.provider_order,
    })
}

// ---------------------------------------------------------------------------
// Per-session model overrides (in-memory only)
// ---------------------------------------------------------------------------

fn session_model_overrides() -> &'static Mutex<HashMap<String, String>> {
    static STORE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Look up a session model override without going through Tauri command dispatch.
pub fn lookup_session_model_override(session_id: &str) -> Option<String> {
    session_model_overrides()
        .lock()
        .ok()?
        .get(session_id)
        .cloned()
}

#[tauri::command]
pub fn set_session_model_override(session_id: String, model: String) -> Result<(), String> {
    let trimmed = model.trim().to_string();
    if trimmed.is_empty() {
        return Err("model must not be empty".into());
    }
    if let Ok(mut map) = session_model_overrides().lock() {
        map.insert(session_id, trimmed);
    }
    Ok(())
}

#[tauri::command]
pub fn get_session_model_override(session_id: String) -> Result<Option<String>, String> {
    let map = session_model_overrides()
        .lock()
        .map_err(|e| e.to_string())?;
    Ok(map.get(&session_id).cloned())
}

#[tauri::command]
pub fn clear_session_model_override(session_id: String) -> Result<(), String> {
    if let Ok(mut map) = session_model_overrides().lock() {
        map.remove(&session_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_optional_string_trims_and_drops_empty_values() {
        assert_eq!(
            normalize_optional_string(Some("  openai/gpt-4.1  ".into())),
            Some("openai/gpt-4.1".into())
        );
        assert_eq!(normalize_optional_string(Some("   ".into())), None);
        assert_eq!(normalize_optional_string(None), None);
    }
}
