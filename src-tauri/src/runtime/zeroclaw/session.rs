use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

fn history_store() -> &'static Mutex<HashMap<String, Vec<(String, String)>>> {
    static STORE: OnceLock<Mutex<HashMap<String, Vec<(String, String)>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn reset_history(session_key: &str) {
    if let Ok(mut guard) = history_store().lock() {
        guard.insert(session_key.to_string(), Vec::new());
    }
}

pub fn append_history(session_key: &str, role: &str, content: &str) {
    if let Ok(mut guard) = history_store().lock() {
        let entry = guard.entry(session_key.to_string()).or_default();
        entry.push((role.to_string(), content.to_string()));
        if entry.len() > 16 {
            let drop_n = entry.len().saturating_sub(16);
            entry.drain(0..drop_n);
        }
    }
}

pub fn build_prompt_with_history(session_key: &str, latest_user_message: &str) -> String {
    build_prompt_with_history_preamble(session_key, latest_user_message, "You are continuing a Doctor troubleshooting chat. Keep continuity with prior turns.\n")
}

pub fn build_prompt_with_history_preamble(session_key: &str, latest_user_message: &str, preamble: &str) -> String {
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
