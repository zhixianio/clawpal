use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::bug_report::settings::{
    normalize_settings as normalize_bug_report_settings, BugReportSettings,
};
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
    #[serde(default)]
    pub show_ssh_transfer_speed_ui: bool,
    #[serde(default)]
    pub show_clawpal_logs_ui: bool,
    #[serde(default)]
    pub show_gateway_logs_ui: bool,
    #[serde(default)]
    pub show_openclaw_context_ui: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct StoredAppPreferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    zeroclaw_model: Option<String>,
    #[serde(default)]
    show_zeroclaw_doctor_ui: bool,
    #[serde(default)]
    show_rescue_bot_ui: bool,
    #[serde(default)]
    show_ssh_transfer_speed_ui: bool,
    #[serde(default)]
    show_clawpal_logs_ui: bool,
    #[serde(default)]
    show_gateway_logs_ui: bool,
    #[serde(default)]
    show_openclaw_context_ui: bool,
    #[serde(default)]
    bug_report: BugReportSettings,
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

fn app_preferences_from_stored(stored: &StoredAppPreferences) -> AppPreferences {
    AppPreferences {
        zeroclaw_model: stored.zeroclaw_model.clone(),
        show_zeroclaw_doctor_ui: stored.show_zeroclaw_doctor_ui,
        show_rescue_bot_ui: stored.show_rescue_bot_ui,
        show_ssh_transfer_speed_ui: stored.show_ssh_transfer_speed_ui,
        show_clawpal_logs_ui: stored.show_clawpal_logs_ui,
        show_gateway_logs_ui: stored.show_gateway_logs_ui,
        show_openclaw_context_ui: stored.show_openclaw_context_ui,
    }
}

fn load_stored_preferences_from_paths(paths: &OpenClawPaths) -> StoredAppPreferences {
    let path = app_preferences_path(paths);
    let mut prefs = read_json::<StoredAppPreferences>(&path).unwrap_or_default();
    prefs.zeroclaw_model = sanitize_zeroclaw_model_preference(prefs.zeroclaw_model);
    prefs.bug_report = normalize_bug_report_settings(prefs.bug_report);
    prefs
}

fn save_stored_preferences_from_paths(
    paths: &OpenClawPaths,
    prefs: &StoredAppPreferences,
) -> Result<(), String> {
    let path = app_preferences_path(paths);
    write_json(&path, prefs)
}

pub fn load_app_preferences_from_paths(paths: &OpenClawPaths) -> AppPreferences {
    let prefs = load_stored_preferences_from_paths(paths);
    app_preferences_from_stored(&prefs)
}

fn save_app_preferences_from_paths(
    paths: &OpenClawPaths,
    prefs: &AppPreferences,
) -> Result<(), String> {
    let mut stored = load_stored_preferences_from_paths(paths);
    stored.zeroclaw_model = prefs.zeroclaw_model.clone();
    stored.show_zeroclaw_doctor_ui = prefs.show_zeroclaw_doctor_ui;
    stored.show_rescue_bot_ui = prefs.show_rescue_bot_ui;
    stored.show_ssh_transfer_speed_ui = prefs.show_ssh_transfer_speed_ui;
    stored.show_clawpal_logs_ui = prefs.show_clawpal_logs_ui;
    stored.show_gateway_logs_ui = prefs.show_gateway_logs_ui;
    stored.show_openclaw_context_ui = prefs.show_openclaw_context_ui;
    save_stored_preferences_from_paths(paths, &stored)
}

pub fn load_bug_report_settings_from_paths(paths: &OpenClawPaths) -> BugReportSettings {
    load_stored_preferences_from_paths(paths).bug_report
}

