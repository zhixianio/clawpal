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

pub fn resolve_gateway_port_from_config(value: &Value) -> u16 {
    json_path_get(value, "gateway.port")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .unwrap_or(18789)
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

pub fn select_json_value_from_str(
    raw: &str,
    dotted_path: Option<&str>,
    invalid_context: &str,
) -> Result<Value, String> {
    let json = parse_json_document(raw, invalid_context)?;
    Ok(dotted_path
        .and_then(|p| json_path_get(&json, p).cloned())
        .unwrap_or(json))
}

pub fn parse_json_document(raw: &str, invalid_context: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|e| format!("invalid {invalid_context} json: {e}"))
}

pub fn parse_json_value_arg(raw: &str, operation_name: &str) -> Result<Value, String> {
    serde_json::from_str(raw)
        .map_err(|e| format!("{operation_name} requires valid JSON value: {e}"))
}

pub fn render_json_document(value: &Value, serialize_context: &str) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| format!("serialize {serialize_context}: {e}"))
}

pub fn delete_json_path_in_str(
    raw: &str,
    dotted_path: &str,
    invalid_context: &str,
    serialize_context: &str,
) -> Result<(String, bool), String> {
    let mut json = parse_json_document(raw, invalid_context)?;
    let deleted = delete_json_path(&mut json, dotted_path);
    let rendered = render_json_document(&json, serialize_context)?;
    Ok((rendered, deleted))
}

pub fn upsert_json_path_in_str(
    raw: &str,
    dotted_path: &str,
    next_value: Value,
    invalid_context: &str,
    serialize_context: &str,
) -> Result<String, String> {
    let mut json = parse_json_document(raw, invalid_context)?;
    upsert_json_path(&mut json, dotted_path, next_value)?;
    render_json_document(&json, serialize_context)
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

pub fn local_openclaw_config_path(openclaw_root: &Path) -> PathBuf {
    openclaw_root.join("openclaw.json")
}

pub fn local_openclaw_config_path_from_env() -> PathBuf {
    local_openclaw_config_path(&local_openclaw_root_from_env())
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

pub fn remote_openclaw_root_probe_script() -> &'static str {
    "printf '%s' \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}\""
}

pub fn remote_openclaw_config_path_probe_script() -> &'static str {
    "echo \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}/openclaw.json\""
}

pub fn remote_sessions_discovery_script() -> &'static str {
    "root=\"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}\"; \
first=\"$(find \"$root/agents\" -type f -path \"*/sessions/sessions.json\" 2>/dev/null | head -n 1)\"; \
if [ -n \"$first\" ]; then printf \"%s\" \"$first\"; else printf \"%s\" \"$root/agents/test/sessions/sessions.json\"; fi"
}

pub fn remote_gateway_error_log_tail_script(lines: usize) -> String {
    format!(
        "tail -{} \"${{OPENCLAW_STATE_DIR:-${{OPENCLAW_HOME:-$HOME/.openclaw}}}}/logs/gateway.err.log\" 2>/dev/null || echo ''",
        lines
    )
}

pub fn remote_openclaw_fix_find_dir_script() -> &'static str {
    "for d in \"$HOME/.npm-global/bin\" \"/opt/homebrew/bin\" \"/usr/local/bin\"; do [ -x \"$d/openclaw\" ] && echo \"$d\" && break; done"
}

pub fn remote_openclaw_fix_patch_script(path_dir: &str) -> String {
    let escaped_dir = path_dir.replace('\'', "'\\''");
    format!(
        "line='export PATH=\"{escaped_dir}:$PATH\"'; \
for f in \"$HOME/.zshrc\" \"$HOME/.bashrc\"; do \
  touch \"$f\"; \
  grep -Fq \"$line\" \"$f\" || printf '\\n%s\\n' \"$line\" >> \"$f\"; \
done; \
command -v openclaw 2>/dev/null || true"
    )
}

pub fn remote_openclaw_version_probe_script() -> &'static str {
    "openclaw --version 2>/dev/null || echo unknown"
}

pub fn openclaw_which_probe_script() -> &'static str {
    "command -v openclaw 2>/dev/null || true"
}

pub fn shell_path_probe_script() -> &'static str {
    "printf '%s' \"$PATH\""
}

pub fn remote_openclaw_gateway_status_script() -> &'static str {
    "openclaw gateway status 2>&1"
}

