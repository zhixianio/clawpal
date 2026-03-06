use crate::doctor::classify_engine_error;
use crate::json_util::extract_json_objects;
use crate::runtime::types::{
    RuntimeAdapter, RuntimeError, RuntimeErrorCode, RuntimeEvent, RuntimeSessionKey,
};
use serde_json::json;
use serde_json::Value;

use super::process::run_zeroclaw_message;
use super::session::{append_history, build_prompt_with_history_fast, reset_history};

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

    fn looks_like_chinese(message: &str) -> bool {
        Self::infer_language_rule(message) == "Simplified Chinese (简体中文)"
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
                    let reason = reason.unwrap_or("先做一次快速排查，再继续。").trim();
                    if Self::looks_like_chinese(reason) {
                        return format!(
                            "我先把这次会话当作“诊断模式”处理：只负责先检查和给建议，不会直接改配置。下一步建议先{}。",
                            reason
                        );
                    }
                    return format!(
                        "I’m running in diagnosis-only mode: I’ll check first and suggest fixes, not make risky changes yet. Next suggestion: {}",
                        reason
                    );
                }
            }
        }
        raw
    }

    fn parse_diagnosis(raw: &str) -> Option<(RuntimeEvent, String)> {
        let result = crate::runtime::zeroclaw::tool_intent::parse_diagnosis_result(raw)?;
        let count = result.items.len();
        let summary = result.summary.clone().unwrap_or_else(|| {
            format!(
                "Diagnosis complete — found {count} issue{}.",
                if count == 1 { "" } else { "s" }
            )
        });
        let items_value = serde_json::to_value(&result.items).ok()?;
        Some((RuntimeEvent::diagnosis_report(items_value), summary))
    }

    fn parse_tool_intent(raw: &str) -> Option<(RuntimeEvent, String)> {
        let intent = crate::runtime::zeroclaw::tool_intent::parse_tool_intent(raw)?;
        let reason = intent
            .reason
            .unwrap_or_else(|| "先跑一条检查命令，确认问题点。".to_string());
        let friendly_reason = reason.trim();
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
        let raw_cmd = format!(
            "{} {}",
            payload["command"].as_str().unwrap_or(""),
            payload["args"]["args"].as_str().unwrap_or("")
        );
        let note = if Self::looks_like_chinese(friendly_reason) {
            format!(
                "我先跑一条检查命令：`{}`，方便确认当前问题。原因：{}",
                raw_cmd.trim(),
                friendly_reason
            )
        } else {
            format!(
                "I will run one diagnostic command: `{}`, because {}. So we can confirm what’s blocking things.",
                raw_cmd.trim(),
                friendly_reason
            )
        };
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
        if let Some((report, summary)) = Self::parse_diagnosis(&text) {
            append_history(&session_key, "assistant", &summary);
            return Ok(vec![RuntimeEvent::chat_final(summary), report]);
        }
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
        let prompt = build_prompt_with_history_fast(&session_key, message);
        let guarded = Self::doctor_domain_prompt(key, &prompt);
        let text = run_zeroclaw_message(&guarded, &key.instance_id, &key.storage_key())
            .map(Self::normalize_doctor_output)
            .map_err(Self::map_error)?;
        append_history(&session_key, "user", message);
        if let Some((report, summary)) = Self::parse_diagnosis(&text) {
            append_history(&session_key, "assistant", &summary);
            return Ok(vec![RuntimeEvent::chat_final(summary), report]);
        }
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
    fn normalize_doctor_output_prefers_friendly_chinese() {
        let raw = r#"{"step":"x","reason":"先收集日志再确认错误点"}"#;
        let parsed = ZeroclawDoctorAdapter::normalize_doctor_output(raw.to_string());
        assert!(parsed.contains("诊断模式"));
        assert!(parsed.contains("先收集日志"));
        assert!(!parsed.contains("不执行安装编排"));
    }

    #[test]
    fn normalize_doctor_output_prefers_friendly_english() {
        let raw = r#"{"step":"x","reason":"check gateway logs to confirm error context."}"#;
        let parsed = ZeroclawDoctorAdapter::normalize_doctor_output(raw.to_string());
        assert!(parsed.contains("diagnosis-only mode"));
        assert!(parsed.contains("check gateway logs"));
    }

    #[test]
    fn parse_tool_intent_note_is_user_friendly() {
        let raw = r#"我先说明一下。{"tool":"clawpal","args":"health check --all","reason":"确认服务是否启动"}"#;
        let parsed = ZeroclawDoctorAdapter::parse_tool_intent(raw);
        assert!(
            parsed.is_some(),
            "should parse tool intent and provide friendly note"
        );
        let (_invoke, note) = parsed.unwrap();
        assert!(note.contains("我先跑一条检查命令"));
        assert!(!note.contains("建议执行诊断命令"));
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
