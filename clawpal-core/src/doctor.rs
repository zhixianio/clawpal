use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorIssue {
    pub id: String,
    pub code: String,
    pub severity: String,
    pub message: String,
    pub auto_fixable: bool,
    pub fix_hint: Option<String>,
    pub source: String,
}

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

pub fn upsert_json_path(
    value: &mut Value,
    dotted_path: &str,
    next_value: Value,
) -> Result<(), String> {
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

pub fn resolve_agent_workspace_from_config(
    value: &Value,
    agent_id: &str,
    fallback_default_workspace: Option<&str>,
) -> Result<String, String> {
    let agents_list = json_path_get(value, "agents.list")
        .and_then(Value::as_array)
        .ok_or_else(|| "agents.list not found".to_string())?;

    let agent = agents_list
        .iter()
        .find(|a| a.get("id").and_then(Value::as_str) == Some(agent_id))
        .ok_or_else(|| format!("Agent '{}' not found", agent_id))?;

    let default_workspace = json_path_get(value, "agents.defaults.workspace")
        .or_else(|| json_path_get(value, "agents.default.workspace"))
        .and_then(Value::as_str)
        .or(fallback_default_workspace);

    agent
        .get("workspace")
        .and_then(Value::as_str)
        .or(default_workspace)
        .map(str::to_string)
        .ok_or_else(|| format!("Agent '{}' has no workspace configured", agent_id))
}

pub fn doctor_json_option_unsupported(stderr: &str, stdout: &str) -> bool {
    let details = format!("{stderr}\n{stdout}").to_ascii_lowercase();
    (details.contains("unknown option")
        || details.contains("unknown argument")
        || details.contains("unrecognized option")
        || details.contains("unexpected argument")
        || details.contains("no such option"))
        && details.contains("--json")
}

pub fn normalize_issue_severity(raw: &str) -> String {
    let value = raw.trim().to_ascii_lowercase();
    if value.contains("error") {
        return "error".into();
    }
    if value.contains("warn") {
        return "warn".into();
    }
    "info".into()
}

pub fn parse_doctor_issues(report: &Value, source: &str) -> Vec<DoctorIssue> {
    let mut items = Vec::new();
    let Some(issues) = report.get("issues").and_then(Value::as_array) else {
        return items;
    };
    for (index, issue) in issues.iter().enumerate() {
        let Some(obj) = issue.as_object() else {
            continue;
        };
        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{source}.doctor.issue.{index}"));
        let code = obj
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("doctor.issue")
            .to_string();
        let severity = normalize_issue_severity(
            obj.get("severity")
                .and_then(Value::as_str)
                .unwrap_or("warn"),
        );
        let message = obj
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Doctor reported an issue")
            .to_string();
        let auto_fixable = obj
            .get("autoFixable")
            .and_then(Value::as_bool)
            .or_else(|| obj.get("auto_fixable").and_then(Value::as_bool))
            .unwrap_or(false);
        let fix_hint = obj
            .get("fixHint")
            .and_then(Value::as_str)
            .or_else(|| obj.get("fix_hint").and_then(Value::as_str))
            .map(str::to_string);
        items.push(DoctorIssue {
            id,
            code,
            severity,
            message,
            auto_fixable,
            fix_hint,
            source: source.to_string(),
        });
    }
    items
}

pub fn dedupe_doctor_issues(issues: &mut Vec<DoctorIssue>) {
    let mut seen = HashSet::new();
    issues.retain(|issue| seen.insert(issue.id.clone()));
}

pub fn classify_doctor_issue_status(issues: &[DoctorIssue]) -> String {
    if issues.iter().any(|issue| issue.severity == "error") {
        return "broken".into();
    }
    if issues.is_empty() {
        return "healthy".into();
    }
    "degraded".into()
}

pub fn summarize_gateway_status(status: &Value) -> Option<String> {
    let running = status
        .get("running")
        .and_then(Value::as_bool)
        .or_else(|| status.pointer("/gateway/running").and_then(Value::as_bool));
    let healthy = status
        .get("healthy")
        .and_then(Value::as_bool)
        .or_else(|| status.pointer("/health/ok").and_then(Value::as_bool))
        .or_else(|| status.pointer("/health/healthy").and_then(Value::as_bool));
    let port = status
        .get("port")
        .and_then(Value::as_u64)
        .or_else(|| status.pointer("/gateway/port").and_then(Value::as_u64));
    let service_status = status
        .pointer("/service/runtime/status")
        .and_then(Value::as_str);
    let service_state = status
        .pointer("/service/runtime/state")
        .and_then(Value::as_str);
    let rpc_ok = status.pointer("/rpc/ok").and_then(Value::as_bool);
    let port_status = status.pointer("/port/status").and_then(Value::as_str);

    let mut parts = Vec::new();
    if let Some(value) = running {
        parts.push(format!("running={value}"));
    }
    if let Some(value) = healthy {
        parts.push(format!("healthy={value}"));
    }
    if let Some(value) = port {
        parts.push(format!("port={value}"));
    }
    if let Some(value) = service_status.or(service_state) {
        parts.push(format!("state={value}"));
    }
    if let Some(value) = rpc_ok {
        parts.push(format!("rpc={value}"));
    }
    if let Some(value) = port_status {
        parts.push(format!("port_status={value}"));
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join(", "))
}

pub fn gateway_output_ok(exit_code: i32, stdout: &str, stderr: &str) -> bool {
    if exit_code != 0 {
        return false;
    }
    let status = parse_json_loose(stdout).or_else(|| parse_json_loose(stderr));
    let Some(status) = status else {
        let details = format!("{stdout}\n{stderr}").to_ascii_lowercase();
        if details.contains("not running")
            || details.contains("already stopped")
            || details.contains("isn't running")
            || details.contains("is not running")
            || details.contains("down")
            || details.contains("stopped")
            || details.contains("unhealthy")
            || details.contains("not healthy")
            || details.contains("inactive")
            || details.contains("failed to start")
            || details.contains("failed to load")
            || details.contains("could not load")
            || details.contains("could not read")
            || details.contains("invalid json")
            || details.contains("json syntax")
            || details.contains("parse error")
            || details.contains("malformed")
        {
            return false;
        }
        return true;
    };
    let running = status
        .get("running")
        .and_then(Value::as_bool)
        .or_else(|| status.pointer("/gateway/running").and_then(Value::as_bool));
    let healthy = status
        .get("healthy")
        .and_then(Value::as_bool)
        .or_else(|| status.pointer("/health/ok").and_then(Value::as_bool))
        .or_else(|| status.pointer("/health/healthy").and_then(Value::as_bool));
    let service_status = status
        .pointer("/service/runtime/status")
        .and_then(Value::as_str)
        .map(|value| value.to_ascii_lowercase());
    let service_state = status
        .pointer("/service/runtime/state")
        .and_then(Value::as_str)
        .map(|value| value.to_ascii_lowercase());
    let service_sub_state = status
        .pointer("/service/runtime/subState")
        .and_then(Value::as_str)
        .map(|value| value.to_ascii_lowercase());
    let rpc_ok = status.pointer("/rpc/ok").and_then(Value::as_bool);
    let port_status = status
        .pointer("/port/status")
        .and_then(Value::as_str)
        .map(|value| value.to_ascii_lowercase());
    let listeners_empty = status
        .pointer("/port/listeners")
        .and_then(Value::as_array)
        .map(|listeners| listeners.is_empty());

    if matches!(running, Some(false)) || matches!(healthy, Some(false)) {
        return false;
    }

    if matches!(
        service_status.as_deref(),
        Some("stopped" | "inactive" | "failed")
    ) || matches!(
        service_state.as_deref(),
        Some("inactive" | "dead" | "failed")
    ) || matches!(
        service_sub_state.as_deref(),
        Some("dead" | "failed" | "exited")
    ) {
        return false;
    }

    if matches!(port_status.as_deref(), Some("free" | "closed"))
        && matches!(listeners_empty, Some(true))
    {
        return false;
    }

    if matches!(rpc_ok, Some(false))
        && (matches!(
            service_status.as_deref(),
            Some("stopped" | "inactive" | "failed")
        ) || matches!(
            service_state.as_deref(),
            Some("inactive" | "dead" | "failed")
        ) || matches!(port_status.as_deref(), Some("free" | "closed")))
    {
        return false;
    }

    true
}

pub fn gateway_output_detail(exit_code: i32, stdout: &str, stderr: &str) -> Option<String> {
    if exit_code == 0 {
        parse_json_loose(stdout)
            .or_else(|| parse_json_loose(stderr))
            .and_then(|status| summarize_gateway_status(&status))
            .or_else(|| {
                let details = command_output_detail(stderr, stdout);
                if details == "no output" {
                    None
                } else {
                    Some(details)
                }
            })
    } else {
        None
    }
}

pub fn trim_for_detail(raw: &str) -> String {
    let compact = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .find(|line| !line.is_empty())
        .unwrap_or("no output");
    let mut detail = compact.to_string();
    const MAX_LEN: usize = 220;
    if detail.chars().count() > MAX_LEN {
        detail = detail.chars().take(MAX_LEN).collect::<String>();
        detail.push('…');
    }
    detail
}

pub fn command_output_detail(stderr: &str, stdout: &str) -> String {
    if !stderr.trim().is_empty() {
        return trim_for_detail(stderr);
    }
    if !stdout.trim().is_empty() {
        return trim_for_detail(stdout);
    }
    "no output".into()
}

pub fn collect_repairable_primary_issue_ids(
    issues: &[DoctorIssue],
    requested_ids: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut seen_repairable = HashSet::new();
    let repairable_ids = issues
        .iter()
        .filter(|issue| is_repairable_primary_issue(&issue.source, &issue.id, issue.auto_fixable))
        .filter_map(|issue| {
            if seen_repairable.insert(issue.id.clone()) {
                Some(issue.id.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    if requested_ids.is_empty() {
        return (repairable_ids, Vec::new());
    }

    let repairable_set = repairable_ids.iter().cloned().collect::<HashSet<_>>();
    let mut selected = Vec::new();
    let mut skipped = Vec::new();
    let mut seen_requested = HashSet::new();
    for id in requested_ids {
        if !seen_requested.insert(id.clone()) {
            continue;
        }
        if repairable_set.contains(id) {
            selected.push(id.clone());
        } else {
            skipped.push(id.clone());
        }
    }
    (selected, skipped)
}

pub fn is_repairable_primary_issue(source: &str, issue_id: &str, _auto_fixable: bool) -> bool {
    if source != "primary" {
        return false;
    }
    issue_id.starts_with("primary.gateway.")
        || issue_id.starts_with("primary.config.")
        || matches!(
            issue_id,
            "field.port" | "field.gateway.port" | "field.bind" | "field.gateway.bind"
        )
}

pub fn is_primary_gateway_recovery_issue(issue_id: &str) -> bool {
    issue_id == "primary.gateway.unhealthy"
}

pub fn is_primary_rescue_permission_issue(
    source: &str,
    issue_id: &str,
    code: &str,
    message: &str,
    fix_hint: Option<&str>,
) -> bool {
    if source != "primary" {
        return false;
    }
    let haystack = format!(
        "{} {} {} {}",
        issue_id,
        code,
        message,
        fix_hint.unwrap_or_default()
    )
    .to_ascii_lowercase();
    [
        "allowlist",
        "allowfrom",
        "groupallowfrom",
        "grouppolicy",
        "mention",
        "permission",
        "approval",
        "sandbox",
        "visibility",
        "tools.allow",
        "tools.profile",
        "tools.exec",
        "sessions.visibility",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}

pub fn build_rescue_permission_baseline_commands(profile: &str) -> Vec<Vec<String>> {
    vec![
        vec![
            "--profile".to_string(),
            profile.to_string(),
            "config".to_string(),
            "set".to_string(),
            "tools.profile".to_string(),
            "\"full\"".to_string(),
            "--json".to_string(),
        ],
        vec![
            "--profile".to_string(),
            profile.to_string(),
            "config".to_string(),
            "set".to_string(),
            "tools.sessions.visibility".to_string(),
            "\"all\"".to_string(),
            "--json".to_string(),
        ],
        vec![
            "--profile".to_string(),
            profile.to_string(),
            "config".to_string(),
            "set".to_string(),
            "tools.allow".to_string(),
            "[\"*\"]".to_string(),
            "--json".to_string(),
        ],
        vec![
            "--profile".to_string(),
            profile.to_string(),
            "config".to_string(),
            "set".to_string(),
            "tools.exec.host".to_string(),
            "\"gateway\"".to_string(),
            "--json".to_string(),
        ],
        vec![
            "--profile".to_string(),
            profile.to_string(),
            "config".to_string(),
            "set".to_string(),
            "tools.exec.security".to_string(),
            "\"full\"".to_string(),
            "--json".to_string(),
        ],
        vec![
            "--profile".to_string(),
            profile.to_string(),
            "config".to_string(),
            "set".to_string(),
            "tools.exec.ask".to_string(),
            "\"off\"".to_string(),
            "--json".to_string(),
        ],
    ]
}

pub fn build_primary_issue_fix_tail(issue_id: &str) -> Option<(String, Vec<String>)> {
    let normalize_primary_model_tail = || {
        vec![
            "config".to_string(),
            "set".to_string(),
            "agents.defaults.model".to_string(),
            "anthropic/claude-sonnet-4-5".to_string(),
            "--json".to_string(),
        ]
    };

    match issue_id {
        "field.agents" => Some((
            "Initialize agents.defaults.model".into(),
            normalize_primary_model_tail(),
        )),
        "json.syntax" => Some((
            "Normalize primary profile config".into(),
            normalize_primary_model_tail(),
        )),
        "field.port" => Some((
            "Normalize gateway port".into(),
            vec![
                "config".to_string(),
                "set".to_string(),
                "gateway.port".to_string(),
                "18789".to_string(),
                "--json".to_string(),
            ],
        )),
        _ => None,
    }
}

pub fn gateway_restart_timeout(stderr: &str, stdout: &str) -> bool {
    let details = format!("{stderr}\n{stdout}").to_ascii_lowercase();
    details.contains("gateway restart timed out")
        || (details.contains("timed out") && details.contains("health check"))
}

pub fn owner_display_parse_error(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("ownerdisplay")
        && (lower.contains("unknown field")
            || lower.contains("invalid field")
            || lower.contains("failed to parse")
            || lower.contains("deserialize"))
}

pub fn rescue_cleanup_noop(
    action: &str,
    command: &[String],
    exit_code: i32,
    stderr: &str,
    stdout: &str,
) -> bool {
    if exit_code == 0 || !matches!(action, "deactivate" | "unset") {
        return false;
    }
    let details = format!("{stderr}\n{stdout}").to_ascii_lowercase();
    if details.contains("profile") && details.contains("not found") {
        return true;
    }
    if command.len() >= 2
        && command[command.len() - 2] == "gateway"
        && command[command.len() - 1] == "stop"
    {
        return details.contains("not running")
            || details.contains("already stopped")
            || details.contains("isn't running")
            || details.contains("is not running");
    }
    if command.len() >= 2
        && command[command.len() - 2] == "gateway"
        && command[command.len() - 1] == "uninstall"
    {
        return details.contains("not installed")
            || details.contains("already uninstalled")
            || details.contains("isn't installed")
            || details.contains("is not installed");
    }
    if command
        .windows(3)
        .any(|window| window[0] == "config" && window[1] == "unset" && window[2] == "gateway.port")
    {
        return details.contains("not found")
            || details.contains("not set")
            || details.contains("does not exist")
            || details.contains("missing");
    }
    if command
        .windows(2)
        .any(|window| window[0] == "gateway" && window[1] == "status")
    {
        return details.contains("not running")
            || details.contains("not installed")
            || details.contains("not found")
            || details.contains("is not running")
            || details.contains("isn't running");
    }
    false
}

pub fn build_rescue_bot_command_plan(
    action: &str,
    profile: &str,
    rescue_port: u16,
    include_configure: bool,
) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let profile_arg = vec!["--profile".to_string(), profile.to_string()];
    let rescue_port_str = rescue_port.to_string();

    match action {
        "set" => {
            if include_configure {
                commands.push({
                    let mut cmd = profile_arg.clone();
                    cmd.push("setup".into());
                    cmd
                });
                commands.push({
                    let mut cmd = profile_arg.clone();
                    cmd.extend([
                        "config".into(),
                        "set".into(),
                        "gateway.port".into(),
                        rescue_port_str,
                        "--json".into(),
                    ]);
                    cmd
                });
            }
            commands.extend(build_rescue_permission_baseline_commands(profile));
        }
        "activate" => {
            commands.extend(build_rescue_bot_command_plan(
                "set",
                profile,
                rescue_port,
                include_configure,
            ));
            if include_configure {
                commands.push({
                    let mut cmd = profile_arg.clone();
                    cmd.extend(["gateway".into(), "stop".into()]);
                    cmd
                });
                commands.push({
                    let mut cmd = profile_arg.clone();
                    cmd.extend(["gateway".into(), "uninstall".into()]);
                    cmd
                });
                commands.push({
                    let mut cmd = profile_arg.clone();
                    cmd.extend(["gateway".into(), "install".into()]);
                    cmd
                });
                commands.push({
                    let mut cmd = profile_arg.clone();
                    cmd.extend(["gateway".into(), "start".into()]);
                    cmd
                });
                commands.push({
                    let mut cmd = profile_arg.clone();
                    cmd.extend(["gateway".into(), "status".into(), "--json".into()]);
                    cmd
                });
            } else {
                commands.push({
                    let mut cmd = profile_arg.clone();
                    cmd.extend(["gateway".into(), "install".into()]);
                    cmd
                });
                commands.push({
                    let mut cmd = profile_arg.clone();
                    cmd.extend(["gateway".into(), "restart".into()]);
                    cmd
                });
                commands.push({
                    let mut cmd = profile_arg.clone();
                    cmd.extend([
                        "gateway".into(),
                        "status".into(),
                        "--no-probe".into(),
                        "--json".into(),
                    ]);
                    cmd
                });
            }
        }
        "status" => {
            commands.push({
                let mut cmd = profile_arg.clone();
                cmd.extend([
                    "gateway".into(),
                    "status".into(),
                    "--no-probe".into(),
                    "--json".into(),
                ]);
                cmd
            });
        }
        "deactivate" => {
            commands.push({
                let mut cmd = profile_arg.clone();
                cmd.extend(["gateway".into(), "stop".into()]);
                cmd
            });
            commands.push({
                let mut cmd = profile_arg;
                cmd.extend([
                    "gateway".into(),
                    "status".into(),
                    "--no-probe".into(),
                    "--json".into(),
                ]);
                cmd
            });
        }
        "unset" => {
            commands.push({
                let mut cmd = profile_arg.clone();
                cmd.extend(["gateway".into(), "stop".into()]);
                cmd
            });
            commands.push({
                let mut cmd = profile_arg.clone();
                cmd.extend(["gateway".into(), "uninstall".into()]);
                cmd
            });
            commands.push({
                let mut cmd = profile_arg;
                cmd.extend(["config".into(), "unset".into(), "gateway.port".into()]);
                cmd
            });
        }
        _ => {}
    }

    commands
}

pub fn command_failure_message(
    command: &[String],
    exit_code: i32,
    stderr: &str,
    stdout: &str,
) -> String {
    let details = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        "no output"
    };
    format!(
        "openclaw {} failed (exit {}): {}",
        command.join(" "),
        exit_code,
        details
    )
}

pub fn is_gateway_restart_command(command: &[String]) -> bool {
    command.len() >= 2
        && command[command.len() - 2] == "gateway"
        && command[command.len() - 1] == "restart"
}

pub fn suggest_rescue_port(main_port: u16) -> u16 {
    let with_large_gap = main_port.saturating_add(1000);
    let min_gap = main_port.saturating_add(20);
    with_large_gap.max(min_gap)
}

pub fn ensure_rescue_port_spacing(main_port: u16, rescue_port: u16) -> Result<(), String> {
    let min_recommended_port = main_port.saturating_add(20);
    if rescue_port < min_recommended_port {
        return Err(format!(
            "rescue port {rescue_port} is too close to primary gateway port {main_port}; \
             choose at least {min_recommended_port} (>= +20)"
        ));
    }
    Ok(())
}

pub fn parse_rescue_port_value(value: &Value) -> Option<u16> {
    match value {
        Value::Number(n) => n.as_u64().and_then(|v| u16::try_from(v).ok()),
        Value::String(s) => s.trim().parse::<u16>().ok(),
        _ => None,
    }
}

pub fn apply_issue_fixes(config: &mut Value, ids: &[String]) -> Result<Vec<String>, String> {
    let mut applied = Vec::new();
    for id in ids {
        match id.as_str() {
            "field.agents" if json_path_get(config, "agents").is_none() => {
                upsert_json_path(
                    config,
                    "agents",
                    serde_json::json!({
                        "defaults": {
                            "model": "anthropic/claude-sonnet-4-5"
                        }
                    }),
                )?;
                applied.push(id.clone());
            }
            "json.syntax" => {
                // Caller already chose a parse strategy; treat as handled once document is available.
                applied.push(id.clone());
            }
            "field.port" => {
                upsert_json_path(
                    config,
                    "gateway.port",
                    Value::Number(serde_json::Number::from(18789_u64)),
                )?;
                applied.push(id.clone());
            }
            _ => {}
        }
    }
    Ok(applied)
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

pub fn extract_json_from_output(raw: &str) -> Option<&str> {
    let end_object = raw.rfind('}');
    let end_array = raw.rfind(']');
    let (end, opener, closer) = match (end_object, end_array) {
        (Some(object_end), Some(array_end)) if object_end > array_end => (object_end, b'{', b'}'),
        (Some(_), Some(array_end)) => (array_end, b'[', b']'),
        (Some(object_end), None) => (object_end, b'{', b'}'),
        (None, Some(array_end)) => (array_end, b'[', b']'),
        (None, None) => return None,
    };

    let bytes = raw.as_bytes();
    let mut depth: i32 = 0;
    for i in (0..=end).rev() {
        let ch = bytes[i];
        if ch == closer {
            depth += 1;
        } else if ch == opener {
            depth -= 1;
            if depth == 0 {
                return Some(&raw[i..=end]);
            }
        }
    }
    None
}

pub fn parse_json_loose(raw: &str) -> Option<Value> {
    if raw.trim().is_empty() {
        return None;
    }
    serde_json::from_str(raw)
        .ok()
        .or_else(|| extract_json_from_output(raw).and_then(|json| serde_json::from_str(json).ok()))
}

pub fn parse_json5_document(raw: &str, invalid_context: &str) -> Result<Value, String> {
    json5::from_str(raw).map_err(|e| format!("invalid {invalid_context} json5: {e}"))
}

pub fn parse_json5_document_or_default(raw: &str) -> Value {
    parse_json5_document(raw, "config").unwrap_or_else(|_| Value::Object(Default::default()))
}

pub fn parse_json_value_arg(raw: &str, operation_name: &str) -> Result<Value, String> {
    serde_json::from_str(raw)
        .map_err(|e| format!("{operation_name} requires valid JSON value: {e}"))
}

pub fn render_json_document(value: &Value, serialize_context: &str) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| format!("serialize {serialize_context}: {e}"))
}

pub fn strip_doctor_banner(text: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let mut in_banner = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.contains("Doctor warnings") && trimmed.contains('╮') {
            in_banner = true;
            continue;
        }
        if in_banner {
            if trimmed.contains('╯') {
                in_banner = false;
            }
            continue;
        }
        if !trimmed.is_empty() {
            lines.push(line);
        }
    }
    let result = lines.join("\n").trim().to_string();
    if result.is_empty() {
        "Command failed".into()
    } else {
        result
    }
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
    remote_gateway_log_tail_script(lines, "gateway.err")
}

pub fn remote_gateway_log_tail_script(lines: usize, filename: &str) -> String {
    let file = filename.trim_start_matches(".log");
    let mut script = String::new();
    script.push_str(
        "gateway_data_root=\"${CLAWPAL_DATA_DIR:-${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}}\"; ",
    );
    script.push_str("log_path=\"\"; ");
    script.push_str("for base in ");
    script.push_str("\"$gateway_data_root\" ");
    script.push_str("\"$gateway_data_root/.openclaw\" ");
    script.push_str("\"$gateway_data_root/.clawpal\" ");
    script.push_str("\"$OPENCLAW_STATE_DIR\" ");
    script.push_str("\"$OPENCLAW_STATE_DIR/.openclaw\" ");
    script.push_str("\"$OPENCLAW_STATE_DIR/.clawpal\" ");
    script.push_str("\"$OPENCLAW_HOME\" ");
    script.push_str("\"$OPENCLAW_HOME/.openclaw\" ");
    script.push_str("\"$OPENCLAW_HOME/.clawpal\" ");
    script.push_str("\"$HOME/.openclaw\" ");
    script.push_str("\"$HOME/.clawpal\"; ");
    script.push_str("do ");
    script.push_str("candidate=\"$base/logs/");
    script.push_str(file);
    script.push_str(".log\"; ");
    script.push_str("[ -f \"$candidate\" ] && log_path=\"$candidate\" && break; ");
    script.push_str("done; ");
    if file == "gateway" {
        script.push_str("[ -n \"$log_path\" ] || [ ! -f \"/tmp/openclaw/gateway-run.log\" ] || log_path=\"/tmp/openclaw/gateway-run.log\"; ");
        script.push_str("if [ -z \"$log_path\" ] && [ -d \"/tmp/openclaw\" ]; then latest_tmp_openclaw=$(ls -1t /tmp/openclaw/openclaw-*.log 2>/dev/null | head -n 1); [ -n \"$latest_tmp_openclaw\" ] && log_path=\"$latest_tmp_openclaw\"; fi; ");
    }
    script.push_str("[ -n \"$log_path\" ] || log_path=\"$gateway_data_root/logs/");
    script.push_str(file);
    script.push_str(".log\"; ");
    script.push_str("tail -");
    script.push_str(&lines.to_string());
    script.push_str(" \"$log_path\" 2>/dev/null || echo ''");
    script
}

pub fn remote_clawpal_log_tail_script(lines: usize, filename: &str) -> String {
    let file = filename.trim_start_matches(".log");
    let mut script = String::new();
    script.push_str(
        "clawpal_data_root=\"${CLAWPAL_DATA_DIR:-${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}}\"; ",
    );
    script.push_str("log_path=\"$clawpal_data_root/.clawpal/logs/");
    script.push_str(file);
    script.push_str(".log\"; ");
    script.push_str("for base in ");
    script.push_str("\"$clawpal_data_root\" ");
    script.push_str("\"$clawpal_data_root/.clawpal\" ");
    script.push_str("\"$OPENCLAW_STATE_DIR\" ");
    script.push_str("\"$OPENCLAW_STATE_DIR/.clawpal\" ");
    script.push_str("\"$OPENCLAW_HOME\" ");
    script.push_str("\"$OPENCLAW_HOME/.clawpal\" ");
    script.push_str("\"$HOME/.openclaw/.clawpal\" ");
    script.push_str("\"$HOME/.clawpal\"; ");
    script.push_str("do ");
    script.push_str("candidate=\"$base/logs/");
    script.push_str(file);
    script.push_str(".log\"; ");
    script.push_str("[ -f \"$candidate\" ] && log_path=\"$candidate\" && break; ");
    script.push_str("done; ");
    script.push_str("tail -");
    script.push_str(&lines.to_string());
    script.push_str(" \"$log_path\" 2>/dev/null || echo ''");
    script
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
    fn resolve_agent_workspace_from_config_prefers_agent_workspace() {
        let doc = json!({
            "agents": {
                "list": [
                    { "id": "main", "workspace": "~/workspace/main" }
                ],
                "defaults": {
                    "workspace": "~/workspace/default"
                }
            }
        });
        let workspace =
            resolve_agent_workspace_from_config(&doc, "main", Some("~/.openclaw/agents"))
                .expect("workspace");
        assert_eq!(workspace, "~/workspace/main");
    }

    #[test]
    fn doctor_json_option_unsupported_matches_unknown_option_errors() {
        assert!(doctor_json_option_unsupported(
            "error: unknown option '--json'",
            ""
        ));
    }

    #[test]
    fn parse_doctor_issues_reads_camel_case_fields() {
        let report = json!({
            "issues": [
                {
                    "id": "primary.test",
                    "code": "primary.test",
                    "severity": "warn",
                    "message": "test issue",
                    "autoFixable": true,
                    "fixHint": "do thing"
                }
            ]
        });
        let issues = parse_doctor_issues(&report, "primary");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, "primary.test");
        assert_eq!(issues[0].severity, "warn");
        assert!(issues[0].auto_fixable);
        assert_eq!(issues[0].fix_hint.as_deref(), Some("do thing"));
    }

    #[test]
    fn normalize_issue_severity_maps_known_levels() {
        assert_eq!(normalize_issue_severity("ERROR"), "error");
        assert_eq!(normalize_issue_severity("warn"), "warn");
        assert_eq!(normalize_issue_severity("notice"), "info");
    }

    #[test]
    fn dedupe_and_classify_doctor_issues() {
        let mut issues = vec![
            DoctorIssue {
                id: "a".into(),
                code: "a".into(),
                severity: "warn".into(),
                message: "warn".into(),
                auto_fixable: false,
                fix_hint: None,
                source: "primary".into(),
            },
            DoctorIssue {
                id: "a".into(),
                code: "a".into(),
                severity: "warn".into(),
                message: "dup".into(),
                auto_fixable: false,
                fix_hint: None,
                source: "primary".into(),
            },
            DoctorIssue {
                id: "b".into(),
                code: "b".into(),
                severity: "error".into(),
                message: "error".into(),
                auto_fixable: false,
                fix_hint: None,
                source: "primary".into(),
            },
        ];
        dedupe_doctor_issues(&mut issues);
        assert_eq!(issues.len(), 2);
        assert_eq!(classify_doctor_issue_status(&issues), "broken");
    }

    #[test]
    fn gateway_output_ok_and_detail_parse_json_payload() {
        let stdout = r#"{"running":true,"healthy":true,"port":18789}"#;
        assert!(gateway_output_ok(0, stdout, ""));
        assert_eq!(
            gateway_output_detail(0, stdout, "").as_deref(),
            Some("running=true, healthy=true, port=18789")
        );
    }

    #[test]
    fn gateway_output_ok_detects_plain_text_down_status() {
        let stdout = "Gateway is not running";
        assert!(!gateway_output_ok(0, stdout, ""));
        assert_eq!(
            gateway_output_detail(0, stdout, "").as_deref(),
            Some("Gateway is not running")
        );
    }

    #[test]
    fn gateway_output_ok_detects_plain_text_config_failure_status() {
        let stdout = "Failed to load openclaw.json: invalid JSON syntax";
        assert!(!gateway_output_ok(0, stdout, ""));
        assert_eq!(
            gateway_output_detail(0, stdout, "").as_deref(),
            Some("Failed to load openclaw.json: invalid JSON syntax")
        );
    }

    #[test]
    fn gateway_output_ok_detects_structured_inactive_gateway_status() {
        let stdout = r#"{
  "service": {
    "runtime": {
      "status": "stopped",
      "state": "inactive",
      "subState": "dead"
    }
  },
  "gateway": {
    "port": 18789
  },
  "port": {
    "status": "free",
    "listeners": []
  },
  "rpc": {
    "ok": false
  }
}"#;
        assert!(!gateway_output_ok(0, stdout, ""));
        assert_eq!(
            gateway_output_detail(0, stdout, "").as_deref(),
            Some("port=18789, state=stopped, rpc=false, port_status=free")
        );
    }

    #[test]
    fn command_output_detail_prefers_stderr_and_trims() {
        let detail = command_output_detail("  first error line\nsecond\n", "ok");
        assert_eq!(detail, "first error line");
    }

    #[test]
    fn collect_repairable_primary_issue_ids_filters_and_dedupes_requested() {
        let issues = vec![
            DoctorIssue {
                id: "field.agents".into(),
                code: "x".into(),
                severity: "warn".into(),
                message: "x".into(),
                auto_fixable: true,
                fix_hint: None,
                source: "primary".into(),
            },
            DoctorIssue {
                id: "field.port".into(),
                code: "x".into(),
                severity: "error".into(),
                message: "x".into(),
                auto_fixable: false,
                fix_hint: None,
                source: "primary".into(),
            },
            DoctorIssue {
                id: "primary.gateway.unhealthy".into(),
                code: "x".into(),
                severity: "error".into(),
                message: "gateway unhealthy".into(),
                auto_fixable: false,
                fix_hint: Some("Restart primary gateway".into()),
                source: "primary".into(),
            },
            DoctorIssue {
                id: "rescue.gateway.unhealthy".into(),
                code: "x".into(),
                severity: "warn".into(),
                message: "x".into(),
                auto_fixable: true,
                fix_hint: None,
                source: "rescue".into(),
            },
        ];
        let (selected, skipped) = collect_repairable_primary_issue_ids(
            &issues,
            &[
                "field.agents".to_string(),
                "primary.gateway.unhealthy".to_string(),
                "field.port".to_string(),
                "field.agents".to_string(),
            ],
        );
        assert_eq!(
            selected,
            vec![
                "primary.gateway.unhealthy".to_string(),
                "field.port".to_string()
            ]
        );
        assert_eq!(skipped, vec!["field.agents".to_string()]);
    }

    #[test]
    fn build_primary_issue_fix_tail_returns_expected_port_command() {
        let (_, args) = build_primary_issue_fix_tail("field.port").expect("port command");
        assert_eq!(
            args,
            vec![
                "config".to_string(),
                "set".to_string(),
                "gateway.port".to_string(),
                "18789".to_string(),
                "--json".to_string()
            ]
        );
    }

    #[test]
    fn gateway_restart_timeout_matches_health_check_timeout() {
        assert!(gateway_restart_timeout(
            "Gateway restart timed out after 60s waiting for health checks.",
            ""
        ));
        assert!(!gateway_restart_timeout(
            "gateway start failed: address already in use",
            ""
        ));
    }

    #[test]
    fn owner_display_parse_error_matches_known_patterns() {
        assert!(owner_display_parse_error(
            "unknown field ownerDisplay while deserialize config"
        ));
        assert!(!owner_display_parse_error("connection refused"));
    }

    #[test]
    fn rescue_cleanup_noop_matches_stop_not_running() {
        let command = vec![
            "--profile".to_string(),
            "rescue".to_string(),
            "gateway".to_string(),
            "stop".to_string(),
        ];
        assert!(rescue_cleanup_noop(
            "deactivate",
            &command,
            1,
            "Gateway is not running",
            ""
        ));
    }

    #[test]
    fn rescue_cleanup_noop_matches_unset_missing_key() {
        let command = vec![
            "--profile".to_string(),
            "rescue".to_string(),
            "config".to_string(),
            "unset".to_string(),
            "gateway.port".to_string(),
        ];
        assert!(rescue_cleanup_noop(
            "unset",
            &command,
            1,
            "config key gateway.port not found",
            ""
        ));
    }

    #[test]
    fn build_rescue_bot_command_plan_for_unset() {
        let commands = build_rescue_bot_command_plan("unset", "rescue", 19789, false);
        assert_eq!(
            commands,
            vec![
                vec!["--profile", "rescue", "gateway", "stop"],
                vec!["--profile", "rescue", "gateway", "uninstall"],
                vec!["--profile", "rescue", "config", "unset", "gateway.port"],
            ]
            .into_iter()
            .map(|items| items.into_iter().map(String::from).collect::<Vec<_>>())
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn build_rescue_bot_command_plan_for_activate_includes_permission_baseline() {
        let commands = build_rescue_bot_command_plan("activate", "rescue", 19789, false);
        let command_strings = commands
            .iter()
            .map(|command| command.join(" "))
            .collect::<Vec<_>>();

        assert!(command_strings
            .iter()
            .any(|command| command.contains("config set tools.profile \"full\" --json")));
        assert!(
            command_strings
                .iter()
                .any(|command| command
                    .contains("config set tools.sessions.visibility \"all\" --json"))
        );
        assert!(command_strings
            .iter()
            .any(|command| command.contains("config set tools.allow [\"*\"] --json")));
        assert!(command_strings
            .iter()
            .any(|command| command.contains("config set tools.exec.host \"gateway\" --json")));
        assert!(command_strings
            .iter()
            .any(|command| command.contains("config set tools.exec.security \"full\" --json")));
        assert!(command_strings
            .iter()
            .any(|command| command.contains("config set tools.exec.ask \"off\" --json")));
    }

    #[test]
    fn command_failure_message_prefers_stderr_then_stdout() {
        let command = vec!["gateway".to_string(), "restart".to_string()];
        let msg = command_failure_message(&command, 1, "boom", "");
        assert!(msg.contains("openclaw gateway restart failed (exit 1): boom"));
    }

    #[test]
    fn is_gateway_restart_command_matches_tail_tokens() {
        assert!(is_gateway_restart_command(&[
            "--profile".to_string(),
            "main".to_string(),
            "gateway".to_string(),
            "restart".to_string()
        ]));
        assert!(!is_gateway_restart_command(&[
            "gateway".to_string(),
            "status".to_string()
        ]));
    }

    #[test]
    fn suggest_rescue_port_prefers_large_gap() {
        assert_eq!(suggest_rescue_port(18789), 19789);
    }

    #[test]
    fn ensure_rescue_port_spacing_rejects_small_gap() {
        let err = ensure_rescue_port_spacing(18789, 18800).expect_err("must fail");
        assert!(err.contains("too close"));
    }

    #[test]
    fn parse_rescue_port_value_supports_number_and_string() {
        assert_eq!(
            parse_rescue_port_value(&serde_json::json!(19789)),
            Some(19789)
        );
        assert_eq!(
            parse_rescue_port_value(&serde_json::json!("19789")),
            Some(19789)
        );
        assert_eq!(parse_rescue_port_value(&serde_json::json!(true)), None);
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
    fn remote_gateway_log_tail_script_supports_generic_log_file() {
        let script = remote_gateway_log_tail_script(77, "gateway");
        assert!(script.contains("tail -77"));
        assert!(script.contains("gateway.log"));
        assert!(script.contains("/tmp/openclaw/gateway-run.log"));
        assert!(script.contains("/tmp/openclaw/openclaw-*.log"));
        assert!(script.contains("OPENCLAW_STATE_DIR"));
        assert!(script.contains("\"$gateway_data_root/.openclaw\""));
        assert!(script.contains("for base in"));
    }

    #[test]
    fn remote_clawpal_log_tail_script_uses_clawpal_data_root() {
        let script = remote_clawpal_log_tail_script(64, "app");
        assert!(script.contains("tail -64"));
        assert!(script.contains("CLAWPAL_DATA_DIR"));
        assert!(script.contains("OPENCLAW_STATE_DIR"));
        assert!(script.contains(".clawpal/logs/app.log"));
        assert!(script.contains("/.clawpal/logs/app.log"));
        assert!(script.contains("for base in"));
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
    fn extract_json_from_output_uses_trailing_balanced_payload() {
        let raw =
            "[plugins] warmup cache\n[warn] using fallback transport\n{\"ok\":false,\"issues\":[{\"id\":\"x\"}]}";
        let json = extract_json_from_output(raw).expect("extract");
        assert_eq!(json, "{\"ok\":false,\"issues\":[{\"id\":\"x\"}]}");
    }

    #[test]
    fn parse_json_loose_handles_leading_bracketed_logs() {
        let raw =
            "[plugins] warmup cache\n[warn] using fallback transport\n{\"running\":false,\"healthy\":false}";
        let parsed = parse_json_loose(raw).expect("expected trailing JSON payload");
        assert_eq!(parsed.get("running").and_then(Value::as_bool), Some(false));
        assert_eq!(parsed.get("healthy").and_then(Value::as_bool), Some(false));
    }

    #[test]
    fn strip_doctor_banner_removes_warning_box_lines() {
        let input = "╭─ Doctor warnings ─╮\n│ noisy │\n╰────────╯\nreal error";
        assert_eq!(strip_doctor_banner(input), "real error");
    }

    #[test]
    fn parse_json5_document_accepts_trailing_commas() {
        let value = parse_json5_document("{a:1,}", "config").expect("json5 parse");
        assert_eq!(value.get("a").and_then(Value::as_i64), Some(1));
    }

    #[test]
    fn render_json_document_pretty_prints() {
        let text = render_json_document(&json!({"a":1}), "config").expect("render");
        assert!(text.contains('\n'));
    }

    #[test]
    fn apply_issue_fixes_updates_expected_paths() {
        let mut doc = json!({});
        let ids = vec![
            "field.agents".to_string(),
            "field.port".to_string(),
            "unknown.issue".to_string(),
        ];
        let applied = apply_issue_fixes(&mut doc, &ids).expect("apply fixes");
        assert_eq!(
            applied,
            vec!["field.agents".to_string(), "field.port".to_string()]
        );
        assert_eq!(
            json_path_get(&doc, "agents.defaults.model")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "anthropic/claude-sonnet-4-5"
        );
        assert_eq!(
            json_path_get(&doc, "gateway.port")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            18789
        );
    }
}
