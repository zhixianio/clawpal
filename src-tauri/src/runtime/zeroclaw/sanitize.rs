use regex::Regex;
use std::sync::OnceLock;

fn ansi_esc_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").expect("valid ansi regex"))
}

fn ansi_literal_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[[0-9;]*m").expect("valid ansi literal regex"))
}

fn is_zeroclaw_trace_line(lower: &str) -> bool {
    lower.contains("zeroclaw::") && (lower.contains(" info ") || lower.contains(" warn "))
}

fn strip_ansi(raw: &str) -> String {
    let escaped = ansi_esc_regex().replace_all(raw, "");
    ansi_literal_regex().replace_all(&escaped, "").into_owned()
}

/// Sanitize a single line of output from zeroclaw.
/// Returns `None` for lines that should be suppressed (empty, ANSI-only, zeroclaw trace lines).
pub fn sanitize_line(raw: &str) -> Option<String> {
    let cleaned = strip_ansi(raw);
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if is_zeroclaw_trace_line(&lower) {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn sanitize_output(raw: &str) -> String {
    let cleaned = strip_ansi(raw);
    let mut lines = Vec::<String>::new();
    for line in cleaned.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if is_zeroclaw_trace_line(&lower) {
            continue;
        }
        lines.push(trimmed.to_string());
    }
    lines.join("\n")
}
