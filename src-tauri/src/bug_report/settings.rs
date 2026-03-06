use serde::{Deserialize, Serialize};

const DEFAULT_MAX_REPORTS_PER_HOUR: u32 = 20;
const MIN_MAX_REPORTS_PER_HOUR: u32 = 1;
const MAX_MAX_REPORTS_PER_HOUR: u32 = 1_000;
const DEFAULT_SENTRY_DSN: &str = "https://0181564e407dbd5b571190741e763b27@o4510996590886912.ingest.de.sentry.io/4510996607467600";

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
            endpoint: built_in_sentry_dsn(),
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

fn normalize_endpoint(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

pub fn normalize_settings(mut settings: BugReportSettings) -> BugReportSettings {
    settings.backend = BugReportBackend::Sentry;
    settings.endpoint = normalize_endpoint(settings.endpoint).or_else(built_in_sentry_dsn);
    settings.max_reports_per_hour = settings
        .max_reports_per_hour
        .clamp(MIN_MAX_REPORTS_PER_HOUR, MAX_MAX_REPORTS_PER_HOUR);
    settings
}

pub fn built_in_sentry_dsn() -> Option<String> {
    option_env!("CLAWPAL_SENTRY_DSN")
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
        .or_else(|| Some(DEFAULT_SENTRY_DSN.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_settings_trims_endpoint_and_clamps_rate_limit() {
        let settings = normalize_settings(BugReportSettings {
            enabled: true,
            backend: BugReportBackend::CustomUrl,
            endpoint: Some("  https://example.com/bug-report  ".into()),
            severity_threshold: BugReportSeverity::Warn,
            max_reports_per_hour: 10_000,
        });
        assert_eq!(settings.backend, BugReportBackend::Sentry);
        assert_eq!(
            settings.endpoint.as_deref(),
            Some("https://example.com/bug-report")
        );
        assert_eq!(settings.max_reports_per_hour, MAX_MAX_REPORTS_PER_HOUR);
    }

    #[test]
    fn normalize_settings_drops_blank_endpoint_and_clamps_to_minimum() {
        let settings = normalize_settings(BugReportSettings {
            enabled: false,
            backend: BugReportBackend::Sentry,
            endpoint: Some("   ".into()),
            severity_threshold: BugReportSeverity::Error,
            max_reports_per_hour: 0,
        });
        assert_eq!(settings.backend, BugReportBackend::Sentry);
        assert!(matches!(settings.endpoint, Some(value) if !value.is_empty()));
        assert_eq!(settings.max_reports_per_hour, MIN_MAX_REPORTS_PER_HOUR);
    }
}
