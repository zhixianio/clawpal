use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config_io::read_openclaw_config;
use crate::models::OpenClawPaths;
use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorEngine {
    ZeroClaw,
}

impl DoctorEngine {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ZeroClaw => "zeroclaw",
        }
    }
}

pub fn parse_engine(input: Option<String>) -> Result<DoctorEngine, String> {
    let raw = input.unwrap_or_else(|| "zeroclaw".to_string());
    match raw.trim().to_ascii_lowercase().as_str() {
        "zeroclaw" | "" => Ok(DoctorEngine::ZeroClaw),
        other => Err(format!("Unsupported doctor engine: {other}")),
    }
}

pub fn classify_engine_error(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();

    // AUTH_EXPIRED: 401/403, invalid key, quota exceeded
    if lower.contains("unauthorized")
        || lower.contains("invalid api key")
        || lower.contains("invalid_api_key")
        || (lower.contains("403") && (lower.contains("forbidden") || lower.contains("quota")))
        || (lower.contains("401") && !lower.contains("model:"))
    {
        return "AUTH_EXPIRED";
    }

    // REGISTRY_CORRUPT: registry parse/json errors
    if (lower.contains("registry") || lower.contains("instances.json"))
        && (lower.contains("parse") || lower.contains("invalid json") || lower.contains("deserialize"))
    {
        return "REGISTRY_CORRUPT";
    }

    // INSTANCE_ORPHANED: container not found
    if lower.contains("no such container")
        || (lower.contains("container") && lower.contains("not found") && !lower.contains("openclaw"))
    {
        return "INSTANCE_ORPHANED";
    }

    // Existing checks below (unchanged)
    if lower.contains("api key not set")
        || lower.contains("no compatible api key")
        || lower.contains("no auth profiles configured")
    {
        return "CONFIG_MISSING";
    }
    if lower.contains("not_found_error")
        || (lower.contains("model:") && lower.contains("404"))
    {
        return "MODEL_UNAVAILABLE";
    }
    if lower.contains("no such file")
        || lower.contains("not found")
        || lower.contains("failed to start")
        || lower.contains("permission denied")
    {
        return "RUNTIME_UNREACHABLE";
    }
    "ENGINE_ERROR"
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DoctorIssue {
    pub id: String,
    pub code: String,
    pub severity: String,
    pub message: String,
    pub auto_fixable: bool,
    pub fix_hint: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DoctorReport {
    pub ok: bool,
    pub score: u8,
    pub issues: Vec<DoctorIssue>,
}

pub fn apply_auto_fixes(paths: &OpenClawPaths, issue_ids: &[String]) -> Vec<String> {
    let text = std::fs::read_to_string(&paths.config_path).unwrap_or_else(|_| "{}".into());
    let mut current = match json5::from_str::<Value>(&text) {
        Ok(v) => v,
        Err(_) => Value::Object(Default::default()),
    };
    let mut fixed = Vec::new();

    if issue_ids.iter().any(|id| id == "field.agents") && current.get("agents").is_none() {
        let mut agents = serde_json::Map::new();
        let mut defaults = serde_json::Map::new();
        defaults.insert("model".into(), Value::String("anthropic/claude-sonnet-4-5".into()));
        agents.insert("defaults".into(), Value::Object(defaults));
        if let Value::Object(map) = &mut current {
            map.insert("agents".into(), Value::Object(agents));
        }
        fixed.push("field.agents".into());
    }

    if issue_ids.iter().any(|id| id == "json.syntax") {
        if current.is_null() {
            if let Ok(safe) = json5::from_str::<Value>("{\"agents\":{\"defaults\":{\"model\":\"anthropic/claude-sonnet-4-5\"}}}") {
                current = safe;
                fixed.push("json.syntax".into());
            }
        }
    }

    if issue_ids.iter().any(|id| id == "field.port") {
        let mut gateway = current
            .get("gateway")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        gateway.insert("port".into(), Value::Number(serde_json::Number::from(18789_u64)));
        if let Value::Object(map) = &mut current {
            map.insert("gateway".into(), Value::Object(gateway));
        }
        fixed.push("field.port".into());
    }

    let maybe_json = serde_json::to_string_pretty(&current).unwrap_or_else(|_| "{}".into());
    if !fixed.is_empty() {
        let _ = clean_and_write_json(paths, &maybe_json);
    }
    fixed
}

fn clean_and_write_json(paths: &OpenClawPaths, text: &str) -> Result<(), String> {
    let trailing = Regex::new(r",(\s*[}\]])").map_err(|e| e.to_string())?;
    let normalized = trailing.replace_all(text, "$1");
    crate::config_io::write_text(&paths.config_path, normalized.as_ref())
}

pub fn run_doctor(paths: &OpenClawPaths) -> DoctorReport {
    let mut issues = Vec::new();
    let mut score: i32 = 100;

    let text = std::fs::read_to_string(&paths.config_path).unwrap_or_else(|_| "{}".into());
    if json5::from_str::<Value>(&text).is_err() {
        issues.push(DoctorIssue {
            id: "json.syntax".into(),
            code: "json.syntax".into(),
            severity: "error".into(),
            message: "Invalid JSON5 syntax".into(),
            auto_fixable: true,
            fix_hint: Some("Try removing trailing commas and unmatched quotes".into()),
        });
        score -= 40;
    }

    if let Ok(cfg) = read_openclaw_config(paths) {
        if cfg.get("agents").is_none() {
            issues.push(DoctorIssue {
                id: "field.agents".into(),
                code: "required.field".into(),
                severity: "warn".into(),
                message: "Missing agents field; recommend initializing defaults".into(),
                auto_fixable: true,
                fix_hint: Some("Add agents.defaults with safe minimal values".into()),
            });
            score -= 10;
        }

        if let Some(port) = cfg.pointer("/gateway/port").and_then(|v| v.as_u64()) {
            if port > 65535 {
                issues.push(DoctorIssue {
                    id: "field.port".into(),
                    code: "invalid.port".into(),
                    severity: "error".into(),
                    message: "Gateway port is invalid".into(),
                    auto_fixable: false,
                    fix_hint: None,
                });
                score -= 20;
            }
        }
    }

    let perms_ok = paths.config_path.exists()
        && std::fs::metadata(&paths.config_path)
            .map(|m| !m.permissions().readonly())
            .unwrap_or(false);
    if !perms_ok {
        issues.push(DoctorIssue {
            id: "permission.config".into(),
            code: "fs.permission".into(),
            severity: "error".into(),
            message: "Config file is readonly or inaccessible".into(),
            auto_fixable: false,
            fix_hint: Some("Grant write permission then retry".into()),
        });
        score -= 20;
    }

    let mut unique = std::collections::HashSet::new();
    issues.retain(|issue| unique.insert(issue.id.clone()));

    DoctorReport {
        ok: score >= 80,
        score: score.max(0) as u8,
        issues,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_auth_expired_401() {
        assert_eq!(classify_engine_error("HTTP 401 unauthorized"), "AUTH_EXPIRED");
    }

    #[test]
    fn classify_auth_expired_403() {
        assert_eq!(classify_engine_error("403 forbidden: quota exceeded"), "AUTH_EXPIRED");
    }

    #[test]
    fn classify_auth_expired_invalid_key() {
        assert_eq!(classify_engine_error("invalid api key provided"), "AUTH_EXPIRED");
    }

    #[test]
    fn classify_registry_corrupt() {
        assert_eq!(classify_engine_error("registry parse error: invalid json at line 5"), "REGISTRY_CORRUPT");
    }

    #[test]
    fn classify_instance_orphaned_container() {
        assert_eq!(classify_engine_error("Error: no such container: abc123"), "INSTANCE_ORPHANED");
    }

    #[test]
    fn classify_instance_orphaned_not_found() {
        assert_eq!(classify_engine_error("container def456 not found"), "INSTANCE_ORPHANED");
    }

    // Ensure existing patterns still work
    #[test]
    fn classify_config_missing_still_works() {
        assert_eq!(classify_engine_error("api key not set"), "CONFIG_MISSING");
    }

    #[test]
    fn classify_model_unavailable_still_works() {
        assert_eq!(classify_engine_error("not_found_error for resource"), "MODEL_UNAVAILABLE");
    }
}
