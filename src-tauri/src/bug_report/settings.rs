use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::models::resolve_paths;

const DEFAULT_MAX_REPORTS_PER_HOUR: u32 = 20;
const MIN_MAX_REPORTS_PER_HOUR: u32 = 1;
const MAX_MAX_REPORTS_PER_HOUR: u32 = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum BugReportBackend {
    #[default]
    Sentry,
    GlitchTip,
    CustomUrl,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BugReportSeverity {
    Info,
    Warn,
    #[default]
    Error,
    Critical,
}

impl BugReportSeverity {
    pub fn rank(&self) -> u8 {
        match self {
            Self::Info => 1,
            Self::Warn => 2,
            Self::Error => 3,
            Self::Critical => 4,
        }
    }

    pub fn meets_threshold(&self, threshold: &Self) -> bool {
        self.rank() >= threshold.rank()
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BugReportSettings {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub backend: BugReportBackend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default = "default_severity_threshold")]
    pub severity_threshold: BugReportSeverity,
    #[serde(default = "default_max_reports_per_hour")]
    pub max_reports_per_hour: u32,
}

impl Default for BugReportSettings {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            backend: BugReportBackend::default(),
            endpoint: None,
            severity_threshold: default_severity_threshold(),
            max_reports_per_hour: default_max_reports_per_hour(),
        }
    }
}

fn default_enabled() -> bool {
    true
}

fn default_severity_threshold() -> BugReportSeverity {
    BugReportSeverity::Error
}

fn default_max_reports_per_hour() -> u32 {
    DEFAULT_MAX_REPORTS_PER_HOUR
}

fn app_preferences_path() -> PathBuf {
    resolve_paths().clawpal_dir.join("app-preferences.json")
}

fn normalize_endpoint(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

pub fn normalize_settings(mut settings: BugReportSettings) -> BugReportSettings {
    settings.endpoint = normalize_endpoint(settings.endpoint);
    settings.max_reports_per_hour = settings
        .max_reports_per_hour
        .clamp(MIN_MAX_REPORTS_PER_HOUR, MAX_MAX_REPORTS_PER_HOUR);
    settings
}

pub fn built_in_sentry_dsn() -> Option<String> {
    option_env!("CLAWPAL_SENTRY_DSN")
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

pub fn load_bug_report_settings() -> BugReportSettings {
    let path = app_preferences_path();
    let text = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(_) => return BugReportSettings::default(),
    };
    let root: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => return BugReportSettings::default(),
    };
    let settings_value = match root.get("bugReport") {
        Some(value) => value.clone(),
        None => return BugReportSettings::default(),
    };
    serde_json::from_value(settings_value)
        .map(normalize_settings)
        .unwrap_or_default()
}

pub fn save_bug_report_settings(input: BugReportSettings) -> Result<BugReportSettings, String> {
    let normalized = normalize_settings(input);
    let path = app_preferences_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let mut root = match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<Value>(&text).unwrap_or_else(|_| Value::Object(Map::new())),
        Err(_) => Value::Object(Map::new()),
    };
    if !root.is_object() {
        root = Value::Object(Map::new());
    }

    if let Some(obj) = root.as_object_mut() {
        obj.insert(
            "bugReport".to_string(),
            serde_json::to_value(&normalized).map_err(|e| e.to_string())?,
        );
    }
    let content = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| e.to_string())?;
    Ok(normalized)
}

