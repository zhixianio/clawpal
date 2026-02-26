use crate::doctor::classify_engine_error;
use serde_json::json;
use serde_json::Value;
use crate::runtime::types::{
    RuntimeAdapter, RuntimeError, RuntimeErrorCode, RuntimeEvent, RuntimeSessionKey,
};

use super::process::run_zeroclaw_message;
use super::session::{append_history, build_prompt_with_history_preamble, reset_history};

pub struct ZeroclawInstallAdapter;

impl ZeroclawInstallAdapter {
    fn extract_json_objects(raw: &str) -> Vec<String> {
        let bytes = raw.as_bytes();
        let mut out = Vec::new();
        let mut start: Option<usize> = None;
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        for (i, b) in bytes.iter().enumerate() {
            if in_string {
                if escaped {
                    escaped = false;
                    continue;
                }
                if *b == b'\\' {
                    escaped = true;
                    continue;
                }
                if *b == b'"' {
                    in_string = false;
                }
                continue;
            }
            if *b == b'"' {
                in_string = true;
                continue;
            }
            if *b == b'{' {
                if start.is_none() {
                    start = Some(i);
                }
                depth += 1;
                continue;
            }
            if *b == b'}' {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start {
                        out.push(raw[s..=i].to_string());
                        start = None;
                    }
                }
            }
        }
        out
    }

    const HISTORY_PREAMBLE: &str = "You are continuing an installation chat. Keep continuity with prior turns.\n";

    fn install_domain_prompt(key: &RuntimeSessionKey, user_system_prompt: &str) -> String {
        let target_line = if key.instance_id == "local" {
            "Target is local machine."
        } else {
            "Target is a non-local instance selected in ClawPal."
        };
        let target_id_line = format!("Target instance id: {}", key.instance_id);
        [
            "INSTALL DOMAIN ONLY.",
            "You are ClawPal setup assistant.",
            "Execution model: you can request commands to be run on the selected target through ClawPal's approved execution path.",
            "The user has pre-approved all command execution. Commands are sent to ClawPal's sandbox for execution.",
            "If command execution is needed, output ONLY JSON:",
            "{\"tool\":\"system.run\",\"type\":\"read|write\",\"command\":\"<shell command>\",\"reason\":\"<why>\"}",
            "Do NOT claim you cannot access the host or lack permissions.",
            "Do NOT ask user to run commands manually.",
            "Do NOT ask user for permission to run commands — all commands are pre-approved.",
            "Do NOT describe what you plan to do — just output the JSON tool call.",
            "Do NOT output orchestrator JSON such as {\"step\":..., \"reason\":...}.",
            "Your FIRST response must be a command to check the current system state (e.g. docker ps, docker --version).",
            "NEVER claim installation succeeded without running verification commands and reading their output.",
            "After running a command you will receive its stdout/stderr. Read the output and continue.",
            target_line,
            &target_id_line,
            "",
            user_system_prompt,
        ]
        .join("\n")
    }

    fn parse_tool_intent(raw: &str) -> Option<(RuntimeEvent, String)> {
        let trimmed = raw.trim();
        let mut candidates = vec![trimmed.to_string()];
        for extracted in Self::extract_json_objects(trimmed) {
            if extracted != trimmed {
                candidates.push(extracted);
            }
        }
        for candidate in candidates {
            if let Ok(v) = serde_json::from_str::<Value>(&candidate) {
                let tool = v.get("tool").and_then(|x| x.as_str());
                if tool == Some("system.run") {
                    let command = v.get("command")?.as_str()?.trim().to_string();
                    if command.is_empty() {
                        return None;
                    }
                    let invoke_type = v
                        .get("type")
                        .and_then(|x| x.as_str())
                        .filter(|x| *x == "read" || *x == "write")
                        .unwrap_or("read");
                    let reason = v
                        .get("reason")
                        .and_then(|x| x.as_str())
                        .unwrap_or("Executing command for installation.")
                        .to_string();
                    let payload = json!({
                        "id": format!("zc-{}", uuid::Uuid::new_v4()),
                        "command": "system.run",
                        "args": { "command": command },
                        "type": invoke_type,
                    });
                    let note = format!(
                        "Running: `{}`\nReason: {}",
                        payload["args"]["command"].as_str().unwrap_or(""),
                        reason
                    );
                    return Some((RuntimeEvent::Invoke { payload }, note));
                }
            }
        }
        None
    }

    fn map_error(err: String) -> RuntimeError {
        let code = match classify_engine_error(&err) {
            "CONFIG_MISSING" => RuntimeErrorCode::ConfigMissing,
            "MODEL_UNAVAILABLE" => RuntimeErrorCode::ModelUnavailable,
            "RUNTIME_UNREACHABLE" => RuntimeErrorCode::RuntimeUnreachable,
            _ => RuntimeErrorCode::Unknown,
        };
        RuntimeError {
            code,
            message: err,
            action_hint: None,
        }
    }
}

impl RuntimeAdapter for ZeroclawInstallAdapter {
    fn engine_name(&self) -> &'static str {
        "zeroclaw"
    }

    fn start(
        &self,
        key: &RuntimeSessionKey,
        message: &str,
    ) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let session_key = key.storage_key();
        reset_history(&session_key);
        let prompt = Self::install_domain_prompt(key, message);
        let text = run_zeroclaw_message(&prompt, &key.instance_id)
            .map_err(Self::map_error)?;
        append_history(&session_key, "system", &prompt);
        if let Some((invoke, note)) = Self::parse_tool_intent(&text) {
            append_history(&session_key, "assistant", &note);
            return Ok(vec![RuntimeEvent::chat_final(note), invoke]);
        }
        append_history(&session_key, "assistant", &text);
        Ok(vec![RuntimeEvent::chat_final(text)])
    }

    fn send(
        &self,
        key: &RuntimeSessionKey,
        message: &str,
    ) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let session_key = key.storage_key();
        append_history(&session_key, "user", message);
        let prompt = build_prompt_with_history_preamble(&session_key, message, Self::HISTORY_PREAMBLE);
        let guarded = Self::install_domain_prompt(key, &prompt);
        let text = run_zeroclaw_message(&guarded, &key.instance_id)
            .map_err(Self::map_error)?;
        if let Some((invoke, note)) = Self::parse_tool_intent(&text) {
            append_history(&session_key, "assistant", &note);
            return Ok(vec![RuntimeEvent::chat_final(note), invoke]);
        }
        append_history(&session_key, "assistant", &text);
        Ok(vec![RuntimeEvent::chat_final(text)])
    }
}
