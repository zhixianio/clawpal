//! E2E test: create an Anthropic profile using a Claude OAuth token
//! (from `claude setup-token` / `openclaw models auth login`), persist it,
//! and verify the token works with a real provider probe via Bearer auth.
//!
//! Requires `CLAUDE_OAUTH_TOKEN` in the environment.  The test is skipped
//! automatically when the token is absent so local `cargo test` still passes.

use std::fs;
use std::sync::Mutex;

use clawpal_core::openclaw::OpenclawCli;
use clawpal_core::profile::{self, ModelProfile};
use uuid::Uuid;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn temp_data_dir() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("clawpal-core-oauth-e2e-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

/// Probe the Anthropic API using OAuth Bearer auth (not x-api-key).
/// OAuth tokens from `claude setup-token` start with `sk-ant-oat`.
fn anthropic_oauth_probe(token: &str, model: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("Authorization", format!("Bearer {}", token.trim()))
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "ping"}]
        }))
        .send()
        .map_err(|e| format!("request failed: {e}"))?;

    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(());
    }
    let body = resp.text().unwrap_or_default();
    Err(format!("OAuth probe failed (HTTP {status}): {body}"))
}

#[test]
fn e2e_create_oauth_profile_and_probe() {
    let oauth_token = match std::env::var("CLAUDE_OAUTH_TOKEN") {
        Ok(k) if !k.trim().is_empty() => k,
        _ => {
            eprintln!("CLAUDE_OAUTH_TOKEN not set — skipping OAuth E2E test");
            return;
        }
    };

    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data_dir = temp_data_dir();
    std::env::set_var("CLAWPAL_DATA_DIR", &data_dir);

    // ── 1. Create & persist profile with OAuth token ───────────────
    let oauth_profile = ModelProfile {
        id: String::new(),
        name: String::new(),
        provider: "anthropic".to_string(),
        model: "claude-sonnet-4-20250514".to_string(),
        auth_ref: "CLAUDE_OAUTH_TOKEN".to_string(),
        api_key: Some(oauth_token.clone()),
        base_url: None,
        description: Some("E2E OAuth token test profile".to_string()),
        enabled: true,
    };

    let cli = OpenclawCli::with_bin("__unused__".to_string());
    let saved = profile::upsert_profile(&cli, oauth_profile).expect("upsert_profile");

    assert!(!saved.id.is_empty(), "profile id should be generated");
    assert_eq!(saved.provider, "anthropic");
    assert_eq!(saved.model, "claude-sonnet-4-20250514");
    assert_eq!(saved.name, "anthropic/claude-sonnet-4-20250514");

    // ── 2. Verify persistence ──────────────────────────────────────
    let profiles = profile::list_profiles(&cli).expect("list_profiles");
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].id, saved.id);
    assert_eq!(profiles[0].auth_ref, "CLAUDE_OAUTH_TOKEN");

    // ── 3. Real OAuth API probe ────────────────────────────────────
    anthropic_oauth_probe(&oauth_token, &saved.model)
        .expect("Anthropic OAuth probe should succeed");
}
