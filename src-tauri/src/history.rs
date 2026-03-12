use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::models::resolve_paths;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SnapshotMeta {
    pub id: String,
    pub recipe_id: Option<String>,
    pub created_at: String,
    pub config_path: String,
    pub source: String,
    pub can_rollback: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub rollback_of: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<crate::recipe_store::Artifact>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SnapshotIndex {
    pub items: Vec<SnapshotMeta>,
}

pub fn parse_snapshot_index_text(text: &str) -> Result<SnapshotIndex, String> {
    if text.trim().is_empty() {
        return Ok(SnapshotIndex::default());
    }
    serde_json::from_str(text).map_err(|e| e.to_string())
}

pub fn render_snapshot_index_text(index: &SnapshotIndex) -> Result<String, String> {
    serde_json::to_string_pretty(index).map_err(|e| e.to_string())
}

pub fn upsert_snapshot(index: &mut SnapshotIndex, snapshot: SnapshotMeta) {
    index.items.retain(|existing| existing.id != snapshot.id);
    index.items.push(snapshot);
    index.items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    if index.items.len() > 200 {
        index.items.truncate(200);
    }
}

pub fn find_snapshot<'a>(index: &'a SnapshotIndex, snapshot_id: &str) -> Option<&'a SnapshotMeta> {
    index.items.iter().find(|item| item.id == snapshot_id)
}

pub fn list_snapshots(path: &std::path::Path) -> Result<SnapshotIndex, String> {
    if !path.exists() {
        return Ok(SnapshotIndex { items: Vec::new() });
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut text = String::new();
    file.read_to_string(&mut text).map_err(|e| e.to_string())?;
    parse_snapshot_index_text(&text)
}

pub fn write_snapshots(path: &std::path::Path, index: &SnapshotIndex) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "invalid metadata path".to_string())?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let text = render_snapshot_index_text(index)?;
    // Atomic write: write to .tmp file, sync, then rename
    let tmp = path.with_extension("tmp");
    {
        let mut file = File::create(&tmp).map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn add_snapshot(
    paths: &PathBuf,
    metadata_path: &PathBuf,
    recipe_id: Option<String>,
    source: &str,
    rollbackable: bool,
    current_config: &str,
    run_id: Option<String>,
    rollback_of: Option<String>,
    artifacts: Vec<crate::recipe_store::Artifact>,
) -> Result<SnapshotMeta, String> {
    fs::create_dir_all(paths).map_err(|e| e.to_string())?;

    let index = list_snapshots(metadata_path).unwrap_or_default();
    let ts = Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let snapshot_recipe_id = recipe_id.clone().unwrap_or_else(|| "manual".into());
    let id = format!("{}-{}", ts, snapshot_recipe_id);
    // Sanitize for safe filename: replace path separators and other problematic chars
    let safe_id: String = id
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            _ => c,
        })
        .collect();
    let snapshot_path = paths.join(format!("{}.json", safe_id));
    fs::write(&snapshot_path, current_config).map_err(|e| e.to_string())?;

    let mut next = index;
    upsert_snapshot(
        &mut next,
        SnapshotMeta {
            id: id.clone(),
            recipe_id,
            created_at: ts.clone(),
            config_path: snapshot_path.to_string_lossy().to_string(),
            source: source.to_string(),
            can_rollback: rollbackable,
            run_id: run_id.clone(),
            rollback_of: rollback_of.clone(),
            artifacts: artifacts.clone(),
        },
    );
    write_snapshots(metadata_path, &next)?;

    let returned = Some(snapshot_recipe_id.clone());

    Ok(SnapshotMeta {
        id,
        recipe_id: returned,
        created_at: ts,
        config_path: snapshot_path.to_string_lossy().to_string(),
        source: source.to_string(),
        can_rollback: rollbackable,
        run_id,
        rollback_of,
        artifacts,
    })
}

pub fn read_snapshot(path: &str) -> Result<String, String> {
    let canonical = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let allowed_base = resolve_paths().history_dir;
    let allowed_base = std::fs::canonicalize(&allowed_base).unwrap_or(allowed_base);
    if !canonical.starts_with(&allowed_base) {
        return Err("Path outside allowed directory".into());
    }
    std::fs::read_to_string(&canonical).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{add_snapshot, list_snapshots, read_snapshot};
    use crate::cli_runner::set_active_clawpal_data_override;
    use crate::recipe_store::Artifact;
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn read_snapshot_allows_files_under_active_history_dir() {
        let temp_root = std::env::temp_dir().join(format!("clawpal-history-{}", Uuid::new_v4()));
        let history_dir = temp_root.join("history");
        fs::create_dir_all(&history_dir).expect("create history dir");
        let snapshot_path = history_dir.join("ok.json");
        fs::write(&snapshot_path, "{\"ok\":true}").expect("write snapshot");

        set_active_clawpal_data_override(Some(temp_root.to_string_lossy().to_string()))
            .expect("set active clawpal data dir");
        let result = read_snapshot(&snapshot_path.to_string_lossy());
        set_active_clawpal_data_override(None).expect("clear active clawpal data dir");

        assert_eq!(result.expect("read snapshot"), "{\"ok\":true}");
        let _ = fs::remove_dir_all(temp_root);
    }

    #[test]
    fn add_snapshot_persists_run_id_and_artifacts_in_metadata() {
        let temp_root = std::env::temp_dir().join(format!("clawpal-history-{}", Uuid::new_v4()));
        let history_dir = temp_root.join("history");
        let metadata_path = temp_root.join("metadata.json");

        let snapshot = add_snapshot(
            &history_dir,
            &metadata_path,
            Some("discord-channel-persona".into()),
            "clawpal",
            true,
            "{\"ok\":true}",
            Some("run_01".into()),
            None,
            vec![Artifact {
                id: "artifact_01".into(),
                kind: "systemdUnit".into(),
                label: "clawpal-job-hourly.service".into(),
                path: None,
            }],
        )
        .expect("write snapshot metadata");
        let index = list_snapshots(&metadata_path).expect("read snapshot metadata");

        assert_eq!(snapshot.run_id.as_deref(), Some("run_01"));
        assert_eq!(
            index.items.first().and_then(|item| item.run_id.as_deref()),
            Some("run_01")
        );
        assert_eq!(snapshot.artifacts.len(), 1);
        assert_eq!(snapshot.artifacts[0].label, "clawpal-job-hourly.service");
        assert_eq!(
            index.items.first().map(|item| item.artifacts.len()),
            Some(1)
        );

        let _ = fs::remove_dir_all(temp_root);
    }
}
