use std::fs;
use std::path::{Component, Path, PathBuf};

use dirs::home_dir;
use serde::Deserialize;
use serde_json::Value;

use crate::config_io::read_openclaw_config;
use crate::models::OpenClawPaths;
use crate::ssh::SshConnectionPool;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocumentTarget {
    scope: String,
    #[serde(default)]
    agent_id: Option<String>,
    path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpsertDocumentPayload {
    target: DocumentTarget,
    content: String,
    mode: String,
    #[serde(default)]
    heading: Option<String>,
    #[serde(default)]
    create_if_missing: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteDocumentPayload {
    target: DocumentTarget,
    #[serde(default)]
    missing_ok: Option<bool>,
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn validate_relative_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("document path is required".into());
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return Err("document path must be relative for this target scope".into());
    }
    for component in candidate.components() {
        match component {
            Component::Normal(_) => {}
            _ => return Err("document path cannot escape its target scope".into()),
        }
    }
    Ok(trimmed.to_string())
}

fn resolve_agent_entry<'a>(cfg: &'a Value, agent_id: &str) -> Result<&'a Value, String> {
    let agents_list = cfg
        .get("agents")
        .and_then(|agents| agents.get("list"))
        .and_then(Value::as_array)
        .ok_or_else(|| "agents.list not found".to_string())?;

    agents_list
        .iter()
        .find(|agent| agent.get("id").and_then(Value::as_str) == Some(agent_id))
        .ok_or_else(|| format!("Agent '{}' not found", agent_id))
}

fn resolve_workspace(
    cfg: &Value,
    agent_id: &str,
    default_workspace: Option<&str>,
) -> Result<String, String> {
    clawpal_core::doctor::resolve_agent_workspace_from_config(cfg, agent_id, default_workspace)
}

fn push_unique_candidate(candidates: &mut Vec<String>, candidate: Option<String>) {
    let Some(candidate) = candidate.map(|value| value.trim().to_string()) else {
        return;
    };
    if candidate.is_empty() || candidates.iter().any(|existing| existing == &candidate) {
        return;
    }
    candidates.push(candidate);
}

fn resolve_agent_dir_candidates(
    cfg: &Value,
    agent_id: &str,
    fallback_agent_root: Option<&str>,
) -> Result<Vec<String>, String> {
    let agent = resolve_agent_entry(cfg, agent_id)?;
    let mut candidates = Vec::new();

    push_unique_candidate(
        &mut candidates,
        agent
            .get("workspace")
            .and_then(Value::as_str)
            .map(str::to_string),
    );
    push_unique_candidate(
        &mut candidates,
        agent
            .get("agentDir")
            .and_then(Value::as_str)
            .map(str::to_string),
    );
    push_unique_candidate(&mut candidates, resolve_workspace(cfg, agent_id, None).ok());
    push_unique_candidate(
        &mut candidates,
        fallback_agent_root
            .map(|root| format!("{}/{}/agent", root.trim_end_matches('/'), agent_id)),
    );

    if candidates.is_empty() {
        return Err(format!(
            "Agent '{}' has no workspace or document directory configured",
            agent_id
        ));
    }

    Ok(candidates)
}

fn normalize_remote_dir(path: &str) -> String {
    if path.starts_with("~/") || path.starts_with('/') {
        path.to_string()
    } else {
        format!("~/{path}")
    }
}

