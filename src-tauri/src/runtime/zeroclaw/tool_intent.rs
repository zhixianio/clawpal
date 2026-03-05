use crate::json_util::extract_json_objects;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolIntent {
    pub tool: String,
    pub args: String,
    pub instance: Option<String>,
    pub reason: Option<String>,
}

pub fn classify_invoke_type(tool: &str, args: &str) -> &'static str {
    let tool_lc = tool.trim().to_ascii_lowercase();
    let args_lc = args.trim().to_ascii_lowercase();
    let is_prefix = |prefix: &str| args_lc == prefix || args_lc.starts_with(&format!("{prefix} "));

    if tool_lc == "clawpal" {
        let write_prefixes = [
            "instance remove",
            "profile add",
            "profile remove",
            "connect docker",
            "connect ssh",
            "install local",
            "install docker",
            "ssh connect",
            "ssh disconnect",
            "doctor exec",
            "doctor fix-openclaw-path",
            "doctor file write",
            "doctor config-upsert",
            "doctor config-delete",
            "doctor sessions-upsert",
            "doctor sessions-delete",
        ];
        if write_prefixes.iter().any(|p| is_prefix(p)) {
            return "write";
        }
        return "read";
    }

    if tool_lc == "openclaw" {
        let write_prefixes = [
            "doctor --fix",
            "config set",
            "config delete",
            "config unset",
            "auth add",
            "auth login",
            "auth remove",
            "gateway start",
            "gateway stop",
            "service install",
            "service uninstall",
            "service restart",
            "channel add",
            "channel remove",
            "channel update",
            "cron add",
            "cron remove",
            "cron update",
        ];
        if write_prefixes.iter().any(|p| is_prefix(p)) {
            return "write";
        }
        return "read";
    }

    // Unknown tool defaults to write for safety: it always requires explicit
    // user confirmation instead of auto-running as read.
    "write"
}

