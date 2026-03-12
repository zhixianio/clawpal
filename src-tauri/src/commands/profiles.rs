use super::*;
#[cfg(test)]
use std::path::Path;

fn local_global_openclaw_base_dir() -> std::path::PathBuf {
    resolve_paths().base_dir
}

fn normalize_profile_key(profile: &ModelProfile) -> String {
    normalize_model_ref(&profile_to_model_value(profile))
}

fn is_non_empty(opt: Option<&str>) -> bool {
    opt.map(str::trim).is_some_and(|v| !v.is_empty())
}

fn profile_auth_ref_option(profile: &ModelProfile) -> Option<String> {
    let auth_ref = profile.auth_ref.trim();
    if auth_ref.is_empty() {
        None
    } else {
        Some(auth_ref.to_string())
    }
}

fn build_resolved_api_key(
    profile: &ModelProfile,
    resolved_key: &str,
    source: Option<ResolvedCredentialSource>,
    resolved_override: Option<bool>,
) -> ResolvedApiKey {
    let trimmed = resolved_key.trim();
    ResolvedApiKey {
        profile_id: profile.id.clone(),
        masked_key: mask_api_key(trimmed),
        credential_kind: infer_resolved_credential_kind(profile, source),
        auth_ref: profile_auth_ref_option(profile),
        resolved: resolved_override.unwrap_or(!trimmed.is_empty()),
    }
}

fn oauth_session_ready(profile: &ModelProfile) -> bool {
    let _ = profile;
    false
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

fn should_skip_session_material_sync(profile: &ModelProfile) -> bool {
    if !is_oauth_provider_alias(&profile.provider) {
        return false;
    }
    if profile
        .api_key
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return false;
    }
    let auth_ref = profile.auth_ref.trim();
    auth_ref.is_empty() || is_oauth_auth_ref(&profile.provider, auth_ref)
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

#[cfg(test)]
fn normalize_oauth_provider_alias(provider: &str) -> Option<&'static str> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "openai-codex" | "openai_codex" | "github-copilot" | "copilot" | "openai" => {
            Some("openai-codex")
        }
        _ => None,
    }
}

#[cfg(test)]
fn oauth_store_provider_ids(provider: &str) -> &'static [&'static str] {
    match normalize_oauth_provider_alias(provider) {
        // Backward compatible:
        // - New auth store variants use `openai-codex:*`
        // - Older store variants may still use `openai:*`
        Some("openai-codex") => &["openai-codex", "openai"],
        _ => &[],
    }
}

#[cfg(test)]
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

#[cfg(test)]
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

fn extract_profiles_from_openclaw_config(
    cfg: &Value,
    profiles: Vec<ModelProfile>,
) -> (Vec<ModelProfile>, ExtractModelProfilesResult) {
    let bindings = collect_model_bindings(cfg, &profiles);
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
        let auth_ref = resolve_auth_ref_for_provider(cfg, provider)
            .unwrap_or_else(|| format!("{provider}:default"));
        let base_url = resolve_model_provider_base_url(cfg, provider);
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

    (
        next_profiles,
        ExtractModelProfilesResult {
            created,
            reused,
            skipped_invalid,
        },
    )
}

