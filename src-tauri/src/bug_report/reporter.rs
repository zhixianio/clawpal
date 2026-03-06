use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use super::settings::{built_in_sentry_dsn, BugReportBackend, BugReportSettings};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BugReportEvent {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_trace: Option<String>,
    pub level: String,
    pub timestamp: String,
    pub session_id: String,
    pub app_version: String,
    pub os_type: String,
    pub os_version: String,
}

#[derive(Debug)]
struct DsnParts {
    endpoint: String,
    public_key: String,
    dsn: String,
}

fn parse_dsn(dsn: &str) -> Result<DsnParts, String> {
    let dsn = dsn.trim();
    if dsn.is_empty() {
        return Err("empty dsn".to_string());
    }

    let (scheme, rest) = dsn
        .split_once("://")
        .ok_or_else(|| "invalid dsn: missing scheme".to_string())?;
    let (public_key, host_and_path) = rest
        .split_once('@')
        .ok_or_else(|| "invalid dsn: missing public key".to_string())?;
    if public_key.trim().is_empty() {
        return Err("invalid dsn: empty public key".to_string());
    }
    let trimmed = host_and_path.trim_end_matches('/');
    let idx = trimmed
        .rfind('/')
        .ok_or_else(|| "invalid dsn: missing project id".to_string())?;
    let host_base = &trimmed[..idx];
    let project_id = &trimmed[idx + 1..];
    if host_base.is_empty() || project_id.is_empty() {
        return Err("invalid dsn: malformed host/project".to_string());
    }
    let endpoint = format!("{scheme}://{host_base}/api/{project_id}/envelope/");
    Ok(DsnParts {
        endpoint,
        public_key: public_key.to_string(),
        dsn: dsn.to_string(),
    })
}

fn http_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())
}

fn post_custom_endpoint(url: &str, event: &BugReportEvent) -> Result<(), String> {
    let client = http_client()?;
    let response = client
        .post(url)
        .json(event)
        .send()
        .map_err(|e| e.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "bug report endpoint returned {}",
            response.status()
        ))
    }
}

fn post_sentry_envelope(dsn: &str, event: &BugReportEvent) -> Result<(), String> {
    let parsed = parse_dsn(dsn)?;
    let event_id = Uuid::new_v4().simple().to_string();
    let envelope_header = json!({
        "event_id": event_id,
        "dsn": parsed.dsn,
        "sent_at": event.timestamp,
    });
    let sentry_event = json!({
        "event_id": event_id,
        "timestamp": event.timestamp,
        "level": event.level,
        "platform": "native",
        "release": format!("clawpal@{}", event.app_version),
        "message": {
            "formatted": event.message,
        },
        "tags": {
            "session_uuid": event.session_id,
            "os_type": event.os_type,
        },
        "extra": {
            "stackTrace": event.stack_trace,
            "osVersion": event.os_version,
            "sessionUuid": event.session_id,
        },
    });
    let payload = format!(
        "{}\n{}\n{}\n",
        envelope_header,
        json!({"type":"event"}),
        sentry_event
    );

    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-sentry-envelope"),
    );
    let auth = format!(
        "Sentry sentry_version=7, sentry_key={}, sentry_client=clawpal/{}",
        parsed.public_key, event.app_version
    );
    let auth_header = HeaderValue::from_str(&auth).map_err(|e| e.to_string())?;
    headers.insert("X-Sentry-Auth", auth_header);

    let client = http_client()?;
    let response = client
        .post(parsed.endpoint)
        .headers(headers)
        .body(payload)
        .send()
        .map_err(|e| e.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("sentry backend returned {}", response.status()))
    }
}

fn dsn_for_backend(settings: &BugReportSettings) -> Result<String, String> {
    let configured = settings
        .endpoint
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    match settings.backend {
        BugReportBackend::Sentry => configured
            .or_else(built_in_sentry_dsn)
            .ok_or_else(|| "no sentry DSN configured".to_string()),
        BugReportBackend::GlitchTip => {
            configured.ok_or_else(|| "glitchtip DSN is required".to_string())
        }
        BugReportBackend::CustomUrl => Err("custom backend does not use DSN".to_string()),
    }
}

pub fn send_report(settings: &BugReportSettings, event: &BugReportEvent) -> Result<(), String> {
    match settings.backend {
        BugReportBackend::CustomUrl => {
            let url = settings
                .endpoint
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "custom endpoint URL is required".to_string())?;
            post_custom_endpoint(&url, event)
        }
        BugReportBackend::Sentry | BugReportBackend::GlitchTip => {
            let dsn = dsn_for_backend(settings)?;
            post_sentry_envelope(&dsn, event)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dsn_builds_envelope_endpoint() {
        let parsed = parse_dsn("https://public@example.com/42").expect("valid dsn");
        assert_eq!(parsed.endpoint, "https://example.com/api/42/envelope/");
        assert_eq!(parsed.public_key, "public");
    }

    #[test]
    fn parse_dsn_rejects_missing_project_id() {
        let err = parse_dsn("https://public@example.com/").expect_err("invalid dsn");
        assert!(err.contains("project id"));
    }
}