#[derive(Debug, Deserialize)]
struct ToolIntentPayload {
    tool: String,
    args: String,
    #[serde(default)]
    instance: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

fn extract_fenced_json(raw: &str) -> Option<String> {
    let marker = "```json";
    let start = raw.find(marker)?;
    let after = &raw[start + marker.len()..];
    let end = after.find("```")?;
    Some(after[..end].trim().to_string())
}

fn validate_payload(payload: ToolIntentPayload) -> Option<ToolIntent> {
    let tool = payload.tool.trim().to_string();
    if tool.is_empty() {
        return None;
    }
    let args = payload.args.trim().to_string();
    if args.is_empty() {
        return None;
    }
    Some(ToolIntent {
        tool,
        args,
        instance: payload
            .instance
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        reason: payload
            .reason
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
    })
}

pub fn parse_tool_intent(raw: &str) -> Option<ToolIntent> {
    let trimmed = raw.trim();
    let mut candidates = vec![trimmed.to_string()];
    if let Some(fenced) = extract_fenced_json(trimmed) {
        if fenced != trimmed {
            candidates.push(fenced);
        }
    }
    for extracted in extract_json_objects(trimmed) {
        if extracted != trimmed {
            candidates.push(extracted);
        }
    }

    for candidate in candidates {
        let Ok(payload) = serde_json::from_str::<ToolIntentPayload>(&candidate) else {
            continue;
        };
        if let Some(intent) = validate_payload(payload) {
            return Some(intent);
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosisSeverity {
    Error,
    Warn,
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosisCitation {
    pub url: String,
    #[serde(default)]
    pub section: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosisItem {
    pub problem: String,
    pub severity: DiagnosisSeverity,
    pub fix_options: Vec<String>,
    #[serde(default)]
    pub root_cause_hypothesis: Option<String>,
    #[serde(default)]
    pub fix_steps: Vec<String>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub citations: Vec<DiagnosisCitation>,
    #[serde(default)]
    pub version_awareness: Option<String>,
    #[serde(default)]
    pub action: Option<ToolIntent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosisResult {
    pub items: Vec<DiagnosisItem>,
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiagnosisPayload {
    diagnosis: Vec<DiagnosisItem>,
    #[serde(default)]
    summary: Option<String>,
}

pub fn parse_diagnosis_result(raw: &str) -> Option<DiagnosisResult> {
    let trimmed = raw.trim();
    let mut candidates = vec![trimmed.to_string()];
    if let Some(fenced) = extract_fenced_json(trimmed) {
        if fenced != trimmed {
            candidates.push(fenced);
        }
    }
    for extracted in extract_json_objects(trimmed) {
        if extracted != trimmed {
            candidates.push(extracted);
        }
    }

    for candidate in candidates {
        let Ok(payload) = serde_json::from_str::<DiagnosisPayload>(&candidate) else {
            continue;
        };
        if payload.diagnosis.is_empty() {
            continue;
        }
        return Some(DiagnosisResult {
            items: payload.diagnosis,
            summary: payload
                .summary
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
        });
    }
    None
}

pub fn export_diagnosis(result: &DiagnosisResult, format: &str) -> String {
    match format {
        "json" => serde_json::to_string_pretty(result).unwrap_or_default(),
        _ => {
            let mut md = String::new();
            if let Some(ref summary) = result.summary {
                md.push_str(&format!("# Diagnosis Summary\n\n{summary}\n\n"));
            }
            for (i, item) in result.items.iter().enumerate() {
                let sev = match item.severity {
                    DiagnosisSeverity::Error => "ERROR",
                    DiagnosisSeverity::Warn => "WARN",
                    DiagnosisSeverity::Info => "INFO",
                };
                md.push_str(&format!("## {} [{sev}] {}\n\n", i + 1, item.problem));
                if let Some(ref hypothesis) = item.root_cause_hypothesis {
                    md.push_str(&format!("**Root cause hypothesis:** {hypothesis}\n\n"));
                }
                if let Some(confidence) = item.confidence {
                    md.push_str(&format!("**Confidence:** {:.2}\n\n", confidence));
                }
                if !item.fix_options.is_empty() {
                    md.push_str("**Fix options:**\n\n");
                    for opt in &item.fix_options {
                        md.push_str(&format!("- {opt}\n"));
                    }
                    md.push('\n');
                }
                if !item.fix_steps.is_empty() {
                    md.push_str("**Fix steps:**\n\n");
                    for step in &item.fix_steps {
                        md.push_str(&format!("- {step}\n"));
                    }
                    md.push('\n');
                }
                if !item.citations.is_empty() {
                    md.push_str("**Citations:**\n\n");
                    for citation in &item.citations {
                        let section = citation
                            .section
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("section unavailable");
                        md.push_str(&format!("- {} ({section})\n", citation.url));
                    }
                    md.push('\n');
                }
            }
            md
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_invoke_type, parse_tool_intent};

    #[test]
    fn parses_embedded_json_tool_intent() {
        let raw =
            "先检查。\n{\"tool\":\"clawpal\",\"args\":\"health check --all\",\"reason\":\"验证\"}";
        let intent = parse_tool_intent(raw).expect("intent");
        assert_eq!(intent.tool, "clawpal");
        assert_eq!(intent.args, "health check --all");
    }

    #[test]
    fn accepts_custom_tool() {
        let raw = "{\"tool\":\"bash\",\"args\":\"-lc \\\"echo 1\\\"\"}";
        let intent = parse_tool_intent(raw).expect("intent");
        assert_eq!(intent.tool, "bash");
    }

    #[test]
    fn parses_fenced_json() {
        let raw = "```json\n{\"tool\":\"openclaw\",\"args\":\"doctor --fix\"}\n```";
        let intent = parse_tool_intent(raw).expect("intent");
        assert_eq!(intent.tool, "openclaw");
        assert_eq!(intent.args, "doctor --fix");
    }

    #[test]
    fn classify_invoke_type_marks_mutations_as_write() {
        assert_eq!(
            classify_invoke_type("clawpal", "doctor file write --domain config --content {}"),
            "write"
        );
        assert_eq!(
            classify_invoke_type(
                "clawpal",
                "doctor exec --tool sudo --args \"rm -rf /tmp/x\""
            ),
            "write"
        );
        assert_eq!(classify_invoke_type("openclaw", "doctor --fix"), "write");
    }

    #[test]
    fn classify_invoke_type_marks_queries_as_read() {
        assert_eq!(
            classify_invoke_type("clawpal", "doctor file read --domain config"),
            "read"
        );
        assert_eq!(classify_invoke_type("openclaw", "gateway status"), "read");
    }

    #[test]
    fn classify_invoke_type_marks_unknown_tool_as_write() {
        assert_eq!(classify_invoke_type("bash", "-lc \"cat /tmp/x\""), "write");
    }

    use super::{export_diagnosis, parse_diagnosis_result, DiagnosisSeverity};

    #[test]
    fn parses_diagnosis_result_from_json() {
        let raw = r#"{"diagnosis":[{"problem":"Config missing","severity":"error","fix_options":["Reinstall","Edit config"]}],"summary":"1 issue found"}"#;
        let result = parse_diagnosis_result(raw).expect("diagnosis");
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].problem, "Config missing");
        assert_eq!(result.items[0].severity, DiagnosisSeverity::Error);
        assert_eq!(result.items[0].fix_options.len(), 2);
        assert_eq!(result.summary.as_deref(), Some("1 issue found"));
    }

    #[test]
    fn parses_diagnosis_result_embedded_in_text() {
        let raw = r#"诊断完成。
{"diagnosis":[{"problem":"Port conflict","severity":"warn","fix_options":["Change port"]}]}"#;
        let result = parse_diagnosis_result(raw).expect("diagnosis");
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].severity, DiagnosisSeverity::Warn);
    }

    #[test]
    fn parse_diagnosis_result_returns_none_for_empty_diagnosis() {
        let raw = r#"{"diagnosis":[]}"#;
        assert!(parse_diagnosis_result(raw).is_none());
    }

    #[test]
    fn parse_diagnosis_result_returns_none_for_non_diagnosis_json() {
        let raw = r#"{"tool":"clawpal","args":"health check"}"#;
        assert!(parse_diagnosis_result(raw).is_none());
    }

    #[test]
    fn export_diagnosis_markdown() {
        let raw = r#"{"diagnosis":[{"problem":"Broken","severity":"error","fix_options":["Fix A"]}],"summary":"Summary"}"#;
        let result = parse_diagnosis_result(raw).unwrap();
        let md = export_diagnosis(&result, "markdown");
        assert!(md.contains("# Diagnosis Summary"));
        assert!(md.contains("[ERROR]"));
        assert!(md.contains("Fix A"));
    }

    #[test]
    fn export_diagnosis_json() {
        let raw = r#"{"diagnosis":[{"problem":"Test","severity":"info","fix_options":[]}]}"#;
        let result = parse_diagnosis_result(raw).unwrap();
        let json = export_diagnosis(&result, "json");
        assert!(json.contains("\"problem\": \"Test\""));
    }
}
