use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn delete_json_path(value: &mut Value, dotted_path: &str) -> bool {
    let parts: Vec<&str> = dotted_path
        .split('.')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return false;
    }
    let mut cursor = value;
    for part in &parts[..parts.len() - 1] {
        if let Some(next) = cursor.get_mut(*part) {
            cursor = next;
        } else {
            return false;
        }
    }
    if let Some(obj) = cursor.as_object_mut() {
        return obj.remove(parts[parts.len() - 1]).is_some();
    }
    false
}

pub fn upsert_json_path(value: &mut Value, dotted_path: &str, next_value: Value) -> Result<(), String> {
    let parts: Vec<&str> = dotted_path
        .split('.')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Err("doctor config-upsert requires non-empty <json.path>".to_string());
    }
    let mut cursor = value;
    for part in &parts[..parts.len() - 1] {
        if cursor.get(*part).is_none() {
            if let Some(obj) = cursor.as_object_mut() {
                obj.insert((*part).to_string(), serde_json::json!({}));
            } else {
                return Err(format!("path segment '{part}' is not an object"));
            }
        }
        cursor = cursor
            .get_mut(*part)
            .ok_or_else(|| format!("path segment '{part}' is missing"))?;
        if !cursor.is_object() {
            return Err(format!("path segment '{part}' is not an object"));
        }
    }
    let leaf = parts[parts.len() - 1];
    let obj = cursor
        .as_object_mut()
        .ok_or_else(|| "target parent is not an object".to_string())?;
    obj.insert(leaf.to_string(), next_value);
    Ok(())
}

pub fn json_path_get<'a>(value: &'a Value, dotted_path: &str) -> Option<&'a Value> {
    let parts: Vec<&str> = dotted_path
        .split('.')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Some(value);
    }
    let mut cursor = value;
    for part in parts {
        cursor = cursor.get(part)?;
    }
    Some(cursor)
}

pub fn validate_doctor_relative_path(path: &str) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("doctor file path cannot be empty".to_string());
    }
    if trimmed.starts_with('/') || trimmed.starts_with('~') {
        return Err("doctor file path must be relative to domain root".to_string());
    }
    if trimmed
        .split('/')
        .any(|seg| seg == ".." || seg.contains('\0') || seg.is_empty() && trimmed.contains("//"))
    {
        return Err("doctor file path contains forbidden traversal segment".to_string());
    }
    Ok(())
}

pub fn local_openclaw_root_from_env() -> PathBuf {
    std::env::var("OPENCLAW_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".openclaw")
        })
}

pub fn resolve_local_sessions_path(openclaw_root: &Path) -> PathBuf {
    let agents_dir = openclaw_root.join("agents");
    if let Ok(agent_entries) = std::fs::read_dir(&agents_dir) {
        for agent_entry in agent_entries.flatten() {
            let candidate = agent_entry.path().join("sessions").join("sessions.json");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    openclaw_root
        .join("agents")
        .join("test")
        .join("sessions")
        .join("sessions.json")
}

pub fn doctor_domain_local_root(openclaw_root: &Path, domain: &str) -> Result<PathBuf, String> {
    match domain {
        "config" => Ok(openclaw_root.to_path_buf()),
        "sessions" => Ok(openclaw_root.join("agents")),
        "logs" => Ok(openclaw_root.join("logs")),
        "state" => Ok(openclaw_root.to_path_buf()),
        _ => Err(format!("unsupported doctor file domain: {domain}")),
    }
}

pub fn doctor_domain_default_relpath(domain: &str) -> Option<&'static str> {
    match domain {
        "config" => Some("openclaw.json"),
        "logs" => Some("gateway.err.log"),
        _ => None,
    }
}

pub fn doctor_domain_remote_root(base: &str, domain: &str) -> Result<String, String> {
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("failed to resolve remote openclaw root".to_string());
    }
    match domain {
        "config" => Ok(base.to_string()),
        "sessions" => Ok(format!("{base}/agents")),
        "logs" => Ok(format!("{base}/logs")),
        "state" => Ok(base.to_string()),
        _ => Err(format!("unsupported doctor file domain: {domain}")),
    }
}

pub fn relpath_from_local_abs(root: &Path, abs: &Path) -> Option<String> {
    abs.strip_prefix(root)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

pub fn relpath_from_remote_abs(root: &str, abs: &str) -> Option<String> {
    let root = root.trim_end_matches('/');
    let prefix = format!("{root}/");
    abs.strip_prefix(&prefix).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn delete_json_path_removes_nested_field() {
        let mut doc = json!({
            "commands": {
                "ownerDisplay": "raw",
                "other": 1
            }
        });
        assert!(delete_json_path(&mut doc, "commands.ownerDisplay"));
        assert!(doc["commands"].get("ownerDisplay").is_none());
    }

    #[test]
    fn upsert_json_path_sets_nested_field() {
        let mut doc = json!({
            "commands": {
                "other": 1
            }
        });
        upsert_json_path(&mut doc, "commands.ownerDisplay", json!("raw")).expect("upsert");
        assert_eq!(doc["commands"]["ownerDisplay"], "raw");
        assert_eq!(doc["commands"]["other"], 1);
    }

    #[test]
    fn json_path_get_reads_nested_field() {
        let doc = json!({
            "commands": {
                "ownerDisplay": "raw"
            }
        });
        assert_eq!(
            json_path_get(&doc, "commands.ownerDisplay")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "raw"
        );
    }

    #[test]
    fn validate_doctor_relative_path_rejects_parent_dir() {
        let err = validate_doctor_relative_path("../secret").expect_err("must fail");
        assert!(err.contains("forbidden traversal"));
    }

    #[test]
    fn resolve_local_sessions_path_uses_default_when_empty() {
        let root = std::env::temp_dir().join("clawpal-doctor-test-root-empty");
        let path = resolve_local_sessions_path(&root);
        assert!(path.ends_with("agents/test/sessions/sessions.json"));
    }

    #[test]
    fn doctor_domain_local_root_maps_sessions_domain() {
        let root = PathBuf::from("/tmp/openclaw");
        let sessions = doctor_domain_local_root(&root, "sessions").expect("sessions root");
        assert_eq!(sessions, PathBuf::from("/tmp/openclaw/agents"));
    }

    #[test]
    fn doctor_domain_remote_root_maps_logs_domain() {
        let logs = doctor_domain_remote_root("/home/a/.openclaw", "logs").expect("logs root");
        assert_eq!(logs, "/home/a/.openclaw/logs");
    }

    #[test]
    fn relpath_from_remote_abs_extracts_relative_path() {
        let rel = relpath_from_remote_abs("/a/b", "/a/b/c/d").expect("relpath");
        assert_eq!(rel, "c/d");
    }
}