fn resolve_local_target_path(
    paths: &OpenClawPaths,
    target: &DocumentTarget,
) -> Result<PathBuf, String> {
    let scope = target.scope.trim();
    match scope {
        "agent" => {
            let agent_id = normalize_optional_text(target.agent_id.as_deref())
                .ok_or_else(|| "agent document target requires agentId".to_string())?;
            let relative = validate_relative_path(&target.path)?;
            let cfg = read_openclaw_config(paths)?;
            let fallback_root = paths
                .openclaw_dir
                .join("agents")
                .to_string_lossy()
                .to_string();
            let candidate_dirs =
                resolve_agent_dir_candidates(&cfg, &agent_id, Some(&fallback_root))?;
            let candidate_paths: Vec<PathBuf> = candidate_dirs
                .into_iter()
                .map(|path| PathBuf::from(shellexpand::tilde(&path).to_string()))
                .collect();
            if let Some(existing) = candidate_paths
                .iter()
                .map(|dir| dir.join(&relative))
                .find(|path| path.exists())
            {
                return Ok(existing);
            }
            candidate_paths
                .first()
                .map(|dir| dir.join(relative))
                .ok_or_else(|| format!("Agent '{}' has no document path candidates", agent_id))
        }
        "home" => {
            let relative = target.path.trim().trim_start_matches("~/");
            let relative = validate_relative_path(relative)?;
            let home = home_dir().ok_or_else(|| "failed to resolve home directory".to_string())?;
            Ok(home.join(relative))
        }
        "absolute" => {
            let absolute = PathBuf::from(target.path.trim());
            if !absolute.is_absolute() {
                return Err("absolute document targets must use an absolute path".into());
            }
            Ok(absolute)
        }
        other => Err(format!("unsupported document target scope: {}", other)),
    }
}

async fn resolve_remote_target_path(
    pool: &SshConnectionPool,
    host_id: &str,
    target: &DocumentTarget,
) -> Result<String, String> {
    let scope = target.scope.trim();
    match scope {
        "agent" => {
            let agent_id = normalize_optional_text(target.agent_id.as_deref())
                .ok_or_else(|| "agent document target requires agentId".to_string())?;
            let relative = validate_relative_path(&target.path)?;
            let (_config_path, _raw, cfg) =
                crate::commands::remote_read_openclaw_config_text_and_json(pool, host_id).await?;
            let candidate_dirs =
                resolve_agent_dir_candidates(&cfg, &agent_id, Some("~/.openclaw/agents"))?;
            let candidate_dirs: Vec<String> = candidate_dirs
                .into_iter()
                .map(|dir| normalize_remote_dir(&dir))
                .collect();
            for dir in &candidate_dirs {
                let candidate = format!("{dir}/{relative}");
                match pool.sftp_read(host_id, &candidate).await {
                    Ok(_) => return Ok(candidate),
                    Err(error) if error.contains("No such file") || error.contains("not found") => {
                    }
                    Err(error) => return Err(error),
                }
            }
            candidate_dirs
                .first()
                .map(|dir| format!("{dir}/{relative}"))
                .ok_or_else(|| format!("Agent '{}' has no document path candidates", agent_id))
        }
        "home" => {
            let relative = target.path.trim().trim_start_matches("~/");
            let relative = validate_relative_path(relative)?;
            Ok(format!("~/{relative}"))
        }
        "absolute" => {
            let absolute = target.path.trim();
            if !absolute.starts_with('/') {
                return Err("absolute document targets must use an absolute path".into());
            }
            Ok(absolute.to_string())
        }
        other => Err(format!("unsupported document target scope: {}", other)),
    }
}

fn format_heading(heading: &str) -> String {
    let trimmed = heading.trim();
    if trimmed.starts_with('#') {
        trimmed.to_string()
    } else {
        format!("## {}", trimmed)
    }
}

pub(crate) fn upsert_markdown_section(existing: &str, heading: &str, content: &str) -> String {
    let normalized = existing.replace("\r\n", "\n");
    let header = format_heading(heading);
    let lines: Vec<&str> = normalized.lines().collect();
    let mut start = None;
    let mut end = lines.len();

    for (index, line) in lines.iter().enumerate() {
        if line.trim() == header {
            start = Some(index);
            for (scan_index, candidate) in lines.iter().enumerate().skip(index + 1) {
                if candidate.starts_with("## ") || candidate.starts_with("# ") {
                    end = scan_index;
                    break;
                }
            }
            break;
        }
    }

    let replacement = if content.trim().is_empty() {
        String::new()
    } else {
        format!("{header}\n{}\n", content.trim_end())
    };

    if let Some(start) = start {
        let before = if start == 0 {
            String::new()
        } else {
            lines[..start].join("\n").trim_end().to_string()
        };
        let after = if end >= lines.len() {
            String::new()
        } else {
            lines[end..].join("\n").trim_start().to_string()
        };
        let mut parts = Vec::new();
        if !before.is_empty() {
            parts.push(before);
        }
        if !replacement.trim().is_empty() {
            parts.push(replacement.trim_end().to_string());
        }
        if !after.is_empty() {
            parts.push(after);
        }
        return parts.join("\n\n") + "\n";
    }

    if normalized.trim().is_empty() {
        return replacement;
    }

    format!("{}\n\n{}", normalized.trim_end(), replacement)
}

