use super::*;

const REMOTE_SNAPSHOT_METADATA_PATH: &str = "~/.clawpal/metadata.json";

fn history_page_from_snapshot_index(index: crate::history::SnapshotIndex) -> HistoryPage {
    HistoryPage {
        items: index
            .items
            .into_iter()
            .map(|item| HistoryItem {
                id: item.id,
                recipe_id: item.recipe_id,
                created_at: item.created_at,
                source: item.source,
                can_rollback: item.can_rollback,
                run_id: item.run_id,
                rollback_of: item.rollback_of,
                artifacts: item.artifacts,
            })
            .collect(),
    }
}

fn fallback_snapshot_meta_from_remote_entry(
    entry: &crate::ssh::SftpEntry,
) -> Option<crate::history::SnapshotMeta> {
    if entry.name.starts_with('.') || entry.is_dir {
        return None;
    }
    let stem = entry.name.trim_end_matches(".json");
    let parts: Vec<&str> = stem.splitn(3, '-').collect();
    let ts_str = parts.first().copied().unwrap_or("0");
    let source = parts.get(1).copied().unwrap_or("unknown");
    let recipe_id = parts.get(2).map(|s| s.to_string());
    let created_at = ts_str.parse::<i64>().unwrap_or(0);
    let created_at_iso = chrono::DateTime::from_timestamp(created_at, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| created_at.to_string());
    Some(crate::history::SnapshotMeta {
        id: entry.name.clone(),
        recipe_id,
        created_at: created_at_iso,
        config_path: format!("~/.clawpal/snapshots/{}", entry.name),
        source: source.to_string(),
        can_rollback: source != "rollback",
        run_id: None,
        rollback_of: None,
        artifacts: Vec::new(),
    })
}

pub(crate) async fn read_remote_snapshot_index(
    pool: &SshConnectionPool,
    host_id: &str,
) -> Result<crate::history::SnapshotIndex, String> {
    match pool.sftp_read(host_id, REMOTE_SNAPSHOT_METADATA_PATH).await {
        Ok(text) => crate::history::parse_snapshot_index_text(&text),
        Err(error) if super::is_remote_missing_path_error(&error) => {
            Ok(crate::history::SnapshotIndex::default())
        }
        Err(error) => Err(format!(
            "Failed to read remote snapshot metadata: {}",
            error
        )),
    }
}

pub(crate) async fn write_remote_snapshot_index(
    pool: &SshConnectionPool,
    host_id: &str,
    index: &crate::history::SnapshotIndex,
) -> Result<(), String> {
    pool.exec(host_id, "mkdir -p ~/.clawpal").await?;
    let text = crate::history::render_snapshot_index_text(index)?;
    pool.sftp_write(host_id, REMOTE_SNAPSHOT_METADATA_PATH, &text)
        .await
}

pub(crate) async fn record_remote_snapshot_metadata(
    pool: &SshConnectionPool,
    host_id: &str,
    snapshot: crate::history::SnapshotMeta,
) -> Result<(), String> {
    let mut index = read_remote_snapshot_index(pool, host_id).await?;
    crate::history::upsert_snapshot(&mut index, snapshot);
    write_remote_snapshot_index(pool, host_id, &index).await
}

async fn resolve_remote_snapshot_meta(
    pool: &SshConnectionPool,
    host_id: &str,
    snapshot_id: &str,
) -> Result<Option<crate::history::SnapshotMeta>, String> {
    let index = read_remote_snapshot_index(pool, host_id).await?;
    Ok(crate::history::find_snapshot(&index, snapshot_id).cloned())
}

