use super::*;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn local_global_openclaw_base_dir() -> std::path::PathBuf {
    resolve_paths().base_dir
}

fn normalize_profile_key(profile: &ModelProfile) -> String {
    normalize_model_ref(&profile_to_model_value(profile))
}

fn is_non_empty(opt: Option<&str>) -> bool {
    opt.map(str::trim).is_some_and(|v| !v.is_empty())
}

fn missing_profile_auth_hint(provider: &str, remote: bool) -> String {
    let normalized = provider.trim().to_ascii_lowercase();
    let target = if remote {
        "on the remote host"
    } else {
        "on local shell"
    };
    if normalized == "anthropic" {
        if remote {
            return " For Claude setup-token, also try exporting ANTHROPIC_OAUTH_TOKEN or ANTHROPIC_AUTH_TOKEN on the remote host.".to_string();
        }
        return " For Claude setup-token, also try exporting ANTHROPIC_OAUTH_TOKEN or ANTHROPIC_AUTH_TOKEN.".to_string();
    }
    if normalized == "openai-codex"
        || normalized == "openai_codex"
        || normalized == "github-copilot"
        || normalized == "copilot"
    {
        return format!(
            " For OpenAI Codex OAuth, run `openclaw models auth login --provider openai-codex` {target}, or export OPENAI_CODEX_TOKEN."
        );
    }
    String::new()
}

fn profile_quality_score(profile: &ModelProfile) -> usize {
    let mut score = 0usize;
    if is_non_empty(profile.api_key.as_deref()) {
        score += 8;
    }
    if !profile.auth_ref.trim().is_empty() {
        score += 4;
    }
    if is_non_empty(profile.base_url.as_deref()) {
        score += 2;
    }
    if profile.enabled {
        score += 1;
    }
    score
}

fn dedupe_profiles_by_model_key(profiles: Vec<ModelProfile>) -> Vec<ModelProfile> {
    let mut deduped: Vec<ModelProfile> = Vec::new();
    let mut key_index: HashMap<String, usize> = HashMap::new();

    for profile in profiles {
        let key = normalize_profile_key(&profile);
        if key.is_empty() {
            deduped.push(profile);
            continue;
        }
        if let Some(existing_idx) = key_index.get(&key).copied() {
            let existing_score = profile_quality_score(&deduped[existing_idx]);
            let incoming_score = profile_quality_score(&profile);
            if incoming_score > existing_score {
                deduped[existing_idx] = profile;
            }
        } else {
            key_index.insert(key, deduped.len());
            deduped.push(profile);
        }
    }

    deduped
}

const ZEROCLAW_OAUTH_LOGIN_CAPTURE_TIMEOUT_SECS: u64 = 4;
const ZEROCLAW_OAUTH_COMPLETE_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZeroclawOauthLoginStartResult {
    pub provider: String,
    pub profile: String,
    pub auth_ref: String,
    pub authorize_url: String,
    pub details: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZeroclawOauthCompleteResult {
    pub provider: String,
    pub profile: String,
    pub auth_ref: String,
    pub details: String,
}

fn normalize_zeroclaw_oauth_provider(provider: &str) -> Option<&'static str> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "openai-codex" | "openai_codex" | "github-copilot" | "copilot" | "openai" => {
            Some("openai-codex")
        }
        _ => None,
    }
}

fn normalize_oauth_profile_name(profile: Option<String>) -> String {
    let profile = profile
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("default");
    profile.to_string()
}

fn oauth_auth_ref(provider: &str, profile: &str) -> String {
    format!("{provider}:{profile}")
}

fn trim_command_details(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    truncate_error_text(trimmed, 1200)
}

fn run_command_with_timeout_capture(
    cmd_path: &Path,
    args: &[String],
    timeout_secs: u64,
    context: &str,
) -> Result<(i32, String, String, bool), String> {
    let mut command = Command::new(cmd_path);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let mut child = command
        .spawn()
        .map_err(|e| format!("failed to start {context}: {e}"))?;

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut timed_out = false;
    loop {
        match child
            .try_wait()
            .map_err(|e| format!("failed while waiting for {context}: {e}"))?
        {
            Some(_) => break,
            None => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    let _ = child.kill();
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed to collect {context} output: {e}"))?;
    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string();
    let stderr = String::from_utf8_lossy(&output.stderr)
        .trim_end()
        .to_string();
    Ok((exit_code, stdout, stderr, timed_out))
}

fn extract_first_http_url(text: &str) -> Option<String> {
    for token in text.split_whitespace() {
        if token.starts_with("http://") || token.starts_with("https://") {
            let cleaned = token
                .trim_matches(|ch: char| matches!(ch, '"' | '\'' | ')' | ']' | '}' | ',' | ';'))
                .trim_end_matches('.');
            if cleaned.starts_with("http://") || cleaned.starts_with("https://") {
                return Some(cleaned.to_string());
            }
        }
    }
    None
}

fn oauth_store_provider_ids(provider: &str) -> &'static [&'static str] {
    match normalize_zeroclaw_oauth_provider(provider) {
        // Backward compatible:
        // - New zeroclaw auth store uses `openai-codex:*`
        // - Older store variants may still use `openai:*`
        Some("openai-codex") => &["openai-codex", "openai"],
        _ => &[],
    }
}

fn oauth_profile_id_matches(profile_id: &str, provider_ids: &[&str], profile_name: &str) -> bool {
    let trimmed = profile_id.trim();
    if trimmed.is_empty() {
        return false;
    }
    let Some((provider, profile)) = trimmed.split_once(':') else {
        return false;
    };
    provider_ids
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(provider.trim()))
        && profile.trim().eq_ignore_ascii_case(profile_name)
}

fn profile_name_from_auth_ref(auth_ref: &str) -> &str {
    auth_ref
        .trim()
        .split_once(':')
        .map(|(_, profile)| profile.trim())
        .filter(|profile| !profile.is_empty())
        .unwrap_or("default")
}

