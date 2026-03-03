//! E2E test: create an Anthropic (Claude) profile, persist it, and verify
//! the API key works with a real provider probe.
//!
//! Requires `ANTHROPIC_API_KEY` in the environment.  The test is skipped
//! automatically when the key is absent so local `cargo test` still passes.

use std::fs;
use std::sync::Mutex;

use clawpal_core::openclaw::OpenclawCli;
use clawpal_core::profile::{self, ModelProfile};
use uuid::Uuid;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn temp_data_dir() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("clawpal-core-profile-e2e-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

/// Lightweight Anthropic API probe — sends a single-token request to verify
/// the key and model are valid.
fn anthropic_probe(api_key: &str, model: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
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
    Err(format!("probe failed (HTTP {status}): {body}"))
}

#[test]
fn e2e_create_anthropic_profile_and_probe() {
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) if !k.trim().is_empty() => k,
        _ => {
            eprintln!("ANTHROPIC_API_KEY not set — skipping E2E profile test");
            return;
        }
    };

    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data_dir = temp_data_dir();
    std::env::set_var("CLAWPAL_DATA_DIR", &data_dir);

    // ── 1. Create & persist profile ────────────────────────────────
    let claude_profile = ModelProfile {
        id: String::new(), // let upsert assign UUID
        name: String::new(),
        provider: "anthropic".to_string(),
        model: "claude-sonnet-4-20250514".to_string(),
        auth_ref: "ANTHROPIC_API_KEY".to_string(),
        api_key: Some(api_key.clone()),
        base_url: None,
        description: Some("E2E test profile".to_string()),
        enabled: true,
    };

    // OpenclawCli is unused by upsert_profile (local storage) but required
    // by the signature.
    let cli = OpenclawCli::with_bin("__unused__".to_string());
    let saved = profile::upsert_profile(&cli, claude_profile).expect("upsert_profile");

    assert!(!saved.id.is_empty(), "profile id should be generated");
    assert_eq!(saved.provider, "anthropic");
    assert_eq!(saved.model, "claude-sonnet-4-20250514");
    assert_eq!(saved.name, "anthropic/claude-sonnet-4-20250514");

    // ── 2. Verify persistence via list ─────────────────────────────
    let profiles = profile::list_profiles(&cli).expect("list_profiles");
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].id, saved.id);
    assert_eq!(profiles[0].provider, "anthropic");

    // ── 3. Real API probe ──────────────────────────────────────────
    anthropic_probe(&api_key, &saved.model).expect("Anthropic API probe should succeed");
}
