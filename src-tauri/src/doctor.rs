use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config_io::read_openclaw_config;
use crate::models::OpenClawPaths;
use regex::Regex;

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
    use std::sync::LazyLock;
    static TRAILING: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r",(\s*[}\]])").unwrap()
    });
    let normalized = TRAILING.replace_all(text, "$1");
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