fn oauth_store_file_has_profile(config_dir: &Path, provider: &str, profile_name: &str) -> bool {
    let provider_ids = oauth_store_provider_ids(provider);
    if provider_ids.is_empty() {
        return false;
    }
    let auth_file = config_dir.join("auth-profiles.json");
    let Ok(raw) = std::fs::read_to_string(auth_file) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };

    if let Some(active_map) = json.get("active_profiles").and_then(Value::as_object) {
        for provider_id in provider_ids {
            if let Some(active) = active_map.get(*provider_id).and_then(Value::as_str) {
                if oauth_profile_id_matches(active, provider_ids, profile_name) {
                    return true;
                }
            }
        }
        for active in active_map.values().filter_map(Value::as_str) {
            if oauth_profile_id_matches(active, provider_ids, profile_name) {
                return true;
            }
        }
    }

    if let Some(profiles_map) = json.get("profiles").and_then(Value::as_object) {
        for (profile_id, entry) in profiles_map {
            if oauth_profile_id_matches(profile_id, provider_ids, profile_name) {
                return true;
            }
            let entry_provider = entry
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            let entry_profile = entry
                .get("profile_name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if provider_ids
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(&entry_provider))
                && entry_profile == profile_name.to_ascii_lowercase()
            {
                return true;
            }
        }
    }

    false
}

fn oauth_store_has_profile(instance_id: &str, provider: &str, profile_name: &str) -> bool {
    let Ok(cfg_dir) =
        crate::runtime::zeroclaw::process::zeroclaw_oauth_config_dir_for_instance(instance_id)
    else {
        return false;
    };
    oauth_store_file_has_profile(&cfg_dir, provider, profile_name)
}

fn oauth_store_has_profile_any_instance(provider: &str, profile_name: &str) -> bool {
    let oauth_root = resolve_paths()
        .clawpal_dir
        .join("zeroclaw-sidecar")
        .join("oauth");
    let Ok(entries) = std::fs::read_dir(oauth_root) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && oauth_store_file_has_profile(&path, provider, profile_name) {
            return true;
        }
    }
    false
}

fn merge_remote_profile_into_local(
    local_profiles: &mut Vec<ModelProfile>,
    remote: &ModelProfile,
    resolved_api_key: Option<String>,
    resolved_base_url: Option<String>,
) -> bool {
    let remote_key = normalize_profile_key(remote);
    let target_idx = local_profiles
        .iter()
        .position(|candidate| candidate.id == remote.id)
        .or_else(|| {
            if remote_key.is_empty() {
                None
            } else {
                local_profiles
                    .iter()
                    .position(|candidate| normalize_profile_key(candidate) == remote_key)
            }
        });

    if let Some(idx) = target_idx {
        let existing = &mut local_profiles[idx];
        if existing.name.trim().is_empty() && !remote.name.trim().is_empty() {
            existing.name = remote.name.clone();
        }
        if existing
            .description
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            existing.description = remote.description.clone();
        }
        if existing.provider.trim().is_empty() && !remote.provider.trim().is_empty() {
            existing.provider = remote.provider.clone();
        }
        if existing.model.trim().is_empty() && !remote.model.trim().is_empty() {
            existing.model = remote.model.clone();
        }
        if existing.auth_ref.trim().is_empty() && !remote.auth_ref.trim().is_empty() {
            existing.auth_ref = remote.auth_ref.clone();
        }
        if !is_non_empty(existing.base_url.as_deref()) && is_non_empty(remote.base_url.as_deref()) {
            existing.base_url = remote.base_url.clone();
        }
        if !is_non_empty(existing.base_url.as_deref()) && is_non_empty(resolved_base_url.as_deref())
        {
            existing.base_url = resolved_base_url;
        }
        if is_non_empty(resolved_api_key.as_deref()) {
            existing.api_key = resolved_api_key;
        } else if !is_non_empty(existing.api_key.as_deref())
            && is_non_empty(remote.api_key.as_deref())
        {
            existing.api_key = remote.api_key.clone();
        }
        if !existing.enabled && remote.enabled {
            existing.enabled = true;
        }
        return false;
    }

    let mut merged = remote.clone();
    if is_non_empty(resolved_api_key.as_deref()) {
        merged.api_key = resolved_api_key;
    }
    if !is_non_empty(merged.base_url.as_deref()) && is_non_empty(resolved_base_url.as_deref()) {
        merged.base_url = resolved_base_url;
    }
    local_profiles.push(merged);
    true
}

