use crate::doctor::classify_engine_error;
use crate::runtime::types::{
    RuntimeAdapter, RuntimeError, RuntimeErrorCode, RuntimeEvent, RuntimeSessionKey,
};
use serde_json::json;

use super::process::run_zeroclaw_message;
use super::session::{append_history, build_prompt_with_history_preamble, reset_history};
use super::streaming::run_zeroclaw_streaming_turn;

pub struct ZeroclawInstallAdapter;

impl ZeroclawInstallAdapter {
    fn install_domain_prompt(key: &RuntimeSessionKey, user_system_prompt: &str) -> String {
        let target_line = if key.instance_id == "local" {
            "Target is local machine."
        } else {
            "Target is a non-local instance selected in ClawPal."
        };
        let template = crate::prompt_templates::install_domain_system();
        crate::prompt_templates::render_template(
            &template,
            &[
                ("{{target_line}}", target_line),
                ("{{instance_id}}", key.instance_id.as_str()),
                ("{{message}}", user_system_prompt),
            ],
        )
    }

    fn parse_tool_intent(raw: &str) -> Option<(RuntimeEvent, String)> {
        let intent = crate::runtime::zeroclaw::tool_intent::parse_tool_intent(raw)?;
        let reason = intent
            .reason
            .unwrap_or_else(|| "Executing command for installation.".to_string());
        let invoke_type =
            crate::runtime::zeroclaw::tool_intent::classify_invoke_type(&intent.tool, &intent.args);
        let payload = json!({
            "id": format!("zc-{}", uuid::Uuid::new_v4()),
            "command": intent.tool,
            "args": {
                "args": intent.args,
                "instance": intent.instance.unwrap_or_default(),
            },
            "type": invoke_type,
        });
        let note = format!(
            "Running: `{} {}`\nReason: {}",
            payload["command"].as_str().unwrap_or(""),
            payload["args"]["args"].as_str().unwrap_or(""),
            reason
        );
        Some((RuntimeEvent::Invoke { payload }, note))
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

impl ZeroclawInstallAdapter {
    pub async fn start_streaming<F>(
        &self,
        key: &RuntimeSessionKey,
        message: &str,
        on_delta: F,
    ) -> Result<Vec<RuntimeEvent>, RuntimeError>
    where
        F: Fn(&str) + Send + 'static,
    {
        let session_key = key.storage_key();
        reset_history(&session_key);
        let prompt = Self::install_domain_prompt(key, message);
        let assistant_events = run_zeroclaw_streaming_turn(
            key,
            &prompt,
            true,
            None,
            on_delta,
            |text| text,
            Self::parse_tool_intent,
            Self::map_error,
        )
        .await?;
        append_history(&session_key, "system", &prompt);
        Ok(assistant_events)
    }

    pub async fn send_streaming<F>(
        &self,
        key: &RuntimeSessionKey,
        message: &str,
        on_delta: F,
    ) -> Result<Vec<RuntimeEvent>, RuntimeError>
    where
        F: Fn(&str) + Send + 'static,
    {
        let session_key = key.storage_key();
        append_history(&session_key, "user", message);
        let preamble = format!("{}\n", crate::prompt_templates::install_history_preamble());
        let prompt = build_prompt_with_history_preamble(&session_key, message, &preamble);
        let guarded = Self::install_domain_prompt(key, &prompt);
        let assistant_events = run_zeroclaw_streaming_turn(
            key,
            &guarded,
            false,
            Some(message),
            on_delta,
            |text| text,
            Self::parse_tool_intent,
            Self::map_error,
        )
        .await?;
        Ok(assistant_events)
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
        let text = run_zeroclaw_message(&prompt, &key.instance_id, &key.storage_key())
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
        let preamble = format!("{}\n", crate::prompt_templates::install_history_preamble());
        let prompt = build_prompt_with_history_preamble(&session_key, message, &preamble);
        let guarded = Self::install_domain_prompt(key, &prompt);
        let text = run_zeroclaw_message(&guarded, &key.instance_id, &key.storage_key())
            .map_err(Self::map_error)?;
        if let Some((invoke, note)) = Self::parse_tool_intent(&text) {
            append_history(&session_key, "assistant", &note);
            return Ok(vec![RuntimeEvent::chat_final(note), invoke]);
        }
        append_history(&session_key, "assistant", &text);
        Ok(vec![RuntimeEvent::chat_final(text)])
    }
}