fn upsert_content(
    existing: Option<&str>,
    payload: &UpsertDocumentPayload,
) -> Result<String, String> {
    let mode = payload.mode.trim();
    match mode {
        "replace" => Ok(payload.content.clone()),
        "upsertSection" => {
            let heading = payload
                .heading
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    "upsert_markdown_document requires heading in upsertSection mode".to_string()
                })?;
            let allow_create = payload.create_if_missing.unwrap_or(true);
            let existing = existing.unwrap_or_default();
            if existing.trim().is_empty() && !allow_create {
                return Err("document does not exist and createIfMissing is false".into());
            }
            Ok(upsert_markdown_section(existing, heading, &payload.content))
        }
        other => Err(format!("unsupported markdown document mode: {}", other)),
    }
}

pub(crate) fn write_local_markdown_document(
    paths: &OpenClawPaths,
    payload: &Value,
) -> Result<(), String> {
    let payload: UpsertDocumentPayload =
        serde_json::from_value(payload.clone()).map_err(|error| error.to_string())?;
    let target_path = resolve_local_target_path(paths, &payload.target)?;
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let existing = fs::read_to_string(&target_path).ok();
    let next = upsert_content(existing.as_deref(), &payload)?;
    fs::write(&target_path, next).map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) async fn write_remote_markdown_document(
    pool: &SshConnectionPool,
    host_id: &str,
    payload: &Value,
) -> Result<(), String> {
    let payload: UpsertDocumentPayload =
        serde_json::from_value(payload.clone()).map_err(|error| error.to_string())?;
    let target_path = resolve_remote_target_path(pool, host_id, &payload.target).await?;
    let existing = match pool.sftp_read(host_id, &target_path).await {
        Ok(content) => Some(content),
        Err(error) if error.contains("No such file") || error.contains("not found") => None,
        Err(error) => return Err(error),
    };
    let next = upsert_content(existing.as_deref(), &payload)?;
    if let Some(parent) = target_path.rsplit_once('/') {
        let _ = pool
            .exec(
                host_id,
                &format!("mkdir -p '{}'", parent.0.replace('\'', "'\\''")),
            )
            .await;
    }
    pool.sftp_write(host_id, &target_path, &next).await?;
    Ok(())
}

pub(crate) fn delete_local_markdown_document(
    paths: &OpenClawPaths,
    payload: &Value,
) -> Result<(), String> {
    let payload: DeleteDocumentPayload =
        serde_json::from_value(payload.clone()).map_err(|error| error.to_string())?;
    let target_path = resolve_local_target_path(paths, &payload.target)?;
    match fs::remove_file(&target_path) {
        Ok(_) => Ok(()),
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && payload.missing_ok.unwrap_or(true) =>
        {
            Ok(())
        }
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) async fn delete_remote_markdown_document(
    pool: &SshConnectionPool,
    host_id: &str,
    payload: &Value,
) -> Result<(), String> {
    let payload: DeleteDocumentPayload =
        serde_json::from_value(payload.clone()).map_err(|error| error.to_string())?;
    let target_path = resolve_remote_target_path(pool, host_id, &payload.target).await?;
    match pool.sftp_remove(host_id, &target_path).await {
        Ok(_) => Ok(()),
        Err(error)
            if (error.contains("No such file") || error.contains("not found"))
                && payload.missing_ok.unwrap_or(true) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{upsert_markdown_section, validate_relative_path};

    #[test]
    fn relative_path_validation_rejects_parent_segments() {
        assert!(validate_relative_path("../secrets.md").is_err());
        assert!(validate_relative_path("notes/../../secrets.md").is_err());
    }

    #[test]
    fn upsert_section_replaces_existing_heading_block() {
        let next = upsert_markdown_section(
            "# Notes\n\n## Persona\nOld\n\n## Other\nStay\n",
            "Persona",
            "New",
        );

        assert_eq!(next, "# Notes\n\n## Persona\nNew\n\n## Other\nStay\n");
    }
}
