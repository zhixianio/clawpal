use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const HISTORY_MAX_ENTRIES: usize = 12;
const HISTORY_MAX_ENTRY_BYTES: usize = 4 * 1024;

fn history_store() -> &'static Mutex<HashMap<String, Vec<(String, String)>>> {
    static STORE: OnceLock<Mutex<HashMap<String, Vec<(String, String)>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn truncate_utf8_prefix(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }
    let mut end = max_bytes.min(input.len());
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_string()
}

fn truncate_utf8_tail(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }
    let mut start = input.len().saturating_sub(max_bytes);
    while start < input.len() && !input.is_char_boundary(start) {
        start += 1;
    }
    input[start..].to_string()
}

fn clamp_history_entry(content: &str) -> String {
    if content.len() <= HISTORY_MAX_ENTRY_BYTES {
        return content.to_string();
    }
    let marker = format!(
        "\n...[history truncated {} bytes]...\n",
        content.len().saturating_sub(HISTORY_MAX_ENTRY_BYTES)
    );
    let keep = HISTORY_MAX_ENTRY_BYTES.saturating_sub(marker.len());
    let head_keep = keep / 2;
    let tail_keep = keep.saturating_sub(head_keep);
    let head = truncate_utf8_prefix(content, head_keep);
    let tail = truncate_utf8_tail(content, tail_keep);
    format!("{head}{marker}{tail}")
}

pub fn reset_history(session_key: &str) {
    if let Ok(mut guard) = history_store().lock() {
        guard.insert(session_key.to_string(), Vec::new());
    }
}

pub fn append_history(session_key: &str, role: &str, content: &str) {
    if let Ok(mut guard) = history_store().lock() {
        let entry = guard.entry(session_key.to_string()).or_default();
        entry.push((role.to_string(), clamp_history_entry(content)));
        if entry.len() > HISTORY_MAX_ENTRIES {
            let drop_n = entry.len().saturating_sub(HISTORY_MAX_ENTRIES);
            entry.drain(0..drop_n);
        }
    }
}

pub fn build_prompt_with_history(session_key: &str, latest_user_message: &str) -> String {
    let preamble = format!("{}\n", crate::prompt_templates::doctor_history_preamble());
    build_prompt_with_history_preamble(session_key, latest_user_message, &preamble)
}

pub fn build_prompt_with_history_preamble(
    session_key: &str,
    latest_user_message: &str,
    preamble: &str,
) -> String {
    let mut prompt = String::from(preamble);
    if let Ok(guard) = history_store().lock() {
        if let Some(history) = guard.get(session_key) {
            if !history.is_empty() {
                prompt.push_str("\nConversation so far:\n");
                for (role, text) in history {
                    prompt.push_str(role);
                    prompt.push_str(": ");
                    prompt.push_str(text);
                    prompt.push('\n');
                }
            }
        }
    }
    prompt.push_str("\nUser: ");
    prompt.push_str(latest_user_message);
    prompt.push_str("\nAssistant:");
    prompt
}

pub fn history_len(session_key: &str) -> usize {
    if let Ok(guard) = history_store().lock() {
        return guard.get(session_key).map(|v| v.len()).unwrap_or(0);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_history_caps_entry_count() {
        let key = "test-history-cap";
        reset_history(key);
        for i in 0..32 {
            append_history(key, "user", &format!("msg-{i}"));
        }
        assert_eq!(history_len(key), HISTORY_MAX_ENTRIES);
    }

    #[test]
    fn append_history_truncates_large_entry() {
        let key = "test-history-truncation";
        reset_history(key);
        let huge = "x".repeat(HISTORY_MAX_ENTRY_BYTES * 3);
        append_history(key, "assistant", &huge);
        let prompt = build_prompt_with_history(key, "ping");
        assert!(prompt.contains("history truncated"));
        assert!(prompt.len() < huge.len());
    }
}
