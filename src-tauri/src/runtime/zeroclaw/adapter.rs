use crate::doctor::classify_engine_error;
use crate::json_util::extract_json_objects;
use crate::runtime::types::{
    RuntimeAdapter, RuntimeError, RuntimeErrorCode, RuntimeEvent, RuntimeSessionKey,
};
use serde_json::json;
use serde_json::Value;

use super::process::{run_zeroclaw_message, run_zeroclaw_message_streaming};
use super::session::{append_history, build_prompt_with_history, reset_history};
use super::streaming::run_zeroclaw_streaming_turn;

pub struct ZeroclawDoctorAdapter;

impl ZeroclawDoctorAdapter {
    fn infer_language_rule(message: &str) -> &'static str {
        if message.contains("Chinese (简体中文)") || message.contains("简体中文") {
            return "Simplified Chinese (简体中文)";
        }
        let cjk_count = message
            .chars()
            .filter(|ch| ('\u{4e00}'..='\u{9fff}').contains(ch))
            .count();
        if cjk_count >= 4 {
            "Simplified Chinese (简体中文)"
        } else {
            "English"
        }
    }

    fn doctor_domain_prompt(key: &RuntimeSessionKey, message: &str) -> String {
        let target_line = if key.instance_id == "local" {
            "Target is local machine."
        } else {
            "Target is a non-local instance selected in ClawPal."
        };
        let language_rule = Self::infer_language_rule(message);
        let template = crate::prompt_templates::doctor_domain_system();
        crate::prompt_templates::render_template(
            &template,
            &[
                ("{{language_rule}}", language_rule),
                ("{{target_line}}", target_line),
                ("{{instance_id}}", key.instance_id.as_str()),
                ("{{message}}", message),
            ],
        )
    }

    fn normalize_doctor_output(raw: String) -> String {
        let trimmed = raw.trim();
        let mut candidates = vec![trimmed.to_string()];
        for extracted in extract_json_objects(trimmed) {
            if extracted != trimmed {
                candidates.push(extracted);
            }
        }
        for candidate in candidates {
            if let Ok(v) = serde_json::from_str::<Value>(&candidate) {
                let step = v.get("step").and_then(|x| x.as_str());
                let reason = v.get("reason").and_then(|x| x.as_str());
                if step.is_some() && reason.is_some() {
                    return format!(
                        "当前是 Doctor 诊断模式，不执行安装编排。诊断建议：{}",
                        reason.unwrap_or("请先收集错误日志并确认运行状态。")
                    );
                }
            }
        }
        raw
    }

    fn parse_tool_intent(raw: &str) -> Option<(RuntimeEvent, String)> {
        let intent = crate::runtime::zeroclaw::tool_intent::parse_tool_intent(raw)?;
        let reason = intent
            .reason
            .unwrap_or_else(|| "需要执行命令以继续诊断。".to_string());
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
            "建议执行诊断命令：`{} {}`\n原因：{}",
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
            "SESSION_INVALID" => RuntimeErrorCode::SessionInvalid,
            "TARGET_UNREACHABLE" => RuntimeErrorCode::TargetUnreachable,
            "AUTH_EXPIRED" => RuntimeErrorCode::AuthExpired,
            "AUTH_MISCONFIGURED" => RuntimeErrorCode::AuthMisconfigured,
            "REGISTRY_CORRUPT" => RuntimeErrorCode::RegistryCorrupt,
            "INSTANCE_ORPHANED" => RuntimeErrorCode::InstanceOrphaned,
            "TRANSPORT_STALE" => RuntimeErrorCode::TransportStale,
            _ => RuntimeErrorCode::Unknown,
        };
        RuntimeError {
            code,
            message: err,
            action_hint: None,
        }
    }
}