async fn read_remote_profiles_storage_text(
    pool: &SshConnectionPool,
    host_id: &str,
) -> Result<String, String> {
    match pool
        .sftp_read(host_id, "~/.clawpal/model-profiles.json")
        .await
    {
        Ok(content) => Ok(content),
        Err(e) if is_remote_missing_path_error(&e) => Ok(r#"{"profiles":[]}"#.to_string()),
        Err(e) => Err(format!("Failed to read remote model profiles: {e}")),
    }
}

pub(super) async fn collect_remote_profiles_from_openclaw(
    pool: &SshConnectionPool,
    host_id: &str,
    persist_storage: bool,
) -> Result<(Vec<ModelProfile>, ExtractModelProfilesResult), String> {
    let (_config_path, _raw, cfg) =
        remote_read_openclaw_config_text_and_json(pool, host_id).await?;
    // TODO(clawpal-profile-hub): keep seeding from remote ~/.clawpal/model-profiles.json
    // for backward compatibility for now, but remove this once auto-sync imports only
    // the profiles that are currently bound in remote OpenClaw config/auth state.
    let profiles_raw = read_remote_profiles_storage_text(pool, host_id).await?;
    let profiles = clawpal_core::profile::list_profiles_from_storage_json(&profiles_raw);
    let (next_profiles, result) = extract_profiles_from_openclaw_config(&cfg, profiles);

    if persist_storage && result.created > 0 {
        let text = clawpal_core::profile::render_profiles_storage_json(&next_profiles)
            .map_err(|e| e.to_string())?;
        let _ = pool.exec(host_id, "mkdir -p ~/.clawpal").await;
        pool.sftp_write(host_id, "~/.clawpal/model-profiles.json", &text)
            .await?;
    }

    Ok((next_profiles, result))
}

pub(super) async fn resolve_remote_api_keys_for_profiles(
    pool: &SshConnectionPool,
    host_id: &str,
    profiles: &[ModelProfile],
) -> Vec<ResolvedApiKey> {
    let auth_cache = RemoteAuthCache::build(pool, host_id, profiles).await.ok();

    let mut out = Vec::new();
    for profile in profiles {
        let (resolved_key, source) = if let Some(ref cache) = auth_cache {
            if let Some((key, source)) = cache.resolve_for_profile_with_source(profile) {
                (key, Some(source))
            } else {
                (String::new(), None)
            }
        } else {
            match resolve_remote_profile_api_key(pool, host_id, profile).await {
                Ok(key) => (key, None),
                Err(_) => (String::new(), None),
            }
        };
        let resolved_override = if resolved_key.trim().is_empty() && oauth_session_ready(profile) {
            Some(true)
        } else {
            None
        };
        out.push(build_resolved_api_key(
            profile,
            &resolved_key,
            source,
            resolved_override,
        ));
    }

    out
}

pub async fn remote_list_model_profiles_with_pool(
    pool: &SshConnectionPool,
    host_id: String,
) -> Result<Vec<ModelProfile>, String> {
    let (profiles, _) = collect_remote_profiles_from_openclaw(pool, &host_id, true).await?;
    Ok(profiles)
}

#[tauri::command]
pub async fn remote_list_model_profiles(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<ModelProfile>, String> {
    remote_list_model_profiles_with_pool(pool.inner(), host_id).await
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
    let (profiles, _) = collect_remote_profiles_from_openclaw(&pool, &host_id, true).await?;
    Ok(resolve_remote_api_keys_for_profiles(&pool, &host_id, &profiles).await)
}

#[tauri::command]
pub async fn remote_test_model_profile(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    profile_id: String,
) -> Result<bool, String> {
    let (profiles, _) = collect_remote_profiles_from_openclaw(&pool, &host_id, true).await?;
    let profile = profiles
        .into_iter()
        .find(|candidate| candidate.id == profile_id)
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
    let (_, result) = collect_remote_profiles_from_openclaw(&pool, &host_id, true).await?;
    Ok(result)
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
    let (remote_profiles, _) = collect_remote_profiles_from_openclaw(&pool, &host_id, true).await?;
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
        if !should_skip_session_material_sync(remote) {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilePushResult {
    pub requested_profiles: usize,
    pub pushed_profiles: usize,
    pub written_model_entries: usize,
    pub written_auth_entries: usize,
    pub blocked_profiles: usize,
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
    auth_ref: &str,
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
    let auth_ref = auth_ref.trim();
    if auth_ref.is_empty() {
        return UpsertAuthStoreResult::Failed;
    }
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
    let replace = match profiles.get(auth_ref) {
        Some(existing) => existing != &auth_payload,
        None => true,
    };
    if replace {
        profiles.insert(auth_ref.to_string(), auth_payload);
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
        last_good_map.insert(provider.to_string(), Value::String(auth_ref.to_string()));
        changed = true;
    }
    if changed {
        UpsertAuthStoreResult::Written
    } else {
        UpsertAuthStoreResult::Unchanged
    }
}

#[derive(Debug, Clone)]
struct PreparedProfilePush {
    profile: ModelProfile,
    provider_key: String,
    model_ref: String,
    target_auth_ref: String,
    credential: Option<InternalProviderCredential>,
}

fn target_auth_ref_for_profile(profile: &ModelProfile, provider_key: &str) -> String {
    let auth_ref = profile.auth_ref.trim();
    if !auth_ref.is_empty()
        && auth_ref.contains(':')
        && !is_valid_env_var_name(auth_ref)
        && !is_oauth_auth_ref(&profile.provider, auth_ref)
    {
        return auth_ref.to_string();
    }
    format!("{provider_key}:default")
}

pub(crate) fn profile_target_auth_ref(profile: &ModelProfile) -> String {
    let provider_key = profile.provider.trim().to_ascii_lowercase();
    target_auth_ref_for_profile(profile, &provider_key)
}

fn prepare_profile_for_push(
    profile: &ModelProfile,
    source_base_dir: &Path,
) -> Result<PreparedProfilePush, String> {
    let provider_key = profile.provider.trim().to_ascii_lowercase();
    let model_name = profile.model.trim();
    if provider_key.is_empty() || model_name.is_empty() {
        return Err("provider/model missing".to_string());
    }

    let resolved = resolve_profile_credential_with_priority(profile, source_base_dir);
    let resolved_kind =
        infer_resolved_credential_kind(profile, resolved.as_ref().map(|(_, _, source)| *source));
    if resolved_kind == ResolvedCredentialKind::OAuth {
        return Err("oauth session cannot be pushed automatically".to_string());
    }

    let credential = resolved.map(|(credential, _, _)| credential);
    if credential.is_none() && !provider_supports_optional_api_key(&profile.provider) {
        return Err("no usable static credential available".to_string());
    }

    Ok(PreparedProfilePush {
        profile: profile.clone(),
        provider_key: provider_key.clone(),
        model_ref: format!("{provider_key}/{model_name}"),
        target_auth_ref: target_auth_ref_for_profile(profile, &provider_key),
        credential,
    })
}

fn collect_selected_profile_pushes(
    paths: &crate::models::OpenClawPaths,
    profile_ids: &[String],
) -> Result<(Vec<PreparedProfilePush>, usize), String> {
    let profiles = load_model_profiles(paths);
    let mut seen = HashSet::<String>::new();
    let mut prepared = Vec::new();
    let mut blocked = 0usize;

    for profile_id in profile_ids {
        let trimmed = profile_id.trim();
        if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
            continue;
        }
        let Some(profile) = profiles.iter().find(|candidate| candidate.id == trimmed) else {
            blocked += 1;
            continue;
        };
        match prepare_profile_for_push(profile, &paths.base_dir) {
            Ok(item) => prepared.push(item),
            Err(_) => blocked += 1,
        }
    }

    Ok((prepared, blocked))
}

fn upsert_model_registration(cfg: &mut Value, push: &PreparedProfilePush) -> Result<bool, String> {
    if !cfg.is_object() {
        *cfg = serde_json::json!({});
    }
    let Some(root_obj) = cfg.as_object_mut() else {
        return Err("failed to prepare config root".to_string());
    };
    let models_val = root_obj
        .entry("models".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !models_val.is_object() {
        *models_val = Value::Object(serde_json::Map::new());
    }
    let Some(models_obj) = models_val.as_object_mut() else {
        return Err("failed to prepare models object".to_string());
    };

    let mut changed = false;
    let model_entry = models_obj
        .entry(push.model_ref.clone())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !model_entry.is_object() {
        *model_entry = Value::Object(serde_json::Map::new());
        changed = true;
    }
    let Some(model_obj) = model_entry.as_object_mut() else {
        return Err("failed to prepare model entry".to_string());
    };
    for (field, value) in [
        ("provider", push.provider_key.as_str()),
        ("model", push.profile.model.trim()),
    ] {
        let needs_update = model_obj
            .get(field)
            .and_then(Value::as_str)
            .map(|current| current != value)
            .unwrap_or(true);
        if needs_update {
            model_obj.insert(field.to_string(), Value::String(value.to_string()));
            changed = true;
        }
    }

    if let Some(base_url) = push
        .profile
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let providers_val = models_obj
            .entry("providers".to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !providers_val.is_object() {
            *providers_val = Value::Object(serde_json::Map::new());
            changed = true;
        }
        let Some(providers_obj) = providers_val.as_object_mut() else {
            return Err("failed to prepare provider config map".to_string());
        };
        let provider_val = providers_obj
            .entry(push.provider_key.clone())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !provider_val.is_object() {
            *provider_val = Value::Object(serde_json::Map::new());
            changed = true;
        }
        let Some(provider_obj) = provider_val.as_object_mut() else {
            return Err("failed to prepare provider config".to_string());
        };
        let needs_update = provider_obj
            .get("baseUrl")
            .and_then(Value::as_str)
            .map(|current| current != base_url)
            .unwrap_or(true);
        if needs_update {
            provider_obj.insert("baseUrl".to_string(), Value::String(base_url.to_string()));
            changed = true;
        }
    }

    Ok(changed)
}

fn parse_auth_store_json(raw: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|e| format!("Failed to parse auth store: {e}"))
}

#[tauri::command]
pub async fn push_related_secrets_to_remote(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<RelatedSecretPushResult, String> {
    let (_, _, cfg) = remote_read_openclaw_config_text_and_json(&pool, &host_id).await?;

    let (remote_profiles, _) = collect_remote_profiles_from_openclaw(&pool, &host_id, true).await?;
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
        let auth_ref = format!("{provider}:default");
        match upsert_auth_store_entry(&mut remote_auth_json, &auth_ref, provider, credential) {
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

#[tauri::command]
pub fn push_model_profiles_to_local_openclaw(
    profile_ids: Vec<String>,
) -> Result<ProfilePushResult, String> {
    let paths = resolve_paths();
    ensure_local_model_profiles_internal(&paths, &profile_ids)
}

pub(crate) fn ensure_local_model_profiles_internal(
    paths: &crate::models::OpenClawPaths,
    profile_ids: &[String],
) -> Result<ProfilePushResult, String> {
    let (prepared, blocked_profiles) = collect_selected_profile_pushes(paths, profile_ids)?;
    if prepared.is_empty() {
        return Ok(ProfilePushResult {
            requested_profiles: profile_ids.len(),
            pushed_profiles: 0,
            written_model_entries: 0,
            written_auth_entries: 0,
            blocked_profiles,
        });
    }

    let mut cfg = read_openclaw_config(&paths)?;
    let mut written_model_entries = 0usize;
    for push in &prepared {
        if upsert_model_registration(&mut cfg, push)? {
            written_model_entries += 1;
        }
    }
    if written_model_entries > 0 {
        write_json(&paths.config_path, &cfg)?;
    }

    let auth_file = paths
        .base_dir
        .join("agents")
        .join("main")
        .join("agent")
        .join("auth-profiles.json");
    let auth_raw = std::fs::read_to_string(&auth_file)
        .unwrap_or_else(|_| r#"{"version":1,"profiles":{}}"#.to_string());
    let mut auth_json = parse_auth_store_json(&auth_raw)?;
    let mut written_auth_entries = 0usize;
    for push in &prepared {
        let Some(credential) = push.credential.as_ref() else {
            continue;
        };
        match upsert_auth_store_entry(
            &mut auth_json,
            &push.target_auth_ref,
            &push.provider_key,
            credential,
        ) {
            UpsertAuthStoreResult::Written => written_auth_entries += 1,
            UpsertAuthStoreResult::Unchanged => {}
            UpsertAuthStoreResult::Failed => {
                return Err(format!(
                    "Failed to write auth entry for {}/{}",
                    push.provider_key, push.profile.model
                ));
            }
        }
    }
    if written_auth_entries > 0 {
        let serialized = serde_json::to_string_pretty(&auth_json)
            .map_err(|e| format!("Failed to serialize local auth store: {e}"))?;
        write_text(&auth_file, &serialized)?;
    }

    Ok(ProfilePushResult {
        requested_profiles: profile_ids.len(),
        pushed_profiles: prepared.len(),
        written_model_entries,
        written_auth_entries,
        blocked_profiles,
    })
}

#[tauri::command]
pub async fn push_model_profiles_to_remote_openclaw(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    profile_ids: Vec<String>,
) -> Result<ProfilePushResult, String> {
    ensure_remote_model_profiles_internal(pool.inner(), &host_id, &profile_ids).await
}

pub(crate) async fn ensure_remote_model_profiles_internal(
    pool: &SshConnectionPool,
    host_id: &str,
    profile_ids: &[String],
) -> Result<ProfilePushResult, String> {
    let paths = resolve_paths();
    let (prepared, blocked_profiles) = collect_selected_profile_pushes(&paths, profile_ids)?;
    if prepared.is_empty() {
        return Ok(ProfilePushResult {
            requested_profiles: profile_ids.len(),
            pushed_profiles: 0,
            written_model_entries: 0,
            written_auth_entries: 0,
            blocked_profiles,
        });
    }

    let (config_path, current_text, mut cfg) =
        remote_read_openclaw_config_text_and_json(pool, host_id).await?;
    let mut written_model_entries = 0usize;
    for push in &prepared {
        if upsert_model_registration(&mut cfg, push)? {
            written_model_entries += 1;
        }
    }
    if written_model_entries > 0 {
        remote_write_config_with_snapshot(
            pool,
            host_id,
            &config_path,
            &current_text,
            &cfg,
            "push-profiles",
        )
        .await?;
    }

    let roots = resolve_remote_openclaw_roots(pool, host_id).await?;
    let root = roots
        .first()
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Failed to resolve remote openclaw root".to_string())?;
    let root = root.trim_end_matches('/');
    let remote_auth_dir = format!("{root}/agents/main/agent");
    let remote_auth_path = format!("{remote_auth_dir}/auth-profiles.json");
    let remote_auth_raw = match pool.sftp_read(host_id, &remote_auth_path).await {
        Ok(content) => content,
        Err(e) if is_remote_missing_path_error(&e) => r#"{"version":1,"profiles":{}}"#.to_string(),
        Err(e) => return Err(format!("Failed to read remote auth store: {e}")),
    };
    let mut remote_auth_json = parse_auth_store_json(&remote_auth_raw)?;
    let mut written_auth_entries = 0usize;
    for push in &prepared {
        let Some(credential) = push.credential.as_ref() else {
            continue;
        };
        match upsert_auth_store_entry(
            &mut remote_auth_json,
            &push.target_auth_ref,
            &push.provider_key,
            credential,
        ) {
            UpsertAuthStoreResult::Written => written_auth_entries += 1,
            UpsertAuthStoreResult::Unchanged => {}
            UpsertAuthStoreResult::Failed => {
                return Err(format!(
                    "Failed to write remote auth entry for {}/{}",
                    push.provider_key, push.profile.model
                ));
            }
        }
    }
    if written_auth_entries > 0 {
        let serialized = serde_json::to_string_pretty(&remote_auth_json)
            .map_err(|e| format!("Failed to serialize remote auth store: {e}"))?;
        let mkdir_cmd = format!("mkdir -p {}", shell_escape(&remote_auth_dir));
        let _ = pool.exec(host_id, &mkdir_cmd).await;
        pool.sftp_write(host_id, &remote_auth_path, &serialized)
            .await?;
    }

    Ok(ProfilePushResult {
        requested_profiles: profile_ids.len(),
        pushed_profiles: prepared.len(),
        written_model_entries,
        written_auth_entries,
        blocked_profiles,
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
            normalize_oauth_provider_alias("openai-codex"),
            Some("openai-codex")
        );
        assert_eq!(
            normalize_oauth_provider_alias("github-copilot"),
            Some("openai-codex")
        );
        assert_eq!(
            normalize_oauth_provider_alias("copilot"),
            Some("openai-codex")
        );
        assert_eq!(
            normalize_oauth_provider_alias("openai"),
            Some("openai-codex")
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

    #[test]
    fn extract_profiles_from_openclaw_config_creates_profiles_without_storage_seed() {
        let cfg = serde_json::json!({
            "agents": {
                "defaults": {
                    "model": "anthropic/claude-4-5"
                }
            },
            "models": {
                "providers": {
                    "anthropic": {
                        "baseUrl": "https://api.anthropic.test/v1"
                    }
                }
            }
        });

        let (profiles, result) = extract_profiles_from_openclaw_config(&cfg, Vec::new());

        assert_eq!(result.created, 1);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].provider, "anthropic");
        assert_eq!(profiles[0].model, "claude-4-5");
        assert_eq!(profiles[0].auth_ref, "anthropic:default");
        assert_eq!(
            profiles[0].base_url.as_deref(),
            Some("https://api.anthropic.test/v1")
        );
    }

    #[test]
    fn prepare_profile_for_push_blocks_oauth_session_profiles() {
        let dir = make_temp_dir("push-oauth");
        let profile = profile(
            "oauth-1",
            "openai-codex",
            "gpt-5.3-codex",
            "openai-codex:default",
            None,
        );

        let error = prepare_profile_for_push(&profile, &dir).expect_err("should block oauth");
        assert!(error.contains("oauth"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn skip_session_material_sync_for_oauth_profiles_without_manual_key() {
        let oauth_ref = profile(
            "oauth-ref",
            "openai-codex",
            "gpt-5.3-codex",
            "openai-codex:default",
            None,
        );
        let oauth_fallback = profile("oauth-fallback", "openai-codex", "gpt-5.3-codex", "", None);
        let manual_key = profile(
            "oauth-manual",
            "openai-codex",
            "gpt-5.3-codex",
            "",
            Some("sk-static"),
        );

        assert!(should_skip_session_material_sync(&oauth_ref));
        assert!(should_skip_session_material_sync(&oauth_fallback));
        assert!(!should_skip_session_material_sync(&manual_key));
    }

    #[test]
    fn prepare_profile_for_push_allows_optional_key_provider_without_secret() {
        let dir = make_temp_dir("push-optional-provider");
        let profile = profile("ollama-1", "ollama", "qwen3:latest", "", None);

        let prepared = prepare_profile_for_push(&profile, &dir).expect("should allow ollama");
        assert!(prepared.credential.is_none());
        assert_eq!(prepared.target_auth_ref, "ollama:default");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn upsert_model_registration_writes_model_and_provider_base_url() {
        let mut cfg = serde_json::json!({});
        let prepared = PreparedProfilePush {
            profile: ModelProfile {
                id: "p-push".to_string(),
                name: "openrouter/deepseek-r1".to_string(),
                provider: "openrouter".to_string(),
                model: "deepseek-r1".to_string(),
                auth_ref: "openrouter:work".to_string(),
                api_key: None,
                base_url: Some("https://openrouter.example/v1".to_string()),
                description: None,
                enabled: true,
            },
            provider_key: "openrouter".to_string(),
            model_ref: "openrouter/deepseek-r1".to_string(),
            target_auth_ref: "openrouter:work".to_string(),
            credential: None,
        };

        let changed = upsert_model_registration(&mut cfg, &prepared).expect("upsert model");
        assert!(changed);
        assert_eq!(
            cfg.pointer("/models/openrouter~1deepseek-r1/provider")
                .and_then(Value::as_str),
            Some("openrouter")
        );
        assert_eq!(
            cfg.pointer("/models/openrouter~1deepseek-r1/model")
                .and_then(Value::as_str),
            Some("deepseek-r1")
        );
        assert_eq!(
            cfg.pointer("/models/providers/openrouter/baseUrl")
                .and_then(Value::as_str),
            Some("https://openrouter.example/v1")
        );
    }

    #[test]
    fn upsert_auth_store_entry_uses_explicit_auth_ref() {
        let mut root = serde_json::json!({ "version": 1 });
        let credential = InternalProviderCredential {
            secret: "sk-work".to_string(),
            kind: InternalAuthKind::ApiKey,
        };

        let result =
            upsert_auth_store_entry(&mut root, "openrouter:work", "openrouter", &credential);

        assert_eq!(result, UpsertAuthStoreResult::Written);
        assert_eq!(
            root.pointer("/profiles/openrouter:work/key")
                .and_then(Value::as_str),
            Some("sk-work")
        );
        assert_eq!(
            root.pointer("/lastGood/openrouter").and_then(Value::as_str),
            Some("openrouter:work")
        );
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
    let (next_profiles, result) = extract_profiles_from_openclaw_config(&cfg, profiles);

    if result.created > 0 {
        save_model_profiles(&paths, &next_profiles)?;
    }

    Ok(result)
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

    // 2. Check env vars
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

    // 3. Check existing model profiles for this provider
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
        let (resolved_key, source) = if let Some((credential, _priority, source)) =
            resolve_profile_credential_with_priority(profile, &global_base)
        {
            (credential.secret, Some(source))
        } else {
            (String::new(), None)
        };
        let resolved_override = if resolved_key.trim().is_empty() && oauth_session_ready(profile) {
            Some(true)
        } else {
            None
        };
        out.push(build_resolved_api_key(
            profile,
            &resolved_key,
            source,
            resolved_override,
        ));
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
    if api_key.trim().is_empty() {
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
