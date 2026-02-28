use crate::runtime::zeroclaw::process::run_zeroclaw_message;
use crate::ssh::SshConnectionPool;
use crate::json_util::extract_json_objects;
use serde::Serialize;
use serde_json::Value;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidanceAction {
    pub label: String,
    pub action_type: String,
    pub tool: Option<String>,
    pub args: Option<String>,
    pub invoke_type: Option<String>,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorGuidance {
    pub message: String,
    pub summary: String,
    pub actions: Vec<String>,
    pub structured_actions: Vec<GuidanceAction>,
    pub source: String,
}

#[derive(Debug, Clone)]
struct GuidanceBody {
    summary: String,
    actions: Vec<String>,
    structured_actions: Vec<GuidanceAction>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenclawProbe {
    openclaw_path: Option<String>,
    path: Option<String>,
    probe_error: Option<String>,
}

fn parse_guidance_json(raw: &str) -> Option<GuidanceBody> {
    for cand in extract_json_objects(raw) {
        let Ok(v) = serde_json::from_str::<Value>(&cand) else {
            continue;
        };
        let Some(summary) = v.get("summary").and_then(Value::as_str) else {
            continue;
        };
        let actions = v
            .get("actions")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        return Some(GuidanceBody {
            summary: summary.trim().to_string(),
            actions,
            structured_actions: vec![],
        });
    }
    None
}

fn rules_fallback(
    error_text: &str,
    transport: &str,
    operation: &str,
    probe: Option<&OpenclawProbe>,
) -> GuidanceBody {
    let lower = error_text.to_lowercase();
    if lower.contains("ownerdisplay")
        && (lower.contains("unknown field")
            || lower.contains("invalid field")
            || lower.contains("failed to parse")
            || lower.contains("deserialize"))
    {
        return GuidanceBody {
            summary: "检测到 openclaw 配置字段不兼容（ownerDisplay）。系统已尝试自动修复并建议复测。"
                .to_string(),
            actions: vec![
                "重新进入该实例并等待 1-2 秒后自动刷新。".to_string(),
                "若仍失败，打开 Doctor 让 Agent继续执行更细粒度修复。".to_string(),
            ],
            structured_actions: vec![],
        };
    }
    if looks_like_openclaw_binary_missing(error_text) {
        let mut summary = "目标实例缺少 openclaw 命令，或登录 shell 的 PATH 未包含该命令。".to_string();
        let mut actions = Vec::new();
        if let Some(result) = probe {
            if let Some(path) = result.openclaw_path.as_deref() {
                summary = format!(
                    "探测到 openclaw 路径为 `{path}`，但当前业务调用仍报命令不存在，通常是登录 shell 初始化不一致。"
                );
                actions.push("检查远程登录 shell 配置（如 `.bashrc` / `.zshrc`）是否在非交互会话加载 PATH。".to_string());
                actions.push("在远程执行 `openclaw --version` 验证同一会话可直接运行。".to_string());
            } else {
                actions.push("自动探测已执行：`command -v openclaw` 未返回可执行路径。".to_string());
                actions.push("在目标实例安装/修复 openclaw 后，重新登录 SSH 会话。".to_string());
            }
            if let Some(path_env) = result.path.as_deref() {
                actions.push(format!("当前远程 PATH：`{path_env}`"));
            }
        }
        if actions.is_empty() {
            actions.push("在目标实例执行 openclaw 安装/修复脚本，并重新登录 shell。".to_string());
            actions.push("确认 `command -v openclaw` 可返回路径后，再重试当前操作。".to_string());
        }
        actions.push("进入 Doctor 页面并点击诊断，让内置 Agent 继续自动排查。".to_string());
        return GuidanceBody {
            summary,
            actions,
            structured_actions: vec![],
        };
    }
    if lower.contains("not connected to remote")
        || lower.contains("ssh")
        || lower.contains("connection refused")
    {
        return GuidanceBody {
            summary: "当前远程连接不可用，导致操作失败。".to_string(),
            actions: vec![
                "先在实例页重新连接 SSH，并确认网络可达。".to_string(),
                "执行一次健康检查，确认网关和配置目录可访问。".to_string(),
                "若仍失败，打开 Doctor 页面执行自动诊断并按建议修复。".to_string(),
            ],
            structured_actions: vec![],
        };
    }

    GuidanceBody {
        summary: format!(
            "操作 `{operation}` 在 `{transport}` 环境执行失败，建议先做诊断再继续。"
        ),
        actions: vec![
            "打开 Doctor 页面运行诊断，获取可执行修复步骤。".to_string(),
            "按诊断结果优先处理阻塞项后，再重试当前操作。".to_string(),
        ],
        structured_actions: vec![],
    }
}

async fn probe_remote_openclaw(pool: &SshConnectionPool, instance_id: &str) -> Option<OpenclawProbe> {
    let which = pool
        .exec_login(instance_id, "command -v openclaw 2>/dev/null || true")
        .await;
    let path = pool.exec_login(instance_id, "printf '%s' \"$PATH\"").await;

    let openclaw_path = which
        .as_ref()
        .ok()
        .map(|r| r.stdout.trim().to_string())
        .filter(|s| !s.is_empty());
    let path_val = path
        .as_ref()
        .ok()
        .map(|r| r.stdout.trim().to_string())
        .filter(|s| !s.is_empty());
    let probe_error = match (which, path) {
        (Err(e), _) => Some(e),
        (_, Err(e)) => Some(e),
        _ => None,
    };

    Some(OpenclawProbe {
        openclaw_path,
        path: path_val,
        probe_error,
    })
}

fn looks_like_openclaw_binary_missing(error_text: &str) -> bool {
    let lower = error_text.to_lowercase();
    (lower.contains("openclaw command not found")
        || lower.contains("command not found: openclaw")
        || lower.contains("openclaw: command not found")
        || (lower.contains("no such file or directory") && lower.contains("openclaw"))
        || lower.contains("failed to run openclaw"))
        && !lower.contains("profile")
}

fn compose_message(summary: &str, actions: &[String]) -> String {
    if actions.is_empty() {
        return summary.to_string();
    }
    let mut lines = vec![summary.to_string(), "".to_string(), "下一步建议：".to_string()];
    for (idx, action) in actions.iter().enumerate() {
        lines.push(format!("{}. {}", idx + 1, action));
    }
    lines.join("\n")
}

#[tauri::command]
pub async fn explain_operation_error(
    pool: State<'_, SshConnectionPool>,
    instance_id: String,
    operation: String,
    transport: String,
    error: String,
    language: Option<String>,
) -> Result<ErrorGuidance, String> {
    let lower_error = error.to_lowercase();
    let should_probe_openclaw = transport == "remote_ssh"
        && looks_like_openclaw_binary_missing(&lower_error);
    let probe = if should_probe_openclaw {
        probe_remote_openclaw(&pool, &instance_id).await
    } else {
        None
    };
    let language = language.unwrap_or_else(|| "en".to_string());
    let prefer_zh = language.to_lowercase().starts_with("zh");
    let language_rule = if prefer_zh {
        "Simplified Chinese (简体中文)"
    } else {
        "English"
    };
    let template = crate::prompt_templates::error_guidance_operation_fallback();
    let probe_json = serde_json::to_string(&probe).unwrap_or_else(|_| "null".to_string());
    let prompt = crate::prompt_templates::render_template(
        &template,
        &[
            ("{{language_rule}}", language_rule),
            ("{{instance_id}}", &instance_id),
            ("{{transport}}", &transport),
            ("{{operation}}", &operation),
            ("{{error}}", &error),
            ("{{probe}}", &probe_json),
            ("{{language}}", &language),
        ],
    );

    let fallback_scope = format!("fallback-{}", uuid::Uuid::new_v4());
    let from_agent = run_zeroclaw_message(&prompt, &instance_id, &fallback_scope)
        .ok()
        .and_then(|raw| parse_guidance_json(&raw));

    let (guidance, source) = if let Some(parsed) = from_agent {
        (parsed, "zeroclaw".to_string())
    } else {
        (
            rules_fallback(&error, &transport, &operation, probe.as_ref()),
            "rules".to_string(),
        )
    };

    let message = compose_message(&guidance.summary, &guidance.actions);
    Ok(ErrorGuidance {
        message,
        summary: guidance.summary,
        actions: guidance.actions,
        structured_actions: guidance.structured_actions,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_guidance_json, rules_fallback, GuidanceAction, OpenclawProbe};

    #[test]
    fn parse_guidance_json_accepts_embedded_json() {
        let raw = "分析如下 {\"summary\":\"远程命令不存在\",\"actions\":[\"安装命令\",\"重试\"]}";
        let parsed = parse_guidance_json(raw).expect("should parse");
        assert_eq!(parsed.summary, "远程命令不存在");
        assert_eq!(parsed.actions.len(), 2);
    }

    #[test]
    fn guidance_action_serializes_inline_fix() {
        let action = GuidanceAction {
            label: "重连 SSH".to_string(),
            action_type: "inline_fix".to_string(),
            tool: Some("clawpal".to_string()),
            args: Some("ssh connect --host test-host".to_string()),
            invoke_type: Some("read".to_string()),
            context: None,
        };
        let json = serde_json::to_value(&action).unwrap();
        assert_eq!(json["actionType"], "inline_fix");
        assert_eq!(json["tool"], "clawpal");
    }

    #[test]
    fn guidance_action_serializes_doctor_handoff() {
        let action = GuidanceAction {
            label: "让小龙虾修复".to_string(),
            action_type: "doctor_handoff".to_string(),
            tool: None,
            args: None,
            invoke_type: None,
            context: Some("Container abc not found".to_string()),
        };
        let json = serde_json::to_value(&action).unwrap();
        assert_eq!(json["actionType"], "doctor_handoff");
        assert!(json["tool"].is_null());
        assert_eq!(json["context"], "Container abc not found");
    }

    #[test]
    fn rules_fallback_handles_openclaw_not_found() {
        let result = rules_fallback(
            "openclaw command not found",
            "remote_ssh",
            "listAgents",
            Some(&OpenclawProbe {
                openclaw_path: None,
                path: Some("/usr/bin:/bin".to_string()),
                probe_error: None,
            }),
        );
        assert!(result.summary.contains("openclaw"));
        assert!(!result.actions.is_empty());
    }
}
