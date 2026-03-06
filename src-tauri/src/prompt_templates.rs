fn extract_prompt_block(markdown: &str) -> String {
    let marker = "```prompt";
    if let Some(start) = markdown.find(marker) {
        let body_start = start + marker.len();
        let rest = &markdown[body_start..];
        if let Some(end) = rest.find("```") {
            return rest[..end].trim().to_string();
        }
    }
    markdown.trim().to_string()
}

pub fn render_template(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in vars {
        out = out.replace(key, value);
    }
    out
}

pub fn doctor_domain_system() -> String {
    extract_prompt_block(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../prompts/doctor/domain-system.md"
    )))
}

pub fn doctor_history_preamble() -> String {
    extract_prompt_block(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../prompts/doctor/history-preamble.md"
    )))
}

pub fn install_domain_system() -> String {
    extract_prompt_block(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../prompts/install/domain-system.md"
    )))
}

pub fn install_history_preamble() -> String {
    extract_prompt_block(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../prompts/install/history-preamble.md"
    )))
}

pub fn error_guidance_operation_fallback() -> String {
    extract_prompt_block(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../prompts/error-guidance/operation-fallback.md"
    )))
}

pub fn install_orchestrator_decider() -> String {
    extract_prompt_block(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../prompts/install/orchestrator-decider.md"
    )))
}

pub fn install_target_decider() -> String {
    extract_prompt_block(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../prompts/install/target-decider.md"
    )))
}

#[cfg(test)]
mod tests {
    use super::{
        doctor_domain_system, doctor_history_preamble, install_domain_system,
        install_history_preamble, render_template,
    };

    #[test]
    fn extracts_prompt_block() {
        let prompt = doctor_domain_system();
        assert!(prompt.contains("DOCTOR DOMAIN ONLY."));
    }

    #[test]
    fn doctor_prompt_contains_doc_guidance_guardrails() {
        let prompt = doctor_domain_system();
        assert!(prompt.contains("If `docGuidance` exists in context"));
        assert!(prompt.contains("root_cause_hypothesis"));
        assert!(prompt.contains("version_awareness"));
        assert!(prompt.contains("non-technical"));
        assert!(prompt.contains("plain words"));
    }

    #[test]
    fn install_prompt_contains_command_guardrails() {
        let prompt = install_domain_system();
        assert!(prompt.contains("INSTALL DOMAIN ONLY."));
        assert!(
            prompt.contains("For tool=\"clawpal\", you MUST use only these supported commands:")
        );
        assert!(prompt.contains("NEVER invent non-existent clawpal commands"));
    }

    #[test]
    fn install_history_preamble_contains_execution_guardrails() {
        let prompt = install_history_preamble();
        assert!(prompt.contains("output ONLY one JSON object in this exact shape"));
        assert!(prompt.contains("Never invent unsupported clawpal commands"));
    }

    #[test]
    fn doctor_history_preamble_contains_doc_guidance_guardrails() {
        let prompt = doctor_history_preamble();
        assert!(prompt.contains("root_cause_hypothesis"));
        assert!(prompt.contains("docGuidance"));
        assert!(prompt.contains("citations"));
        assert!(prompt.contains("beginner-friendly"));
    }

    #[test]
    fn renders_tokens() {
        let rendered = render_template("a {{x}} b", &[("{{x}}", "ok")]);
        assert_eq!(rendered, "a ok b");
    }
}
