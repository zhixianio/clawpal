use crate::runtime::types::{RuntimeError, RuntimeEvent, RuntimeSessionKey};

use super::process::run_zeroclaw_message_streaming;
use super::session::append_history;
use super::session::reset_history;

pub(crate) async fn run_zeroclaw_streaming_turn<FDelta, FNormalize, FIntent, FMapError>(
    key: &RuntimeSessionKey,
    prompt: &str,
    reset_session: bool,
    user_message: Option<&str>,
    on_delta: FDelta,
    normalize_output: FNormalize,
    parse_tool_intent: FIntent,
    map_error: FMapError,
) -> Result<Vec<RuntimeEvent>, RuntimeError>
where
    FDelta: Fn(&str) + Send + Sync + 'static,
    FNormalize: Fn(String) -> String,
    FIntent: Fn(&str) -> Option<(RuntimeEvent, String)>,
    FMapError: Fn(String) -> RuntimeError,
{
    let session_key = key.storage_key();
    if reset_session {
        reset_history(&session_key);
    }
    if let Some(message) = user_message {
        append_history(&session_key, "user", message);
    }

    let text = run_zeroclaw_message_streaming(prompt, &key.instance_id, &key.storage_key(), on_delta)
        .await
        .map(normalize_output)
        .map_err(map_error)?;

    if let Some((invoke, note)) = parse_tool_intent(&text) {
        append_history(&session_key, "assistant", &note);
        return Ok(vec![RuntimeEvent::chat_final(note), invoke]);
    }

    append_history(&session_key, "assistant", &text);
    Ok(vec![RuntimeEvent::chat_final(text)])
}