impl ZeroclawDoctorAdapter {
    pub async fn start_streaming<F>(
        &self,
        key: &RuntimeSessionKey,
        message: &str,
        on_delta: F,
    ) -> Result<Vec<RuntimeEvent>, RuntimeError>
    where
        F: Fn(&str) + Send + 'static,
    {
        let prompt = Self::doctor_domain_prompt(key, message);
        let assistant_events = run_zeroclaw_streaming_turn(
            key,
            &prompt,
            true,
            None,
            on_delta,
            Self::normalize_doctor_output,
            Self::parse_tool_intent,
            Self::map_error,
        )
        .await?;
        let session_key = key.storage_key();
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
        let prompt = build_prompt_with_history(&key.storage_key(), message);
        let guarded = Self::doctor_domain_prompt(key, &prompt);
        let assistant_events = run_zeroclaw_streaming_turn(
            key,
            &guarded,
            false,
            Some(message),
            on_delta,
            Self::normalize_doctor_output,
            Self::parse_tool_intent,
            Self::map_error,
        )
        .await?;
        Ok(assistant_events)
    }
}

impl RuntimeAdapter for ZeroclawDoctorAdapter {
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
        let prompt = Self::doctor_domain_prompt(key, message);
        let text = run_zeroclaw_message(&prompt, &key.instance_id, &key.storage_key())
            .map(Self::normalize_doctor_output)
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
        let prompt = build_prompt_with_history(&session_key, message);
        let guarded = Self::doctor_domain_prompt(key, &prompt);
        let text = run_zeroclaw_message(&guarded, &key.instance_id, &key.storage_key())
            .map(Self::normalize_doctor_output)
            .map_err(Self::map_error)?;
        if let Some((invoke, note)) = Self::parse_tool_intent(&text) {
            append_history(&session_key, "assistant", &note);
            return Ok(vec![RuntimeEvent::chat_final(note), invoke]);
        }
        append_history(&session_key, "assistant", &text);
        Ok(vec![RuntimeEvent::chat_final(text)])
    }
}

#[cfg(test)]
mod tests {
    use super::ZeroclawDoctorAdapter;
    use crate::runtime::types::RuntimeErrorCode;

    #[test]
    fn parse_tool_intent_handles_mixed_text_with_embedded_json() {
        let raw = r#"好的，我来检查。
{"tool":"clawpal","args":"instance list","reason":"查看目录结构"}"#;
        let parsed = ZeroclawDoctorAdapter::parse_tool_intent(raw);
        assert!(parsed.is_some(), "should parse tool intent from mixed text");
    }

    #[test]
    fn parse_tool_intent_picks_tool_json_when_multiple_json_objects_exist() {
        let raw = r#"前置说明 {"step":"verify","reason":"ignore this"} 然后执行 {"tool":"clawpal","args":"health check --all","reason":"确认状态"}"#;
        let parsed = ZeroclawDoctorAdapter::parse_tool_intent(raw);
        assert!(
            parsed.is_some(),
            "should parse tool JSON even if another JSON appears first"
        );
    }

    #[test]
    fn infer_language_rule_prefers_chinese_when_prompt_declares_it() {
        let rule = ZeroclawDoctorAdapter::infer_language_rule(
            "Respond in Chinese (简体中文). Analyze issues directly.",
        );
        assert_eq!(rule, "Simplified Chinese (简体中文)");
    }

    #[test]
    fn infer_language_rule_defaults_to_english() {
        let rule = ZeroclawDoctorAdapter::infer_language_rule("Respond in English.");
        assert_eq!(rule, "English");
    }

    #[test]
    fn map_error_recognizes_auth_expired() {
        let err = ZeroclawDoctorAdapter::map_error("unauthorized: invalid api key".to_string());
        assert_eq!(err.code, RuntimeErrorCode::AuthExpired);
    }

    #[test]
    fn map_error_recognizes_registry_corrupt() {
        let err = ZeroclawDoctorAdapter::map_error(
            "instances.json parse failed: invalid json".to_string(),
        );
        assert_eq!(err.code, RuntimeErrorCode::RegistryCorrupt);
    }
}