pub fn save_bug_report_settings_from_paths(
    paths: &OpenClawPaths,
    settings: BugReportSettings,
) -> Result<BugReportSettings, String> {
    let mut stored = load_stored_preferences_from_paths(paths);
    stored.bug_report = normalize_bug_report_settings(settings);
    let saved = stored.bug_report.clone();
    save_stored_preferences_from_paths(paths, &stored)?;
    Ok(saved)
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
pub fn get_bug_report_settings() -> Result<BugReportSettings, String> {
    let paths = resolve_paths();
    Ok(load_bug_report_settings_from_paths(&paths))
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
pub fn set_bug_report_settings(settings: BugReportSettings) -> Result<BugReportSettings, String> {
    let paths = resolve_paths();
    save_bug_report_settings_from_paths(&paths, settings)
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

#[tauri::command]
pub fn set_ssh_transfer_speed_ui_preference(show_ui: bool) -> Result<AppPreferences, String> {
    let paths = resolve_paths();
    let mut prefs = load_app_preferences_from_paths(&paths);
    prefs.show_ssh_transfer_speed_ui = show_ui;
    save_app_preferences_from_paths(&paths, &prefs)?;
    Ok(prefs)
}

#[tauri::command]
pub fn set_clawpal_logs_ui_preference(show_ui: bool) -> Result<AppPreferences, String> {
    let paths = resolve_paths();
    let mut prefs = load_app_preferences_from_paths(&paths);
    prefs.show_clawpal_logs_ui = show_ui;
    save_app_preferences_from_paths(&paths, &prefs)?;
    Ok(prefs)
}

#[tauri::command]
pub fn set_gateway_logs_ui_preference(show_ui: bool) -> Result<AppPreferences, String> {
    let paths = resolve_paths();
    let mut prefs = load_app_preferences_from_paths(&paths);
    prefs.show_gateway_logs_ui = show_ui;
    save_app_preferences_from_paths(&paths, &prefs)?;
    Ok(prefs)
}

#[tauri::command]
pub fn set_openclaw_context_ui_preference(show_ui: bool) -> Result<AppPreferences, String> {
    let paths = resolve_paths();
    let mut prefs = load_app_preferences_from_paths(&paths);
    prefs.show_openclaw_context_ui = show_ui;
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
    use crate::bug_report::settings::{BugReportBackend, BugReportSeverity};
    use crate::models::OpenClawPaths;

    #[test]
    fn normalize_optional_string_trims_and_drops_empty_values() {
        assert_eq!(
            normalize_optional_string(Some("  openai/gpt-4.1  ".into())),
            Some("openai/gpt-4.1".into())
        );
        assert_eq!(normalize_optional_string(Some("   ".into())), None);
        assert_eq!(normalize_optional_string(None), None);
    }

    fn test_paths() -> (OpenClawPaths, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("clawpal-pref-tests-{}", uuid::Uuid::new_v4()));
        let openclaw_dir = root.join(".openclaw");
        let clawpal_dir = root.join(".clawpal");
        std::fs::create_dir_all(&openclaw_dir).unwrap();
        std::fs::create_dir_all(&clawpal_dir).unwrap();
        (
            OpenClawPaths {
                openclaw_dir: openclaw_dir.clone(),
                config_path: openclaw_dir.join("openclaw.json"),
                base_dir: openclaw_dir.clone(),
                clawpal_dir: clawpal_dir.clone(),
                history_dir: clawpal_dir.join("history"),
                metadata_path: clawpal_dir.join("metadata.json"),
            },
            root,
        )
    }

    #[test]
    fn saving_app_preferences_preserves_bug_report_settings() {
        let (paths, root) = test_paths();
        let bug_report = BugReportSettings {
            enabled: false,
            backend: BugReportBackend::CustomUrl,
            endpoint: Some("https://example.com/report".into()),
            severity_threshold: BugReportSeverity::Critical,
            max_reports_per_hour: 42,
        };
        save_bug_report_settings_from_paths(&paths, bug_report.clone()).unwrap();

        save_app_preferences_from_paths(
            &paths,
            &AppPreferences {
                zeroclaw_model: Some("anthropic/claude-sonnet-4-5".into()),
                show_zeroclaw_doctor_ui: true,
                show_rescue_bot_ui: true,
                show_ssh_transfer_speed_ui: false,
                show_clawpal_logs_ui: true,
                show_gateway_logs_ui: false,
                show_openclaw_context_ui: true,
            },
        )
        .unwrap();

        let saved = load_bug_report_settings_from_paths(&paths);
        assert_eq!(saved.enabled, false);
        assert_eq!(saved.backend, BugReportBackend::Sentry);
        assert_eq!(
            saved.endpoint.as_deref(),
            Some("https://example.com/report")
        );
        assert_eq!(saved.severity_threshold, BugReportSeverity::Critical);
        assert_eq!(saved.max_reports_per_hour, 42);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn saving_bug_report_settings_preserves_existing_app_preferences() {
        let (paths, root) = test_paths();
        save_app_preferences_from_paths(
            &paths,
            &AppPreferences {
                zeroclaw_model: None,
                show_zeroclaw_doctor_ui: true,
                show_rescue_bot_ui: false,
                show_ssh_transfer_speed_ui: true,
                show_clawpal_logs_ui: true,
                show_gateway_logs_ui: true,
                show_openclaw_context_ui: true,
            },
        )
        .unwrap();

        save_bug_report_settings_from_paths(
            &paths,
            BugReportSettings {
                enabled: true,
                backend: BugReportBackend::Sentry,
                endpoint: None,
                severity_threshold: BugReportSeverity::Error,
                max_reports_per_hour: 10,
            },
        )
        .unwrap();

        let app_prefs = load_app_preferences_from_paths(&paths);
        assert_eq!(app_prefs.zeroclaw_model, None);
        assert!(app_prefs.show_zeroclaw_doctor_ui);
        assert!(!app_prefs.show_rescue_bot_ui);
        assert!(app_prefs.show_ssh_transfer_speed_ui);
        assert!(app_prefs.show_clawpal_logs_ui);
        assert!(app_prefs.show_gateway_logs_ui);
        assert!(app_prefs.show_openclaw_context_ui);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_log_visibility_preferences_default_to_hidden() {
        let (paths, root) = test_paths();
        let prefs_path = app_preferences_path(&paths);
        std::fs::write(
            &prefs_path,
            r#"{"showSshTransferSpeedUi":true}"#,
        )
        .unwrap();

        let app_prefs = load_app_preferences_from_paths(&paths);
        assert!(app_prefs.show_ssh_transfer_speed_ui);
        assert!(!app_prefs.show_clawpal_logs_ui);
        assert!(!app_prefs.show_gateway_logs_ui);
        assert!(!app_prefs.show_openclaw_context_ui);
        let _ = std::fs::remove_dir_all(root);
    }
}