pub fn remote_openclaw_gateway_process_probe_script() -> &'static str {
    "pgrep -f '[o]penclaw-gateway' >/dev/null 2>&1"
}

pub fn remote_uname_s_script() -> &'static str {
    "uname -s"
}

pub fn remote_uname_m_script() -> &'static str {
    "uname -m"
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
    fn resolve_gateway_port_from_config_uses_default_when_missing() {
        let doc = json!({});
        assert_eq!(resolve_gateway_port_from_config(&doc), 18789);
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
    fn local_openclaw_config_path_from_env_ends_with_openclaw_json() {
        let path = local_openclaw_config_path_from_env();
        assert!(path.ends_with("openclaw.json"));
    }

    #[test]
    fn local_openclaw_config_path_joins_root_and_filename() {
        let path = local_openclaw_config_path(Path::new("/tmp/openclaw"));
        assert_eq!(path, PathBuf::from("/tmp/openclaw/openclaw.json"));
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

    #[test]
    fn remote_probe_scripts_reference_openclaw_state_env() {
        assert!(remote_openclaw_root_probe_script().contains("OPENCLAW_STATE_DIR"));
        assert!(remote_openclaw_config_path_probe_script().contains("openclaw.json"));
        assert!(remote_sessions_discovery_script().contains("sessions.json"));
    }

    #[test]
    fn remote_gateway_error_log_tail_script_contains_lines_and_log_path() {
        let script = remote_gateway_error_log_tail_script(100);
        assert!(script.contains("tail -100"));
        assert!(script.contains("gateway.err.log"));
    }

    #[test]
    fn remote_openclaw_fix_scripts_include_openclaw_lookup_and_path_export() {
        assert!(remote_openclaw_fix_find_dir_script().contains("openclaw"));
        let patch = remote_openclaw_fix_patch_script("/opt/homebrew/bin");
        assert!(patch.contains("export PATH="));
        assert!(patch.contains("command -v openclaw"));
    }

    #[test]
    fn remote_probe_scripts_cover_status_and_platform() {
        assert!(remote_openclaw_version_probe_script().contains("openclaw --version"));
        assert!(openclaw_which_probe_script().contains("command -v openclaw"));
        assert!(shell_path_probe_script().contains("printf '%s'"));
        assert!(remote_openclaw_gateway_status_script().contains("gateway status"));
        assert!(remote_openclaw_gateway_process_probe_script().contains("pgrep -f"));
        assert_eq!(remote_uname_s_script(), "uname -s");
        assert_eq!(remote_uname_m_script(), "uname -m");
    }

    #[test]
    fn select_json_value_from_str_reads_nested_value() {
        let value = select_json_value_from_str(
            r#"{"commands":{"ownerDisplay":"raw"}}"#,
            Some("commands.ownerDisplay"),
            "config",
        )
        .expect("select");
        assert_eq!(value.as_str().unwrap_or_default(), "raw");
    }

    #[test]
    fn delete_json_path_in_str_renders_updated_doc() {
        let (rendered, deleted) = delete_json_path_in_str(
            r#"{"commands":{"ownerDisplay":"raw","other":1}}"#,
            "commands.ownerDisplay",
            "config",
            "config",
        )
        .expect("delete");
        assert!(deleted);
        let parsed: Value = serde_json::from_str(&rendered).expect("parse rendered");
        assert!(parsed["commands"].get("ownerDisplay").is_none());
    }

    #[test]
    fn upsert_json_path_in_str_renders_updated_doc() {
        let rendered = upsert_json_path_in_str(
            r#"{"commands":{"other":1}}"#,
            "commands.ownerDisplay",
            json!("raw"),
            "config",
            "config",
        )
        .expect("upsert");
        let parsed: Value = serde_json::from_str(&rendered).expect("parse rendered");
        assert_eq!(parsed["commands"]["ownerDisplay"], "raw");
    }

    #[test]
    fn parse_json_value_arg_returns_error_for_invalid_json() {
        let err = parse_json_value_arg("{oops", "doctor config-upsert").expect_err("must fail");
        assert!(err.contains("requires valid JSON value"));
    }

    #[test]
    fn parse_json_document_returns_contextual_error() {
        let err = parse_json_document("{oops", "config").expect_err("must fail");
        assert!(err.contains("invalid config json"));
    }

    #[test]
    fn render_json_document_pretty_prints() {
        let text = render_json_document(&json!({"a":1}), "config").expect("render");
        assert!(text.contains('\n'));
    }
}