#[tauri::command]
pub async fn remote_list_model_profiles(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<ModelProfile>, String> {
    let content = pool
        .sftp_read(&host_id, "~/.clawpal/model-profiles.json")
        .await
        .unwrap_or_else(|_| r#"{"profiles":[]}"#.to_string());
    Ok(clawpal_core::profile::list_profiles_from_storage_json(
        &content,
    ))
}

#[tauri::command]
pub async fn remote_upsert_model_profile(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    profile: ModelProfile,
) -> Result<ModelProfile, String> {
    let content = pool
        .sftp_read(&host_id, "~/.clawpal/model-profiles.json")
        .await
        .unwrap_or_else(|_| r#"{"profiles":[]}"#.to_string());
    let (saved, next_json) =
        clawpal_core::profile::upsert_profile_in_storage_json(&content, profile)
            .map_err(|e| e.to_string())?;

    let _ = pool.exec(&host_id, "mkdir -p ~/.clawpal").await;
    pool.sftp_write(&host_id, "~/.clawpal/model-profiles.json", &next_json)
        .await?;
    Ok(saved)
}

#[tauri::command]
pub async fn remote_delete_model_profile(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    profile_id: String,
) -> Result<bool, String> {
    let content = pool
        .sftp_read(&host_id, "~/.clawpal/model-profiles.json")
        .await
        .unwrap_or_else(|_| r#"{"profiles":[]}"#.to_string());
    let (removed, next_json) =
        clawpal_core::profile::delete_profile_from_storage_json(&content, &profile_id)
            .map_err(|e| e.to_string())?;
    if !removed {
        return Ok(false);
    }
    pool.sftp_write(&host_id, "~/.clawpal/model-profiles.json", &next_json)
        .await?;
    Ok(true)
}

#[tauri::command]
pub async fn remote_resolve_api_keys(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<ResolvedApiKey>, String> {
    let content = match pool
        .sftp_read(&host_id, "~/.clawpal/model-profiles.json")
        .await
    {
        Ok(content) => content,
        Err(e) if is_remote_missing_path_error(&e) => r#"{"profiles":[]}"#.to_string(),
        Err(e) => return Err(format!("Failed to read remote model profiles: {e}")),
    };
    let profiles = clawpal_core::profile::list_profiles_from_storage_json(&content);
    let mut out = Vec::new();
    for profile in &profiles {
        let masked = if let Some(ref key) = profile.api_key {
            if key.len() > 8 {
                format!("{}...{}", &key[..4], &key[key.len() - 4..])
            } else if !key.is_empty() {
                "****".to_string()
            } else if !profile.auth_ref.is_empty() {
                format!("via {}", profile.auth_ref)
            } else {
                "not set".to_string()
            }
        } else if !profile.auth_ref.is_empty() {
            format!("via {}", profile.auth_ref)
        } else {
            "not set".to_string()
        };
        out.push(ResolvedApiKey {
            profile_id: profile.id.clone(),
            masked_key: masked,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn remote_test_model_profile(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    profile_id: String,
) -> Result<bool, String> {
    let content = match pool
        .sftp_read(&host_id, "~/.clawpal/model-profiles.json")
        .await
    {
        Ok(content) => content,
        Err(e) if is_remote_missing_path_error(&e) => r#"{"profiles":[]}"#.to_string(),
        Err(e) => return Err(format!("Failed to read remote model profiles: {e}")),
    };
    let profile = clawpal_core::profile::find_profile_in_storage_json(&content, &profile_id)
        .map_err(|e| format!("Failed to parse remote model profiles: {e}"))?
        .ok_or_else(|| format!("Profile not found: {profile_id}"))?;

    if !profile.enabled {
        return Err("Profile is disabled".into());
    }

    let api_key = resolve_remote_profile_api_key(&pool, &host_id, &profile).await?;
    if api_key.trim().is_empty() && !provider_supports_optional_api_key(&profile.provider) {
        let hint = missing_profile_auth_hint(&profile.provider, true);
        return Err(
            format!("No API key resolved for this remote profile. Set apiKey directly, configure auth_ref in remote auth store (auth-profiles.json/auth.json), or export auth_ref on remote shell.{hint}"),
        );
    }

    let resolved_base_url = resolve_remote_profile_base_url(&pool, &host_id, &profile).await?;

    tauri::async_runtime::spawn_blocking(move || {
        run_provider_probe(profile.provider, profile.model, resolved_base_url, api_key)
    })
    .await
    .map_err(|e| format!("Task join failed: {e}"))??;

    Ok(true)
}

#[tauri::command]
pub async fn remote_extract_model_profiles_from_config(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<ExtractModelProfilesResult, String> {
    let (_config_path, _raw, cfg) =
        remote_read_openclaw_config_text_and_json(&pool, &host_id).await?;

    let profiles_raw = pool
        .sftp_read(&host_id, "~/.clawpal/model-profiles.json")
        .await
        .unwrap_or_else(|_| r#"{"profiles":[]}"#.to_string());
    let profiles = clawpal_core::profile::list_profiles_from_storage_json(&profiles_raw);

    let bindings = collect_model_bindings(&cfg, &profiles);
    let mut created = 0usize;
    let mut reused = 0usize;
    let mut skipped_invalid = 0usize;
    let mut seen = HashSet::new();

    let mut next_profiles = profiles;
    let mut model_profile_map: HashMap<String, String> = HashMap::new();
    for profile in &next_profiles {
        model_profile_map.insert(
            normalize_model_ref(&profile_to_model_value(profile)),
            profile.id.clone(),
        );
    }

    for binding in bindings {
        let scope_label = match binding.scope.as_str() {
            "global" => "global".to_string(),
            "agent" => format!("agent:{}", binding.scope_id),
            "channel" => format!("channel:{}", binding.scope_id),
            _ => binding.scope_id,
        };
        let Some(model_ref) = binding.model_value else {
            continue;
        };
        let model_ref = normalize_model_ref(&model_ref);
        if model_ref.trim().is_empty() {
            continue;
        }
        if model_profile_map.contains_key(&model_ref) || seen.contains(&model_ref) {
            reused += 1;
            continue;
        }
        let mut parts = model_ref.splitn(2, '/');
        let provider = parts.next().unwrap_or("").trim();
        let model = parts.next().unwrap_or("").trim();
        if provider.is_empty() || model.is_empty() {
            skipped_invalid += 1;
            continue;
        }
        let auth_ref = resolve_auth_ref_for_provider(&cfg, provider)
            .unwrap_or_else(|| format!("{provider}:default"));
        let base_url = resolve_model_provider_base_url(&cfg, provider);
        let new_profile = ModelProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name: format!("{scope_label} model profile"),
            provider: provider.to_string(),
            model: model.to_string(),
            auth_ref,
            api_key: None,
            base_url,
            description: Some(format!("Extracted from config ({scope_label})")),
            enabled: true,
        };
        let key = profile_to_model_value(&new_profile);
        model_profile_map.insert(normalize_model_ref(&key), new_profile.id.clone());
        next_profiles.push(new_profile);
        seen.insert(model_ref);
        created += 1;
    }

    if created > 0 {
        let text = clawpal_core::profile::render_profiles_storage_json(&next_profiles)
            .map_err(|e| e.to_string())?;
        let _ = pool.exec(&host_id, "mkdir -p ~/.clawpal").await;
        pool.sftp_write(&host_id, "~/.clawpal/model-profiles.json", &text)
            .await?;
    }

    Ok(ExtractModelProfilesResult {
        created,
        reused,
        skipped_invalid,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAuthSyncResult {
    pub total_remote_profiles: usize,
    pub synced_profiles: usize,
    pub created_profiles: usize,
    pub updated_profiles: usize,
    pub resolved_keys: usize,
    pub unresolved_keys: usize,
    pub failed_key_resolves: usize,
}

#[tauri::command]
pub async fn remote_sync_profiles_to_local_auth(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<RemoteAuthSyncResult, String> {
    let content = match pool
        .sftp_read(&host_id, "~/.clawpal/model-profiles.json")
        .await
    {
        Ok(content) => content,
        Err(e) if is_remote_missing_path_error(&e) => r#"{"profiles":[]}"#.to_string(),
        Err(e) => return Err(format!("Failed to read remote model profiles: {e}")),
    };
    let remote_profiles = clawpal_core::profile::list_profiles_from_storage_json(&content);
    if remote_profiles.is_empty() {
        return Ok(RemoteAuthSyncResult {
            total_remote_profiles: 0,
            synced_profiles: 0,
            created_profiles: 0,
            updated_profiles: 0,
            resolved_keys: 0,
            unresolved_keys: 0,
            failed_key_resolves: 0,
        });
    }

    let paths = resolve_paths();
    let mut local_profiles = dedupe_profiles_by_model_key(load_model_profiles(&paths));

    let mut created_profiles = 0usize;
    let mut updated_profiles = 0usize;
    let mut resolved_keys = 0usize;
    let mut unresolved_keys = 0usize;
    let mut failed_key_resolves = 0usize;

    // Pre-fetch all needed remote env vars and auth-store files in bulk
    // (~3 SSH calls total instead of 5-7 per profile).
    let auth_cache = match RemoteAuthCache::build(&pool, &host_id, &remote_profiles).await {
        Ok(cache) => Some(cache),
        Err(_) => None,
    };

    for remote in &remote_profiles {
        let mut resolved_api_key: Option<String> = None;
        if let Some(ref cache) = auth_cache {
            let key = cache.resolve_for_profile(remote);
            if !key.trim().is_empty() {
                resolved_api_key = Some(key);
                resolved_keys += 1;
            } else {
                unresolved_keys += 1;
            }
        } else {
            // Fallback to per-profile resolution if cache build failed.
            match resolve_remote_profile_api_key(&pool, &host_id, remote).await {
                Ok(api_key) if !api_key.trim().is_empty() => {
                    resolved_api_key = Some(api_key);
                    resolved_keys += 1;
                }
                Ok(_) => {
                    unresolved_keys += 1;
                }
                Err(_) => {
                    failed_key_resolves += 1;
                }
            }
        }

        let resolved_base_url = if remote
            .base_url
            .as_deref()
            .map(str::trim)
            .is_some_and(|v| !v.is_empty())
        {
            None
        } else {
            match resolve_remote_profile_base_url(&pool, &host_id, remote).await {
                Ok(Some(remote_base)) if !remote_base.trim().is_empty() => {
                    Some(remote_base.trim().to_string())
                }
                _ => None,
            }
        };

        if merge_remote_profile_into_local(
            &mut local_profiles,
            remote,
            resolved_api_key,
            resolved_base_url,
        ) {
            created_profiles += 1;
        } else {
            updated_profiles += 1;
        }
    }

    save_model_profiles(&paths, &local_profiles)?;

    Ok(RemoteAuthSyncResult {
        total_remote_profiles: remote_profiles.len(),
        synced_profiles: created_profiles + updated_profiles,
        created_profiles,
        updated_profiles,
        resolved_keys,
        unresolved_keys,
        failed_key_resolves,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedSecretPushResult {
    pub total_related_providers: usize,
    pub resolved_secrets: usize,
    pub written_secrets: usize,
    pub skipped_providers: usize,
    pub failed_providers: usize,
}

fn provider_from_model_ref(model_ref: &str) -> Option<String> {
    let trimmed = normalize_model_ref(model_ref);
    let mut parts = trimmed.splitn(2, '/');
    let provider = parts.next()?.trim().to_ascii_lowercase();
    let model = parts.next().unwrap_or("").trim();
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some(provider)
}

fn collect_related_remote_providers(
    cfg: &Value,
    remote_profiles: &[ModelProfile],
) -> HashSet<String> {
    let mut out = HashSet::<String>::new();

    for profile in remote_profiles.iter().filter(|profile| profile.enabled) {
        let provider = profile.provider.trim().to_ascii_lowercase();
        if !provider.is_empty() {
            out.insert(provider);
        }
        if let Some(provider_from_model) = provider_from_model_ref(&profile.model) {
            out.insert(provider_from_model);
        }
    }

    let bindings = collect_model_bindings(cfg, remote_profiles);
    for binding in bindings {
        let Some(model_ref) = binding.model_value else {
            continue;
        };
        if let Some(provider) = provider_from_model_ref(&model_ref) {
            out.insert(provider);
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpsertAuthStoreResult {
    Written,
    Unchanged,
    Failed,
}

fn upsert_auth_store_entry(
    root: &mut Value,
    provider: &str,
    credential: &InternalProviderCredential,
) -> UpsertAuthStoreResult {
    if provider.trim().is_empty() {
        return UpsertAuthStoreResult::Failed;
    }
    if !root.is_object() {
        *root = serde_json::json!({ "version": 1 });
    }
    let Some(root_obj) = root.as_object_mut() else {
        return UpsertAuthStoreResult::Failed;
    };
    if !root_obj.contains_key("version") {
        root_obj.insert("version".into(), Value::from(1_u64));
    }
    let profiles_val = root_obj
        .entry("profiles".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !profiles_val.is_object() {
        return UpsertAuthStoreResult::Failed;
    }
    let auth_ref = format!("{provider}:default");
    let auth_payload = match credential.kind {
        InternalAuthKind::Authorization => serde_json::json!({
            "type": "token",
            "provider": provider,
            "token": credential.secret,
        }),
        InternalAuthKind::ApiKey => serde_json::json!({
            "type": "api_key",
            "provider": provider,
            "key": credential.secret,
        }),
    };
    let mut changed = false;
    let Some(profiles) = profiles_val.as_object_mut() else {
        return UpsertAuthStoreResult::Failed;
    };
    let replace = match profiles.get(&auth_ref) {
        Some(existing) => existing != &auth_payload,
        None => true,
    };
    if replace {
        profiles.insert(auth_ref.clone(), auth_payload);
        changed = true;
    }
    let last_good = root_obj
        .entry("lastGood".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !last_good.is_object() {
        return UpsertAuthStoreResult::Failed;
    }
    let Some(last_good_map) = last_good.as_object_mut() else {
        return UpsertAuthStoreResult::Failed;
    };
    let needs_update = last_good_map
        .get(provider)
        .and_then(Value::as_str)
        .map(|value| value != auth_ref)
        .unwrap_or(true);
    if needs_update {
        last_good_map.insert(provider.to_string(), Value::String(auth_ref));
        changed = true;
    }
    if changed {
        UpsertAuthStoreResult::Written
    } else {
        UpsertAuthStoreResult::Unchanged
    }
}

#[tauri::command]
pub async fn push_related_secrets_to_remote(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<RelatedSecretPushResult, String> {
    let (_, _, cfg) = remote_read_openclaw_config_text_and_json(&pool, &host_id).await?;

    let remote_profiles_raw = match pool
        .sftp_read(&host_id, "~/.clawpal/model-profiles.json")
        .await
    {
        Ok(content) => content,
        Err(e) if is_remote_missing_path_error(&e) => r#"{"profiles":[]}"#.to_string(),
        Err(e) => return Err(format!("Failed to read remote model profiles: {e}")),
    };
    let remote_profiles =
        clawpal_core::profile::list_profiles_from_storage_json(&remote_profiles_raw);
    let related = collect_related_remote_providers(&cfg, &remote_profiles);

    if related.is_empty() {
        return Ok(RelatedSecretPushResult {
            total_related_providers: 0,
            resolved_secrets: 0,
            written_secrets: 0,
            skipped_providers: 0,
            failed_providers: 0,
        });
    }

    // Secret provider resolution may execute external commands with timeouts.
    // Run it on the blocking pool so async command threads stay responsive.
    let local_credentials =
        tauri::async_runtime::spawn_blocking(collect_provider_credentials_for_internal)
            .await
            .map_err(|e| format!("Failed to resolve local provider credentials: {e}"))?;
    let mut providers = related.into_iter().collect::<Vec<_>>();
    providers.sort();

    let mut selected = Vec::<(String, InternalProviderCredential)>::new();
    let mut skipped = 0usize;
    for provider in &providers {
        if let Some(credential) = local_credentials.get(provider) {
            selected.push((provider.clone(), credential.clone()));
        } else {
            skipped += 1;
        }
    }

    if selected.is_empty() {
        return Ok(RelatedSecretPushResult {
            total_related_providers: providers.len(),
            resolved_secrets: 0,
            written_secrets: 0,
            skipped_providers: skipped,
            failed_providers: 0,
        });
    }

    let roots = resolve_remote_openclaw_roots(&pool, &host_id).await?;
    let root = roots
        .first()
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Failed to resolve remote openclaw root".to_string())?;
    let root = root.trim_end_matches('/');
    let remote_auth_dir = format!("{root}/agents/main/agent");
    let remote_auth_path = format!("{remote_auth_dir}/auth-profiles.json");
    let remote_auth_raw = match pool.sftp_read(&host_id, &remote_auth_path).await {
        Ok(content) => content,
        Err(e) if is_remote_missing_path_error(&e) => r#"{"version":1,"profiles":{}}"#.to_string(),
        Err(e) => return Err(format!("Failed to read remote auth store: {e}")),
    };
    let mut remote_auth_json: Value = serde_json::from_str(&remote_auth_raw)
        .map_err(|e| format!("Failed to parse remote auth store at {remote_auth_path}: {e}"))?;

    let mut written = 0usize;
    let mut failed = 0usize;
    for (provider, credential) in &selected {
        match upsert_auth_store_entry(&mut remote_auth_json, provider, credential) {
            UpsertAuthStoreResult::Written => written += 1,
            UpsertAuthStoreResult::Unchanged => {}
            UpsertAuthStoreResult::Failed => failed += 1,
        }
    }

    if written > 0 {
        let serialized = serde_json::to_string_pretty(&remote_auth_json)
            .map_err(|e| format!("Failed to serialize remote auth store: {e}"))?;
        let mkdir_cmd = format!("mkdir -p {}", shell_escape(&remote_auth_dir));
        let _ = pool.exec(&host_id, &mkdir_cmd).await;
        pool.sftp_write(&host_id, &remote_auth_path, &serialized)
            .await?;
    }

    Ok(RelatedSecretPushResult {
        total_related_providers: providers.len(),
        resolved_secrets: selected.len(),
        written_secrets: written,
        skipped_providers: skipped,
        failed_providers: failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("clawpal-profiles-{prefix}-{unique}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn profile(
        id: &str,
        provider: &str,
        model: &str,
        auth_ref: &str,
        api_key: Option<&str>,
    ) -> ModelProfile {
        ModelProfile {
            id: id.to_string(),
            name: format!("{provider}/{model}"),
            provider: provider.to_string(),
            model: model.to_string(),
            auth_ref: auth_ref.to_string(),
            api_key: api_key.map(|v| v.to_string()),
            base_url: None,
            description: None,
            enabled: true,
        }
    }

    #[test]
    fn merge_remote_profile_reuses_local_entry_by_provider_model() {
        let mut local = vec![profile(
            "local-1",
            "anthropic",
            "claude-4-5",
            "anthropic:default",
            Some("local-key"),
        )];
        let remote = profile(
            "remote-9",
            "anthropic",
            "claude-4-5",
            "anthropic:remote",
            None,
        );

        let created = merge_remote_profile_into_local(&mut local, &remote, None, None);

        assert!(!created);
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].id, "local-1");
        assert_eq!(local[0].api_key.as_deref(), Some("local-key"));
        assert_eq!(local[0].auth_ref, "anthropic:default");
    }

    #[test]
    fn merge_remote_profile_fills_missing_local_key_from_resolved_remote() {
        let mut local = vec![profile(
            "local-2",
            "openai",
            "gpt-4.1",
            "openai:default",
            None,
        )];
        let remote = profile("remote-2", "openai", "gpt-4.1", "openai:default", None);

        let created = merge_remote_profile_into_local(
            &mut local,
            &remote,
            Some("resolved-remote-key".to_string()),
            None,
        );

        assert!(!created);
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].api_key.as_deref(), Some("resolved-remote-key"));
    }

    #[test]
    fn merge_remote_profile_prefers_resolved_key_over_stale_remote_key() {
        let mut local = vec![profile(
            "local-3",
            "anthropic",
            "claude-4-5",
            "anthropic:default",
            None,
        )];
        let remote = profile(
            "remote-3",
            "anthropic",
            "claude-4-5",
            "anthropic:default",
            Some("stale-remote-key"),
        );

        let created = merge_remote_profile_into_local(
            &mut local,
            &remote,
            Some("resolved-valid-key".to_string()),
            None,
        );

        assert!(!created);
        assert_eq!(local[0].api_key.as_deref(), Some("resolved-valid-key"));
    }

    #[test]
    fn dedupe_profiles_prefers_entry_with_api_key() {
        let weak = profile("weak", "anthropic", "claude-4-5", "", None);
        let strong = profile(
            "strong",
            "anthropic",
            "claude-4-5",
            "anthropic:default",
            Some("k-123"),
        );

        let deduped = dedupe_profiles_by_model_key(vec![weak, strong]);

        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].id, "strong");
        assert_eq!(deduped[0].api_key.as_deref(), Some("k-123"));
    }

    #[test]
    fn oauth_provider_normalization_maps_codex_aliases() {
        assert_eq!(
            normalize_zeroclaw_oauth_provider("openai-codex"),
            Some("openai-codex")
        );
        assert_eq!(
            normalize_zeroclaw_oauth_provider("github-copilot"),
            Some("openai-codex")
        );
        assert_eq!(
            normalize_zeroclaw_oauth_provider("copilot"),
            Some("openai-codex")
        );
        assert_eq!(
            normalize_zeroclaw_oauth_provider("openai"),
            Some("openai-codex")
        );
    }

    #[test]
    fn extract_first_http_url_parses_login_output() {
        let text = "Open this URL in your browser:\nhttps://auth.openai.com/oauth/authorize?foo=bar\nWaiting for callback ...";
        let url = extract_first_http_url(text).expect("url");
        assert!(url.starts_with("https://auth.openai.com/oauth/authorize"));
    }

    #[test]
    fn oauth_auth_ref_uses_provider_and_profile_name() {
        assert_eq!(
            oauth_auth_ref("openai-codex", "default"),
            "openai-codex:default"
        );
    }

    #[test]
    fn oauth_store_file_detects_openai_codex_profile_in_new_format() {
        let dir = make_temp_dir("oauth-openai-codex-new");
        std::fs::write(
            dir.join("auth-profiles.json"),
            r#"{
              "active_profiles": { "openai-codex": "openai-codex:default" },
              "profiles": {
                "openai-codex:default": {
                  "provider": "openai-codex",
                  "profile_name": "default"
                }
              }
            }"#,
        )
        .expect("write auth profiles");

        assert!(oauth_store_file_has_profile(
            &dir,
            "openai-codex",
            "default"
        ));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn oauth_store_file_detects_openai_codex_profile_in_legacy_openai_format() {
        let dir = make_temp_dir("oauth-openai-codex-legacy");
        std::fs::write(
            dir.join("auth-profiles.json"),
            r#"{
              "active_profiles": { "openai": "openai:default" },
              "profiles": {
                "openai:default": {
                  "provider": "openai",
                  "profile_name": "default"
                }
              }
            }"#,
        )
        .expect("write auth profiles");

        assert!(oauth_store_file_has_profile(
            &dir,
            "openai-codex",
            "default"
        ));

        let _ = std::fs::remove_dir_all(dir);
    }
}

#[tauri::command]
pub fn get_cached_model_catalog() -> Result<Vec<ModelCatalogProvider>, String> {
    let paths = resolve_paths();
    let cache_path = model_catalog_cache_path(&paths);
    let current_version = resolve_openclaw_version();
    if let Some(catalog) = select_catalog_from_cache(
        read_model_catalog_cache(&cache_path).as_ref(),
        &current_version,
    ) {
        return Ok(catalog);
    }
    Ok(Vec::new())
}

#[tauri::command]
pub fn refresh_model_catalog() -> Result<Vec<ModelCatalogProvider>, String> {
    let paths = resolve_paths();
    load_model_catalog(&paths)
}

#[tauri::command]
pub fn list_model_profiles() -> Result<Vec<ModelProfile>, String> {
    let openclaw = clawpal_core::openclaw::OpenclawCli::new();
    clawpal_core::profile::list_profiles(&openclaw).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn extract_model_profiles_from_config() -> Result<ExtractModelProfilesResult, String> {
    let paths = resolve_paths();
    let cfg = read_openclaw_config(&paths)?;
    let profiles = load_model_profiles(&paths);
    let bindings = collect_model_bindings(&cfg, &profiles);
    let mut created = 0usize;
    let mut reused = 0usize;
    let mut skipped_invalid = 0usize;
    let mut seen = HashSet::new();

    let mut next_profiles = profiles;
    let mut model_profile_map: HashMap<String, String> = HashMap::new();
    for profile in &next_profiles {
        model_profile_map.insert(
            normalize_model_ref(&profile_to_model_value(profile)),
            profile.id.clone(),
        );
    }

    for binding in bindings {
        let scope_label = match binding.scope.as_str() {
            "global" => "global".to_string(),
            "agent" => format!("agent:{}", binding.scope_id),
            "channel" => format!("channel:{}", binding.scope_id),
            _ => binding.scope_id,
        };
        let Some(model_ref) = binding.model_value else {
            continue;
        };
        let model_ref = normalize_model_ref(&model_ref);
        if model_ref.trim().is_empty() {
            continue;
        }
        if model_profile_map.contains_key(&model_ref) || seen.contains(&model_ref) {
            reused += 1;
            continue;
        }
        let mut parts = model_ref.splitn(2, '/');
        let provider = parts.next().unwrap_or("").trim();
        let model = parts.next().unwrap_or("").trim();
        if provider.is_empty() || model.is_empty() {
            skipped_invalid += 1;
            continue;
        }
        let auth_ref = resolve_auth_ref_for_provider(&cfg, provider)
            .unwrap_or_else(|| format!("{provider}:default"));
        let base_url = resolve_model_provider_base_url(&cfg, provider);
        let profile = ModelProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name: format!("{scope_label} model profile"),
            provider: provider.to_string(),
            model: model.to_string(),
            auth_ref,
            api_key: None,
            base_url,
            description: Some(format!("Extracted from config ({scope_label})")),
            enabled: true,
        };
        let key = profile_to_model_value(&profile);
        model_profile_map.insert(normalize_model_ref(&key), profile.id.clone());
        next_profiles.push(profile);
        seen.insert(model_ref);
        created += 1;
    }

    if created > 0 {
        save_model_profiles(&paths, &next_profiles)?;
    }

    Ok(ExtractModelProfilesResult {
        created,
        reused,
        skipped_invalid,
    })
}

#[tauri::command]
pub fn upsert_model_profile(profile: ModelProfile) -> Result<ModelProfile, String> {
    let paths = resolve_paths();
    let path = model_profiles_path(&paths);
    let content = std::fs::read_to_string(&path).unwrap_or_else(|_| r#"{"profiles":[]}"#.into());
    let (saved, next_json) =
        clawpal_core::profile::upsert_profile_in_storage_json(&content, profile)
            .map_err(|e| e.to_string())?;
    crate::config_io::write_text(&path, &next_json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(saved)
}

#[tauri::command]
pub async fn start_zeroclaw_oauth_login(
    provider: String,
    profile: Option<String>,
    instance_id: Option<String>,
) -> Result<ZeroclawOauthLoginStartResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let normalized_provider = normalize_zeroclaw_oauth_provider(&provider)
            .ok_or_else(|| format!("Unsupported OAuth provider: {}", provider.trim()))?;
        let profile_name = normalize_oauth_profile_name(profile);
        let auth_ref = oauth_auth_ref(normalized_provider, &profile_name);
        let instance = instance_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("local")
            .to_string();
        let cfg_dir =
            crate::runtime::zeroclaw::process::zeroclaw_oauth_config_dir_for_instance(&instance)?;
        let cmd_path =
            crate::runtime::zeroclaw::process::resolve_zeroclaw_command_path_for_internal()
                .ok_or_else(|| "zeroclaw binary not found in bundled resources".to_string())?;
        let cfg_arg = cfg_dir.to_string_lossy().to_string();
        let args = vec![
            "--config-dir".to_string(),
            cfg_arg,
            "auth".to_string(),
            "login".to_string(),
            "--provider".to_string(),
            normalized_provider.to_string(),
            "--profile".to_string(),
            profile_name.clone(),
        ];
        let (exit_code, stdout, stderr, timed_out) = run_command_with_timeout_capture(
            &cmd_path,
            &args,
            ZEROCLAW_OAUTH_LOGIN_CAPTURE_TIMEOUT_SECS,
            "zeroclaw auth login",
        )?;
        let merged = format!("{}\n{}", stdout, stderr);
        let details = trim_command_details(&merged);
        let authorize_url = extract_first_http_url(&merged).ok_or_else(|| {
            if !timed_out && exit_code != 0 {
                format!(
                    "zeroclaw auth login failed ({exit_code}). {}",
                    if details.is_empty() {
                        "No diagnostic details from zeroclaw.".to_string()
                    } else {
                        details.clone()
                    }
                )
            } else if timed_out {
                format!(
                    "OAuth login timed out after {}s and no authorization URL was detected. {}",
                    ZEROCLAW_OAUTH_LOGIN_CAPTURE_TIMEOUT_SECS,
                    if details.is_empty() {
                        "Try again and ensure zeroclaw auth login output is visible.".to_string()
                    } else {
                        details.clone()
                    }
                )
            } else {
                "OAuth login did not return an authorization URL.".to_string()
            }
        })?;
        Ok(ZeroclawOauthLoginStartResult {
            provider: normalized_provider.to_string(),
            profile: profile_name,
            auth_ref,
            authorize_url,
            details,
        })
    })
    .await
    .map_err(|e| format!("Task join failed: {e}"))?
}

#[tauri::command]
pub async fn complete_zeroclaw_oauth_login(
    provider: String,
    redirect_input: String,
    profile: Option<String>,
    instance_id: Option<String>,
) -> Result<ZeroclawOauthCompleteResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let normalized_provider = normalize_zeroclaw_oauth_provider(&provider)
            .ok_or_else(|| format!("Unsupported OAuth provider: {}", provider.trim()))?;
        let profile_name = normalize_oauth_profile_name(profile);
        let auth_ref = oauth_auth_ref(normalized_provider, &profile_name);
        let input = redirect_input.trim();
        if input.is_empty() {
            return Err("OAuth redirect URL or code is required.".to_string());
        }
        let instance = instance_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("local")
            .to_string();
        let cfg_dir =
            crate::runtime::zeroclaw::process::zeroclaw_oauth_config_dir_for_instance(&instance)?;
        let cmd_path =
            crate::runtime::zeroclaw::process::resolve_zeroclaw_command_path_for_internal()
                .ok_or_else(|| "zeroclaw binary not found in bundled resources".to_string())?;
        let cfg_arg = cfg_dir.to_string_lossy().to_string();
        let paste_args = vec![
            "--config-dir".to_string(),
            cfg_arg.clone(),
            "auth".to_string(),
            "paste-redirect".to_string(),
            "--provider".to_string(),
            normalized_provider.to_string(),
            "--profile".to_string(),
            profile_name.clone(),
            "--input".to_string(),
            input.to_string(),
        ];
        let (paste_code, paste_stdout, paste_stderr, paste_timed_out) =
            run_command_with_timeout_capture(
                &cmd_path,
                &paste_args,
                ZEROCLAW_OAUTH_COMPLETE_TIMEOUT_SECS,
                "zeroclaw auth paste-redirect",
            )?;
        if paste_timed_out {
            return Err(format!(
                "zeroclaw auth paste-redirect timed out after {}s.",
                ZEROCLAW_OAUTH_COMPLETE_TIMEOUT_SECS
            ));
        }
        if paste_code != 0 {
            let details = trim_command_details(&format!("{}\n{}", paste_stdout, paste_stderr));
            return Err(format!(
                "zeroclaw auth paste-redirect failed ({paste_code}): {}",
                if details.is_empty() {
                    "No diagnostic details from zeroclaw.".to_string()
                } else {
                    details
                }
            ));
        }

        let use_args = vec![
            "--config-dir".to_string(),
            cfg_arg,
            "auth".to_string(),
            "use".to_string(),
            "--provider".to_string(),
            normalized_provider.to_string(),
            "--profile".to_string(),
            profile_name.clone(),
        ];
        let (use_code, use_stdout, use_stderr, use_timed_out) = run_command_with_timeout_capture(
            &cmd_path,
            &use_args,
            ZEROCLAW_OAUTH_COMPLETE_TIMEOUT_SECS,
            "zeroclaw auth use",
        )?;
        if use_timed_out {
            return Err(format!(
                "zeroclaw auth use timed out after {}s.",
                ZEROCLAW_OAUTH_COMPLETE_TIMEOUT_SECS
            ));
        }
        if use_code != 0 {
            let details = trim_command_details(&format!("{}\n{}", use_stdout, use_stderr));
            return Err(format!(
                "zeroclaw auth use failed ({use_code}): {}",
                if details.is_empty() {
                    "No diagnostic details from zeroclaw.".to_string()
                } else {
                    details
                }
            ));
        }

        let details = trim_command_details(&format!(
            "{}\n{}\n{}\n{}",
            paste_stdout, paste_stderr, use_stdout, use_stderr
        ));
        Ok(ZeroclawOauthCompleteResult {
            provider: normalized_provider.to_string(),
            profile: profile_name,
            auth_ref,
            details,
        })
    })
    .await
    .map_err(|e| format!("Task join failed: {e}"))?
}

#[tauri::command]
pub fn delete_model_profile(profile_id: String) -> Result<bool, String> {
    let openclaw = clawpal_core::openclaw::OpenclawCli::new();
    clawpal_core::profile::delete_profile(&openclaw, &profile_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resolve_provider_auth(provider: String) -> Result<ProviderAuthSuggestion, String> {
    let provider_trimmed = provider.trim();
    if provider_trimmed.is_empty() {
        return Ok(ProviderAuthSuggestion {
            auth_ref: None,
            has_key: false,
            source: String::new(),
        });
    }
    let paths = resolve_paths();
    let cfg = read_openclaw_config(&paths)?;
    let global_base = local_global_openclaw_base_dir();

    // 1. Check openclaw config auth profiles
    if let Some(auth_ref) = resolve_auth_ref_for_provider(&cfg, provider_trimmed) {
        let probe_profile = ModelProfile {
            id: "provider-auth-probe".into(),
            name: "provider-auth-probe".into(),
            provider: provider_trimmed.to_string(),
            model: "probe".into(),
            auth_ref: auth_ref.clone(),
            api_key: None,
            base_url: None,
            description: None,
            enabled: true,
        };
        let key = resolve_profile_api_key(&probe_profile, &global_base);
        if !key.trim().is_empty() {
            return Ok(ProviderAuthSuggestion {
                auth_ref: Some(auth_ref),
                has_key: true,
                source: "openclaw auth profile".into(),
            });
        }
    }

    // 2. Check zeroclaw OAuth auth store (local instance)
    if let Some(normalized_provider) = normalize_zeroclaw_oauth_provider(provider_trimmed) {
        let default_profile = "default";
        if oauth_store_has_profile("local", normalized_provider, default_profile)
            || oauth_store_has_profile_any_instance(normalized_provider, default_profile)
        {
            return Ok(ProviderAuthSuggestion {
                auth_ref: Some(oauth_auth_ref(normalized_provider, default_profile)),
                has_key: true,
                source: "zeroclaw oauth profile".into(),
            });
        }
    }

    // 3. Check env vars
    for env_name in provider_env_var_candidates(provider_trimmed) {
        if std::env::var(&env_name)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            return Ok(ProviderAuthSuggestion {
                auth_ref: Some(env_name),
                has_key: true,
                source: "environment variable".into(),
            });
        }
    }

    // 4. Check existing model profiles for this provider
    let profiles = load_model_profiles(&paths);
    for p in &profiles {
        if p.provider.eq_ignore_ascii_case(provider_trimmed) {
            let key = resolve_profile_api_key(p, &global_base);
            if !key.is_empty() {
                let auth_ref = if !p.auth_ref.trim().is_empty() {
                    Some(p.auth_ref.clone())
                } else {
                    None
                };
                return Ok(ProviderAuthSuggestion {
                    auth_ref,
                    has_key: true,
                    source: format!("existing profile {}/{}", p.provider, p.model),
                });
            }
        }
    }

    Ok(ProviderAuthSuggestion {
        auth_ref: None,
        has_key: false,
        source: String::new(),
    })
}

#[tauri::command]
pub fn resolve_api_keys() -> Result<Vec<ResolvedApiKey>, String> {
    let paths = resolve_paths();
    let profiles = load_model_profiles(&paths);
    let global_base = local_global_openclaw_base_dir();
    let mut out = Vec::new();
    for profile in &profiles {
        let key = resolve_profile_api_key(profile, &global_base);
        let masked = mask_api_key(&key);
        out.push(ResolvedApiKey {
            profile_id: profile.id.clone(),
            masked_key: masked,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn test_model_profile(profile_id: String) -> Result<bool, String> {
    let paths = resolve_paths();
    let profiles = load_model_profiles(&paths);
    let profile = profiles
        .into_iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| format!("Profile not found: {profile_id}"))?;

    if !profile.enabled {
        return Err("Profile is disabled".into());
    }

    let global_base = local_global_openclaw_base_dir();
    let api_key = resolve_profile_api_key(&profile, &global_base);
    let normalized_oauth_provider = normalize_zeroclaw_oauth_provider(&profile.provider);
    if api_key.trim().is_empty() {
        if let Some(oauth_provider) = normalized_oauth_provider {
            let oauth_profile_name = profile_name_from_auth_ref(&profile.auth_ref);
            if oauth_store_has_profile("local", oauth_provider, oauth_profile_name)
                || oauth_store_has_profile_any_instance(oauth_provider, oauth_profile_name)
            {
                return Ok(true);
            }
            return Err(format!(
                "No OAuth session found for {oauth_provider}:{oauth_profile_name}. Start OAuth login from Settings and complete authorization first."
            ));
        }
        if !provider_supports_optional_api_key(&profile.provider) {
            let hint = missing_profile_auth_hint(&profile.provider, false);
            return Err(
                format!("No API key resolved for this profile. Set apiKey directly, configure auth_ref in auth store (auth-profiles.json/auth.json), or export auth_ref on local shell.{hint}"),
            );
        }
    }

    let resolved_base_url = profile
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(|| {
            read_openclaw_config(&paths)
                .ok()
                .and_then(|cfg| resolve_model_provider_base_url(&cfg, &profile.provider))
        });

    tauri::async_runtime::spawn_blocking(move || {
        run_provider_probe(profile.provider, profile.model, resolved_base_url, api_key)
    })
    .await
    .map_err(|e| format!("Task join failed: {e}"))??;

    Ok(true)
}

#[tauri::command]
pub async fn remote_refresh_model_catalog(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<ModelCatalogProvider>, String> {
    let paths = resolve_paths();
    let cache_path = remote_model_catalog_cache_path(&paths, &host_id);
    let remote_version = match pool.exec_login(&host_id, "openclaw --version").await {
        Ok(r) => {
            extract_version_from_text(&r.stdout).unwrap_or_else(|| r.stdout.trim().to_string())
        }
        Err(_) => "unknown".into(),
    };
    let cached = read_model_catalog_cache(&cache_path);
    if let Some(selected) = select_catalog_from_cache(cached.as_ref(), &remote_version) {
        return Ok(selected);
    }

    let result = pool
        .exec_login(&host_id, "openclaw models list --all --json --no-color")
        .await;
    if let Ok(r) = result {
        if r.exit_code == 0 && !r.stdout.trim().is_empty() {
            if let Some(catalog) = parse_model_catalog_from_cli_output(&r.stdout) {
                let cache = ModelCatalogProviderCache {
                    cli_version: remote_version,
                    updated_at: unix_timestamp_secs(),
                    providers: catalog.clone(),
                    source: "openclaw models list --all --json".into(),
                    error: None,
                };
                let _ = save_model_catalog_cache(&cache_path, &cache);
                return Ok(catalog);
            }
        }
    }
    if let Some(previous) = cached {
        if !previous.providers.is_empty() && previous.error.is_none() {
            return Ok(previous.providers);
        }
    }
    Err("Failed to load remote model catalog from openclaw CLI".into())
}