#[tauri::command]
pub async fn remote_read_raw_config(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<String, String> {
    // openclaw config get requires a path — there's no way to dump the full config via CLI.
    // Use sftp_read directly since this function's purpose is returning the entire raw config.
    let config_path = remote_resolve_openclaw_config_path(&pool, &host_id).await?;
    pool.sftp_read(&host_id, &config_path).await
}

#[tauri::command]
pub async fn remote_write_raw_config(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    content: String,
) -> Result<bool, String> {
    // Validate it's valid config JSON using core module
    let next = clawpal_core::config::validate_config_json(&content)
        .map_err(|e| format!("Invalid JSON: {e}"))?;
    // Read current for snapshot
    let config_path = remote_resolve_openclaw_config_path(&pool, &host_id).await?;
    let current = pool
        .sftp_read(&host_id, &config_path)
        .await
        .unwrap_or_default();
    remote_write_config_with_snapshot(&pool, &host_id, &config_path, &current, &next, "raw-edit")
        .await?;
    Ok(true)
}

#[tauri::command]
pub async fn remote_apply_config_patch(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    patch_template: String,
    params: Map<String, Value>,
) -> Result<ApplyResult, String> {
    let (config_path, current_text, current) =
        remote_read_openclaw_config_text_and_json(&pool, &host_id).await?;

    // Use core function to build candidate config
    let (candidate, _changes) =
        clawpal_core::config::build_candidate_config(&current, &patch_template, &params)?;

    remote_write_config_with_snapshot(
        &pool,
        &host_id,
        &config_path,
        &current_text,
        &candidate,
        "config-patch",
    )
    .await?;
    Ok(ApplyResult {
        ok: true,
        snapshot_id: None,
        config_path,
        backup_path: None,
        warnings: Vec::new(),
        errors: Vec::new(),
    })
}

#[tauri::command]
pub async fn remote_list_history(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<HistoryPage, String> {
    // Ensure dir exists
    pool.exec(&host_id, "mkdir -p ~/.clawpal/snapshots").await?;
    let entries = pool.sftp_list(&host_id, "~/.clawpal/snapshots").await?;
    let mut index = read_remote_snapshot_index(&pool, &host_id).await?;
    let known_ids = index
        .items
        .iter()
        .map(|item| item.id.clone())
        .collect::<std::collections::HashSet<_>>();
    for entry in entries {
        if known_ids.contains(&entry.name) {
            continue;
        }
        if let Some(snapshot) = fallback_snapshot_meta_from_remote_entry(&entry) {
            crate::history::upsert_snapshot(&mut index, snapshot);
        }
    }
    Ok(history_page_from_snapshot_index(index))
}

#[tauri::command]
pub async fn remote_preview_rollback(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    snapshot_id: String,
) -> Result<PreviewResult, String> {
    let snapshot_path = resolve_remote_snapshot_meta(&pool, &host_id, &snapshot_id)
        .await?
        .map(|snapshot| snapshot.config_path)
        .unwrap_or_else(|| format!("~/.clawpal/snapshots/{snapshot_id}"));
    let snapshot_text = pool.sftp_read(&host_id, &snapshot_path).await?;
    let target = clawpal_core::config::validate_config_json(&snapshot_text)
        .map_err(|e| format!("Failed to parse snapshot: {e}"))?;

    let (_config_path, _current_text, current) =
        remote_read_openclaw_config_text_and_json(&pool, &host_id).await?;

    let before = clawpal_core::config::format_config_diff(&current, &current);
    let after = clawpal_core::config::format_config_diff(&target, &target);
    let diff = clawpal_core::config::format_config_diff(&current, &target);

    Ok(PreviewResult {
        recipe_id: "rollback".into(),
        diff,
        config_before: before,
        config_after: after,
        changes: Vec::new(), // Core module doesn't expose change paths directly
        overwrites_existing: true,
        can_rollback: true,
        impact_level: "medium".into(),
        warnings: vec!["Rollback will replace current configuration".into()],
    })
}

#[tauri::command]
pub async fn remote_rollback(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    snapshot_id: String,
) -> Result<ApplyResult, String> {
    let snapshot_meta = resolve_remote_snapshot_meta(&pool, &host_id, &snapshot_id).await?;
    let snapshot_path = snapshot_meta
        .as_ref()
        .map(|snapshot| snapshot.config_path.clone())
        .unwrap_or_else(|| format!("~/.clawpal/snapshots/{snapshot_id}"));
    let target_text = pool.sftp_read(&host_id, &snapshot_path).await?;
    let target = clawpal_core::config::validate_config_json(&target_text)
        .map_err(|e| format!("Failed to parse snapshot: {e}"))?;

    let (config_path, current_text, _current) =
        remote_read_openclaw_config_text_and_json(&pool, &host_id).await?;
    let mut warnings = Vec::new();
    if let Some(snapshot) = snapshot_meta.as_ref() {
        warnings.extend(super::cleanup_remote_recipe_snapshot(&pool, &host_id, snapshot).await);
    }
    remote_write_config_with_snapshot(
        &pool,
        &host_id,
        &config_path,
        &current_text,
        &target,
        "rollback",
    )
    .await?;

    Ok(ApplyResult {
        ok: true,
        snapshot_id: Some(snapshot_id),
        config_path,
        backup_path: None,
        warnings,
        errors: Vec::new(),
    })
}

#[tauri::command]
pub fn read_raw_config() -> Result<String, String> {
    let paths = resolve_paths();
    let cfg = read_openclaw_config(&paths)?;
    serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn apply_config_patch(
    patch_template: String,
    params: Map<String, Value>,
) -> Result<ApplyResult, String> {
    let paths = resolve_paths();
    ensure_dirs(&paths)?;
    let current = read_openclaw_config(&paths)?;
    let current_text = serde_json::to_string_pretty(&current).map_err(|e| e.to_string())?;
    let snapshot = add_snapshot(
        &paths.history_dir,
        &paths.metadata_path,
        Some("config-patch".into()),
        "apply",
        true,
        &current_text,
        None,
        None,
        Vec::new(),
    )?;
    let (candidate, _changes) =
        build_candidate_config_from_template(&current, &patch_template, &params)?;
    write_json(&paths.config_path, &candidate)?;
    let mut warnings = Vec::new();
    if let Err(err) = sync_main_auth_for_config(&paths, &candidate) {
        warnings.push(format!("main auth sync skipped: {err}"));
    }
    Ok(ApplyResult {
        ok: true,
        snapshot_id: Some(snapshot.id),
        config_path: paths.config_path.to_string_lossy().to_string(),
        backup_path: Some(snapshot.config_path),
        warnings,
        errors: Vec::new(),
    })
}

#[tauri::command]
pub fn list_history(limit: usize, offset: usize) -> Result<HistoryPage, String> {
    let paths = resolve_paths();
    let index = list_snapshots(&paths.metadata_path)?;
    let items = history_page_from_snapshot_index(index)
        .items
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect();
    Ok(HistoryPage { items })
}

#[tauri::command]
pub fn preview_rollback(snapshot_id: String) -> Result<PreviewResult, String> {
    let paths = resolve_paths();
    let index = list_snapshots(&paths.metadata_path)?;
    let target = index
        .items
        .into_iter()
        .find(|s| s.id == snapshot_id)
        .ok_or_else(|| "snapshot not found".to_string())?;
    if !target.can_rollback {
        return Err("snapshot is not rollbackable".to_string());
    }

    let current = read_openclaw_config(&paths)?;
    let target_text = read_snapshot(&target.config_path)?;
    let target_json = clawpal_core::doctor::parse_json5_document_or_default(&target_text);
    let before_text = serde_json::to_string_pretty(&current).unwrap_or_else(|_| "{}".into());
    let after_text = serde_json::to_string_pretty(&target_json).unwrap_or_else(|_| "{}".into());
    Ok(PreviewResult {
        recipe_id: "rollback".into(),
        diff: format_diff(&current, &target_json),
        config_before: before_text,
        config_after: after_text,
        changes: collect_change_paths(&current, &target_json),
        overwrites_existing: true,
        can_rollback: true,
        impact_level: "medium".into(),
        warnings: vec!["Rollback will replace current configuration".into()],
    })
}

#[tauri::command]
pub fn rollback(snapshot_id: String) -> Result<ApplyResult, String> {
    let paths = resolve_paths();
    ensure_dirs(&paths)?;
    let index = list_snapshots(&paths.metadata_path)?;
    let target = index
        .items
        .into_iter()
        .find(|s| s.id == snapshot_id)
        .ok_or_else(|| "snapshot not found".to_string())?;
    if !target.can_rollback {
        return Err("snapshot is not rollbackable".to_string());
    }
    let target_text = read_snapshot(&target.config_path)?;
    let backup = read_openclaw_config(&paths)?;
    let backup_text = serde_json::to_string_pretty(&backup).map_err(|e| e.to_string())?;
    let warnings = super::cleanup_local_recipe_snapshot(&target);
    let _ = add_snapshot(
        &paths.history_dir,
        &paths.metadata_path,
        target.recipe_id.clone(),
        "rollback",
        true,
        &backup_text,
        None,
        Some(target.id.clone()),
        Vec::new(),
    )?;
    write_text(&paths.config_path, &target_text)?;
    Ok(ApplyResult {
        ok: true,
        snapshot_id: Some(target.id),
        config_path: paths.config_path.to_string_lossy().to_string(),
        backup_path: None,
        warnings,
        errors: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::history_page_from_snapshot_index;
    use crate::history::{SnapshotIndex, SnapshotMeta};
    use crate::recipe_store::Artifact;

    #[test]
    fn history_page_from_snapshot_index_preserves_run_id_and_artifacts() {
        let page = history_page_from_snapshot_index(SnapshotIndex {
            items: vec![SnapshotMeta {
                id: "1710240000-clawpal-discord-channel-persona.json".into(),
                recipe_id: Some("discord-channel-persona".into()),
                created_at: "2026-03-12T00:00:00Z".into(),
                config_path: "~/.clawpal/snapshots/1710240000-clawpal-discord-channel-persona.json"
                    .into(),
                source: "clawpal".into(),
                can_rollback: true,
                run_id: Some("run_remote_01".into()),
                rollback_of: None,
                artifacts: vec![Artifact {
                    id: "artifact_01".into(),
                    kind: "systemdUnit".into(),
                    label: "clawpal-job-hourly.service".into(),
                    path: None,
                }],
            }],
        });

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].run_id.as_deref(), Some("run_remote_01"));
        assert_eq!(
            page.items[0].recipe_id.as_deref(),
            Some("discord-channel-persona")
        );
        assert_eq!(page.items[0].artifacts.len(), 1);
        assert_eq!(
            page.items[0].artifacts[0].label,
            "clawpal-job-hourly.service"
        );
    }
}
