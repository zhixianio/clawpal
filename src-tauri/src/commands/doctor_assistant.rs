use super::*;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

const DOCTOR_ASSISTANT_TARGET_PROFILE: &str = "primary";
const DOCTOR_ASSISTANT_TEMP_SCOPE_LOCAL: &str = "local";
const DOCTOR_ASSISTANT_TEMP_REPAIR_ROUNDS: usize = 2;
const DOCTOR_ASSISTANT_TEMP_PROFILE_PREFIX: &str = "clawpal-doctor-";
const DOCTOR_ASSISTANT_TEMP_MARKER_FILE: &str = ".clawpal-doctor-temp";
const DOCTOR_ASSISTANT_TEMP_PROVIDER_SETUP_REQUIRED_PREFIX: &str =
    "__doctor_assistant_temp_provider_setup_required__:";
const DOCTOR_ASSISTANT_REMOTE_SKIP_AGENT_REPAIR: bool = false;
const DOCTOR_ASSISTANT_REMOTE_TIMEOUT_RECOVERY_ATTEMPTS: usize = 8;
const DOCTOR_ASSISTANT_REMOTE_TIMEOUT_RECOVERY_DELAY_MS: u64 = 3_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DoctorAssistantProgressEvent {
    run_id: String,
    phase: String,
    line: String,
    progress: f32,
    attempt: usize,
    resolved_issue_id: Option<String>,
    resolved_issue_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DoctorTempGatewaySessionRecord {
    instance_id: String,
    profile: String,
    port: u16,
    created_at: String,
    status: String,
    main_profile: String,
    main_port: u16,
    last_step: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DoctorTempGatewaySessionStore {
    sessions: Vec<DoctorTempGatewaySessionRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteAuthStoreCandidate {
    provider: String,
    auth_ref: String,
    credential: InternalProviderCredential,
}

#[derive(Debug, Clone)]
struct LocalDonorConfigLoad {
    main_config_path: String,
    donor_cfg: serde_json::Value,
    source_mode: &'static str,
    defaults_source_path: Option<String>,
}

#[derive(Debug, Clone)]
struct RemoteDonorConfigLoad {
    main_config_path: String,
    donor_cfg: serde_json::Value,
    source_mode: &'static str,
    defaults_source_path: Option<String>,
}

fn emit_doctor_assistant_progress(
    app: &AppHandle,
    run_id: &str,
    phase: &str,
    line: impl Into<String>,
    progress: f32,
    attempt: usize,
    resolved_issue_id: Option<String>,
    resolved_issue_label: Option<String>,
) {
    let payload = DoctorAssistantProgressEvent {
        run_id: run_id.to_string(),
        phase: phase.to_string(),
        line: line.into(),
        progress: progress.clamp(0.0, 1.0),
        attempt,
        resolved_issue_id,
        resolved_issue_label,
    };
    let _ = app.emit("doctor:assistant-progress", payload);
}

fn doctor_temp_gateway_store_path(paths: &crate::models::OpenClawPaths) -> std::path::PathBuf {
    paths.clawpal_dir.join("doctor-temp-gateways.json")
}

fn load_doctor_temp_gateway_store(
    paths: &crate::models::OpenClawPaths,
) -> DoctorTempGatewaySessionStore {
    crate::config_io::read_json(&doctor_temp_gateway_store_path(paths)).unwrap_or_default()
}

fn save_doctor_temp_gateway_store(
    paths: &crate::models::OpenClawPaths,
    store: &DoctorTempGatewaySessionStore,
) -> Result<(), String> {
    let path = doctor_temp_gateway_store_path(paths);
    if store.sessions.is_empty() {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    } else {
        crate::config_io::write_json(&path, store)
    }
}

fn upsert_doctor_temp_gateway_record(
    paths: &crate::models::OpenClawPaths,
    record: DoctorTempGatewaySessionRecord,
) -> Result<(), String> {
    let mut store = load_doctor_temp_gateway_store(paths);
    store
        .sessions
        .retain(|item| !(item.instance_id == record.instance_id && item.profile == record.profile));
    store.sessions.push(record);
    save_doctor_temp_gateway_store(paths, &store)
}

fn remove_doctor_temp_gateway_record(
    paths: &crate::models::OpenClawPaths,
    instance_id: &str,
    profile: &str,
) -> Result<(), String> {
    let mut store = load_doctor_temp_gateway_store(paths);
    store
        .sessions
        .retain(|item| !(item.instance_id == instance_id && item.profile == profile));
    save_doctor_temp_gateway_store(paths, &store)
}

fn remove_doctor_temp_gateway_records_for_instance(
    paths: &crate::models::OpenClawPaths,
    instance_id: &str,
) -> Result<(), String> {
    let mut store = load_doctor_temp_gateway_store(paths);
    store
        .sessions
        .retain(|item| item.instance_id != instance_id);
    save_doctor_temp_gateway_store(paths, &store)
}

fn doctor_assistant_issue_label(issue: &RescuePrimaryIssue) -> String {
    let text = issue.message.trim();
    if text.is_empty() {
        issue.id.clone()
    } else {
        text.to_string()
    }
}

fn collect_resolved_issues(
    before: &RescuePrimaryDiagnosisResult,
    after: &RescuePrimaryDiagnosisResult,
) -> Vec<(String, String)> {
    let remaining = after
        .issues
        .iter()
        .map(|issue| issue.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    before
        .issues
        .iter()
        .filter(|issue| !remaining.contains(issue.id.as_str()))
        .map(|issue| (issue.id.clone(), doctor_assistant_issue_label(issue)))
        .collect()
}

fn doctor_assistant_completed_result(
    attempted_at: String,
    rescue_profile: String,
    selected_issue_ids: Vec<String>,
    applied_issue_ids: Vec<String>,
    skipped_issue_ids: Vec<String>,
    failed_issue_ids: Vec<String>,
    steps: Vec<RescuePrimaryRepairStep>,
    before: RescuePrimaryDiagnosisResult,
    after: RescuePrimaryDiagnosisResult,
) -> RescuePrimaryRepairResult {
    RescuePrimaryRepairResult {
        status: "completed".into(),
        attempted_at,
        target_profile: DOCTOR_ASSISTANT_TARGET_PROFILE.into(),
        rescue_profile,
        selected_issue_ids,
        applied_issue_ids,
        skipped_issue_ids,
        failed_issue_ids,
        pending_action: None,
        steps,
        before,
        after,
    }
}

fn doctor_assistant_pending_temp_provider_result(
    attempted_at: String,
    rescue_profile: String,
    selected_issue_ids: Vec<String>,
    applied_issue_ids: Vec<String>,
    skipped_issue_ids: Vec<String>,
    failed_issue_ids: Vec<String>,
    steps: Vec<RescuePrimaryRepairStep>,
    before: RescuePrimaryDiagnosisResult,
    after: RescuePrimaryDiagnosisResult,
    temp_provider_profile_id: Option<String>,
    reason: String,
) -> RescuePrimaryRepairResult {
    RescuePrimaryRepairResult {
        status: "needsTempProviderSetup".into(),
        attempted_at,
        target_profile: DOCTOR_ASSISTANT_TARGET_PROFILE.into(),
        rescue_profile,
        selected_issue_ids,
        applied_issue_ids,
        skipped_issue_ids,
        failed_issue_ids,
        pending_action: Some(RescuePrimaryPendingAction {
            kind: "tempProviderSetup".into(),
            reason,
            temp_provider_profile_id,
        }),
        steps,
        before,
        after,
    }
}

fn doctor_assistant_temp_provider_setup_required(reason: impl Into<String>) -> String {
    format!(
        "{}{}",
        DOCTOR_ASSISTANT_TEMP_PROVIDER_SETUP_REQUIRED_PREFIX,
        reason.into()
    )
}

fn doctor_assistant_extract_temp_provider_setup_reason(error: &str) -> Option<String> {
    error
        .strip_prefix(DOCTOR_ASSISTANT_TEMP_PROVIDER_SETUP_REQUIRED_PREFIX)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn doctor_assistant_is_cleanup_noop(
    action: RescueBotAction,
    command: &[String],
    output: &OpenclawCommandOutput,
) -> bool {
    clawpal_core::doctor::rescue_cleanup_noop(
        action.as_str(),
        command,
        output.exit_code,
        &output.stderr,
        &output.stdout,
    )
}

fn default_main_profile_root(profile: &str) -> String {
    let trimmed = profile.trim();
    if trimmed.is_empty() || trimmed == DOCTOR_ASSISTANT_TARGET_PROFILE {
        "~/.openclaw".into()
    } else {
        format!("~/.openclaw-{trimmed}")
    }
}

fn default_main_config_path(profile: &str) -> String {
    format!("{}/openclaw.json", default_main_profile_root(profile))
}

fn expand_remote_home_path(path: &str, home_dir: &str) -> String {
    let trimmed_home = home_dir.trim().trim_end_matches('/');
    if let Some(rest) = path.strip_prefix("~/") {
        format!("{trimmed_home}/{rest}")
    } else if path.trim() == "~" {
        trimmed_home.to_string()
    } else {
        path.to_string()
    }
}

fn truncate_for_prompt(raw: &str, max_chars: usize) -> String {
    let trimmed = raw.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    out.push_str("\n...[truncated]...");
    out
}

fn format_agent_repair_guidance(
    guidance: Option<&crate::openclaw_doc_resolver::DocGuidance>,
) -> String {
    let Some(guidance) = guidance else {
        return "No additional doc guidance was resolved.".into();
    };
    let hypotheses = if guidance.root_cause_hypotheses.is_empty() {
        "None".into()
    } else {
        guidance
            .root_cause_hypotheses
            .iter()
            .map(|item| format!("- {}: {}", item.title, item.reason))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let fix_steps = if guidance.fix_steps.is_empty() {
        "None".into()
    } else {
        guidance
            .fix_steps
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let citations = if guidance.citations.is_empty() {
        "None".into()
    } else {
        guidance
            .citations
            .iter()
            .map(|item| format!("- {} ({})", item.section, item.url))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "Resolver strategy: {}\nConfidence: {:.2}\nRoot cause hypotheses:\n{}\nSuggested fix steps:\n{}\nDoc citations:\n{}",
        guidance.source_strategy, guidance.confidence, hypotheses, fix_steps, citations
    )
}

fn format_diagnosis_for_agent(diagnosis: &RescuePrimaryDiagnosisResult) -> String {
    let mut lines = vec![
        format!("Summary status: {}", diagnosis.summary.status),
        format!("Headline: {}", diagnosis.summary.headline),
        format!(
            "Recommended action: {}",
            diagnosis.summary.recommended_action
        ),
    ];
    if diagnosis.issues.is_empty() {
        lines.push("Issues: none".into());
    } else {
        lines.push("Issues:".into());
        lines.extend(
            diagnosis
                .issues
                .iter()
                .map(|issue| format!("- [{}] {} :: {}", issue.severity, issue.id, issue.message)),
        );
    }
    lines.join("\n")
}

fn build_temp_gateway_agent_repair_prompt(
    log_excerpt: &str,
    config_content: &str,
    diagnosis: &RescuePrimaryDiagnosisResult,
    guidance: Option<&crate::openclaw_doc_resolver::DocGuidance>,
) -> String {
    format!(
        concat!(
            "You are running inside a temporary OpenClaw profile created by ClawPal to repair the primary gateway.\n",
            "Goal: repair the PRIMARY gateway configuration/files so the primary gateway becomes healthy again.\n",
            "Primary profile: {primary_profile}\n",
            "Constraints:\n",
            "- Operate on the primary gateway config/auth files, not the temporary profile.\n",
            "- Read the supplied logs first and use them as the primary evidence source.\n",
            "- Use the supplied docs guidance as supporting context.\n",
            "- Prefer the smallest targeted edit that restores startup and health.\n",
            "- If config syntax is broken, repair syntax first before semantic changes.\n",
            "- After edits, restart or re-check the primary gateway as needed.\n",
            "- Do not leave unrelated changes behind.\n\n",
            "Current diagnosis:\n{diagnosis}\n\n",
            "Relevant gateway logs from /tmp/openclaw/*:\n{logs}\n\n",
            "Current primary config content:\n{config}\n\n",
            "Docs guidance:\n{guidance}\n"
        ),
        primary_profile = DOCTOR_ASSISTANT_TARGET_PROFILE,
        diagnosis = truncate_for_prompt(&format_diagnosis_for_agent(diagnosis), 8_000),
        logs = truncate_for_prompt(log_excerpt, 16_000),
        config = truncate_for_prompt(config_content, 20_000),
        guidance = truncate_for_prompt(&format_agent_repair_guidance(guidance), 8_000),
    )
}

fn collect_local_gateway_log_excerpt() -> String {
    let log_dir = std::path::Path::new("/tmp/openclaw");
    if !log_dir.exists() {
        return "No gateway logs found under /tmp/openclaw".into();
    }

    let mut targets = Vec::<std::path::PathBuf>::new();
    let gateway_run = log_dir.join("gateway-run.log");
    if gateway_run.exists() {
        targets.push(gateway_run);
    }

    let mut openclaw_logs = std::fs::read_dir(log_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|value| value.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("openclaw-") && name.ends_with(".log"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    openclaw_logs.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
    targets.extend(openclaw_logs.into_iter().take(3));

    let mut sections = Vec::new();
    for path in targets {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let tail = text
            .lines()
            .rev()
            .take(120)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("==> {} <==\n{}", path.display(), tail));
    }

    if sections.is_empty() {
        "No gateway logs found under /tmp/openclaw".into()
    } else {
        sections.join("\n\n")
    }
}

async fn collect_remote_gateway_log_excerpt(pool: &SshConnectionPool, host_id: &str) -> String {
    let command = r#"files=""
if [ -f /tmp/openclaw/gateway-run.log ]; then files="/tmp/openclaw/gateway-run.log"; fi
latest=$(ls -t /tmp/openclaw/openclaw-*.log 2>/dev/null | head -3 || true)
for f in $latest; do files="$files $f"; done
if [ -z "$files" ]; then
  echo "No gateway logs found under /tmp/openclaw"
  exit 0
fi
for f in $files; do
  [ -f "$f" ] || continue
  echo "==> $f <=="
  tail -120 "$f" 2>/dev/null || true
  echo
done"#;
    match pool.exec_login(host_id, command).await {
        Ok(output) if !output.stdout.trim().is_empty() => output.stdout,
        Ok(_) => "No gateway logs found under /tmp/openclaw".into(),
        Err(error) => format!("Failed to read /tmp/openclaw logs: {error}"),
    }
}

fn read_local_primary_config_text(target_profile: &str) -> String {
    let paths = resolve_paths();
    let path = if target_profile == DOCTOR_ASSISTANT_TARGET_PROFILE {
        paths.config_path
    } else {
        derive_profile_root_path(&paths.openclaw_dir, target_profile).join("openclaw.json")
    };
    std::fs::read_to_string(path).unwrap_or_default()
}

async fn read_remote_primary_config_text(
    pool: &SshConnectionPool,
    host_id: &str,
    target_profile: &str,
) -> String {
    let home_dir = pool
        .get_home_dir(host_id)
        .await
        .unwrap_or_else(|_| "/root".into());
    let config_path = expand_remote_home_path(&default_main_config_path(target_profile), &home_dir);
    let command = format!(
        "config_path=\"{}\"; test -f \"$config_path\" && cat \"$config_path\" || true",
        config_path.replace('"', "\\\""),
    );
    pool.exec_login(host_id, &command)
        .await
        .map(|output| output.stdout)
        .unwrap_or_default()
}

fn skip_json5_ws_and_comments(text: &str, mut index: usize) -> usize {
    let bytes = text.as_bytes();
    while index < bytes.len() {
        match bytes[index] {
            b' ' | b'\t' | b'\r' | b'\n' => {
                index += 1;
            }
            b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'/' => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'*' => {
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                if index + 1 < bytes.len() {
                    index += 2;
                }
            }
            _ => break,
        }
    }
    index
}

fn scan_json5_string_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let quote = *bytes.get(start)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let mut index = start + 1;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == quote {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

fn scan_json5_value_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let start = skip_json5_ws_and_comments(text, start);
    let first = *bytes.get(start)?;
    if first == b'"' || first == b'\'' {
        return scan_json5_string_end(text, start);
    }
    if first != b'{' && first != b'[' {
        let mut index = start;
        while index < bytes.len() {
            index = skip_json5_ws_and_comments(text, index);
            if index >= bytes.len() {
                break;
            }
            match bytes[index] {
                b',' | b'}' => break,
                b'"' | b'\'' => {
                    index = scan_json5_string_end(text, index)?;
                }
                _ => index += 1,
            }
        }
        return Some(index);
    }

    let mut stack = vec![first];
    let mut index = start + 1;
    while index < bytes.len() {
        index = skip_json5_ws_and_comments(text, index);
        if index >= bytes.len() {
            break;
        }
        match bytes[index] {
            b'"' | b'\'' => {
                index = scan_json5_string_end(text, index)?;
            }
            b'{' | b'[' => {
                stack.push(bytes[index]);
                index += 1;
            }
            b'}' => {
                let open = stack.pop()?;
                if open != b'{' {
                    return None;
                }
                index += 1;
                if stack.is_empty() {
                    return Some(index);
                }
            }
            b']' => {
                let open = stack.pop()?;
                if open != b'[' {
                    return None;
                }
                index += 1;
                if stack.is_empty() {
                    return Some(index);
                }
            }
            _ => index += 1,
        }
    }
    None
}

fn extract_json5_top_level_value(text: &str, key: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        index = skip_json5_ws_and_comments(text, index);
        if index >= bytes.len() {
            break;
        }
        match bytes[index] {
            b'{' => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            b'"' | b'\'' if depth == 1 => {
                let end = scan_json5_string_end(text, index)?;
                let raw_key = &text[index + 1..end - 1];
                let after_key = skip_json5_ws_and_comments(text, end);
                if raw_key == key && bytes.get(after_key) == Some(&b':') {
                    let value_start = skip_json5_ws_and_comments(text, after_key + 1);
                    let value_end = scan_json5_value_end(text, value_start)?;
                    return Some(text[value_start..value_end].trim().to_string());
                }
                index = end;
            }
            b'"' | b'\'' => {
                index = scan_json5_string_end(text, index)?;
            }
            _ => index += 1,
        }
    }
    None
}

fn salvage_donor_cfg_from_text(text: &str) -> serde_json::Value {
    let mut root = serde_json::Map::new();
    for key in ["secrets", "auth", "models", "agents"] {
        let Some(raw_value) = extract_json5_top_level_value(text, key) else {
            continue;
        };
        let Ok(value) = json5::from_str::<serde_json::Value>(&raw_value) else {
            continue;
        };
        root.insert(key.to_string(), value);
    }
    serde_json::Value::Object(root)
}

#[cfg(test)]
fn overlay_agent_defaults_from_first_valid_json(
    root: &mut serde_json::Value,
    candidate_texts: &[String],
) -> bool {
    for text in candidate_texts {
        let Ok(candidate) = serde_json::from_str::<serde_json::Value>(text) else {
            continue;
        };
        let mut applied = false;
        for (source, dest) in [
            ("/agents/defaults/model", ["agents", "defaults", "model"]),
            ("/agents/default/model", ["agents", "defaults", "model"]),
            ("/agents/defaults/models", ["agents", "defaults", "models"]),
            ("/agents/default/models", ["agents", "defaults", "models"]),
        ] {
            if let Some(value) = candidate.pointer(source).cloned() {
                set_json_object_path(root, &dest, value);
                applied = true;
            }
        }
        if applied {
            return true;
        }
    }
    false
}

fn overlay_agent_defaults_from_named_candidates(
    root: &mut serde_json::Value,
    candidates: &[(String, String)],
) -> Option<String> {
    for (path, text) in candidates {
        let Ok(candidate) = serde_json::from_str::<serde_json::Value>(text) else {
            continue;
        };
        let mut applied = false;
        for (source, dest) in [
            ("/models/providers", &["models", "providers"][..]),
            ("/auth/profiles", &["auth", "profiles"][..]),
            (
                "/agents/defaults/model",
                &["agents", "defaults", "model"][..],
            ),
            (
                "/agents/default/model",
                &["agents", "defaults", "model"][..],
            ),
            (
                "/agents/defaults/models",
                &["agents", "defaults", "models"][..],
            ),
            (
                "/agents/default/models",
                &["agents", "defaults", "models"][..],
            ),
        ] {
            if let Some(value) = candidate.pointer(source).cloned() {
                set_json_object_path(root, dest, value);
                applied = true;
            }
        }
        if applied {
            return Some(path.clone());
        }
    }
    None
}

fn rank_openclaw_json_candidate(name: &str) -> Option<(u8, u32, String)> {
    if name == "openclaw.json" {
        Some((0, 0, name.to_string()))
    } else if name == "openclaw.json.bak" {
        Some((1, 0, name.to_string()))
    } else if let Some(rest) = name.strip_prefix("openclaw.json.bak.") {
        Some((2, rest.parse::<u32>().ok()?, name.to_string()))
    } else if let Some(rest) = name.strip_prefix("openclaw.bak.") {
        Some((3, rest.parse::<u32>().ok()?, name.to_string()))
    } else if name.starts_with("openclaw.json.pre-restore.") && name.ends_with(".bak") {
        Some((4, 0, name.to_string()))
    } else {
        None
    }
}

fn ordered_openclaw_json_candidate_names<I>(names: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut ranked = names
        .into_iter()
        .filter_map(|name| rank_openclaw_json_candidate(&name))
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    ranked.into_iter().map(|(_, _, name)| name).collect()
}

fn read_local_openclaw_json_candidates(root: &std::path::Path) -> Vec<(String, String)> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let names = ordered_openclaw_json_candidate_names(entries.filter_map(|entry| {
        entry
            .ok()
            .and_then(|item| {
                item.file_type()
                    .ok()
                    .filter(|ft| ft.is_file())
                    .map(|_| item)
            })
            .and_then(|item| item.file_name().into_string().ok())
    }));
    names
        .into_iter()
        .filter_map(|name| {
            let path = root.join(&name);
            let text = std::fs::read_to_string(&path).ok()?;
            Some((path.to_string_lossy().to_string(), text))
        })
        .collect()
}

async fn read_remote_openclaw_json_candidates(
    pool: &SshConnectionPool,
    host_id: &str,
    root: &str,
) -> Vec<(String, String)> {
    let entries = match pool.sftp_list(host_id, root).await {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let names = ordered_openclaw_json_candidate_names(
        entries
            .into_iter()
            .filter(|entry| !entry.is_dir)
            .map(|entry| entry.name),
    );
    let mut out = Vec::new();
    for name in names {
        let path = format!("{}/{}", root.trim_end_matches('/'), name);
        if let Ok(text) = pool.sftp_read(host_id, &path).await {
            out.push((path, text));
        }
    }
    out
}

fn collect_provider_keys(donor_cfg: &serde_json::Value) -> Vec<String> {
    donor_cfg
        .pointer("/models/providers")
        .and_then(serde_json::Value::as_object)
        .map(|providers| {
            let mut keys = providers.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            keys
        })
        .unwrap_or_default()
}

fn doctor_failure_backup_name() -> String {
    format!(
        "clawpal-doctor-failure-{}-{}",
        unix_timestamp_secs(),
        Uuid::new_v4().simple(),
    )
}

fn first_valid_backup_candidate(
    candidates: &[(String, String)],
    main_config_path: &str,
) -> Option<(String, String)> {
    candidates.iter().find_map(|(path, text)| {
        if path == main_config_path {
            return None;
        }
        serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .map(|_| (path.clone(), text.clone()))
    })
}

fn summarize_config_validate_output(output: &OpenclawCommandOutput) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(&output.stdout).ok()?;
    let issues = value.pointer("/issues")?.as_array()?;
    let lines = issues
        .iter()
        .filter_map(|issue| {
            let path = issue.pointer("/path").and_then(serde_json::Value::as_str)?;
            let message = issue
                .pointer("/message")
                .and_then(serde_json::Value::as_str)?;
            Some(format!("{}: {}", path, message))
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join(" | "))
    }
}

fn local_primary_config_validation_detail() -> Option<String> {
    let command = build_profile_command(
        DOCTOR_ASSISTANT_TARGET_PROFILE,
        &["config", "validate", "--json"],
    );
    let output = run_openclaw_dynamic(&command).ok()?;
    summarize_config_validate_output(&output)
}

async fn remote_primary_config_validation_detail(
    pool: &SshConnectionPool,
    host_id: &str,
) -> Option<String> {
    let command = build_profile_command(
        DOCTOR_ASSISTANT_TARGET_PROFILE,
        &["config", "validate", "--json"],
    );
    let output = run_remote_openclaw_dynamic(pool, host_id, command)
        .await
        .ok()?;
    summarize_config_validate_output(&output)
}

fn write_local_doctor_failure_artifacts(
    failure_reason: &str,
    selected_backup_path: Option<&str>,
    parse_error: Option<&str>,
) -> Result<(std::path::PathBuf, String), String> {
    let paths = resolve_paths();
    let failure_name = doctor_failure_backup_name();
    let failure_dir = paths
        .clawpal_dir
        .join("doctor-failures")
        .join(&failure_name);
    std::fs::create_dir_all(&failure_dir).map_err(|error| error.to_string())?;
    let damaged = read_local_primary_config_text(DOCTOR_ASSISTANT_TARGET_PROFILE);
    std::fs::write(failure_dir.join("openclaw.json.damaged"), damaged)
        .map_err(|error| error.to_string())?;
    let metadata = serde_json::json!({
        "transport": "local",
        "configPath": paths.config_path,
        "selectedBackupPath": selected_backup_path,
        "parseError": parse_error,
        "failureReason": failure_reason,
        "createdAt": format_timestamp_from_unix(unix_timestamp_secs()),
    });
    std::fs::write(
        failure_dir.join("metadata.json"),
        serde_json::to_string_pretty(&metadata).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok((failure_dir, failure_name))
}

async fn write_remote_doctor_failure_artifacts(
    pool: &SshConnectionPool,
    host_id: &str,
    failure_reason: &str,
    selected_backup_path: Option<&str>,
    parse_error: Option<&str>,
) -> Result<(String, String), String> {
    let home_dir = pool
        .get_home_dir(host_id)
        .await
        .unwrap_or_else(|_| "/root".into());
    let failure_name = doctor_failure_backup_name();
    let failure_dir = format!(
        "{}/.clawpal/doctor-failures/{}",
        home_dir.trim_end_matches('/'),
        failure_name
    );
    let mkdir_cmd = format!("mkdir -p {}", shell_escape(&failure_dir));
    let mkdir_result = pool.exec_login(host_id, &mkdir_cmd).await?;
    if mkdir_result.exit_code != 0 {
        return Err(format!(
            "Failed to create remote failure dir: {}",
            mkdir_result.stderr
        ));
    }
    let damaged_path = format!("{}/openclaw.json.damaged", failure_dir);
    pool.sftp_write(
        host_id,
        &damaged_path,
        &read_remote_primary_config_text(pool, host_id, DOCTOR_ASSISTANT_TARGET_PROFILE).await,
    )
    .await?;
    let metadata = serde_json::json!({
        "transport": "remote_ssh",
        "configPath": expand_remote_home_path(&default_main_config_path(DOCTOR_ASSISTANT_TARGET_PROFILE), &home_dir),
        "selectedBackupPath": selected_backup_path,
        "parseError": parse_error,
        "failureReason": failure_reason,
        "createdAt": format_timestamp_from_unix(unix_timestamp_secs()),
        "hostId": host_id,
    });
    let metadata_path = format!("{}/metadata.json", failure_dir);
    pool.sftp_write(
        host_id,
        &metadata_path,
        &serde_json::to_string_pretty(&metadata).map_err(|error| error.to_string())?,
    )
    .await?;
    Ok((failure_dir, failure_name))
}

fn fallback_restore_local_primary_config(
    app: &AppHandle,
    run_id: &str,
    steps: &mut Vec<RescuePrimaryRepairStep>,
    failure_reason: &str,
) -> Result<Option<RescuePrimaryDiagnosisResult>, String> {
    let paths = resolve_paths();
    let parse_error = local_primary_config_validation_detail();
    emit_doctor_assistant_progress(
        app,
        run_id,
        "cleanup",
        "Saving damaged config before fallback restore",
        0.95,
        0,
        None,
        None,
    );
    let (failure_dir, _) =
        write_local_doctor_failure_artifacts(failure_reason, None, parse_error.as_deref())?;
    append_step(
        steps,
        "repair.fallback.backup_damaged_config",
        "Backup damaged primary config",
        true,
        format!("Saved damaged config to {}", failure_dir.display()),
        None,
    );
    let candidates = read_local_openclaw_json_candidates(&paths.openclaw_dir);
    let main_config_path = paths.config_path.to_string_lossy().to_string();
    let Some((backup_path, backup_text)) =
        first_valid_backup_candidate(&candidates, &main_config_path)
    else {
        append_step(
            steps,
            "repair.fallback.select_valid_backup",
            "Select valid backup config",
            false,
            "No alternate valid OpenClaw backup config was found",
            None,
        );
        return Ok(None);
    };
    append_step(
        steps,
        "repair.fallback.select_valid_backup",
        "Select valid backup config",
        true,
        format!("Selected {} as fallback restore source", backup_path),
        None,
    );
    let metadata = serde_json::json!({
        "transport": "local",
        "configPath": paths.config_path,
        "selectedBackupPath": backup_path,
        "parseError": parse_error,
        "failureReason": failure_reason,
        "createdAt": format_timestamp_from_unix(unix_timestamp_secs()),
    });
    std::fs::write(
        failure_dir.join("metadata.json"),
        serde_json::to_string_pretty(&metadata).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    emit_doctor_assistant_progress(
        app,
        run_id,
        "cleanup",
        "Restoring first valid backup into primary config",
        0.965,
        0,
        None,
        None,
    );
    std::fs::write(&paths.config_path, backup_text).map_err(|error| error.to_string())?;
    append_step(
        steps,
        "repair.fallback.restore_primary_config",
        "Restore primary config from fallback backup",
        true,
        format!(
            "Restored {} into {}",
            backup_path,
            paths.config_path.display()
        ),
        None,
    );
    emit_doctor_assistant_progress(
        app,
        run_id,
        "cleanup",
        "Restarting gateway from recovered config",
        0.98,
        0,
        None,
        None,
    );
    let mut commands = Vec::new();
    super::run_local_gateway_restart_fallback(DOCTOR_ASSISTANT_TARGET_PROFILE, &mut commands)?;
    append_step(
        steps,
        "repair.fallback.restart_gateway",
        "Restart primary gateway",
        true,
        format!(
            "Restarted primary gateway using {} command(s)",
            commands.len()
        ),
        None,
    );
    let after = diagnose_doctor_assistant_local_impl(app, run_id, DOCTOR_ASSISTANT_TARGET_PROFILE)?;
    append_step(
        steps,
        "repair.fallback.recheck",
        "Re-check primary gateway after fallback restore",
        diagnose_doctor_assistant_status(&after),
        if diagnose_doctor_assistant_status(&after) {
            "Primary gateway recovered after fallback restore".to_string()
        } else {
            "Primary gateway remained unhealthy after fallback restore".to_string()
        },
        None,
    );
    Ok(Some(after))
}

async fn fallback_restore_remote_primary_config(
    pool: &SshConnectionPool,
    host_id: &str,
    app: &AppHandle,
    run_id: &str,
    steps: &mut Vec<RescuePrimaryRepairStep>,
    failure_reason: &str,
) -> Result<Option<RescuePrimaryDiagnosisResult>, String> {
    let home_dir = pool
        .get_home_dir(host_id)
        .await
        .unwrap_or_else(|_| "/root".into());
    let main_config_path = expand_remote_home_path(
        &default_main_config_path(DOCTOR_ASSISTANT_TARGET_PROFILE),
        &home_dir,
    );
    let parse_error = remote_primary_config_validation_detail(pool, host_id).await;
    emit_doctor_assistant_progress(
        app,
        run_id,
        "cleanup",
        "Saving damaged config before fallback restore",
        0.95,
        0,
        None,
        None,
    );
    let (failure_dir, _) = write_remote_doctor_failure_artifacts(
        pool,
        host_id,
        failure_reason,
        None,
        parse_error.as_deref(),
    )
    .await?;
    append_step(
        steps,
        "repair.fallback.backup_damaged_config",
        "Backup damaged primary config",
        true,
        format!("Saved damaged config to {}", failure_dir),
        None,
    );
    let main_root = resolve_remote_main_root(pool, host_id).await;
    let candidates = read_remote_openclaw_json_candidates(pool, host_id, &main_root).await;
    let Some((backup_path, backup_text)) =
        first_valid_backup_candidate(&candidates, &main_config_path)
    else {
        append_step(
            steps,
            "repair.fallback.select_valid_backup",
            "Select valid backup config",
            false,
            "No alternate valid OpenClaw backup config was found",
            None,
        );
        return Ok(None);
    };
    append_step(
        steps,
        "repair.fallback.select_valid_backup",
        "Select valid backup config",
        true,
        format!("Selected {} as fallback restore source", backup_path),
        None,
    );
    let metadata = serde_json::json!({
        "transport": "remote_ssh",
        "configPath": main_config_path,
        "selectedBackupPath": backup_path,
        "parseError": parse_error,
        "failureReason": failure_reason,
        "createdAt": format_timestamp_from_unix(unix_timestamp_secs()),
        "hostId": host_id,
    });
    pool.sftp_write(
        host_id,
        &format!("{}/metadata.json", failure_dir),
        &serde_json::to_string_pretty(&metadata).map_err(|error| error.to_string())?,
    )
    .await?;
    emit_doctor_assistant_progress(
        app,
        run_id,
        "cleanup",
        "Restoring first valid backup into primary config",
        0.965,
        0,
        None,
        None,
    );
    pool.sftp_write(host_id, &main_config_path, &backup_text)
        .await?;
    append_step(
        steps,
        "repair.fallback.restore_primary_config",
        "Restore primary config from fallback backup",
        true,
        format!("Restored {} into {}", backup_path, main_config_path),
        None,
    );
    emit_doctor_assistant_progress(
        app,
        run_id,
        "cleanup",
        "Restarting gateway from recovered config",
        0.98,
        0,
        None,
        None,
    );
    let mut commands = Vec::new();
    super::run_remote_gateway_restart_fallback(
        pool,
        host_id,
        DOCTOR_ASSISTANT_TARGET_PROFILE,
        &mut commands,
    )
    .await?;
    append_step(
        steps,
        "repair.fallback.restart_gateway",
        "Restart primary gateway",
        true,
        format!(
            "Restarted primary gateway using {} command(s)",
            commands.len()
        ),
        None,
    );
    let after = diagnose_doctor_assistant_remote_impl(
        pool,
        host_id,
        app,
        run_id,
        DOCTOR_ASSISTANT_TARGET_PROFILE,
    )
    .await?;
    append_step(
        steps,
        "repair.fallback.recheck",
        "Re-check primary gateway after fallback restore",
        diagnose_doctor_assistant_status(&after),
        if diagnose_doctor_assistant_status(&after) {
            "Primary gateway recovered after fallback restore".to_string()
        } else {
            "Primary gateway remained unhealthy after fallback restore".to_string()
        },
        None,
    );
    Ok(Some(after))
}

fn load_local_donor_cfg_fallback() -> LocalDonorConfigLoad {
    let paths = resolve_paths();
    let config_path = paths.config_path.to_string_lossy().to_string();
    match read_openclaw_config(&paths) {
        Ok(cfg) => LocalDonorConfigLoad {
            main_config_path: config_path.clone(),
            donor_cfg: cfg,
            source_mode: "parsed_main_config",
            defaults_source_path: Some(config_path),
        },
        Err(_) => {
            let mut donor = salvage_donor_cfg_from_text(&read_local_primary_config_text(
                DOCTOR_ASSISTANT_TARGET_PROFILE,
            ));
            let candidate_texts = read_local_openclaw_json_candidates(&paths.openclaw_dir);
            let defaults_source_path =
                overlay_agent_defaults_from_named_candidates(&mut donor, &candidate_texts);
            crate::commands::logs::log_dev(format!(
                "[dev][doctor_assistant] local donor candidates root={} files={} selected={}",
                paths.openclaw_dir.display(),
                if candidate_texts.is_empty() {
                    "none".to_string()
                } else {
                    candidate_texts
                        .iter()
                        .map(|(path, _)| path.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                },
                defaults_source_path
                    .clone()
                    .unwrap_or_else(|| "none".to_string())
            ));
            LocalDonorConfigLoad {
                main_config_path: config_path,
                donor_cfg: donor,
                source_mode: "salvaged_main_config",
                defaults_source_path,
            }
        }
    }
}

async fn load_remote_donor_cfg_fallback(
    pool: &SshConnectionPool,
    host_id: &str,
) -> RemoteDonorConfigLoad {
    let home_dir = pool
        .get_home_dir(host_id)
        .await
        .unwrap_or_else(|_| "/root".into());
    match super::remote_read_openclaw_config_text_and_json(pool, host_id).await {
        Ok((config_path, _, cfg)) => {
            let config_path = expand_remote_home_path(&config_path, &home_dir);
            RemoteDonorConfigLoad {
                main_config_path: config_path.clone(),
                donor_cfg: cfg,
                source_mode: "parsed_main_config",
                defaults_source_path: Some(config_path),
            }
        }
        Err(_) => {
            let config_path = expand_remote_home_path(
                &default_main_config_path(DOCTOR_ASSISTANT_TARGET_PROFILE),
                &home_dir,
            );
            let raw =
                read_remote_primary_config_text(pool, host_id, DOCTOR_ASSISTANT_TARGET_PROFILE)
                    .await;
            let mut donor = salvage_donor_cfg_from_text(&raw);
            let main_root = std::path::Path::new(&config_path)
                .parent()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("{}/.openclaw", home_dir.trim_end_matches('/')));
            let candidate_texts =
                read_remote_openclaw_json_candidates(pool, host_id, &main_root).await;
            let defaults_source_path =
                overlay_agent_defaults_from_named_candidates(&mut donor, &candidate_texts);
            crate::commands::logs::log_dev(format!(
                "[dev][doctor_assistant] remote donor candidates root={} files={} selected={}",
                main_root,
                if candidate_texts.is_empty() {
                    "none".to_string()
                } else {
                    candidate_texts
                        .iter()
                        .map(|(path, _)| path.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                },
                defaults_source_path
                    .clone()
                    .unwrap_or_else(|| "none".to_string())
            ));
            RemoteDonorConfigLoad {
                main_config_path: config_path,
                donor_cfg: donor,
                source_mode: "salvaged_main_config",
                defaults_source_path,
            }
        }
    }
}

async fn resolve_remote_main_root(pool: &SshConnectionPool, host_id: &str) -> String {
    let home_dir = pool
        .get_home_dir(host_id)
        .await
        .unwrap_or_else(|_| "/root".into());
    super::remote_read_openclaw_config_text_and_json(pool, host_id)
        .await
        .ok()
        .and_then(|(config_path, _, _)| {
            std::path::Path::new(&expand_remote_home_path(&config_path, &home_dir))
                .parent()
                .map(|path| path.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| format!("{}/.openclaw", home_dir.trim_end_matches('/')))
}

async fn resolve_remote_openclaw_version(
    pool: &SshConnectionPool,
    host_id: &str,
) -> Option<String> {
    crate::cli_runner::run_openclaw_remote(pool, host_id, &["--version"])
        .await
        .ok()
        .and_then(|output| {
            let text = if output.stdout.trim().is_empty() {
                output.stderr.trim()
            } else {
                output.stdout.trim()
            };
            (!text.is_empty()).then(|| text.to_string())
        })
}

fn load_local_doctor_provider_profiles() -> Vec<ModelProfile> {
    super::load_model_profiles(&resolve_paths())
}

fn doctor_provider_from_model_ref(model_ref: Option<&str>) -> Option<String> {
    let trimmed = model_ref?.trim();
    let (provider, _) = trimmed.split_once('/')?;
    let provider = provider.trim().to_ascii_lowercase();
    if provider.is_empty() {
        None
    } else {
        Some(provider)
    }
}

fn preferred_auto_provider_profile(
    cfg: &serde_json::Value,
    profiles: &[ModelProfile],
    paths: &crate::models::OpenClawPaths,
) -> Option<ModelProfile> {
    let preferred_provider = doctor_provider_from_model_ref(
        cfg.pointer("/agents/defaults/model/primary")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                cfg.pointer("/agents/defaults/model")
                    .and_then(serde_json::Value::as_str)
            }),
    );
    let provider_keys = cfg
        .pointer("/models/providers")
        .and_then(serde_json::Value::as_object)
        .map(|providers| {
            providers
                .keys()
                .map(|key| key.trim().to_ascii_lowercase())
                .collect::<std::collections::HashSet<_>>()
        })
        .unwrap_or_default();
    let mut scored = profiles
        .iter()
        .filter(|profile| profile.enabled)
        .filter_map(|profile| {
            let provider_key = profile.provider.trim().to_ascii_lowercase();
            if provider_key.is_empty() {
                return None;
            }
            let resolved = super::resolve_profile_api_key(profile, &paths.base_dir);
            if resolved.trim().is_empty()
                && !super::provider_supports_optional_api_key(&profile.provider)
            {
                return None;
            }
            let mut score = 0usize;
            if preferred_provider
                .as_deref()
                .map(|provider| provider == provider_key)
                .unwrap_or(false)
            {
                score += 20;
            }
            if provider_keys.contains(&provider_key) {
                score += 10;
            }
            if !resolved.trim().is_empty() {
                score += 5;
            }
            if profile
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some()
            {
                score += 3;
            }
            Some((score, profile.clone()))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().next().map(|(_, profile)| profile)
}

fn select_doctor_provider_profile(
    cfg: &serde_json::Value,
    explicit_profile_id: Option<&str>,
) -> Result<Option<ModelProfile>, String> {
    let paths = resolve_paths();
    let profiles = load_local_doctor_provider_profiles();
    if let Some(profile_id) = explicit_profile_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let Some(profile) = profiles
            .into_iter()
            .find(|candidate| candidate.id == profile_id)
        else {
            return Err(doctor_assistant_temp_provider_setup_required(
                "The selected temporary provider profile no longer exists. Add it again and retry repair.",
            ));
        };
        let resolved = super::resolve_profile_api_key(&profile, &paths.base_dir);
        if resolved.trim().is_empty()
            && !super::provider_supports_optional_api_key(&profile.provider)
        {
            return Err(doctor_assistant_temp_provider_setup_required(format!(
                "Profile {} has no usable static credential. Add an API key or local env-backed auth_ref for the temporary gateway.",
                profile.name
            )));
        }
        return Ok(Some(profile));
    }
    Ok(preferred_auto_provider_profile(cfg, &profiles, &paths))
}

fn resolve_internal_provider_credential_for_profile(
    profile: &ModelProfile,
) -> Option<InternalProviderCredential> {
    let paths = resolve_paths();
    let secret = super::resolve_profile_api_key(profile, &paths.base_dir);
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(InternalProviderCredential {
        secret: trimmed.to_string(),
        kind: infer_auth_kind(&profile.provider, trimmed, InternalAuthKind::ApiKey),
    })
}

fn apply_provider_credential_to_object(
    provider_obj: &mut serde_json::Map<String, serde_json::Value>,
    credential: &InternalProviderCredential,
) {
    for field in [
        "authRef",
        "auth_ref",
        "secretRef",
        "keyRef",
        "tokenRef",
        "apiKeyRef",
        "api_key_ref",
        "accessRef",
        "apiKey",
        "key",
        "token",
        "access",
    ] {
        provider_obj.remove(field);
    }
    match credential.kind {
        InternalAuthKind::ApiKey => {
            provider_obj.insert(
                "apiKey".into(),
                serde_json::Value::String(credential.secret.clone()),
            );
        }
        InternalAuthKind::Authorization => {
            provider_obj.insert(
                "token".into(),
                serde_json::Value::String(credential.secret.clone()),
            );
        }
    }
}

fn build_single_provider_config(
    donor_cfg: &serde_json::Value,
    profile: &ModelProfile,
    credential: Option<&InternalProviderCredential>,
) -> serde_json::Value {
    let provider_key = profile.provider.trim().to_ascii_lowercase();
    let mut provider_obj = donor_cfg
        .pointer(&format!("/models/providers/{provider_key}"))
        .and_then(serde_json::Value::as_object)
        .cloned()
        .or_else(|| {
            donor_cfg
                .pointer("/models/providers")
                .and_then(serde_json::Value::as_object)
                .and_then(|providers| providers.values().find_map(serde_json::Value::as_object))
                .cloned()
        })
        .unwrap_or_default();

    if let Some(base_url) = profile
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| super::resolve_model_provider_base_url(donor_cfg, &profile.provider))
        .or_else(|| super::default_base_url_for_provider(&profile.provider).map(str::to_string))
    {
        provider_obj.insert("baseUrl".into(), serde_json::Value::String(base_url));
    }

    for field in [
        "authRef",
        "auth_ref",
        "secretRef",
        "keyRef",
        "tokenRef",
        "apiKeyRef",
        "api_key_ref",
        "accessRef",
        "apiKey",
        "key",
        "token",
        "access",
    ] {
        provider_obj.remove(field);
    }

    if let Some(credential) = credential {
        apply_provider_credential_to_object(&mut provider_obj, credential);
    }

    let mut providers = serde_json::Map::new();
    providers.insert(provider_key, serde_json::Value::Object(provider_obj));
    serde_json::Value::Object(providers)
}

fn build_auth_profiles_for_provider(profile: &ModelProfile) -> serde_json::Value {
    let provider_key = profile.provider.trim().to_ascii_lowercase();
    let auth_ref = if !profile.auth_ref.trim().is_empty()
        && profile.auth_ref.contains(':')
        && !super::is_valid_env_var_name(&profile.auth_ref)
    {
        profile.auth_ref.trim().to_string()
    } else {
        format!("{provider_key}:default")
    };
    let mut profiles = serde_json::Map::new();
    profiles.insert(
        auth_ref,
        serde_json::json!({
            "provider": provider_key,
            "name": provider_key,
        }),
    );
    serde_json::Value::Object(profiles)
}

#[allow(dead_code)]
fn repair_local_main_gateway_provider_consistency(
    explicit_profile_id: Option<&str>,
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<bool, String> {
    let paths = resolve_paths();
    let donor_cfg = read_openclaw_config(&paths)?;
    let Some(profile) = select_doctor_provider_profile(&donor_cfg, explicit_profile_id)? else {
        return Ok(false);
    };
    let credential = resolve_internal_provider_credential_for_profile(&profile);
    if credential.is_none() && !super::provider_supports_optional_api_key(&profile.provider) {
        return Err(doctor_assistant_temp_provider_setup_required(format!(
            "Temporary gateway still has no usable provider credential for {}. Add a provider profile with an API key, then retry repair.",
            profile.provider
        )));
    }
    let providers = build_single_provider_config(&donor_cfg, &profile, credential.as_ref());
    apply_local_profile_json_value(
        DOCTOR_ASSISTANT_TARGET_PROFILE,
        "models.providers",
        &providers,
    )?;
    append_step(
        steps,
        "primary.realign.providers",
        "Realign primary provider configuration",
        true,
        format!(
            "Primary gateway provider configuration was reset to {}",
            profile.provider
        ),
        None,
    );

    let auth_profiles = build_auth_profiles_for_provider(&profile);
    apply_local_profile_json_value(
        DOCTOR_ASSISTANT_TARGET_PROFILE,
        "auth.profiles",
        &auth_profiles,
    )?;
    append_step(
        steps,
        "primary.realign.auth_profiles",
        "Realign primary auth profile bindings",
        true,
        format!(
            "Primary gateway auth bindings now point to {}",
            profile.provider
        ),
        None,
    );

    let default_model = serde_json::Value::String(super::profile_to_model_value(&profile));
    apply_local_profile_json_value(
        DOCTOR_ASSISTANT_TARGET_PROFILE,
        "agents.defaults.model.primary",
        &default_model,
    )?;
    append_step(
        steps,
        "primary.realign.default_model",
        "Realign primary default model",
        true,
        format!(
            "Primary gateway default model was set to {}",
            super::profile_to_model_value(&profile)
        ),
        None,
    );

    let restart_command =
        build_profile_command(DOCTOR_ASSISTANT_TARGET_PROFILE, &["gateway", "restart"]);
    let restart_output = run_openclaw_dynamic(&restart_command)?;
    let restart_ok = restart_output.exit_code == 0;
    append_step(
        steps,
        "primary.realign.restart",
        "Restart primary gateway",
        restart_ok,
        build_step_detail(&restart_command, &restart_output),
        Some(restart_command.clone()),
    );
    if !restart_ok {
        return Err(command_failure_message(&restart_command, &restart_output));
    }

    Ok(true)
}

#[allow(dead_code)]
async fn repair_remote_main_gateway_provider_consistency(
    pool: &SshConnectionPool,
    host_id: &str,
    explicit_profile_id: Option<&str>,
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<bool, String> {
    let (_, _, donor_cfg) = super::remote_read_openclaw_config_text_and_json(pool, host_id).await?;
    let Some(profile) = select_doctor_provider_profile(&donor_cfg, explicit_profile_id)? else {
        return Ok(false);
    };
    let credential = resolve_internal_provider_credential_for_profile(&profile);
    if credential.is_none() && !super::provider_supports_optional_api_key(&profile.provider) {
        return Err(doctor_assistant_temp_provider_setup_required(format!(
            "Temporary gateway still has no usable provider credential for {}. Add a provider profile with an API key, then retry repair.",
            profile.provider
        )));
    }
    let providers = build_single_provider_config(&donor_cfg, &profile, credential.as_ref());
    apply_remote_profile_json_value(
        pool,
        host_id,
        DOCTOR_ASSISTANT_TARGET_PROFILE,
        "models.providers",
        &providers,
    )
    .await?;
    append_step(
        steps,
        "primary.realign.providers",
        "Realign primary provider configuration",
        true,
        format!(
            "Primary gateway provider configuration was reset to {}",
            profile.provider
        ),
        None,
    );

    let auth_profiles = build_auth_profiles_for_provider(&profile);
    apply_remote_profile_json_value(
        pool,
        host_id,
        DOCTOR_ASSISTANT_TARGET_PROFILE,
        "auth.profiles",
        &auth_profiles,
    )
    .await?;
    append_step(
        steps,
        "primary.realign.auth_profiles",
        "Realign primary auth profile bindings",
        true,
        format!(
            "Primary gateway auth bindings now point to {}",
            profile.provider
        ),
        None,
    );

    let default_model = serde_json::Value::String(super::profile_to_model_value(&profile));
    apply_remote_profile_json_value(
        pool,
        host_id,
        DOCTOR_ASSISTANT_TARGET_PROFILE,
        "agents.defaults.model.primary",
        &default_model,
    )
    .await?;
    append_step(
        steps,
        "primary.realign.default_model",
        "Realign primary default model",
        true,
        format!(
            "Primary gateway default model was set to {}",
            super::profile_to_model_value(&profile)
        ),
        None,
    );

    let restart_command =
        build_profile_command(DOCTOR_ASSISTANT_TARGET_PROFILE, &["gateway", "restart"]);
    let restart_output =
        run_remote_openclaw_dynamic(pool, host_id, restart_command.clone()).await?;
    let restart_ok = restart_output.exit_code == 0;
    append_step(
        steps,
        "primary.realign.restart",
        "Restart primary gateway",
        restart_ok,
        build_step_detail(&restart_command, &restart_output),
        Some(restart_command.clone()),
    );
    if !restart_ok {
        return Err(command_failure_message(&restart_command, &restart_output));
    }

    Ok(true)
}

#[allow(dead_code)]
fn doctor_fix_flag_unsupported(output: &OpenclawCommandOutput) -> bool {
    let text = format!("{}\n{}", output.stderr, output.stdout).to_ascii_lowercase();
    (text.contains("unknown option")
        || text.contains("unknown argument")
        || text.contains("unrecognized option")
        || text.contains("unexpected argument")
        || text.contains("no such option"))
        && text.contains("--fix")
}

fn diagnose_doctor_assistant_status(diagnosis: &RescuePrimaryDiagnosisResult) -> bool {
    diagnosis.status == "healthy"
        && diagnosis
            .sections
            .iter()
            .all(|section| section.status == "healthy")
}

fn build_doctor_assistant_diagnosis(
    target_profile: &str,
    config: Option<&serde_json::Value>,
    mut runtime_checks: Vec<RescuePrimaryCheckItem>,
    primary_doctor_output: &OpenclawCommandOutput,
    primary_gateway_status: &OpenclawCommandOutput,
) -> RescuePrimaryDiagnosisResult {
    let mut checks = Vec::new();
    checks.append(&mut runtime_checks);
    let mut issues: Vec<clawpal_core::doctor::DoctorIssue> = Vec::new();

    let doctor_report = clawpal_core::doctor::parse_json_loose(&primary_doctor_output.stdout)
        .or_else(|| clawpal_core::doctor::parse_json_loose(&primary_doctor_output.stderr));
    let doctor_issues = doctor_report
        .as_ref()
        .map(|report| clawpal_core::doctor::parse_doctor_issues(report, "primary"))
        .unwrap_or_default();
    let doctor_issue_count = doctor_issues.len();
    let doctor_score = doctor_report
        .as_ref()
        .and_then(|report| report.get("score"))
        .and_then(serde_json::Value::as_i64);
    let doctor_ok_from_report = doctor_report
        .as_ref()
        .and_then(|report| report.get("ok"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(primary_doctor_output.exit_code == 0);
    let doctor_has_error = doctor_issues.iter().any(|issue| issue.severity == "error");
    let doctor_check_ok = doctor_ok_from_report && !doctor_has_error;

    let doctor_detail = if let Some(score) = doctor_score {
        format!("score={score}, issues={doctor_issue_count}")
    } else {
        command_detail(primary_doctor_output)
    };
    checks.push(RescuePrimaryCheckItem {
        id: "primary.doctor".into(),
        title: "OpenClaw doctor report".into(),
        ok: doctor_check_ok,
        detail: doctor_detail,
    });

    if doctor_report.is_none() && primary_doctor_output.exit_code != 0 {
        issues.push(clawpal_core::doctor::DoctorIssue {
            id: "primary.doctor.failed".into(),
            code: "primary.doctor.failed".into(),
            severity: "error".into(),
            message: "OpenClaw doctor command failed".into(),
            auto_fixable: false,
            fix_hint: Some("Review doctor output and gateway logs for details".into()),
            source: "primary".into(),
        });
    }
    issues.extend(doctor_issues);

    let primary_gateway_ok = gateway_output_ok(primary_gateway_status);
    checks.push(RescuePrimaryCheckItem {
        id: "primary.gateway.status".into(),
        title: "Primary gateway status".into(),
        ok: primary_gateway_ok,
        detail: gateway_output_detail(primary_gateway_status),
    });
    if config.is_none() {
        issues.push(clawpal_core::doctor::DoctorIssue {
            id: "primary.config.unreadable".into(),
            code: "primary.config.unreadable".into(),
            severity: if primary_gateway_ok {
                "warn".into()
            } else {
                "error".into()
            },
            message: "Primary configuration could not be read".into(),
            auto_fixable: false,
            fix_hint: Some("Repair openclaw.json parsing errors and re-run diagnosis".into()),
            source: "primary".into(),
        });
    }
    if !primary_gateway_ok {
        issues.push(clawpal_core::doctor::DoctorIssue {
            id: "primary.gateway.unhealthy".into(),
            code: "primary.gateway.unhealthy".into(),
            severity: "error".into(),
            message: "Primary gateway is not healthy".into(),
            auto_fixable: true,
            fix_hint: Some(
                "Restart the primary gateway and inspect logs if it stays unhealthy".into(),
            ),
            source: "primary".into(),
        });
    }

    clawpal_core::doctor::dedupe_doctor_issues(&mut issues);
    let status = clawpal_core::doctor::classify_doctor_issue_status(&issues);
    let issues: Vec<RescuePrimaryIssue> = issues
        .into_iter()
        .map(|issue| RescuePrimaryIssue {
            id: issue.id,
            code: issue.code,
            severity: issue.severity,
            message: issue.message,
            auto_fixable: issue.auto_fixable,
            fix_hint: issue.fix_hint,
            source: issue.source,
        })
        .collect();
    let sections = build_rescue_primary_sections(config, &checks, &issues);
    let summary = build_rescue_primary_summary(&sections, &issues);

    RescuePrimaryDiagnosisResult {
        status,
        checked_at: format_timestamp_from_unix(unix_timestamp_secs()),
        target_profile: target_profile.to_string(),
        rescue_profile: "temporary".into(),
        rescue_configured: false,
        rescue_port: None,
        summary,
        sections,
        checks,
        issues,
    }
}

fn diagnose_doctor_assistant_local_impl(
    app: &AppHandle,
    run_id: &str,
    target_profile: &str,
) -> Result<RescuePrimaryDiagnosisResult, String> {
    emit_doctor_assistant_progress(
        app,
        run_id,
        "diagnose",
        "Cleaning temporary gateway traces",
        0.06,
        0,
        None,
        None,
    );
    let paths = resolve_paths();
    let cleaned = cleanup_local_stale_temp_gateways(&paths).unwrap_or(0);
    emit_doctor_assistant_progress(
        app,
        run_id,
        "diagnose",
        if cleaned > 0 {
            format!("Removed {cleaned} temporary gateway trace(s)")
        } else {
            "Running OpenClaw Doctor".into()
        },
        0.12,
        0,
        None,
        None,
    );
    let config = read_openclaw_config(&paths).ok();
    let config_content = std::fs::read_to_string(&paths.config_path)
        .ok()
        .and_then(|raw| {
            clawpal_core::config::parse_and_normalize_config(&raw)
                .ok()
                .map(|(_, normalized)| normalized)
        })
        .or_else(|| {
            config
                .as_ref()
                .and_then(|cfg| serde_json::to_string_pretty(cfg).ok())
        })
        .unwrap_or_default();
    let primary_doctor_output = run_local_primary_doctor_with_fallback(target_profile)?;

    emit_doctor_assistant_progress(
        app,
        run_id,
        "diagnose",
        "Checking gateway health",
        0.45,
        0,
        None,
        None,
    );
    let primary_gateway_output =
        run_openclaw_dynamic(&build_gateway_status_command(target_profile, true))?;

    emit_doctor_assistant_progress(
        app,
        run_id,
        "diagnose",
        "Running ClawPal checklist",
        0.72,
        0,
        None,
        None,
    );
    let runtime_checks = collect_local_rescue_runtime_checks(config.as_ref());
    let diagnosis = build_doctor_assistant_diagnosis(
        target_profile,
        config.as_ref(),
        runtime_checks,
        &primary_doctor_output,
        &primary_gateway_output,
    );
    let doc_request = build_doc_resolve_request(
        "local",
        "local",
        Some(resolve_openclaw_version()),
        &diagnosis.issues,
        config_content,
        Some(gateway_output_detail(&primary_gateway_output)),
    );
    let guidance = tauri::async_runtime::block_on(resolve_local_doc_guidance(&doc_request, &paths));
    let diagnosis = apply_doc_guidance_to_diagnosis(diagnosis, Some(guidance));

    emit_doctor_assistant_progress(
        app,
        run_id,
        "diagnose",
        "Diagnosis complete",
        1.0,
        0,
        None,
        None,
    );
    Ok(diagnosis)
}

async fn diagnose_doctor_assistant_remote_impl(
    pool: &SshConnectionPool,
    host_id: &str,
    app: &AppHandle,
    run_id: &str,
    target_profile: &str,
) -> Result<RescuePrimaryDiagnosisResult, String> {
    emit_doctor_assistant_progress(
        app,
        run_id,
        "diagnose",
        "Cleaning temporary gateway traces over SSH",
        0.06,
        0,
        None,
        None,
    );
    let paths = resolve_paths();
    let cleaned = cleanup_remote_stale_temp_gateways(pool, host_id, &paths)
        .await
        .unwrap_or(0);
    emit_doctor_assistant_progress(
        app,
        run_id,
        "diagnose",
        if cleaned > 0 {
            format!("Removed {cleaned} temporary gateway trace(s) over SSH")
        } else {
            "Running OpenClaw Doctor over SSH".into()
        },
        0.12,
        0,
        None,
        None,
    );
    let remote_config = remote_read_openclaw_config_text_and_json(pool, host_id)
        .await
        .ok();
    let config_content = remote_config
        .as_ref()
        .map(|(_, normalized, _)| normalized.clone())
        .unwrap_or_default();
    let config = remote_config.as_ref().map(|(_, _, cfg)| cfg.clone());
    let primary_doctor_output =
        run_remote_primary_doctor_with_fallback(pool, host_id, target_profile).await?;

    emit_doctor_assistant_progress(
        app,
        run_id,
        "diagnose",
        "Checking gateway health",
        0.45,
        0,
        None,
        None,
    );
    let primary_gateway_output = run_remote_openclaw_dynamic(
        pool,
        host_id,
        build_gateway_status_command(target_profile, true),
    )
    .await?;

    emit_doctor_assistant_progress(
        app,
        run_id,
        "diagnose",
        "Running ClawPal checklist",
        0.72,
        0,
        None,
        None,
    );
    let runtime_checks = collect_remote_rescue_runtime_checks(pool, host_id, config.as_ref()).await;
    let diagnosis = build_doctor_assistant_diagnosis(
        target_profile,
        config.as_ref(),
        runtime_checks,
        &primary_doctor_output,
        &primary_gateway_output,
    );
    let remote_version = resolve_remote_openclaw_version(pool, host_id).await;
    let guidance = resolve_remote_doc_guidance(
        pool,
        host_id,
        &build_doc_resolve_request(
            host_id,
            "remote_ssh",
            remote_version,
            &diagnosis.issues,
            config_content,
            Some(gateway_output_detail(&primary_gateway_output)),
        ),
        &resolve_paths(),
    )
    .await;
    let diagnosis = apply_doc_guidance_to_diagnosis(diagnosis, Some(guidance));

    emit_doctor_assistant_progress(
        app,
        run_id,
        "diagnose",
        "Diagnosis complete",
        1.0,
        0,
        None,
        None,
    );
    Ok(diagnosis)
}

fn append_step(
    steps: &mut Vec<RescuePrimaryRepairStep>,
    id: impl Into<String>,
    title: impl Into<String>,
    ok: bool,
    detail: impl Into<String>,
    command: Option<Vec<String>>,
) {
    steps.push(RescuePrimaryRepairStep {
        id: id.into(),
        title: title.into(),
        ok,
        detail: detail.into(),
        command,
    });
}

#[allow(dead_code)]
fn run_local_direct_doctor_fix_attempt(
    profile: &str,
    attempt: usize,
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<bool, String> {
    let command = build_profile_command(profile, &["doctor", "--fix", "--yes"]);
    let output = run_openclaw_dynamic(&command)?;
    let ok = output.exit_code == 0;
    append_step(
        steps,
        format!("direct.doctor.fix.{attempt}"),
        format!("Run openclaw doctor --fix --yes (attempt {attempt})"),
        ok,
        build_step_detail(&command, &output),
        Some(command.clone()),
    );
    if ok {
        return Ok(true);
    }
    if !doctor_fix_flag_unsupported(&output) {
        return Ok(false);
    }

    let fallback_command = build_profile_command(profile, &["doctor", "--yes"]);
    let fallback_output = run_openclaw_dynamic(&fallback_command)?;
    let fallback_ok = fallback_output.exit_code == 0;
    append_step(
        steps,
        format!("direct.doctor.fix.{attempt}.fallback"),
        format!("Fallback to openclaw doctor --yes (attempt {attempt})"),
        fallback_ok,
        build_step_detail(&fallback_command, &fallback_output),
        Some(fallback_command),
    );
    Ok(fallback_ok)
}

#[allow(dead_code)]
async fn run_remote_direct_doctor_fix_attempt(
    pool: &SshConnectionPool,
    host_id: &str,
    profile: &str,
    attempt: usize,
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<bool, String> {
    let command = build_profile_command(profile, &["doctor", "--fix", "--yes"]);
    let output = run_remote_openclaw_dynamic(pool, host_id, command.clone()).await?;
    let ok = output.exit_code == 0;
    append_step(
        steps,
        format!("direct.doctor.fix.{attempt}"),
        format!("Run openclaw doctor --fix --yes (attempt {attempt})"),
        ok,
        build_step_detail(&command, &output),
        Some(command.clone()),
    );
    if ok {
        return Ok(true);
    }
    if !doctor_fix_flag_unsupported(&output) {
        return Ok(false);
    }

    let fallback_command = build_profile_command(profile, &["doctor", "--yes"]);
    let fallback_output =
        run_remote_openclaw_dynamic(pool, host_id, fallback_command.clone()).await?;
    let fallback_ok = fallback_output.exit_code == 0;
    append_step(
        steps,
        format!("direct.doctor.fix.{attempt}.fallback"),
        format!("Fallback to openclaw doctor --yes (attempt {attempt})"),
        fallback_ok,
        build_step_detail(&fallback_command, &fallback_output),
        Some(fallback_command),
    );
    Ok(fallback_ok)
}

fn merge_issue_lists(target: &mut Vec<String>, more: impl IntoIterator<Item = String>) {
    for item in more {
        if !target.contains(&item) {
            target.push(item);
        }
    }
}

fn choose_temp_gateway_profile_name() -> String {
    let suffix = Uuid::new_v4().simple().to_string();
    format!("{DOCTOR_ASSISTANT_TEMP_PROFILE_PREFIX}{}", &suffix[..8])
}

fn choose_temp_gateway_port(main_port: u16) -> u16 {
    let base = clawpal_core::doctor::suggest_rescue_port(main_port);
    let extra = ((unix_timestamp_secs() % 5) as u16) * 20;
    base.saturating_add(extra)
}

fn derive_profile_root_path(root: &std::path::Path, profile: &str) -> std::path::PathBuf {
    if profile.trim().is_empty() {
        return root.to_path_buf();
    }
    let parent = root.parent().unwrap_or_else(|| std::path::Path::new(""));
    let file_name = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(".openclaw");
    let base_name = if file_name.starts_with(".openclaw") {
        ".openclaw".to_string()
    } else {
        file_name.to_string()
    };
    parent.join(format!("{base_name}-{profile}"))
}

fn derive_profile_root_parent_and_base_name(
    root: &std::path::Path,
) -> (std::path::PathBuf, String) {
    let parent = root
        .parent()
        .unwrap_or_else(|| std::path::Path::new(""))
        .to_path_buf();
    let file_name = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(".openclaw");
    let base_name = if file_name.starts_with(".openclaw") {
        ".openclaw".to_string()
    } else {
        file_name.to_string()
    };
    (parent, base_name)
}

fn derive_profile_root_string(root: &str, profile: &str) -> String {
    derive_profile_root_path(std::path::Path::new(root), profile)
        .to_string_lossy()
        .to_string()
}

fn build_temp_gateway_marker_contents(instance_id: &str, profile: &str) -> String {
    format!("owner=clawpal-doctor-assistant\ninstance_id={instance_id}\nprofile={profile}\n")
}

fn local_temp_gateway_has_marker(root: &std::path::Path, profile: &str) -> bool {
    derive_profile_root_path(root, profile)
        .join(DOCTOR_ASSISTANT_TEMP_MARKER_FILE)
        .exists()
}

fn write_local_temp_gateway_marker(
    root: &std::path::Path,
    instance_id: &str,
    profile: &str,
) -> Result<(), String> {
    let profile_root = derive_profile_root_path(root, profile);
    std::fs::create_dir_all(&profile_root).map_err(|error| error.to_string())?;
    let marker_path = profile_root.join(DOCTOR_ASSISTANT_TEMP_MARKER_FILE);
    std::fs::write(
        &marker_path,
        build_temp_gateway_marker_contents(instance_id, profile),
    )
    .map_err(|error| error.to_string())
}

async fn remote_temp_gateway_has_marker(
    pool: &SshConnectionPool,
    host_id: &str,
    root: &str,
    profile: &str,
) -> bool {
    let marker_path = format!(
        "{}/{}",
        derive_profile_root_string(root, profile).trim_end_matches('/'),
        DOCTOR_ASSISTANT_TEMP_MARKER_FILE
    );
    pool.sftp_read(host_id, &marker_path)
        .await
        .map(|text| text.contains("owner=clawpal-doctor-assistant"))
        .unwrap_or(false)
}

async fn write_remote_temp_gateway_marker(
    pool: &SshConnectionPool,
    host_id: &str,
    root: &str,
    instance_id: &str,
    profile: &str,
) -> Result<(), String> {
    let profile_root = derive_profile_root_string(root, profile);
    pool.exec(
        host_id,
        &format!("mkdir -p {}", super::shell_escape(&profile_root)),
    )
    .await?;
    let marker_path = format!(
        "{}/{}",
        profile_root.trim_end_matches('/'),
        DOCTOR_ASSISTANT_TEMP_MARKER_FILE
    );
    pool.sftp_write(
        host_id,
        &marker_path,
        &build_temp_gateway_marker_contents(instance_id, profile),
    )
    .await
}

fn list_local_temp_gateway_profiles(root: &std::path::Path) -> Result<Vec<String>, String> {
    let (parent, base_name) = derive_profile_root_parent_and_base_name(root);
    let prefix = format!("{base_name}-{DOCTOR_ASSISTANT_TEMP_PROFILE_PREFIX}");
    let strip_prefix = format!("{base_name}-");
    let entries = match std::fs::read_dir(&parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut profiles = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(&prefix) {
            continue;
        }
        let profile = name
            .strip_prefix(&strip_prefix)
            .unwrap_or(&name)
            .to_string();
        if !local_temp_gateway_has_marker(root, &profile) {
            continue;
        }
        profiles.push(profile);
    }
    Ok(profiles)
}

async fn list_remote_temp_gateway_profiles(
    pool: &SshConnectionPool,
    host_id: &str,
    root: &str,
) -> Result<Vec<String>, String> {
    let (parent, base_name) = derive_profile_root_parent_and_base_name(std::path::Path::new(root));
    let parent = parent.to_string_lossy().to_string();
    let prefix = format!("{base_name}-{DOCTOR_ASSISTANT_TEMP_PROFILE_PREFIX}");
    let strip_prefix = format!("{base_name}-");
    let entries = match pool.sftp_list(host_id, &parent).await {
        Ok(entries) => entries,
        Err(error) if super::is_remote_missing_path_error(&error) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut profiles = Vec::new();
    for entry in entries
        .into_iter()
        .filter(|entry| entry.is_dir && entry.name.starts_with(&prefix))
    {
        let profile = entry
            .name
            .strip_prefix(&strip_prefix)
            .unwrap_or(entry.name.as_str())
            .to_string();
        if remote_temp_gateway_has_marker(pool, host_id, root, &profile).await {
            profiles.push(profile);
        }
    }
    Ok(profiles)
}

fn prune_local_temp_gateway_profile_roots(
    root: &std::path::Path,
) -> Result<Vec<std::path::PathBuf>, String> {
    let mut removed = Vec::new();
    for profile in list_local_temp_gateway_profiles(root)? {
        let path = derive_profile_root_path(root, &profile);
        std::fs::remove_dir_all(&path).map_err(|error| error.to_string())?;
        removed.push(path);
    }
    Ok(removed)
}

async fn prune_remote_temp_gateway_profile_roots(
    pool: &SshConnectionPool,
    host_id: &str,
    root: &str,
) -> Result<Vec<String>, String> {
    let mut removed = Vec::new();
    for profile in list_remote_temp_gateway_profiles(pool, host_id, root).await? {
        let path = derive_profile_root_string(root, &profile);
        let command = format!("rm -rf {}", super::shell_escape(&path));
        let output = pool.exec(host_id, &command).await?;
        if output.exit_code != 0 {
            return Err(format!(
                "remote temp profile cleanup failed (exit {}): command=`{}` stdout=`{}` stderr=`{}`",
                output.exit_code,
                command,
                output.stdout.trim(),
                output.stderr.trim(),
            ));
        }
        removed.push(path);
    }
    Ok(removed)
}

fn cleanup_local_stale_temp_gateways(
    paths: &crate::models::OpenClawPaths,
) -> Result<usize, String> {
    let profiles = list_local_temp_gateway_profiles(&paths.openclaw_dir)?;
    for profile in &profiles {
        let mut steps = Vec::new();
        let _ = run_local_temp_gateway_action(
            RescueBotAction::Unset,
            profile,
            0,
            false,
            &mut steps,
            "temp.cleanup.stale",
        );
    }
    let _ = prune_local_temp_gateway_profile_roots(&paths.openclaw_dir)?;
    let _ =
        remove_doctor_temp_gateway_records_for_instance(paths, DOCTOR_ASSISTANT_TEMP_SCOPE_LOCAL);
    Ok(profiles.len())
}

async fn cleanup_remote_stale_temp_gateways(
    pool: &SshConnectionPool,
    host_id: &str,
    paths: &crate::models::OpenClawPaths,
) -> Result<usize, String> {
    let main_root = resolve_remote_main_root(pool, host_id).await;
    let profiles = list_remote_temp_gateway_profiles(pool, host_id, &main_root).await?;
    for profile in &profiles {
        let mut steps = Vec::new();
        let _ = run_remote_temp_gateway_action(
            pool,
            host_id,
            RescueBotAction::Unset,
            profile,
            0,
            false,
            &mut steps,
            "temp.cleanup.stale",
        )
        .await;
    }
    let _ = prune_remote_temp_gateway_profile_roots(pool, host_id, &main_root).await?;
    let _ = remove_doctor_temp_gateway_records_for_instance(paths, host_id);
    Ok(profiles.len())
}

fn build_temp_gateway_default_model_value(
    donor_cfg: &serde_json::Value,
    donor_profiles: &[ModelProfile],
) -> Option<serde_json::Value> {
    let configured = donor_cfg
        .pointer("/agents/defaults/model")
        .cloned()
        .or_else(|| donor_cfg.pointer("/agents/default/model").cloned());
    if let Some(value) = configured {
        let configured_model_ref = match &value {
            serde_json::Value::String(model_ref) => Some(model_ref.trim().to_string()),
            serde_json::Value::Object(map) => map
                .get("primary")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            _ => None,
        };
        if configured_model_ref
            .as_deref()
            .and_then(|model_ref| doctor_provider_from_model_ref(Some(model_ref)))
            .and_then(|provider| donor_cfg.pointer(&format!("/models/providers/{provider}")))
            .is_some()
        {
            return Some(value);
        }
    }

    let providers = donor_cfg
        .pointer("/models/providers")
        .and_then(serde_json::Value::as_object);
    if let Some(providers) = providers {
        for (provider_name, provider_cfg) in providers {
            let provider_name = provider_name.trim().to_ascii_lowercase();
            if provider_name.is_empty() {
                continue;
            }
            let model_id = provider_cfg
                .pointer("/models/0/id")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    provider_cfg
                        .pointer("/models/0/name")
                        .and_then(serde_json::Value::as_str)
                })
                .or_else(|| {
                    provider_cfg
                        .pointer("/defaultModel")
                        .and_then(serde_json::Value::as_str)
                })
                .or_else(|| {
                    provider_cfg
                        .pointer("/model")
                        .and_then(serde_json::Value::as_str)
                })
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if let Some(model_id) = model_id {
                return Some(serde_json::json!({
                    "primary": format!("{provider_name}/{model_id}")
                }));
            }
        }
    }

    donor_profiles
        .iter()
        .find(|profile| profile.enabled)
        .map(|profile| serde_json::Value::String(super::profile_to_model_value(profile)))
}

fn build_default_models_from_default_model_value(
    default_model: &serde_json::Value,
) -> Option<serde_json::Value> {
    let model_ref = match default_model {
        serde_json::Value::String(value) => Some(value.trim().to_string()),
        serde_json::Value::Object(map) => map
            .get("primary")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        _ => None,
    }?;
    if model_ref.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        model_ref: {}
    }))
}

fn resolve_temp_gateway_default_model_value(
    donor_cfg: &serde_json::Value,
    selected_profile: Option<&ModelProfile>,
    donor_profiles: &[ModelProfile],
) -> Option<serde_json::Value> {
    selected_profile
        .map(|profile| serde_json::Value::String(super::profile_to_model_value(profile)))
        .or_else(|| donor_cfg.pointer("/agents/defaults/model").cloned())
        .or_else(|| donor_cfg.pointer("/agents/default/model").cloned())
        .or_else(|| build_temp_gateway_default_model_value(donor_cfg, donor_profiles))
}

fn resolve_temp_gateway_default_models_value(
    donor_cfg: &serde_json::Value,
    selected_profile: Option<&ModelProfile>,
    default_model: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    if let Some(profile) = selected_profile {
        return Some(serde_json::json!({
            super::profile_to_model_value(profile): {}
        }));
    }
    donor_cfg
        .pointer("/agents/defaults/models")
        .cloned()
        .or_else(|| donor_cfg.pointer("/agents/default/models").cloned())
        .or_else(|| default_model.and_then(build_default_models_from_default_model_value))
}

fn materialize_temp_gateway_provider_config(
    donor_cfg: &serde_json::Value,
    provider_credentials: &std::collections::HashMap<String, InternalProviderCredential>,
) -> Option<serde_json::Value> {
    let providers = donor_cfg
        .pointer("/models/providers")
        .and_then(serde_json::Value::as_object)?;
    let mut next = providers.clone();
    for (provider_name, provider_cfg) in &mut next {
        let provider_key = provider_name.trim().to_ascii_lowercase();
        let Some(provider_obj) = provider_cfg.as_object_mut() else {
            continue;
        };
        let Some(credential) = provider_credentials.get(&provider_key) else {
            continue;
        };
        for field in [
            "secretRef",
            "keyRef",
            "tokenRef",
            "apiKeyRef",
            "api_key_ref",
            "accessRef",
        ] {
            provider_obj.remove(field);
        }
        match credential.kind {
            InternalAuthKind::ApiKey => {
                provider_obj.remove("token");
                provider_obj.remove("access");
                provider_obj.insert(
                    "apiKey".into(),
                    serde_json::Value::String(credential.secret.clone()),
                );
            }
            InternalAuthKind::Authorization => {
                provider_obj.remove("apiKey");
                provider_obj.remove("key");
                provider_obj.insert(
                    "token".into(),
                    serde_json::Value::String(credential.secret.clone()),
                );
            }
        }
    }
    Some(serde_json::Value::Object(next))
}

fn apply_local_profile_json_value(
    profile: &str,
    path: &str,
    value: &serde_json::Value,
) -> Result<(), String> {
    let serialized = serde_json::to_string(value).map_err(|error| error.to_string())?;
    let command = build_profile_command(
        profile,
        &["config", "set", path, serialized.as_str(), "--json"],
    );
    let output = run_openclaw_dynamic(&command)?;
    if output.exit_code != 0 {
        return Err(command_failure_message(&command, &output));
    }
    Ok(())
}

#[allow(dead_code)]
async fn apply_remote_profile_json_value(
    pool: &SshConnectionPool,
    host_id: &str,
    profile: &str,
    path: &str,
    value: &serde_json::Value,
) -> Result<(), String> {
    let serialized = serde_json::to_string(value).map_err(|error| error.to_string())?;
    let command = build_profile_command(
        profile,
        &["config", "set", path, serialized.as_str(), "--json"],
    );
    let output = run_remote_openclaw_dynamic(pool, host_id, command.clone()).await?;
    if output.exit_code != 0 {
        return Err(command_failure_message(&command, &output));
    }
    Ok(())
}

fn set_json_object_path(root: &mut serde_json::Value, path: &[&str], value: serde_json::Value) {
    if path.is_empty() {
        *root = value;
        return;
    }
    let mut current = root;
    for segment in &path[..path.len() - 1] {
        if !current.is_object() {
            *current = serde_json::json!({});
        }
        let map = current.as_object_mut().expect("object ensured");
        current = map
            .entry((*segment).to_string())
            .or_insert_with(|| serde_json::json!({}));
    }
    if !current.is_object() {
        *current = serde_json::json!({});
    }
    current
        .as_object_mut()
        .expect("object ensured")
        .insert(path[path.len() - 1].to_string(), value);
}

async fn write_remote_temp_gateway_config_snapshot(
    pool: &SshConnectionPool,
    host_id: &str,
    temp_root: &str,
    providers: serde_json::Value,
    auth_profiles: Option<serde_json::Value>,
    default_model: Option<serde_json::Value>,
    default_models: Option<serde_json::Value>,
) -> Result<(), String> {
    let config_path = format!("{}/openclaw.json", temp_root.trim_end_matches('/'));
    let raw = pool
        .sftp_read(host_id, &config_path)
        .await
        .unwrap_or_else(|_| "{}".into());
    let mut cfg =
        json5::from_str::<serde_json::Value>(&raw).unwrap_or_else(|_| serde_json::json!({}));
    set_json_object_path(&mut cfg, &["models", "providers"], providers);
    if let Some(auth_profiles) = auth_profiles {
        set_json_object_path(&mut cfg, &["auth", "profiles"], auth_profiles);
    }
    if let Some(default_model) = default_model {
        set_json_object_path(&mut cfg, &["agents", "defaults", "model"], default_model);
    }
    if let Some(default_models) = default_models {
        set_json_object_path(&mut cfg, &["agents", "defaults", "models"], default_models);
    }
    let text = serde_json::to_string_pretty(&cfg).map_err(|error| error.to_string())?;
    pool.sftp_write(host_id, &config_path, &text).await?;
    crate::commands::logs::log_dev(format!(
        "[dev][doctor_assistant] temp snapshot write path={} providers={} auth_profiles={} default_model={} default_models={}",
        config_path,
        cfg.pointer("/models/providers").is_some(),
        cfg.pointer("/auth/profiles").is_some(),
        cfg.pointer("/agents/defaults/model").is_some(),
        cfg.pointer("/agents/defaults/models").is_some(),
    ));
    Ok(())
}

async fn inspect_remote_temp_gateway_config_snapshot(
    pool: &SshConnectionPool,
    host_id: &str,
    temp_root: &str,
) -> Result<(String, bool, bool, bool), String> {
    let config_path = format!("{}/openclaw.json", temp_root.trim_end_matches('/'));
    let raw = pool.sftp_read(host_id, &config_path).await?;
    let cfg = json5::from_str::<serde_json::Value>(&raw).map_err(|error| error.to_string())?;
    let has_providers = cfg.pointer("/models/providers").is_some();
    let has_default_model = cfg.pointer("/agents/defaults/model").is_some();
    let has_default_models = cfg.pointer("/agents/defaults/models").is_some();
    Ok((
        config_path,
        has_providers,
        has_default_model,
        has_default_models,
    ))
}

fn copy_local_auth_store_into_profile(
    main_root: &std::path::Path,
    temp_root: &std::path::Path,
) -> Result<usize, String> {
    let agents_root = main_root.join("agents");
    let entries = match std::fs::read_dir(&agents_root) {
        Ok(entries) => entries,
        Err(_) => return Ok(0),
    };
    let mut copied = 0usize;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if !file_type.is_dir() {
            continue;
        }
        let agent_name = entry.file_name();
        let source_agent_dir = entry.path().join("agent");
        for file_name in ["auth-profiles.json", "auth.json"] {
            let source = source_agent_dir.join(file_name);
            if !source.exists() {
                continue;
            }
            let dest_dir = temp_root.join("agents").join(&agent_name).join("agent");
            std::fs::create_dir_all(&dest_dir).map_err(|error| error.to_string())?;
            let dest = dest_dir.join(file_name);
            std::fs::copy(&source, &dest).map_err(|error| error.to_string())?;
            copied += 1;
        }
    }
    Ok(copied)
}

async fn copy_remote_auth_store_into_profile(
    pool: &SshConnectionPool,
    host_id: &str,
    main_root: &str,
    temp_root: &str,
) -> Result<usize, String> {
    let main_agent_source_dir = format!("{}/agents/main/agent", main_root.trim_end_matches('/'));
    let main_agent_dest_dir = format!("{}/agents/main/agent", temp_root.trim_end_matches('/'));
    let bootstrap_script = format!(
        "mkdir -p {dest}; count=0; \
         for f in auth-profiles.json auth.json; do \
           if [ -f {src}/$f ]; then cp {src}/$f {dest}/$f && count=$((count+1)); fi; \
         done; printf '%s' \"$count\"",
        src = super::shell_escape(&main_agent_source_dir),
        dest = super::shell_escape(&main_agent_dest_dir),
    );
    let bootstrap_result = pool.exec(host_id, &bootstrap_script).await?;
    let mut copied = bootstrap_result.stdout.trim().parse::<usize>().unwrap_or(0);

    let agents_root = format!("{}/agents", main_root.trim_end_matches('/'));
    let entries = match pool.sftp_list(host_id, &agents_root).await {
        Ok(entries) => entries,
        Err(error) if super::is_remote_missing_path_error(&error) => return Ok(copied),
        Err(error) => return Err(error),
    };
    for entry in entries
        .into_iter()
        .filter(|entry| entry.is_dir && entry.name != "main")
    {
        let source_agent_dir = format!("{agents_root}/{}/agent", entry.name);
        for file_name in ["auth-profiles.json", "auth.json"] {
            let source = format!("{source_agent_dir}/{file_name}");
            let text = match pool.sftp_read(host_id, &source).await {
                Ok(text) => text,
                Err(_) => continue,
            };
            let dest_dir = format!(
                "{}/agents/{}/agent",
                temp_root.trim_end_matches('/'),
                entry.name
            );
            pool.exec(
                host_id,
                &format!("mkdir -p {}", super::shell_escape(&dest_dir)),
            )
            .await?;
            let dest = format!("{dest_dir}/{file_name}");
            pool.sftp_write(host_id, &dest, &text).await?;
            copied += 1;
        }
    }
    Ok(copied)
}

async fn read_remote_auth_store_values_from_root(
    pool: &SshConnectionPool,
    host_id: &str,
    main_root: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let agents_root = format!("{}/agents", main_root.trim_end_matches('/'));
    let entries = match pool.sftp_list(host_id, &agents_root).await {
        Ok(entries) => entries,
        Err(error) if super::is_remote_missing_path_error(&error) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut values = Vec::new();
    for entry in entries.into_iter().filter(|entry| entry.is_dir) {
        let agent_dir = format!("{agents_root}/{}/agent", entry.name);
        for file_name in ["auth-profiles.json", "auth.json"] {
            let auth_file = format!("{agent_dir}/{file_name}");
            let Ok(text) = pool.sftp_read(host_id, &auth_file).await else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            values.push(value);
        }
    }
    Ok(values)
}

fn extract_remote_auth_store_credential(
    provider: &str,
    entry: &serde_json::Value,
) -> Option<InternalProviderCredential> {
    let auth_type = entry
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let kind_from_type = match auth_type.as_str() {
        "oauth" | "token" | "authorization" => Some(InternalAuthKind::Authorization),
        "api_key" | "api-key" | "apikey" => Some(InternalAuthKind::ApiKey),
        _ => None,
    };
    for field in ["token", "key", "apiKey", "api_key", "access"] {
        let Some(secret) = entry.get(field).and_then(serde_json::Value::as_str) else {
            continue;
        };
        let trimmed = secret.trim();
        if trimmed.is_empty() {
            continue;
        }
        let fallback_kind = if provider.trim().eq_ignore_ascii_case("anthropic") {
            InternalAuthKind::ApiKey
        } else {
            match field {
                "token" | "access" => InternalAuthKind::Authorization,
                _ => InternalAuthKind::ApiKey,
            }
        };
        let kind = if provider.trim().eq_ignore_ascii_case("anthropic") {
            infer_auth_kind(provider, trimmed, InternalAuthKind::ApiKey)
        } else {
            infer_auth_kind(provider, trimmed, kind_from_type.unwrap_or(fallback_kind))
        };
        return Some(InternalProviderCredential {
            secret: trimmed.to_string(),
            kind,
        });
    }
    None
}

fn extend_remote_auth_store_candidates(
    out: &mut Vec<RemoteAuthStoreCandidate>,
    entries: &serde_json::Map<String, serde_json::Value>,
) {
    for (auth_ref, entry) in entries {
        let provider = entry
            .get("provider")
            .or_else(|| entry.get("name"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase());
        let Some(provider) = provider else {
            continue;
        };
        let Some(credential) = extract_remote_auth_store_credential(&provider, entry) else {
            continue;
        };
        out.push(RemoteAuthStoreCandidate {
            provider,
            auth_ref: auth_ref.trim().to_string(),
            credential,
        });
    }
}

fn extract_remote_auth_store_candidates(
    auth_store_values: &[serde_json::Value],
) -> Vec<RemoteAuthStoreCandidate> {
    let mut out = Vec::new();
    for value in auth_store_values {
        if let Some(entries) = value.get("profiles").and_then(serde_json::Value::as_object) {
            extend_remote_auth_store_candidates(&mut out, entries);
        }
        if let Some(entries) = value.as_object() {
            let filtered = entries
                .iter()
                .filter(|(key, _)| key.as_str() != "profiles" && key.as_str() != "version")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<serde_json::Map<_, _>>();
            extend_remote_auth_store_candidates(&mut out, &filtered);
        }
    }
    out
}

fn push_provider_model_ref(out: &mut Vec<String>, provider: &str, model_ref: Option<&str>) {
    let Some(model_ref) = model_ref.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    if !doctor_provider_from_model_ref(Some(model_ref))
        .map(|candidate| candidate == provider)
        .unwrap_or(false)
    {
        return;
    }
    if !out
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(model_ref))
    {
        out.push(model_ref.to_string());
    }
}

fn collect_provider_model_refs(donor_cfg: &serde_json::Value, provider: &str) -> Vec<String> {
    let provider = provider.trim().to_ascii_lowercase();
    let mut out = Vec::new();
    push_provider_model_ref(
        &mut out,
        &provider,
        donor_cfg
            .pointer("/agents/defaults/model/primary")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                donor_cfg
                    .pointer("/agents/defaults/model")
                    .and_then(serde_json::Value::as_str)
            })
            .or_else(|| {
                donor_cfg
                    .pointer("/agents/default/model/primary")
                    .and_then(serde_json::Value::as_str)
            })
            .or_else(|| {
                donor_cfg
                    .pointer("/agents/default/model")
                    .and_then(serde_json::Value::as_str)
            }),
    );
    if let Some(agents) = donor_cfg
        .pointer("/agents/list")
        .and_then(serde_json::Value::as_array)
    {
        for agent in agents {
            push_provider_model_ref(
                &mut out,
                &provider,
                agent
                    .pointer("/model/primary")
                    .and_then(serde_json::Value::as_str),
            );
            push_provider_model_ref(
                &mut out,
                &provider,
                agent.pointer("/model").and_then(serde_json::Value::as_str),
            );
        }
    }
    if let Some(provider_cfg) = donor_cfg
        .pointer(&format!("/models/providers/{provider}"))
        .and_then(serde_json::Value::as_object)
    {
        if let Some(model_id) = provider_cfg
            .get("models")
            .and_then(serde_json::Value::as_array)
            .and_then(|models| models.first())
            .and_then(|model| {
                model
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| model.get("name").and_then(serde_json::Value::as_str))
            })
            .or_else(|| {
                provider_cfg
                    .get("defaultModel")
                    .and_then(serde_json::Value::as_str)
            })
            .or_else(|| {
                provider_cfg
                    .get("model")
                    .and_then(serde_json::Value::as_str)
            })
        {
            let model_ref = if model_id.contains('/') {
                model_id.trim().to_string()
            } else {
                format!("{provider}/{}", model_id.trim())
            };
            push_provider_model_ref(&mut out, &provider, Some(&model_ref));
        }
    }
    out
}

fn build_remote_auth_store_provider_fallback_snapshot(
    donor_cfg: &serde_json::Value,
    auth_store_values: &[serde_json::Value],
) -> Option<(
    serde_json::Value,
    Option<serde_json::Value>,
    Option<serde_json::Value>,
    String,
)> {
    let mut candidates = extract_remote_auth_store_candidates(auth_store_values)
        .into_iter()
        .map(|candidate| {
            let model_ref = collect_provider_model_refs(donor_cfg, &candidate.provider)
                .into_iter()
                .next();
            let mut score = 0usize;
            if candidate.auth_ref.to_ascii_lowercase().contains("manual") {
                score += 100;
            }
            if model_ref.is_some() {
                score += 50;
            }
            if donor_cfg
                .pointer(&format!("/models/providers/{}", candidate.provider))
                .is_some()
            {
                score += 20;
            }
            if super::default_base_url_for_provider(&candidate.provider).is_some() {
                score += 10;
            }
            (score, candidate, model_ref)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    let (_, candidate, model_ref) = candidates.into_iter().next()?;
    let provider_key = candidate.provider.trim().to_ascii_lowercase();
    let mut provider_obj = donor_cfg
        .pointer(&format!("/models/providers/{provider_key}"))
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(base_url) = super::resolve_model_provider_base_url(donor_cfg, &provider_key)
        .or_else(|| super::default_base_url_for_provider(&provider_key).map(str::to_string))
    {
        provider_obj.insert("baseUrl".into(), serde_json::Value::String(base_url));
    }
    apply_provider_credential_to_object(&mut provider_obj, &candidate.credential);
    if !provider_obj.contains_key("models") {
        if let Some(model_ref) = model_ref.as_deref() {
            if let Some((_, model_id)) = model_ref.split_once('/') {
                provider_obj.insert(
                    "models".into(),
                    serde_json::json!([{ "id": model_id, "name": model_id }]),
                );
            }
        }
    }
    let mut providers = serde_json::Map::new();
    providers.insert(
        provider_key.clone(),
        serde_json::Value::Object(provider_obj),
    );
    let default_model = model_ref.map(|model_ref| serde_json::json!({ "primary": model_ref }));
    Some((
        serde_json::Value::Object(providers),
        None,
        default_model,
        provider_key,
    ))
}

async fn rebuild_remote_temp_gateway_provider_context_from_auth_store(
    pool: &SshConnectionPool,
    host_id: &str,
    main_root: &str,
    temp_root: &str,
    donor_cfg: &serde_json::Value,
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<(), String> {
    let auth_store_values =
        read_remote_auth_store_values_from_root(pool, host_id, main_root).await?;
    let Some((providers, auth_profiles, default_model, provider_key)) =
        build_remote_auth_store_provider_fallback_snapshot(donor_cfg, &auth_store_values)
    else {
        return Err(doctor_assistant_temp_provider_setup_required(
            "Temporary gateway could not build a usable provider from the primary auth store. Add a provider profile with a static API key in Doctor, then retry repair.",
        ));
    };
    let default_models = default_model
        .as_ref()
        .and_then(build_default_models_from_default_model_value);
    write_remote_temp_gateway_config_snapshot(
        pool,
        host_id,
        temp_root,
        providers,
        auth_profiles.clone(),
        default_model.clone(),
        default_models.clone(),
    )
    .await?;
    append_step(
        steps,
        "temp.sync.remote_auth_fallback.providers",
        "Rebuild temporary provider from remote auth store",
        true,
        format!("Temporary gateway rebuilt its provider configuration from remote auth store entry for {provider_key}"),
        None,
    );
    if auth_profiles.is_some() {
        append_step(
            steps,
            "temp.sync.remote_auth_fallback.auth_profiles",
            "Rebuild temporary auth profiles from remote auth store",
            true,
            "Temporary gateway auth.profiles now reference the recovered remote auth entry",
            None,
        );
    }
    if default_model.is_some() {
        append_step(
            steps,
            "temp.sync.remote_auth_fallback.default_model",
            "Rebuild temporary default model from remote auth store",
            true,
            "Temporary gateway default model now points at a model reference recovered from the primary config",
            None,
        );
    }
    if default_models.is_some() {
        append_step(
            steps,
            "temp.sync.remote_auth_fallback.default_models",
            "Rebuild temporary default model registry from remote auth store",
            true,
            "Temporary gateway default model registry now matches the recovered provider model reference",
            None,
        );
    }
    let copied = copy_remote_auth_store_into_profile(pool, host_id, main_root, temp_root).await?;
    if copied > 0 {
        append_step(
            steps,
            "temp.sync.remote_auth_fallback.auth_store",
            "Copy auth store into temporary gateway",
            true,
            format!("Copied {copied} auth store file(s) into the temporary gateway profile after remote auth fallback"),
            None,
        );
    }
    Ok(())
}

fn provider_copy_source_rank(source: super::ResolvedCredentialSource) -> u8 {
    match source {
        super::ResolvedCredentialSource::ManualApiKey => 4,
        super::ResolvedCredentialSource::ExplicitAuthRef => 3,
        super::ResolvedCredentialSource::ProviderFallbackAuthRef => 2,
        super::ResolvedCredentialSource::ProviderEnvVar => 1,
    }
}

fn build_local_preferred_provider_credentials(
    paths: &crate::models::OpenClawPaths,
) -> std::collections::HashMap<String, InternalProviderCredential> {
    let profiles = super::load_model_profiles(paths);
    let mut ranked = std::collections::HashMap::<String, (InternalProviderCredential, u8)>::new();
    for profile in profiles.iter().filter(|profile| profile.enabled) {
        let provider_key = profile.provider.trim().to_ascii_lowercase();
        if provider_key.is_empty() {
            continue;
        }
        let Some((credential, _, source)) =
            super::resolve_profile_credential_with_priority(profile, &paths.base_dir)
        else {
            continue;
        };
        let rank = provider_copy_source_rank(source);
        match ranked.get_mut(&provider_key) {
            Some((existing, existing_rank)) if rank > *existing_rank => {
                *existing = credential;
                *existing_rank = rank;
            }
            None => {
                ranked.insert(provider_key, (credential, rank));
            }
            _ => {}
        }
    }
    let mut out = ranked
        .into_iter()
        .map(|(provider, (credential, _))| (provider, credential))
        .collect::<std::collections::HashMap<_, _>>();
    super::augment_provider_credentials_from_openclaw_config(paths, &mut out);
    out
}

fn select_probe_profile(
    donor_cfg: &serde_json::Value,
    donor_profiles: &[ModelProfile],
) -> Option<ModelProfile> {
    let default_model = build_temp_gateway_default_model_value(donor_cfg, donor_profiles)
        .and_then(|value| value.as_str().map(str::to_string));
    if let Some(default_model) = default_model {
        if let Some(profile) = donor_profiles.iter().find(|profile| {
            profile.enabled
                && super::profile_to_model_value(profile).eq_ignore_ascii_case(default_model.trim())
        }) {
            return Some(profile.clone());
        }
    }
    donor_profiles
        .iter()
        .find(|profile| profile.enabled)
        .cloned()
}

fn probe_local_temp_gateway_inference(
    donor_cfg: &serde_json::Value,
    donor_profiles: &[ModelProfile],
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<(), String> {
    let paths = resolve_paths();
    let Some(profile) = select_probe_profile(donor_cfg, donor_profiles) else {
        let detail = "No enabled model profile is available for temporary gateway inference probe";
        append_step(
            steps,
            "temp.probe.inference",
            "Probe temporary gateway inference",
            false,
            detail,
            None,
        );
        return Err(detail.into());
    };
    let api_key = super::resolve_profile_api_key(&profile, &paths.base_dir);
    let base_url = profile
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or_else(|| super::resolve_model_provider_base_url(donor_cfg, &profile.provider));
    match super::run_provider_probe(
        profile.provider.clone(),
        profile.model.clone(),
        base_url,
        api_key,
    ) {
        Ok(()) => {
            append_step(
                steps,
                "temp.probe.inference",
                "Probe temporary gateway inference",
                true,
                format!("Temporary gateway can reach provider {}", profile.provider),
                None,
            );
            Ok(())
        }
        Err(error) => {
            append_step(
                steps,
                "temp.probe.inference",
                "Probe temporary gateway inference",
                false,
                error.clone(),
                None,
            );
            Err(error)
        }
    }
}

fn inspect_local_temp_gateway_config_snapshot(
    openclaw_dir: &std::path::Path,
    temp_profile: &str,
) -> Result<(String, bool, bool, bool, bool), String> {
    let config_path = derive_profile_root_path(openclaw_dir, temp_profile).join("openclaw.json");
    let raw = std::fs::read_to_string(&config_path).map_err(|error| error.to_string())?;
    let json =
        serde_json::from_str::<serde_json::Value>(&raw).map_err(|error| error.to_string())?;
    Ok((
        config_path.display().to_string(),
        json.pointer("/models/providers")
            .and_then(serde_json::Value::as_object)
            .map(|value| !value.is_empty())
            .unwrap_or(false),
        json.pointer("/auth/profiles")
            .and_then(serde_json::Value::as_object)
            .map(|value| !value.is_empty())
            .unwrap_or(false),
        json.pointer("/agents/defaults/model").is_some(),
        json.pointer("/agents/defaults/models").is_some(),
    ))
}

fn probe_local_temp_gateway_agent_smoke(
    temp_profile: &str,
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<(String, String), String> {
    let command = vec![
        "--profile".to_string(),
        temp_profile.to_string(),
        "agent".to_string(),
        "--local".to_string(),
        "--agent".to_string(),
        "main".to_string(),
        "--message".to_string(),
        "Reply with exactly READY.".to_string(),
        "--json".to_string(),
        "--no-color".to_string(),
    ];
    let output = run_openclaw_dynamic(&command)?;
    let ok = output.exit_code == 0;
    let detail = clawpal_core::doctor::command_output_detail(&output.stderr, &output.stdout);
    append_step(
        steps,
        "temp.probe.agent",
        "Probe temporary gateway agent inference",
        ok,
        detail.clone(),
        None,
    );
    if !ok {
        if let Some(reason) = temp_gateway_provider_setup_reason_from_output(&detail) {
            return Err(doctor_assistant_temp_provider_setup_required(reason));
        }
        return Err(command_failure_message(&command, &output));
    }
    let Some((provider, model)) = extract_agent_provider_identity(&output.stdout) else {
        return Err(doctor_assistant_temp_provider_setup_required(
            "Temporary gateway provider check failed: the temp agent responded, but provider/model metadata was missing.",
        ));
    };
    append_step(
        steps,
        "temp.probe.agent.identity",
        "Inspect temporary gateway provider identity",
        true,
        format!("Temporary gateway replied using {provider}/{model}"),
        None,
    );
    Ok((provider, model))
}

fn sync_local_temp_gateway_provider_context(
    temp_profile: &str,
    explicit_profile_id: Option<&str>,
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<(String, String), String> {
    let paths = resolve_paths();
    let donor_load = load_local_donor_cfg_fallback();
    let donor_cfg = donor_load.donor_cfg.clone();
    let provider_keys = collect_provider_keys(&donor_cfg);
    append_step(
        steps,
        "temp.sync.donor_source",
        "Detect donor configuration source",
        true,
        format!(
            "Main donor config={} mode={} defaults_source={} providers={}",
            donor_load.main_config_path,
            donor_load.source_mode,
            donor_load
                .defaults_source_path
                .clone()
                .unwrap_or_else(|| "none".into()),
            if provider_keys.is_empty() {
                "none".into()
            } else {
                provider_keys.join(", ")
            }
        ),
        None,
    );
    let selected_profile = select_doctor_provider_profile(&donor_cfg, explicit_profile_id)?;
    let donor_profiles = selected_profile
        .clone()
        .map(|profile| vec![profile])
        .unwrap_or_else(|| load_local_doctor_provider_profiles());
    let providers = if let Some(profile) = selected_profile.as_ref() {
        let credential = resolve_internal_provider_credential_for_profile(profile);
        if credential.is_none() && !super::provider_supports_optional_api_key(&profile.provider) {
            return Err(doctor_assistant_temp_provider_setup_required(format!(
                "Temporary gateway has no usable credential for {}. Add a provider profile with API key, then continue repair.",
                profile.provider
            )));
        }
        build_single_provider_config(&donor_cfg, profile, credential.as_ref())
    } else {
        let provider_credentials = build_local_preferred_provider_credentials(&paths);
        materialize_temp_gateway_provider_config(&donor_cfg, &provider_credentials)
            .ok_or_else(|| {
                doctor_assistant_temp_provider_setup_required(
                    "Temporary gateway has no provider configuration to copy. Add a provider profile in Doctor, then continue repair.",
                )
            })?
    };
    apply_local_profile_json_value(temp_profile, "models.providers", &providers)?;
    append_step(
        steps,
        "temp.sync.providers",
        "Copy provider configuration into temporary gateway",
        true,
        "Temporary gateway inherited provider configuration from the primary gateway",
        None,
    );

    if let Some(profile) = selected_profile.as_ref() {
        let auth_profiles = build_auth_profiles_for_provider(profile);
        apply_local_profile_json_value(temp_profile, "auth.profiles", &auth_profiles)?;
        append_step(
            steps,
            "temp.sync.auth_profiles",
            "Copy auth profiles into temporary gateway",
            true,
            "Temporary gateway inherited auth.profiles from the primary gateway",
            None,
        );
    } else if let Some(auth_profiles) = donor_cfg.pointer("/auth/profiles").cloned() {
        apply_local_profile_json_value(temp_profile, "auth.profiles", &auth_profiles)?;
        append_step(
            steps,
            "temp.sync.auth_profiles",
            "Copy auth profiles into temporary gateway",
            true,
            "Temporary gateway inherited auth.profiles from the primary gateway",
            None,
        );
    }

    let default_model = resolve_temp_gateway_default_model_value(
        &donor_cfg,
        selected_profile.as_ref(),
        &donor_profiles,
    );
    let default_models = resolve_temp_gateway_default_models_value(
        &donor_cfg,
        selected_profile.as_ref(),
        default_model.as_ref(),
    );

    if let Some(default_model_value) = default_model.as_ref() {
        apply_local_profile_json_value(temp_profile, "agents.defaults.model", default_model_value)?;
        append_step(
            steps,
            "temp.sync.default_model",
            "Copy default model into temporary gateway",
            true,
            "Temporary gateway inherited the primary default model binding",
            None,
        );
    }
    if let Some(default_models_value) = default_models.as_ref() {
        apply_local_profile_json_value(
            temp_profile,
            "agents.defaults.models",
            default_models_value,
        )?;
        append_step(
            steps,
            "temp.sync.default_models",
            "Copy default model registry into temporary gateway",
            true,
            "Temporary gateway inherited the primary default model registry",
            None,
        );
    }

    let (
        written_config_path,
        has_providers,
        has_auth_profiles,
        has_default_model,
        has_default_models,
    ) = inspect_local_temp_gateway_config_snapshot(&paths.openclaw_dir, temp_profile)?;
    append_step(
        steps,
        "temp.sync.verify_snapshot",
        "Verify temporary openclaw.json snapshot",
        true,
        format!(
            "Wrote {} -> providers={} auth_profiles={} default_model={} default_models={}",
            written_config_path,
            has_providers,
            has_auth_profiles,
            has_default_model,
            has_default_models
        ),
        None,
    );
    if !has_providers {
        return Err(format!(
            "Temporary gateway snapshot write did not persist models.providers to {}",
            written_config_path
        ));
    }
    if donor_cfg.pointer("/auth/profiles").is_some() && !has_auth_profiles {
        return Err(format!(
            "Temporary gateway snapshot write did not persist auth.profiles to {}",
            written_config_path
        ));
    }
    if default_model.is_some() && !has_default_model {
        return Err(format!(
            "Temporary gateway snapshot write did not persist agents.defaults.model to {}",
            written_config_path
        ));
    }
    if default_models.is_some() && !has_default_models {
        return Err(format!(
            "Temporary gateway snapshot write did not persist agents.defaults.models to {}",
            written_config_path
        ));
    }

    let copied = copy_local_auth_store_into_profile(
        &paths.openclaw_dir,
        &derive_profile_root_path(&paths.openclaw_dir, temp_profile),
    )?;
    if copied > 0 {
        append_step(
            steps,
            "temp.sync.auth_store",
            "Copy auth store into temporary gateway",
            true,
            format!("Copied {copied} auth store file(s) into the temporary gateway profile"),
            None,
        );
    }
    let _ = probe_local_temp_gateway_inference(&donor_cfg, &donor_profiles, steps);
    probe_local_temp_gateway_agent_smoke(temp_profile, steps)
}

async fn sync_remote_temp_gateway_provider_context(
    pool: &SshConnectionPool,
    host_id: &str,
    temp_profile: &str,
    explicit_profile_id: Option<&str>,
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<(String, String, serde_json::Value), String> {
    let donor_load = load_remote_donor_cfg_fallback(pool, host_id).await;
    let main_config_path = donor_load.main_config_path.clone();
    let donor_cfg = donor_load.donor_cfg.clone();
    let selected_profile = explicit_profile_id
        .map(|profile_id| select_doctor_provider_profile(&donor_cfg, Some(profile_id)))
        .transpose()?
        .flatten();
    let donor_profiles = selected_profile
        .clone()
        .map(|profile| vec![profile])
        .unwrap_or_default();
    let providers = if let Some(profile) = selected_profile.as_ref() {
        let credential = resolve_internal_provider_credential_for_profile(profile);
        if credential.is_none() && !super::provider_supports_optional_api_key(&profile.provider) {
            return Err(doctor_assistant_temp_provider_setup_required(format!(
                "Temporary gateway has no usable credential for {}. Add a provider profile with API key, then continue repair.",
                profile.provider
            )));
        }
        build_single_provider_config(&donor_cfg, profile, credential.as_ref())
    } else if let Some(providers) = donor_cfg.pointer("/models/providers").cloned() {
        providers
    } else {
        return Err(doctor_assistant_temp_provider_setup_required(
            "Temporary gateway has no provider configuration to copy from the primary gateway. Add a provider profile in Doctor, then continue repair.",
        ));
    };
    let auth_profiles = if let Some(profile) = selected_profile.as_ref() {
        Some(build_auth_profiles_for_provider(profile))
    } else {
        donor_cfg.pointer("/auth/profiles").cloned()
    };
    let default_model = resolve_temp_gateway_default_model_value(
        &donor_cfg,
        selected_profile.as_ref(),
        &donor_profiles,
    );
    let default_models = resolve_temp_gateway_default_models_value(
        &donor_cfg,
        selected_profile.as_ref(),
        default_model.as_ref(),
    );

    let main_root = std::path::Path::new(&main_config_path)
        .parent()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "~/.openclaw".into());
    let temp_root = derive_profile_root_string(&main_root, temp_profile);
    let provider_keys = donor_cfg
        .pointer("/models/providers")
        .and_then(serde_json::Value::as_object)
        .map(|providers| providers.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    append_step(
        steps,
        "temp.sync.donor_source",
        "Detect donor configuration source",
        true,
        format!(
            "Main donor config={} mode={} defaults_source={} providers={}",
            main_config_path,
            donor_load.source_mode,
            donor_load
                .defaults_source_path
                .clone()
                .unwrap_or_else(|| "none".into()),
            if provider_keys.is_empty() {
                "none".into()
            } else {
                provider_keys.join(", ")
            }
        ),
        None,
    );
    write_remote_temp_gateway_config_snapshot(
        pool,
        host_id,
        &temp_root,
        providers,
        auth_profiles.clone(),
        default_model.clone(),
        default_models.clone(),
    )
    .await?;
    let (written_config_path, has_providers, has_default_model, has_default_models) =
        inspect_remote_temp_gateway_config_snapshot(pool, host_id, &temp_root).await?;
    append_step(
        steps,
        "temp.sync.providers",
        "Copy provider configuration into temporary gateway",
        true,
        "Temporary gateway snapshot now includes provider configuration from the primary gateway",
        None,
    );
    if auth_profiles.is_some() {
        append_step(
            steps,
            "temp.sync.auth_profiles",
            "Copy auth profiles into temporary gateway",
            true,
            "Temporary gateway snapshot now includes auth.profiles from the primary gateway",
            None,
        );
    }
    if default_model.is_some() {
        append_step(
            steps,
            "temp.sync.default_model",
            "Copy default model into temporary gateway",
            true,
            "Temporary gateway snapshot now includes a default model binding",
            None,
        );
    }
    if default_models.is_some() {
        append_step(
            steps,
            "temp.sync.default_models",
            "Copy default model registry into temporary gateway",
            true,
            "Temporary gateway snapshot now includes the donor default model registry",
            None,
        );
    }
    append_step(
        steps,
        "temp.sync.verify_snapshot",
        "Verify temporary openclaw.json snapshot",
        true,
        format!(
            "Wrote {} -> providers={} default_model={} default_models={}",
            written_config_path, has_providers, has_default_model, has_default_models
        ),
        None,
    );
    if !has_providers {
        return Err(format!(
            "Temporary gateway snapshot write did not persist models.providers to {}",
            written_config_path
        ));
    }
    if default_model.is_some() && !has_default_model {
        return Err(format!(
            "Temporary gateway snapshot write did not persist agents.defaults.model to {}",
            written_config_path
        ));
    }
    if default_models.is_some() && !has_default_models {
        return Err(format!(
            "Temporary gateway snapshot write did not persist agents.defaults.models to {}",
            written_config_path
        ));
    }
    let copied = copy_remote_auth_store_into_profile(pool, host_id, &main_root, &temp_root).await?;
    if copied > 0 {
        append_step(
            steps,
            "temp.sync.auth_store",
            "Copy auth store into temporary gateway",
            true,
            format!("Copied {copied} auth store file(s) into the temporary gateway profile"),
            None,
        );
    }
    Ok((main_root, temp_root, donor_cfg))
}

fn temp_gateway_provider_setup_reason_from_output(detail: &str) -> Option<String> {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("no api key found for provider")
        || lower.contains("auth store:")
        || lower.contains("no enabled model profile")
        || lower.contains("no model configured")
        || lower.contains("unknown model")
        || lower.contains("invalid model")
        || lower.contains("is not a valid model id")
        || lower.contains("model_not_found")
    {
        let concise = detail
            .lines()
            .map(str::trim)
            .find(|line| {
                let lower = line.to_ascii_lowercase();
                lower.contains("no api key found for provider")
                    || lower.contains("auth store:")
                    || lower.contains("invalid model")
                    || lower.contains("valid model id")
                    || lower.contains("model_not_found")
            })
            .unwrap_or("Temporary gateway still has no usable inference provider.")
            .trim_matches('"')
            .to_string();
        Some(format!(
            "Temporary gateway provider check failed: {concise}"
        ))
    } else {
        None
    }
}

fn doctor_assistant_is_remote_exec_timeout(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("russh exec timed out") || lower.contains("exec timed out after")
}

async fn remote_primary_gateway_healthy(
    pool: &SshConnectionPool,
    host_id: &str,
) -> Result<bool, String> {
    if super::remote_read_openclaw_config_text_and_json(pool, host_id)
        .await
        .is_err()
    {
        return Ok(false);
    }
    let status = run_remote_openclaw_dynamic(
        pool,
        host_id,
        build_gateway_status_command(DOCTOR_ASSISTANT_TARGET_PROFILE, true),
    )
    .await?;
    Ok(gateway_output_ok(&status))
}

async fn remote_wait_for_primary_gateway_recovery_after_timeout(
    pool: &SshConnectionPool,
    host_id: &str,
    app: &AppHandle,
    run_id: &str,
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<bool, String> {
    for attempt in 1..=DOCTOR_ASSISTANT_REMOTE_TIMEOUT_RECOVERY_ATTEMPTS {
        emit_doctor_assistant_progress(
            app,
            run_id,
            "agent_repair",
            format!(
                "Repair command timed out; re-checking primary gateway health ({attempt}/{})",
                DOCTOR_ASSISTANT_REMOTE_TIMEOUT_RECOVERY_ATTEMPTS
            ),
            0.87,
            attempt,
            None,
            None,
        );
        match remote_primary_gateway_healthy(pool, host_id).await {
            Ok(true) => {
                append_step(
                    steps,
                    "temp.agent_repair.timeout_recovered",
                    "Confirm primary gateway health after repair timeout",
                    true,
                    "The temporary repair command timed out locally, but the primary gateway recovered successfully.",
                    None,
                );
                return Ok(true);
            }
            Ok(false) => {}
            Err(error) => {
                append_step(
                    steps,
                    format!("temp.agent_repair.timeout_recheck.{attempt}"),
                    "Re-check primary gateway health after repair timeout",
                    false,
                    error,
                    None,
                );
            }
        }
        sleep(Duration::from_millis(
            DOCTOR_ASSISTANT_REMOTE_TIMEOUT_RECOVERY_DELAY_MS,
        ))
        .await;
    }
    append_step(
        steps,
        "temp.agent_repair.timeout_recovered",
        "Confirm primary gateway health after repair timeout",
        false,
        "The temporary repair command timed out and the primary gateway was still unhealthy after post-timeout checks.",
        None,
    );
    Ok(false)
}

fn extract_agent_provider_identity(stdout: &str) -> Option<(String, String)> {
    let value = serde_json::from_str::<serde_json::Value>(stdout).ok()?;
    let provider = value
        .pointer("/meta/agentMeta/provider")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let model = value
        .pointer("/meta/agentMeta/model")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    Some((provider, model))
}

async fn probe_remote_temp_gateway_agent_smoke(
    pool: &SshConnectionPool,
    host_id: &str,
    temp_profile: &str,
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<(String, String), String> {
    let command = vec![
        "--profile".to_string(),
        temp_profile.to_string(),
        "agent".to_string(),
        "--local".to_string(),
        "--agent".to_string(),
        "main".to_string(),
        "--message".to_string(),
        "Reply with exactly READY.".to_string(),
        "--json".to_string(),
        "--no-color".to_string(),
    ];
    let output = run_remote_openclaw_dynamic(pool, host_id, command.clone()).await?;
    let ok = output.exit_code == 0;
    let detail = clawpal_core::doctor::command_output_detail(&output.stderr, &output.stdout);
    append_step(
        steps,
        "temp.probe.agent",
        "Probe temporary gateway agent inference",
        ok,
        detail.clone(),
        None,
    );
    if !ok {
        if let Some(reason) = temp_gateway_provider_setup_reason_from_output(&detail) {
            return Err(doctor_assistant_temp_provider_setup_required(reason));
        }
        return Err(command_failure_message(&command, &output));
    }
    let Some((provider, model)) = extract_agent_provider_identity(&output.stdout) else {
        return Err(doctor_assistant_temp_provider_setup_required(
            "Temporary gateway provider check failed: the temp agent responded, but provider/model metadata was missing.",
        ));
    };
    append_step(
        steps,
        "temp.probe.agent.identity",
        "Inspect temporary gateway provider identity",
        true,
        format!("Temporary gateway replied using {provider}/{model}"),
        None,
    );
    Ok((provider, model))
}

fn run_local_temp_gateway_action(
    action: RescueBotAction,
    profile: &str,
    rescue_port: u16,
    should_configure: bool,
    steps: &mut Vec<RescuePrimaryRepairStep>,
    step_prefix: &str,
) -> Result<(), String> {
    let plan = build_rescue_bot_command_plan(action, profile, rescue_port, should_configure);
    for (index, command) in plan.into_iter().enumerate() {
        let output = run_openclaw_dynamic(&command)?;
        let ok = output.exit_code == 0;
        append_step(
            steps,
            format!("{step_prefix}.{}", index + 1),
            format!(
                "{} temp gateway",
                if matches!(action, RescueBotAction::Unset) {
                    "Cleanup"
                } else {
                    "Prepare"
                }
            ),
            ok,
            build_step_detail(&command, &output),
            Some(command.clone()),
        );
        if !ok {
            if doctor_assistant_is_cleanup_noop(action, &command, &output) {
                continue;
            }
            return Err(command_failure_message(&command, &output));
        }
    }
    Ok(())
}

async fn run_remote_temp_gateway_action(
    pool: &SshConnectionPool,
    host_id: &str,
    action: RescueBotAction,
    profile: &str,
    rescue_port: u16,
    should_configure: bool,
    steps: &mut Vec<RescuePrimaryRepairStep>,
    step_prefix: &str,
) -> Result<(), String> {
    let plan = build_rescue_bot_command_plan(action, profile, rescue_port, should_configure);
    for (index, command) in plan.into_iter().enumerate() {
        let output = run_remote_openclaw_dynamic(pool, host_id, command.clone()).await?;
        let ok = output.exit_code == 0;
        append_step(
            steps,
            format!("{step_prefix}.{}", index + 1),
            format!(
                "{} temp gateway",
                if matches!(action, RescueBotAction::Unset) {
                    "Cleanup"
                } else {
                    "Prepare"
                }
            ),
            ok,
            build_step_detail(&command, &output),
            Some(command.clone()),
        );
        if !ok {
            if doctor_assistant_is_cleanup_noop(action, &command, &output) {
                continue;
            }
            return Err(command_failure_message(&command, &output));
        }
    }
    Ok(())
}

fn run_local_temp_gateway_agent_repair_round(
    app: &AppHandle,
    run_id: &str,
    temp_profile: &str,
    current: &RescuePrimaryDiagnosisResult,
    round: usize,
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<(), String> {
    let paths = resolve_paths();
    let log_excerpt = collect_local_gateway_log_excerpt();
    let config_content = read_local_primary_config_text(DOCTOR_ASSISTANT_TARGET_PROFILE);
    let guidance = tauri::async_runtime::block_on(resolve_local_doc_guidance(
        &build_doc_resolve_request(
            "local",
            "local",
            Some(resolve_openclaw_version()),
            &current.issues,
            config_content.clone(),
            Some(current.summary.headline.clone()),
        ),
        &paths,
    ));
    emit_doctor_assistant_progress(
        app,
        run_id,
        "agent_repair",
        format!("Temporary gateway agent is analyzing logs and docs ({round}/{DOCTOR_ASSISTANT_TEMP_REPAIR_ROUNDS})"),
        0.58 + (round as f32 * 0.03),
        round,
        None,
        None,
    );
    let prompt = build_temp_gateway_agent_repair_prompt(
        &log_excerpt,
        &config_content,
        current,
        Some(&guidance),
    );
    let command = vec![
        "--profile".to_string(),
        temp_profile.to_string(),
        "agent".to_string(),
        "--local".to_string(),
        "--agent".to_string(),
        "main".to_string(),
        "--message".to_string(),
        prompt,
        "--json".to_string(),
        "--no-color".to_string(),
    ];
    let output = run_openclaw_dynamic(&command)?;
    let ok = output.exit_code == 0;
    append_step(
        steps,
        format!("temp.agent_repair.{round}"),
        "Temporary gateway agent repair",
        ok,
        clawpal_core::doctor::command_output_detail(&output.stderr, &output.stdout),
        None,
    );
    if !ok {
        let detail = clawpal_core::doctor::command_output_detail(&output.stderr, &output.stdout);
        if let Some(reason) = temp_gateway_provider_setup_reason_from_output(&detail) {
            return Err(doctor_assistant_temp_provider_setup_required(reason));
        }
        return Err(command_failure_message(&command, &output));
    }
    Ok(())
}

async fn run_remote_temp_gateway_agent_repair_round(
    pool: &SshConnectionPool,
    host_id: &str,
    app: &AppHandle,
    run_id: &str,
    temp_profile: &str,
    current: &RescuePrimaryDiagnosisResult,
    round: usize,
    steps: &mut Vec<RescuePrimaryRepairStep>,
) -> Result<(), String> {
    let paths = resolve_paths();
    let log_excerpt = collect_remote_gateway_log_excerpt(pool, host_id).await;
    let config_content =
        read_remote_primary_config_text(pool, host_id, DOCTOR_ASSISTANT_TARGET_PROFILE).await;
    let remote_version = resolve_remote_openclaw_version(pool, host_id).await;
    let guidance = resolve_remote_doc_guidance(
        pool,
        host_id,
        &build_doc_resolve_request(
            host_id,
            "remote_ssh",
            remote_version,
            &current.issues,
            config_content.clone(),
            Some(current.summary.headline.clone()),
        ),
        &paths,
    )
    .await;
    emit_doctor_assistant_progress(
        app,
        run_id,
        "agent_repair",
        format!("Temporary gateway agent is analyzing logs and docs ({round}/{DOCTOR_ASSISTANT_TEMP_REPAIR_ROUNDS})"),
        0.58 + (round as f32 * 0.03),
        round,
        None,
        None,
    );
    let prompt = build_temp_gateway_agent_repair_prompt(
        &log_excerpt,
        &config_content,
        current,
        Some(&guidance),
    );
    let command = vec![
        "--profile".to_string(),
        temp_profile.to_string(),
        "agent".to_string(),
        "--local".to_string(),
        "--agent".to_string(),
        "main".to_string(),
        "--message".to_string(),
        prompt,
        "--json".to_string(),
        "--no-color".to_string(),
    ];
    let output = run_remote_openclaw_dynamic(pool, host_id, command.clone()).await?;
    let ok = output.exit_code == 0;
    let detail = clawpal_core::doctor::command_output_detail(&output.stderr, &output.stdout);
    append_step(
        steps,
        format!("temp.agent_repair.{round}"),
        "Temporary gateway agent repair",
        ok,
        detail.clone(),
        None,
    );
    if !ok {
        if let Some(reason) = temp_gateway_provider_setup_reason_from_output(&detail) {
            return Err(doctor_assistant_temp_provider_setup_required(reason));
        }
        return Err(format!(
            "Temporary gateway repair round failed: {}",
            detail.lines().next().unwrap_or("unknown error").trim()
        ));
    }
    Ok(())
}

fn build_temp_gateway_record(
    instance_id: &str,
    profile: &str,
    port: u16,
    status: &str,
    main_port: u16,
    last_step: Option<String>,
) -> DoctorTempGatewaySessionRecord {
    DoctorTempGatewaySessionRecord {
        instance_id: instance_id.to_string(),
        profile: profile.to_string(),
        port,
        created_at: format_timestamp_from_unix(unix_timestamp_secs()),
        status: status.to_string(),
        main_profile: DOCTOR_ASSISTANT_TARGET_PROFILE.to_string(),
        main_port,
        last_step,
    }
}

#[tauri::command]
pub async fn diagnose_doctor_assistant(
    app: AppHandle,
) -> Result<RescuePrimaryDiagnosisResult, String> {
    let run_id = Uuid::new_v4().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        diagnose_doctor_assistant_local_impl(&app, &run_id, DOCTOR_ASSISTANT_TARGET_PROFILE)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn remote_diagnose_doctor_assistant(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    app: AppHandle,
) -> Result<RescuePrimaryDiagnosisResult, String> {
    let run_id = Uuid::new_v4().to_string();
    diagnose_doctor_assistant_remote_impl(
        &pool,
        &host_id,
        &app,
        &run_id,
        DOCTOR_ASSISTANT_TARGET_PROFILE,
    )
    .await
}

#[tauri::command]
pub async fn repair_doctor_assistant(
    current_diagnosis: Option<RescuePrimaryDiagnosisResult>,
    temp_provider_profile_id: Option<String>,
    app: AppHandle,
) -> Result<RescuePrimaryRepairResult, String> {
    let run_id = Uuid::new_v4().to_string();
    tauri::async_runtime::spawn_blocking(move || -> Result<RescuePrimaryRepairResult, String> {
        let paths = resolve_paths();
        let before = match current_diagnosis {
            Some(diagnosis) => diagnosis,
            None => diagnose_doctor_assistant_local_impl(
                &app,
                &run_id,
                DOCTOR_ASSISTANT_TARGET_PROFILE,
            )?,
        };
        let attempted_at = format_timestamp_from_unix(unix_timestamp_secs());
        let (selected_issue_ids, skipped_issue_ids) =
            collect_repairable_primary_issue_ids(&before, &before.summary.selected_fix_issue_ids);
        let mut applied_issue_ids = Vec::new();
        let mut failed_issue_ids = Vec::new();
        let mut steps = Vec::new();
        let mut current = before.clone();

        if diagnose_doctor_assistant_status(&before) {
            append_step(
                &mut steps,
                "repair.noop",
                "No automatic repairs needed",
                true,
                "The primary gateway is already healthy",
                None,
            );
            return Ok(doctor_assistant_completed_result(
                attempted_at,
                "temporary".into(),
                selected_issue_ids,
                applied_issue_ids,
                skipped_issue_ids,
                failed_issue_ids,
                steps,
                before.clone(),
                before,
            ));
        }

        if !diagnose_doctor_assistant_status(&current) {
            let temp_profile = choose_temp_gateway_profile_name();
            let temp_port = choose_temp_gateway_port(resolve_main_port_from_diagnosis(&current));
            emit_doctor_assistant_progress(
                &app,
                &run_id,
                "bootstrap_temp_gateway",
                "Bootstrapping temporary gateway",
                0.56,
                0,
                None,
                None,
            );
            upsert_doctor_temp_gateway_record(
                &paths,
                build_temp_gateway_record(
                    DOCTOR_ASSISTANT_TEMP_SCOPE_LOCAL,
                    &temp_profile,
                    temp_port,
                    "bootstrapping",
                    resolve_main_port_from_diagnosis(&current),
                    Some("bootstrap".into()),
                ),
            )?;

            let temp_flow = (|| -> Result<(), String> {
                run_local_temp_gateway_action(
                    RescueBotAction::Set,
                    &temp_profile,
                    temp_port,
                    true,
                    &mut steps,
                    "temp.setup",
                )?;
                write_local_temp_gateway_marker(
                    &paths.openclaw_dir,
                    DOCTOR_ASSISTANT_TEMP_SCOPE_LOCAL,
                    &temp_profile,
                )?;
                emit_doctor_assistant_progress(
                    &app,
                    &run_id,
                    "bootstrap_temp_gateway",
                    "Syncing provider configuration into temporary gateway",
                    0.58,
                    0,
                    None,
                    None,
                );
                let (provider, model) = sync_local_temp_gateway_provider_context(
                    &temp_profile,
                    temp_provider_profile_id.as_deref(),
                    &mut steps,
                )?;
                emit_doctor_assistant_progress(
                    &app,
                    &run_id,
                    "bootstrap_temp_gateway",
                    format!("Temporary gateway ready: {provider}/{model}"),
                    0.64,
                    0,
                    None,
                    None,
                );
                upsert_doctor_temp_gateway_record(
                    &paths,
                    build_temp_gateway_record(
                        DOCTOR_ASSISTANT_TEMP_SCOPE_LOCAL,
                        &temp_profile,
                        temp_port,
                        "repairing",
                        resolve_main_port_from_diagnosis(&current),
                        Some("repair".into()),
                    ),
                )?;

                for round in 1..=DOCTOR_ASSISTANT_TEMP_REPAIR_ROUNDS {
                    run_local_temp_gateway_agent_repair_round(
                        &app,
                        &run_id,
                        &temp_profile,
                        &current,
                        round,
                        &mut steps,
                    )?;
                    let next = diagnose_doctor_assistant_local_impl(
                        &app,
                        &run_id,
                        DOCTOR_ASSISTANT_TARGET_PROFILE,
                    )?;
                    for (issue_id, label) in collect_resolved_issues(&current, &next) {
                        merge_issue_lists(
                            &mut applied_issue_ids,
                            std::iter::once(issue_id.clone()),
                        );
                        emit_doctor_assistant_progress(
                            &app,
                            &run_id,
                            "agent_repair",
                            format!("{label} fixed"),
                            0.6 + (round as f32 * 0.03),
                            round,
                            Some(issue_id),
                            Some(label),
                        );
                    }
                    current = next;
                    if diagnose_doctor_assistant_status(&current) {
                        break;
                    }
                }
                Ok(())
            })();
            let temp_flow_error = temp_flow.as_ref().err().cloned();
            let pending_reason = temp_flow_error
                .as_ref()
                .and_then(|error| doctor_assistant_extract_temp_provider_setup_reason(error));

            emit_doctor_assistant_progress(
                &app,
                &run_id,
                "cleanup",
                "Cleaning up temporary gateway",
                0.94,
                0,
                None,
                None,
            );
            let cleanup_result = run_local_temp_gateway_action(
                RescueBotAction::Unset,
                &temp_profile,
                temp_port,
                false,
                &mut steps,
                "temp.cleanup",
            );
            let _ = remove_doctor_temp_gateway_record(
                &paths,
                DOCTOR_ASSISTANT_TEMP_SCOPE_LOCAL,
                &temp_profile,
            );
            match cleanup_result {
                Ok(()) => match prune_local_temp_gateway_profile_roots(&paths.openclaw_dir) {
                    Ok(removed) => append_step(
                        &mut steps,
                        "temp.cleanup.roots",
                        "Delete temporary gateway profiles",
                        true,
                        if removed.is_empty() {
                            "No temporary gateway profiles remained on disk".into()
                        } else {
                            format!(
                                "Removed {} temporary gateway profile directorie(s)",
                                removed.len()
                            )
                        },
                        None,
                    ),
                    Err(error) => append_step(
                        &mut steps,
                        "temp.cleanup.roots",
                        "Delete temporary gateway profiles",
                        false,
                        error,
                        None,
                    ),
                },
                Err(error) => append_step(
                    &mut steps,
                    "temp.cleanup.error",
                    "Cleanup temporary gateway",
                    false,
                    error,
                    None,
                ),
            }
            if temp_flow_error.is_some() || !diagnose_doctor_assistant_status(&current) {
                let fallback_reason = pending_reason
                    .clone()
                    .or(temp_flow_error.clone())
                    .unwrap_or_else(|| {
                        "Temporary gateway repair finished with remaining issues".into()
                    });
                match fallback_restore_local_primary_config(
                    &app,
                    &run_id,
                    &mut steps,
                    &fallback_reason,
                ) {
                    Ok(Some(next)) => {
                        for (issue_id, label) in collect_resolved_issues(&current, &next) {
                            merge_issue_lists(
                                &mut applied_issue_ids,
                                std::iter::once(issue_id.clone()),
                            );
                            emit_doctor_assistant_progress(
                                &app,
                                &run_id,
                                "cleanup",
                                format!("{label} fixed"),
                                0.94,
                                0,
                                Some(issue_id),
                                Some(label),
                            );
                        }
                        current = next
                    }
                    Ok(None) => {}
                    Err(error) => append_step(
                        &mut steps,
                        "repair.fallback.error",
                        "Fallback restore primary config",
                        false,
                        error,
                        None,
                    ),
                }
            }
            if let Some(reason) = pending_reason {
                if !diagnose_doctor_assistant_status(&current) {
                    emit_doctor_assistant_progress(
                        &app, &run_id, "cleanup", &reason, 0.96, 0, None, None,
                    );
                    return Ok(doctor_assistant_pending_temp_provider_result(
                        attempted_at,
                        temp_profile,
                        selected_issue_ids.clone(),
                        applied_issue_ids.clone(),
                        skipped_issue_ids.clone(),
                        selected_issue_ids
                            .iter()
                            .filter(|id| !applied_issue_ids.contains(id))
                            .cloned()
                            .collect(),
                        steps,
                        before,
                        current,
                        temp_provider_profile_id,
                        reason,
                    ));
                }
            }
        }

        let after =
            diagnose_doctor_assistant_local_impl(&app, &run_id, DOCTOR_ASSISTANT_TARGET_PROFILE)?;
        for (issue_id, _label) in collect_resolved_issues(&current, &after) {
            merge_issue_lists(&mut applied_issue_ids, std::iter::once(issue_id));
        }
        let remaining = after
            .issues
            .iter()
            .map(|issue| issue.id.clone())
            .collect::<Vec<_>>();
        failed_issue_ids = selected_issue_ids
            .iter()
            .filter(|id| remaining.contains(id))
            .cloned()
            .collect();

        emit_doctor_assistant_progress(
            &app,
            &run_id,
            "cleanup",
            if diagnose_doctor_assistant_status(&after) {
                "Repair complete"
            } else {
                "Repair finished with remaining issues"
            },
            1.0,
            0,
            None,
            None,
        );

        Ok(doctor_assistant_completed_result(
            attempted_at,
            current.rescue_profile.clone(),
            selected_issue_ids,
            applied_issue_ids,
            skipped_issue_ids,
            failed_issue_ids,
            steps,
            before,
            after,
        ))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn remote_repair_doctor_assistant(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    current_diagnosis: Option<RescuePrimaryDiagnosisResult>,
    temp_provider_profile_id: Option<String>,
    app: AppHandle,
) -> Result<RescuePrimaryRepairResult, String> {
    let run_id = Uuid::new_v4().to_string();
    let paths = resolve_paths();
    let before = match current_diagnosis {
        Some(diagnosis) => diagnosis,
        None => {
            diagnose_doctor_assistant_remote_impl(
                &pool,
                &host_id,
                &app,
                &run_id,
                DOCTOR_ASSISTANT_TARGET_PROFILE,
            )
            .await?
        }
    };
    let attempted_at = format_timestamp_from_unix(unix_timestamp_secs());
    let (selected_issue_ids, skipped_issue_ids) =
        collect_repairable_primary_issue_ids(&before, &before.summary.selected_fix_issue_ids);
    let mut applied_issue_ids = Vec::new();
    let mut failed_issue_ids = Vec::new();
    let mut steps = Vec::new();
    let mut current = before.clone();

    if diagnose_doctor_assistant_status(&before) {
        append_step(
            &mut steps,
            "repair.noop",
            "No automatic repairs needed",
            true,
            "The primary gateway is already healthy",
            None,
        );
        return Ok(doctor_assistant_completed_result(
            attempted_at,
            "temporary".into(),
            selected_issue_ids,
            applied_issue_ids,
            skipped_issue_ids,
            failed_issue_ids,
            steps,
            before.clone(),
            before,
        ));
    }

    if !diagnose_doctor_assistant_status(&current) {
        let temp_profile = choose_temp_gateway_profile_name();
        let temp_port = choose_temp_gateway_port(resolve_main_port_from_diagnosis(&current));
        emit_doctor_assistant_progress(
            &app,
            &run_id,
            "bootstrap_temp_gateway",
            "Bootstrapping temporary gateway",
            0.56,
            0,
            None,
            None,
        );
        upsert_doctor_temp_gateway_record(
            &paths,
            build_temp_gateway_record(
                &host_id,
                &temp_profile,
                temp_port,
                "bootstrapping",
                resolve_main_port_from_diagnosis(&current),
                Some("bootstrap".into()),
            ),
        )?;

        let mut temp_flow = async {
            run_remote_temp_gateway_action(
                &pool,
                &host_id,
                RescueBotAction::Set,
                &temp_profile,
                temp_port,
                true,
                &mut steps,
                "temp.setup",
            )
            .await?;
            let main_root = resolve_remote_main_root(&pool, &host_id).await;
            if let Err(error) = write_remote_temp_gateway_marker(
                &pool,
                &host_id,
                &main_root,
                &host_id,
                &temp_profile,
            )
            .await
            {
                append_step(
                    &mut steps,
                    "temp.marker",
                    "Mark temporary gateway ownership",
                    false,
                    error,
                    None,
                );
            }
            emit_doctor_assistant_progress(
                &app,
                &run_id,
                "bootstrap_temp_gateway",
                "Syncing provider configuration into temporary gateway",
                0.58,
                0,
                None,
                None,
            );
            let (main_root, temp_root, donor_cfg) = sync_remote_temp_gateway_provider_context(
                &pool,
                &host_id,
                &temp_profile,
                temp_provider_profile_id.as_deref(),
                &mut steps,
            )
            .await?;
            let mut provider_identity = None;
            if let Err(error) = probe_remote_temp_gateway_agent_smoke(
                &pool,
                &host_id,
                &temp_profile,
                &mut steps,
            )
            .await
            {
                let should_retry_from_remote_auth_store = temp_provider_profile_id.is_none()
                    && doctor_assistant_extract_temp_provider_setup_reason(&error).is_some();
                if !should_retry_from_remote_auth_store {
                    return Err(error);
                }
                emit_doctor_assistant_progress(
                    &app,
                    &run_id,
                    "bootstrap_temp_gateway",
                    "Rebuilding temporary gateway provider from remote auth store",
                    0.62,
                    0,
                    None,
                    None,
                );
                rebuild_remote_temp_gateway_provider_context_from_auth_store(
                    &pool,
                    &host_id,
                    &main_root,
                    &temp_root,
                    &donor_cfg,
                    &mut steps,
                )
                .await?;
                probe_remote_temp_gateway_agent_smoke(
                    &pool,
                    &host_id,
                    &temp_profile,
                    &mut steps,
                )
                .await
                .map(|identity| provider_identity = Some(identity))?;
            } else {
                provider_identity = steps
                    .iter()
                    .rev()
                    .find(|step| step.id == "temp.probe.agent.identity")
                    .and_then(|step| {
                        let detail = step.detail.trim();
                        detail
                            .strip_prefix("Temporary gateway replied using ")
                            .and_then(|value| value.split_once('/'))
                            .map(|(provider, model)| (provider.to_string(), model.to_string()))
                    });
            }
            if let Some((provider, model)) = provider_identity.as_ref() {
                emit_doctor_assistant_progress(
                    &app,
                    &run_id,
                    "bootstrap_temp_gateway",
                    format!("Temporary gateway ready: {provider}/{model}"),
                    0.64,
                    0,
                    None,
                    None,
                );
            }
            upsert_doctor_temp_gateway_record(
                &paths,
                build_temp_gateway_record(
                    &host_id,
                    &temp_profile,
                    temp_port,
                    "repairing",
                    resolve_main_port_from_diagnosis(&current),
                    Some("repair".into()),
                ),
            )?;

            if DOCTOR_ASSISTANT_REMOTE_SKIP_AGENT_REPAIR {
                append_step(
                    &mut steps,
                    "temp.debug.skip_agent_repair",
                    "Skip temporary gateway repair loop",
                    true,
                    "Remote Doctor debug mode leaves the primary gateway unchanged after temp bootstrap so the temporary gateway configuration can be inspected in isolation.",
                    None,
                );
            } else {
                for round in 1..=DOCTOR_ASSISTANT_TEMP_REPAIR_ROUNDS {
                    run_remote_temp_gateway_agent_repair_round(
                        &pool,
                        &host_id,
                        &app,
                        &run_id,
                        &temp_profile,
                        &current,
                        round,
                        &mut steps,
                    )
                    .await?;
                    let next = diagnose_doctor_assistant_remote_impl(
                        &pool,
                        &host_id,
                        &app,
                        &run_id,
                        DOCTOR_ASSISTANT_TARGET_PROFILE,
                    )
                    .await?;
                    for (issue_id, label) in collect_resolved_issues(&current, &next) {
                        merge_issue_lists(&mut applied_issue_ids, std::iter::once(issue_id.clone()));
                        emit_doctor_assistant_progress(
                            &app,
                            &run_id,
                            "agent_repair",
                            format!("{label} fixed"),
                            0.6 + (round as f32 * 0.03),
                            round,
                            Some(issue_id),
                            Some(label),
                        );
                    }
                    current = next;
                    if diagnose_doctor_assistant_status(&current) {
                        break;
                    }
                }
            }
            Ok::<(), String>(())
        }
        .await;
        if let Err(error) = temp_flow.as_ref() {
            if doctor_assistant_is_remote_exec_timeout(error) {
                let recovered = remote_wait_for_primary_gateway_recovery_after_timeout(
                    &pool, &host_id, &app, &run_id, &mut steps,
                )
                .await?;
                if recovered {
                    temp_flow = Ok(());
                } else {
                    temp_flow = Err(
                        "Temporary gateway repair timed out before health could be confirmed. Open Gateway Logs and inspect the latest repair output."
                            .into(),
                    );
                }
            }
        }
        let temp_flow_error = temp_flow.as_ref().err().cloned();
        let pending_reason = temp_flow_error
            .as_ref()
            .and_then(|error| doctor_assistant_extract_temp_provider_setup_reason(error));

        emit_doctor_assistant_progress(
            &app,
            &run_id,
            "cleanup",
            "Cleaning up temporary gateway",
            0.94,
            0,
            None,
            None,
        );
        let cleanup_result = run_remote_temp_gateway_action(
            &pool,
            &host_id,
            RescueBotAction::Unset,
            &temp_profile,
            temp_port,
            false,
            &mut steps,
            "temp.cleanup",
        )
        .await;
        let _ = remove_doctor_temp_gateway_record(&paths, &host_id, &temp_profile);
        if let Err(error) = cleanup_result {
            append_step(
                &mut steps,
                "temp.cleanup.error",
                "Cleanup temporary gateway",
                false,
                error,
                None,
            );
        }
        let main_root = resolve_remote_main_root(&pool, &host_id).await;
        match prune_remote_temp_gateway_profile_roots(&pool, &host_id, &main_root).await {
            Ok(removed) => append_step(
                &mut steps,
                "temp.cleanup.roots",
                "Delete temporary gateway profiles",
                true,
                if removed.is_empty() {
                    "No temporary gateway profiles remained on disk".into()
                } else {
                    format!(
                        "Removed {} temporary gateway profile directorie(s)",
                        removed.len()
                    )
                },
                None,
            ),
            Err(error) => append_step(
                &mut steps,
                "temp.cleanup.roots",
                "Delete temporary gateway profiles",
                false,
                error,
                None,
            ),
        }
        if temp_flow_error.is_some() || !diagnose_doctor_assistant_status(&current) {
            let fallback_reason = pending_reason
                .clone()
                .or(temp_flow_error.clone())
                .unwrap_or_else(|| {
                    "Temporary gateway repair finished with remaining issues".into()
                });
            match fallback_restore_remote_primary_config(
                &pool,
                &host_id,
                &app,
                &run_id,
                &mut steps,
                &fallback_reason,
            )
            .await
            {
                Ok(Some(next)) => {
                    for (issue_id, label) in collect_resolved_issues(&current, &next) {
                        merge_issue_lists(
                            &mut applied_issue_ids,
                            std::iter::once(issue_id.clone()),
                        );
                        emit_doctor_assistant_progress(
                            &app,
                            &run_id,
                            "cleanup",
                            format!("{label} fixed"),
                            0.94,
                            0,
                            Some(issue_id),
                            Some(label),
                        );
                    }
                    current = next
                }
                Ok(None) => {}
                Err(error) => append_step(
                    &mut steps,
                    "repair.fallback.error",
                    "Fallback restore primary config",
                    false,
                    error,
                    None,
                ),
            }
        }
        if let Some(reason) = pending_reason {
            if !diagnose_doctor_assistant_status(&current) {
                emit_doctor_assistant_progress(
                    &app, &run_id, "cleanup", &reason, 0.96, 0, None, None,
                );
                return Ok(doctor_assistant_pending_temp_provider_result(
                    attempted_at,
                    temp_profile,
                    selected_issue_ids.clone(),
                    applied_issue_ids.clone(),
                    skipped_issue_ids.clone(),
                    selected_issue_ids
                        .iter()
                        .filter(|id| !applied_issue_ids.contains(id))
                        .cloned()
                        .collect(),
                    steps,
                    before,
                    current,
                    temp_provider_profile_id,
                    reason,
                ));
            }
        }
    }

    let after = diagnose_doctor_assistant_remote_impl(
        &pool,
        &host_id,
        &app,
        &run_id,
        DOCTOR_ASSISTANT_TARGET_PROFILE,
    )
    .await?;
    for (issue_id, _label) in collect_resolved_issues(&current, &after) {
        merge_issue_lists(&mut applied_issue_ids, std::iter::once(issue_id));
    }
    let remaining = after
        .issues
        .iter()
        .map(|issue| issue.id.clone())
        .collect::<Vec<_>>();
    failed_issue_ids = selected_issue_ids
        .iter()
        .filter(|id| remaining.contains(id))
        .cloned()
        .collect();

    emit_doctor_assistant_progress(
        &app,
        &run_id,
        "cleanup",
        if diagnose_doctor_assistant_status(&after) {
            "Repair complete"
        } else {
            "Repair finished with remaining issues"
        },
        1.0,
        0,
        None,
        None,
    );

    Ok(doctor_assistant_completed_result(
        attempted_at,
        current.rescue_profile.clone(),
        selected_issue_ids,
        applied_issue_ids,
        skipped_issue_ids,
        failed_issue_ids,
        steps,
        before,
        after,
    ))
}

fn resolve_main_port_from_diagnosis(diagnosis: &RescuePrimaryDiagnosisResult) -> u16 {
    diagnosis
        .checks
        .iter()
        .find(|check| check.id == "primary.gateway.status")
        .and_then(|check| {
            check
                .detail
                .split(',')
                .find_map(|part| part.trim().strip_prefix("port="))
                .and_then(|value| value.parse::<u16>().ok())
        })
        .unwrap_or(18789)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::OpenClawPaths;
    use std::fs;
    use std::path::{Path, PathBuf};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "clawpal-doctor-assistant-{label}-{}",
                Uuid::new_v4().simple()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn make_paths(temp: &TempDirGuard) -> OpenClawPaths {
        let openclaw_dir = temp.path().join(".openclaw");
        let clawpal_dir = temp.path().join(".clawpal");
        fs::create_dir_all(&openclaw_dir).unwrap();
        fs::create_dir_all(&clawpal_dir).unwrap();
        OpenClawPaths {
            openclaw_dir: openclaw_dir.clone(),
            config_path: openclaw_dir.join("openclaw.json"),
            base_dir: openclaw_dir.clone(),
            clawpal_dir: clawpal_dir.clone(),
            history_dir: clawpal_dir.join("history"),
            metadata_path: clawpal_dir.join("metadata.json"),
            recipe_runtime_dir: clawpal_dir.join("recipe-runtime"),
        }
    }

    fn temp_profile(suffix: &str) -> String {
        format!("{DOCTOR_ASSISTANT_TEMP_PROFILE_PREFIX}{suffix}")
    }

    fn create_profile_dir(root: &Path, profile: &str) -> PathBuf {
        let path = derive_profile_root_path(root, profile);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn sample_record(instance_id: &str, profile: &str) -> DoctorTempGatewaySessionRecord {
        DoctorTempGatewaySessionRecord {
            instance_id: instance_id.to_string(),
            profile: profile.to_string(),
            port: 19899,
            created_at: "2026-03-09T00:00:00Z".into(),
            status: "bootstrapping".into(),
            main_profile: DOCTOR_ASSISTANT_TARGET_PROFILE.into(),
            main_port: 18789,
            last_step: Some("setup".into()),
        }
    }

    #[test]
    fn choose_temp_gateway_profile_name_uses_clawpal_prefix() {
        let profile = choose_temp_gateway_profile_name();
        assert!(profile.starts_with(DOCTOR_ASSISTANT_TEMP_PROFILE_PREFIX));
        assert!(profile.len() > DOCTOR_ASSISTANT_TEMP_PROFILE_PREFIX.len());
    }

    #[test]
    fn derive_profile_root_path_normalizes_openclaw_prefixed_root_name() {
        let root = Path::new("/tmp/example/.openclaw-rescue");
        let derived = derive_profile_root_path(root, &temp_profile("owned"));
        assert_eq!(
            derived,
            Path::new("/tmp/example").join(format!(".openclaw-{}", temp_profile("owned")))
        );
    }

    #[test]
    fn derive_profile_root_path_uses_custom_base_name_when_not_openclaw() {
        let root = Path::new("/tmp/example/custom-root");
        let derived = derive_profile_root_path(root, &temp_profile("owned"));
        assert_eq!(
            derived,
            Path::new("/tmp/example").join(format!("custom-root-{}", temp_profile("owned")))
        );
    }

    #[test]
    fn derive_profile_root_string_matches_path_variant() {
        let root = "/tmp/example/.openclaw";
        let profile = temp_profile("owned");
        assert_eq!(
            derive_profile_root_string(root, &profile),
            derive_profile_root_path(Path::new(root), &profile)
                .to_string_lossy()
                .to_string()
        );
    }

    #[test]
    fn expand_remote_home_path_expands_tilde_prefix() {
        assert_eq!(
            expand_remote_home_path("~/.openclaw/openclaw.json", "/root"),
            "/root/.openclaw/openclaw.json"
        );
        assert_eq!(expand_remote_home_path("~", "/root"), "/root");
    }

    #[test]
    fn expand_remote_home_path_keeps_absolute_paths() {
        assert_eq!(
            expand_remote_home_path("/root/.openclaw/openclaw.json", "/root"),
            "/root/.openclaw/openclaw.json"
        );
    }

    #[test]
    fn salvage_donor_cfg_from_text_recovers_sections_from_malformed_config() {
        let raw = r#"
        {
          "auth": {
            "profiles": {
              "openai-codex:default": { "provider": "openai-codex", "mode": "oauth" }
            }
          },
          "models": {
            "providers": {
              "openrouter": {
                "baseUrl": "https://openrouter.ai/api/v1",
                "apiKey": "sk-test",
                "models": [{ "id": "gpt-5.2" }]
              }
            }
          },
          "agents": {
            "defaults": {
              "model": { "primary": "openai-codex/gpt-5.3-codex" }
            }
          },
          "bindings": [
            { "agentId": "code-review", "match": { "channel": "discord" } ,
            { "agentId": "coder" }
          ]
        }
        "#;

        let salvaged = salvage_donor_cfg_from_text(raw);

        assert_eq!(
            salvaged.pointer("/models/providers/openrouter/apiKey"),
            Some(&serde_json::Value::String("sk-test".into()))
        );
        assert_eq!(
            salvaged.pointer("/auth/profiles/openai-codex:default/provider"),
            Some(&serde_json::Value::String("openai-codex".into()))
        );
        assert_eq!(
            salvaged.pointer("/agents/defaults/model/primary"),
            Some(&serde_json::Value::String(
                "openai-codex/gpt-5.3-codex".into()
            ))
        );
        assert!(salvaged.pointer("/bindings").is_none());
    }

    #[test]
    fn overlay_agent_defaults_uses_first_valid_json_candidate() {
        let mut donor = serde_json::json!({
            "agents": {
                "defaults": {
                    "model": {
                        "primary": "openrouter/gpt-5.2"
                    }
                }
            }
        });
        let candidates = vec![
            "{ invalid".to_string(),
            serde_json::json!({
                "agents": {
                    "defaults": {
                        "model": {
                            "primary": "anthropic/claude-sonnet-4-6"
                        },
                        "models": {
                            "anthropic/claude-sonnet-4-6": {}
                        }
                    }
                }
            })
            .to_string(),
            serde_json::json!({
                "agents": {
                    "defaults": {
                        "model": {
                            "primary": "openrouter/gpt-5.2"
                        }
                    }
                }
            })
            .to_string(),
        ];

        assert!(overlay_agent_defaults_from_first_valid_json(
            &mut donor,
            &candidates
        ));
        assert_eq!(
            donor.pointer("/agents/defaults/model/primary"),
            Some(&serde_json::Value::String(
                "anthropic/claude-sonnet-4-6".into()
            ))
        );
        assert_eq!(
            donor
                .pointer("/agents/defaults/models")
                .and_then(serde_json::Value::as_object)
                .and_then(|models| models.get("anthropic/claude-sonnet-4-6")),
            Some(&serde_json::json!({}))
        );
    }

    #[test]
    fn overlay_agent_defaults_accepts_legacy_agents_default_shape() {
        let mut donor = serde_json::json!({});
        let candidates = vec![serde_json::json!({
            "agents": {
                "default": {
                    "model": {
                        "primary": "openrouter/gpt-5.2"
                    },
                    "models": {
                        "openrouter/gpt-5.2": {}
                    }
                }
            }
        })
        .to_string()];

        assert!(overlay_agent_defaults_from_first_valid_json(
            &mut donor,
            &candidates
        ));
        assert_eq!(
            donor.pointer("/agents/defaults/model/primary"),
            Some(&serde_json::Value::String("openrouter/gpt-5.2".into()))
        );
        assert_eq!(
            donor
                .pointer("/agents/defaults/models")
                .and_then(serde_json::Value::as_object)
                .and_then(|models| models.get("openrouter/gpt-5.2")),
            Some(&serde_json::json!({}))
        );
    }

    #[test]
    fn resolve_temp_gateway_default_values_prefer_donor_defaults_even_when_provider_mismatches() {
        let donor_cfg = serde_json::json!({
            "models": {
                "providers": {
                    "openrouter": {
                        "models": [{ "id": "gpt-5.2" }]
                    }
                }
            },
            "agents": {
                "defaults": {
                    "model": {
                        "primary": "openai-codex/gpt-5.3-codex"
                    },
                    "models": {
                        "openai-codex/gpt-5.3-codex": {}
                    }
                }
            }
        });

        let default_model =
            resolve_temp_gateway_default_model_value(&donor_cfg, None, &[]).unwrap();
        let default_models =
            resolve_temp_gateway_default_models_value(&donor_cfg, None, Some(&default_model))
                .unwrap();

        assert_eq!(
            default_model.pointer("/primary"),
            Some(&serde_json::Value::String(
                "openai-codex/gpt-5.3-codex".into()
            ))
        );
        assert_eq!(
            default_models
                .as_object()
                .and_then(|models| models.get("openai-codex/gpt-5.3-codex")),
            Some(&serde_json::json!({}))
        );
    }

    #[test]
    fn build_default_models_from_default_model_value_creates_single_entry_map() {
        let default_model = serde_json::json!({
            "primary": "anthropic/claude-sonnet-4-6"
        });

        let default_models = build_default_models_from_default_model_value(&default_model).unwrap();

        assert_eq!(
            default_models
                .as_object()
                .and_then(|models| models.get("anthropic/claude-sonnet-4-6")),
            Some(&serde_json::json!({}))
        );
    }

    #[test]
    fn build_temp_gateway_default_model_value_falls_back_to_first_provider_model_when_primary_is_invalid(
    ) {
        let donor_cfg = serde_json::json!({
            "models": {
                "providers": {
                    "openrouter": {
                        "baseUrl": "https://openrouter.ai/api/v1",
                        "apiKey": "sk-test",
                        "models": [
                            { "id": "gpt-5.2" }
                        ]
                    }
                }
            },
            "agents": {
                "defaults": {
                    "model": {
                        "primary": "openai-codex/gpt-5.3-codex"
                    }
                }
            }
        });

        let value = build_temp_gateway_default_model_value(&donor_cfg, &[]).unwrap();

        assert_eq!(
            value.pointer("/primary"),
            Some(&serde_json::Value::String("openrouter/gpt-5.2".into()))
        );
    }

    #[test]
    fn set_json_object_path_creates_nested_objects_and_overwrites_scalars() {
        let mut cfg = serde_json::json!({
            "agents": "broken"
        });

        set_json_object_path(
            &mut cfg,
            &["models", "providers"],
            serde_json::json!({
                "openrouter": {
                    "apiKey": "sk-test"
                }
            }),
        );
        set_json_object_path(
            &mut cfg,
            &["agents", "defaults", "model"],
            serde_json::json!({
                "primary": "openrouter/gpt-5.2"
            }),
        );

        assert_eq!(
            cfg.pointer("/models/providers/openrouter/apiKey"),
            Some(&serde_json::Value::String("sk-test".into()))
        );
        assert_eq!(
            cfg.pointer("/agents/defaults/model/primary"),
            Some(&serde_json::Value::String("openrouter/gpt-5.2".into()))
        );
    }

    #[test]
    fn write_local_temp_gateway_marker_creates_marker_and_detects_ownership() {
        let temp = TempDirGuard::new("marker");
        let paths = make_paths(&temp);
        let profile = temp_profile("owned");

        assert!(!local_temp_gateway_has_marker(
            &paths.openclaw_dir,
            &profile
        ));

        write_local_temp_gateway_marker(&paths.openclaw_dir, "ssh:hetzner", &profile).unwrap();

        let marker_path = derive_profile_root_path(&paths.openclaw_dir, &profile)
            .join(DOCTOR_ASSISTANT_TEMP_MARKER_FILE);
        let marker_text = fs::read_to_string(marker_path).unwrap();
        assert!(local_temp_gateway_has_marker(&paths.openclaw_dir, &profile));
        assert!(marker_text.contains("owner=clawpal-doctor-assistant"));
        assert!(marker_text.contains("instance_id=ssh:hetzner"));
        assert!(marker_text.contains(&format!("profile={profile}")));
    }

    #[test]
    fn list_local_temp_gateway_profiles_returns_only_marked_clawpal_profiles() {
        let temp = TempDirGuard::new("list");
        let paths = make_paths(&temp);
        let owned = temp_profile("owned");
        let unmarked = temp_profile("unmarked");
        let legacy = "repair-legacy";

        write_local_temp_gateway_marker(&paths.openclaw_dir, "ssh:hetzner", &owned).unwrap();
        create_profile_dir(&paths.openclaw_dir, &unmarked);
        write_local_temp_gateway_marker(&paths.openclaw_dir, "ssh:hetzner", legacy).unwrap();

        let mut profiles = list_local_temp_gateway_profiles(&paths.openclaw_dir).unwrap();
        profiles.sort();

        assert_eq!(profiles, vec![owned]);
    }

    #[test]
    fn list_local_temp_gateway_profiles_returns_empty_for_missing_parent() {
        let temp = TempDirGuard::new("missing-parent");
        let root = temp.path().join("nested").join(".openclaw");
        let profiles = list_local_temp_gateway_profiles(&root).unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn prune_local_temp_gateway_profile_roots_removes_only_marked_owned_profiles() {
        let temp = TempDirGuard::new("prune");
        let paths = make_paths(&temp);
        let owned = temp_profile("owned");
        let unmarked = temp_profile("unmarked");
        let legacy = "repair-legacy";

        let owned_path = create_profile_dir(&paths.openclaw_dir, &owned);
        write_local_temp_gateway_marker(&paths.openclaw_dir, "ssh:hetzner", &owned).unwrap();
        let unmarked_path = create_profile_dir(&paths.openclaw_dir, &unmarked);
        let legacy_path = create_profile_dir(&paths.openclaw_dir, legacy);
        write_local_temp_gateway_marker(&paths.openclaw_dir, "ssh:hetzner", legacy).unwrap();

        let removed = prune_local_temp_gateway_profile_roots(&paths.openclaw_dir).unwrap();

        assert_eq!(removed, vec![owned_path.clone()]);
        assert!(!owned_path.exists());
        assert!(unmarked_path.exists());
        assert!(legacy_path.exists());
    }

    #[test]
    fn prune_local_temp_gateway_profile_roots_returns_empty_when_nothing_owned() {
        let temp = TempDirGuard::new("prune-empty");
        let paths = make_paths(&temp);
        create_profile_dir(&paths.openclaw_dir, &temp_profile("unmarked"));

        let removed = prune_local_temp_gateway_profile_roots(&paths.openclaw_dir).unwrap();

        assert!(removed.is_empty());
    }

    #[test]
    fn save_doctor_temp_gateway_store_deletes_file_when_empty() {
        let temp = TempDirGuard::new("store-empty");
        let paths = make_paths(&temp);
        let store_path = doctor_temp_gateway_store_path(&paths);

        save_doctor_temp_gateway_store(&paths, &DoctorTempGatewaySessionStore::default()).unwrap();

        assert!(!store_path.exists());
    }

    #[test]
    fn remove_doctor_temp_gateway_record_deletes_store_when_last_record_removed() {
        let temp = TempDirGuard::new("store-remove-last");
        let paths = make_paths(&temp);
        let store_path = doctor_temp_gateway_store_path(&paths);
        let record = sample_record("ssh:hetzner", &temp_profile("owned"));

        upsert_doctor_temp_gateway_record(&paths, record.clone()).unwrap();
        assert!(store_path.exists());

        remove_doctor_temp_gateway_record(&paths, &record.instance_id, &record.profile).unwrap();

        assert!(!store_path.exists());
    }

    #[test]
    fn remove_doctor_temp_gateway_records_for_instance_keeps_other_instances() {
        let temp = TempDirGuard::new("store-instance");
        let paths = make_paths(&temp);
        let owned = sample_record("ssh:hetzner", &temp_profile("owned"));
        let other = sample_record("ssh:other", &temp_profile("other"));

        upsert_doctor_temp_gateway_record(&paths, owned.clone()).unwrap();
        upsert_doctor_temp_gateway_record(&paths, other.clone()).unwrap();

        remove_doctor_temp_gateway_records_for_instance(&paths, "ssh:hetzner").unwrap();

        let store = load_doctor_temp_gateway_store(&paths);
        assert_eq!(store.sessions.len(), 1);
        assert_eq!(store.sessions[0].instance_id, "ssh:other");
        assert_eq!(store.sessions[0].profile, other.profile);
    }

    #[test]
    fn build_remote_auth_store_provider_fallback_snapshot_prefers_manual_provider_with_model_ref() {
        let donor_cfg = serde_json::json!({
            "models": {
                "providers": {
                    "openrouter": {
                        "baseUrl": "https://openrouter.ai/api/v1",
                        "models": [
                            { "id": "gpt-5.2" }
                        ]
                    }
                }
            },
            "agents": {
                "defaults": {
                    "model": {
                        "primary": "openrouter/gpt-5.2"
                    }
                },
                "list": [
                    {
                        "name": "main",
                        "model": {
                            "primary": "anthropic/claude-sonnet-4-6"
                        }
                    }
                ]
            }
        });
        let auth_store_values = vec![serde_json::json!({
            "profiles": {
                "openrouter:default": {
                    "provider": "openrouter",
                    "type": "api_key",
                    "key": "or-key"
                },
                "anthropic:manual": {
                    "provider": "anthropic",
                    "type": "token",
                    "token": "sk-ant-api-key"
                }
            }
        })];

        let (providers, auth_profiles, default_model, provider_key) =
            build_remote_auth_store_provider_fallback_snapshot(&donor_cfg, &auth_store_values)
                .unwrap();

        assert_eq!(provider_key, "anthropic");
        assert_eq!(
            providers.pointer("/anthropic/apiKey"),
            Some(&serde_json::Value::String("sk-ant-api-key".into()))
        );
        assert_eq!(
            providers.pointer("/anthropic/baseUrl"),
            Some(&serde_json::Value::String(
                "https://api.anthropic.com/v1".into()
            ))
        );
        assert_eq!(
            default_model
                .as_ref()
                .and_then(|value| value.pointer("/primary")),
            Some(&serde_json::Value::String(
                "anthropic/claude-sonnet-4-6".into()
            ))
        );
        assert!(auth_profiles.is_none());
    }

    #[test]
    fn collect_provider_model_refs_reads_agent_list_refs() {
        let donor_cfg = serde_json::json!({
            "agents": {
                "list": [
                    {
                        "name": "main",
                        "model": {
                            "primary": "anthropic/claude-sonnet-4-6"
                        }
                    },
                    {
                        "name": "backup",
                        "model": "anthropic/claude-3-7-sonnet"
                    }
                ]
            }
        });

        let refs = collect_provider_model_refs(&donor_cfg, "anthropic");

        assert_eq!(
            refs,
            vec![
                "anthropic/claude-sonnet-4-6".to_string(),
                "anthropic/claude-3-7-sonnet".to_string()
            ]
        );
    }

    #[test]
    fn temp_gateway_provider_setup_reason_detects_invalid_model_responses() {
        let reason = temp_gateway_provider_setup_reason_from_output(
            "400 openrouter/gpt-5.2 is not a valid model ID",
        );
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("valid model ID"));
    }

    #[test]
    fn extract_agent_provider_identity_reads_meta_provider_and_model() {
        let raw = serde_json::json!({
            "payloads": [{"text": "READY"}],
            "meta": {
                "agentMeta": {
                    "provider": "anthropic",
                    "model": "claude-sonnet-4-6"
                }
            }
        })
        .to_string();

        let identity = extract_agent_provider_identity(&raw);

        assert_eq!(
            identity,
            Some(("anthropic".to_string(), "claude-sonnet-4-6".to_string()))
        );
    }

    #[test]
    fn build_temp_gateway_agent_repair_prompt_includes_logs_docs_and_diagnosis() {
        let diagnosis = RescuePrimaryDiagnosisResult {
            status: "broken".into(),
            checked_at: "2026-03-09T00:00:00Z".into(),
            target_profile: DOCTOR_ASSISTANT_TARGET_PROFILE.into(),
            rescue_profile: "temporary".into(),
            rescue_configured: false,
            rescue_port: None,
            summary: RescuePrimarySummary {
                status: "broken".into(),
                headline: "Primary gateway is not healthy".into(),
                recommended_action: "Inspect logs".into(),
                fixable_issue_count: 1,
                selected_fix_issue_ids: vec!["primary.gateway.unhealthy".into()],
                root_cause_hypotheses: vec![],
                fix_steps: vec![],
                confidence: None,
                citations: vec![],
                version_awareness: None,
            },
            sections: vec![],
            checks: vec![],
            issues: vec![RescuePrimaryIssue {
                id: "primary.gateway.unhealthy".into(),
                code: "gateway.unhealthy".into(),
                severity: "error".into(),
                message: "gateway stopped".into(),
                auto_fixable: true,
                fix_hint: None,
                source: "primary".into(),
            }],
        };
        let guidance = crate::openclaw_doc_resolver::DocGuidance {
            status: "grounded".into(),
            source_strategy: "local-first".into(),
            root_cause_hypotheses: vec![crate::openclaw_doc_resolver::RootCauseHypothesis {
                title: "Config syntax broken".into(),
                reason: "Gateway log points at JSON5 parse error".into(),
                score: 0.97,
            }],
            fix_steps: vec!["Repair malformed JSON5 before restart.".into()],
            confidence: 0.97,
            citations: vec![crate::openclaw_doc_resolver::DocCitation {
                url: "https://docs.openclaw.ai/gateway/configuration-reference.md".into(),
                section: "Configuration Reference".into(),
            }],
            version_awareness: "match".into(),
            resolver_meta: crate::openclaw_doc_resolver::ResolverMeta {
                cache_hit: false,
                sources_checked: vec!["local".into()],
                rules_matched: vec!["gateway".into()],
                fetched_pages: 1,
                fallback_used: false,
            },
        };

        let prompt = build_temp_gateway_agent_repair_prompt(
            "==> /tmp/openclaw/openclaw-2026-03-09.log <==\nparse error",
            "{\"gateway\":{\"port\":18789}}",
            &diagnosis,
            Some(&guidance),
        );

        assert!(prompt.contains("Relevant gateway logs from /tmp/openclaw/*"));
        assert!(prompt.contains("Docs guidance:"));
        assert!(prompt.contains("Primary gateway is not healthy"));
        assert!(prompt.contains("Configuration Reference"));
    }
}
