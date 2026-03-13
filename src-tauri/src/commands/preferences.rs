use std::collections::HashMap;
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
    #[serde(default)]
    pub show_ssh_transfer_speed_ui: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct StoredAppPreferences {
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

fn app_preferences_from_stored(stored: &StoredAppPreferences) -> AppPreferences {
    AppPreferences {
        show_ssh_transfer_speed_ui: stored.show_ssh_transfer_speed_ui,
    }
}

fn load_stored_preferences_from_paths(paths: &OpenClawPaths) -> StoredAppPreferences {
    let path = app_preferences_path(paths);
    let mut prefs = read_json::<StoredAppPreferences>(&path).unwrap_or_default();
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
    stored.show_ssh_transfer_speed_ui = prefs.show_ssh_transfer_speed_ui;
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
pub fn set_bug_report_settings(settings: BugReportSettings) -> Result<BugReportSettings, String> {
    let paths = resolve_paths();
    save_bug_report_settings_from_paths(&paths, settings)
}

#[tauri::command]
pub fn set_ssh_transfer_speed_ui_preference(show_ui: bool) -> Result<AppPreferences, String> {
    let paths = resolve_paths();
    let mut prefs = load_app_preferences_from_paths(&paths);
    prefs.show_ssh_transfer_speed_ui = show_ui;
    save_app_preferences_from_paths(&paths, &prefs)?;
    Ok(prefs)
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
                recipe_runtime_dir: clawpal_dir.join("recipe-runtime"),
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
                show_ssh_transfer_speed_ui: false,
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
                show_ssh_transfer_speed_ui: true,
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
        assert!(app_prefs.show_ssh_transfer_speed_ui);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_removed_ui_preferences_still_load_cleanly() {
        let (paths, root) = test_paths();
        let prefs_path = app_preferences_path(&paths);
        std::fs::write(&prefs_path, r#"{"showSshTransferSpeedUi":true}"#).unwrap();

        let app_prefs = load_app_preferences_from_paths(&paths);
        assert!(app_prefs.show_ssh_transfer_speed_ui);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_zeroclaw_preference_fields_are_ignored() {
        let (paths, root) = test_paths();
        let prefs_path = app_preferences_path(&paths);
        std::fs::write(
            &prefs_path,
            r#"{
                "zeroclawModel":"anthropic/claude-sonnet-4-5",
                "showZeroclawDoctorUi":true,
                "showRescueBotUi":true,
                "showGatewayLogsUi":true
            }"#,
        )
        .unwrap();

        let app_prefs = load_app_preferences_from_paths(&paths);
        assert!(!app_prefs.show_ssh_transfer_speed_ui);
        let _ = std::fs::remove_dir_all(root);
    }
}
