use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use crate::config_io::read_openclaw_config;
use crate::models::OpenClawPaths;
use crate::ssh::SshConnectionPool;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct IdentityDocument {
    name: Option<String>,
    emoji: Option<String>,
    persona: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersonaChange<'a> {
    Preserve,
    Set(&'a str),
    Clear,
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_identity_content(text: &str) -> IdentityDocument {
    let mut result = IdentityDocument::default();
    let normalized = text.replace("\r\n", "\n");
    let mut sections = normalized.splitn(2, "\n## Persona\n");
    let header = sections.next().unwrap_or_default();
    let persona = sections.next().map(|value| value.trim_end_matches('\n'));

    for line in header.lines() {
        if let Some(name) = line.strip_prefix("- Name:") {
            result.name = normalize_optional_text(Some(name));
        } else if let Some(emoji) = line.strip_prefix("- Emoji:") {
            result.emoji = normalize_optional_text(Some(emoji));
        }
    }

    result.persona = normalize_optional_text(persona);
    result
}

fn merge_identity_document(
    existing: Option<&str>,
    default_name: Option<&str>,
    default_emoji: Option<&str>,
    name: Option<&str>,
    emoji: Option<&str>,
    persona: PersonaChange<'_>,
) -> Result<IdentityDocument, String> {
    let existing = existing.map(parse_identity_content).unwrap_or_default();
    let name = normalize_optional_text(name)
        .or(existing.name.clone())
        .or(normalize_optional_text(default_name));
    let emoji = normalize_optional_text(emoji)
        .or(existing.emoji.clone())
        .or(normalize_optional_text(default_emoji));
    let persona = match persona {
        PersonaChange::Preserve => existing.persona.clone(),
        PersonaChange::Set(persona) => {
            normalize_optional_text(Some(persona)).or(existing.persona.clone())
        }
        PersonaChange::Clear => None,
    };

    let Some(name) = name else {
        return Err(
            "agent identity requires a name when no existing IDENTITY.md is present".into(),
        );
    };

    Ok(IdentityDocument {
        name: Some(name),
        emoji,
        persona,
    })
}

fn identity_content(
    existing: Option<&str>,
    default_name: Option<&str>,
    default_emoji: Option<&str>,
    name: Option<&str>,
    emoji: Option<&str>,
    persona: PersonaChange<'_>,
) -> Result<String, String> {
    let merged =
        merge_identity_document(existing, default_name, default_emoji, name, emoji, persona)?;
    let mut content = format!(
        "- Name: {}\n",
        merged.name.as_deref().unwrap_or_default().trim()
    );
    if let Some(emoji) = merged.emoji.as_deref() {
        content.push_str(&format!("- Emoji: {}\n", emoji));
    }
    if let Some(persona) = merged.persona.as_deref() {
        content.push_str("\n## Persona\n");
        content.push_str(persona);
        content.push('\n');
    }
    Ok(content)
}

fn upsert_persona_content(
    existing: Option<&str>,
    explicit_name: Option<&str>,
    explicit_emoji: Option<&str>,
    default_name: Option<&str>,
    default_emoji: Option<&str>,
    persona: PersonaChange<'_>,
) -> Result<String, String> {
    match existing {
        Some(existing_text) => {
            let parsed = parse_identity_content(existing_text);
            let has_structured_identity = parsed.name.is_some() || parsed.emoji.is_some();
            if !has_structured_identity
                && (normalize_optional_text(explicit_name).is_some()
                    || normalize_optional_text(explicit_emoji).is_some())
            {
                return identity_content(
                    None,
                    default_name,
                    default_emoji,
                    explicit_name,
                    explicit_emoji,
                    persona,
                );
            }
            Ok(match persona {
                PersonaChange::Preserve => existing_text.to_string(),
                PersonaChange::Set(persona_text) => {
                    crate::markdown_document::upsert_markdown_section(
                        existing_text,
                        "Persona",
                        persona_text,
                    )
                }
                PersonaChange::Clear => {
                    crate::markdown_document::upsert_markdown_section(existing_text, "Persona", "")
                }
            })
        }
        None => identity_content(
            existing,
            default_name,
            default_emoji,
            explicit_name,
            explicit_emoji,
            persona,
        ),
    }
}

fn resolve_workspace(
    cfg: &Value,
    agent_id: &str,
    default_workspace: Option<&str>,
) -> Result<String, String> {
    clawpal_core::doctor::resolve_agent_workspace_from_config(cfg, agent_id, default_workspace)
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

fn resolve_identity_explicit_defaults(
    cfg: &Value,
    agent_id: &str,
) -> Result<IdentityDocument, String> {
    let agent = resolve_agent_entry(cfg, agent_id)?;
    let name = agent
        .get("identity")
        .and_then(|value| value.get("name"))
        .or_else(|| agent.get("identityName"))
        .or_else(|| agent.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let emoji = agent
        .get("identity")
        .and_then(|value| value.get("emoji"))
        .or_else(|| agent.get("identityEmoji"))
        .or_else(|| agent.get("emoji"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    Ok(IdentityDocument {
        name,
        emoji,
        persona: None,
    })
}

fn resolve_identity_defaults(cfg: &Value, agent_id: &str) -> Result<IdentityDocument, String> {
    let mut defaults = resolve_identity_explicit_defaults(cfg, agent_id)?;
    if defaults.name.is_none() {
        defaults.name = Some(agent_id.to_string());
    }
    Ok(defaults)
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

fn resolve_identity_dir_candidates(
    cfg: &Value,
    agent_id: &str,
    fallback_agent_root: Option<&str>,
) -> Result<Vec<String>, String> {
    let agent = resolve_agent_entry(cfg, agent_id)?;
    let mut candidates = Vec::new();

    push_unique_candidate(
        &mut candidates,
        agent
            .get("agentDir")
            .and_then(Value::as_str)
            .map(str::to_string),
    );
    push_unique_candidate(
        &mut candidates,
        fallback_agent_root
            .map(|root| format!("{}/{}/agent", root.trim_end_matches('/'), agent_id)),
    );
    push_unique_candidate(
        &mut candidates,
        agent
            .get("workspace")
            .and_then(Value::as_str)
            .map(str::to_string),
    );
    push_unique_candidate(&mut candidates, resolve_workspace(cfg, agent_id, None).ok());

    if candidates.is_empty() {
        return Err(format!(
            "Agent '{}' has no workspace or identity directory configured",
            agent_id
        ));
    }

    Ok(candidates)
}

fn resolve_local_identity_path(
    cfg: &Value,
    paths: &OpenClawPaths,
    agent_id: &str,
) -> Result<PathBuf, String> {
    let fallback_root = paths
        .openclaw_dir
        .join("agents")
        .to_string_lossy()
        .to_string();
    let candidate_dirs = resolve_identity_dir_candidates(cfg, agent_id, Some(&fallback_root))?;
    let candidate_paths: Vec<PathBuf> = candidate_dirs
        .into_iter()
        .map(|path| PathBuf::from(shellexpand::tilde(&path).to_string()))
        .collect();

    if let Some(existing) = candidate_paths
        .iter()
        .map(|dir| dir.join("IDENTITY.md"))
        .find(|path| path.exists())
    {
        return Ok(existing);
    }

    let agent = resolve_agent_entry(cfg, agent_id)?;
    let create_dir = agent
        .get("workspace")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| resolve_workspace(cfg, agent_id, None).ok())
        .or_else(|| {
            agent
                .get("agentDir")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| Some(format!("{}/{}/agent", fallback_root, agent_id)));

    create_dir
        .map(|dir| PathBuf::from(shellexpand::tilde(&dir).to_string()).join("IDENTITY.md"))
        .ok_or_else(|| format!("Agent '{}' has no identity path candidates", agent_id))
}

fn normalize_remote_dir(path: &str) -> String {
    if path.starts_with("~/") || path.starts_with('/') {
        path.to_string()
    } else {
        format!("~/{path}")
    }
}

async fn resolve_remote_identity_path(
    pool: &SshConnectionPool,
    host_id: &str,
    cfg: &Value,
    agent_id: &str,
) -> Result<String, String> {
    let fallback_root = "~/.openclaw/agents";
    let candidate_dirs = resolve_identity_dir_candidates(cfg, agent_id, Some(fallback_root))?;
    let candidate_dirs: Vec<String> = candidate_dirs
        .into_iter()
        .map(|dir| normalize_remote_dir(&dir))
        .collect();

    for dir in &candidate_dirs {
        let identity_path = format!("{dir}/IDENTITY.md");
        match pool.sftp_read(host_id, &identity_path).await {
            Ok(_) => return Ok(identity_path),
            Err(error) if error.contains("No such file") || error.contains("not found") => continue,
            Err(error) => return Err(error),
        }
    }

    let agent = resolve_agent_entry(cfg, agent_id)?;
    let create_dir = agent
        .get("workspace")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| resolve_workspace(cfg, agent_id, None).ok())
        .or_else(|| {
            agent
                .get("agentDir")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| Some(format!("{fallback_root}/{agent_id}/agent")));

    create_dir
        .map(|dir| format!("{}/IDENTITY.md", normalize_remote_dir(&dir)))
        .ok_or_else(|| format!("Agent '{}' has no identity path candidates", agent_id))
}

pub fn write_local_agent_identity(
    paths: &OpenClawPaths,
    agent_id: &str,
    name: Option<&str>,
    emoji: Option<&str>,
    persona: Option<&str>,
) -> Result<(), String> {
    let cfg = read_openclaw_config(paths)?;
    let identity_path = resolve_local_identity_path(&cfg, paths, agent_id)?;
    let defaults = resolve_identity_defaults(&cfg, agent_id)?;
    let identity_dir = identity_path
        .parent()
        .ok_or_else(|| "Failed to resolve identity directory".to_string())?;
    fs::create_dir_all(identity_dir)
        .map_err(|error| format!("Failed to create workspace dir: {}", error))?;
    let existing = fs::read_to_string(&identity_path).ok();
    fs::write(
        &identity_path,
        identity_content(
            existing.as_deref(),
            defaults.name.as_deref(),
            defaults.emoji.as_deref(),
            name,
            emoji,
            persona
                .map(PersonaChange::Set)
                .unwrap_or(PersonaChange::Preserve),
        )?,
    )
    .map_err(|error| format!("Failed to write IDENTITY.md: {}", error))?;
    Ok(())
}

fn shell_escape(value: &str) -> String {
    let escaped = value.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

pub async fn write_remote_agent_identity(
    pool: &SshConnectionPool,
    host_id: &str,
    agent_id: &str,
    name: Option<&str>,
    emoji: Option<&str>,
    persona: Option<&str>,
) -> Result<(), String> {
    let (_config_path, _raw, cfg) =
        crate::commands::remote_read_openclaw_config_text_and_json(pool, host_id)
            .await
            .map_err(|error| format!("Failed to parse config: {error}"))?;

    let identity_path = resolve_remote_identity_path(pool, host_id, &cfg, agent_id).await?;
    let defaults = resolve_identity_defaults(&cfg, agent_id)?;
    let remote_workspace = identity_path
        .strip_suffix("/IDENTITY.md")
        .ok_or_else(|| "Failed to resolve remote identity directory".to_string())?;
    pool.exec(
        host_id,
        &format!("mkdir -p {}", shell_escape(&remote_workspace)),
    )
    .await?;
    let existing = match pool.sftp_read(host_id, &identity_path).await {
        Ok(text) => Some(text),
        Err(error) if error.contains("No such file") || error.contains("not found") => None,
        Err(error) => return Err(error),
    };
    pool.sftp_write(
        host_id,
        &identity_path,
        &identity_content(
            existing.as_deref(),
            defaults.name.as_deref(),
            defaults.emoji.as_deref(),
            name,
            emoji,
            persona
                .map(PersonaChange::Set)
                .unwrap_or(PersonaChange::Preserve),
        )?,
    )
    .await?;
    Ok(())
}

pub fn set_local_agent_persona(
    paths: &OpenClawPaths,
    agent_id: &str,
    persona: &str,
) -> Result<(), String> {
    let cfg = read_openclaw_config(paths)?;
    let identity_path = resolve_local_identity_path(&cfg, paths, agent_id)?;
    let explicit_defaults = resolve_identity_explicit_defaults(&cfg, agent_id)?;
    let defaults = resolve_identity_defaults(&cfg, agent_id)?;
    let identity_dir = identity_path
        .parent()
        .ok_or_else(|| "Failed to resolve identity directory".to_string())?;
    fs::create_dir_all(identity_dir).map_err(|error| error.to_string())?;
    let existing = fs::read_to_string(&identity_path).ok();
    fs::write(
        &identity_path,
        upsert_persona_content(
            existing.as_deref(),
            explicit_defaults.name.as_deref(),
            explicit_defaults.emoji.as_deref(),
            defaults.name.as_deref(),
            defaults.emoji.as_deref(),
            PersonaChange::Set(persona),
        )?,
    )
    .map_err(|error| format!("Failed to write IDENTITY.md: {}", error))?;
    Ok(())
}

pub fn clear_local_agent_persona(paths: &OpenClawPaths, agent_id: &str) -> Result<(), String> {
    let cfg = read_openclaw_config(paths)?;
    let identity_path = resolve_local_identity_path(&cfg, paths, agent_id)?;
    let explicit_defaults = resolve_identity_explicit_defaults(&cfg, agent_id)?;
    let defaults = resolve_identity_defaults(&cfg, agent_id)?;
    let identity_dir = identity_path
        .parent()
        .ok_or_else(|| "Failed to resolve identity directory".to_string())?;
    fs::create_dir_all(identity_dir).map_err(|error| error.to_string())?;
    let existing = fs::read_to_string(&identity_path).ok();
    fs::write(
        &identity_path,
        upsert_persona_content(
            existing.as_deref(),
            explicit_defaults.name.as_deref(),
            explicit_defaults.emoji.as_deref(),
            defaults.name.as_deref(),
            defaults.emoji.as_deref(),
            PersonaChange::Clear,
        )?,
    )
    .map_err(|error| format!("Failed to write IDENTITY.md: {}", error))?;
    Ok(())
}

pub async fn set_remote_agent_persona(
    pool: &SshConnectionPool,
    host_id: &str,
    agent_id: &str,
    persona: &str,
) -> Result<(), String> {
    let (_config_path, _raw, cfg) =
        crate::commands::remote_read_openclaw_config_text_and_json(pool, host_id)
            .await
            .map_err(|error| format!("Failed to parse config: {error}"))?;
    let identity_path = resolve_remote_identity_path(pool, host_id, &cfg, agent_id).await?;
    let explicit_defaults = resolve_identity_explicit_defaults(&cfg, agent_id)?;
    let defaults = resolve_identity_defaults(&cfg, agent_id)?;
    let remote_workspace = identity_path
        .strip_suffix("/IDENTITY.md")
        .ok_or_else(|| "Failed to resolve remote identity directory".to_string())?;
    pool.exec(
        host_id,
        &format!("mkdir -p {}", shell_escape(remote_workspace)),
    )
    .await?;
    let existing = match pool.sftp_read(host_id, &identity_path).await {
        Ok(text) => Some(text),
        Err(error) if error.contains("No such file") || error.contains("not found") => None,
        Err(error) => return Err(error),
    };
    pool.sftp_write(
        host_id,
        &identity_path,
        &upsert_persona_content(
            existing.as_deref(),
            explicit_defaults.name.as_deref(),
            explicit_defaults.emoji.as_deref(),
            defaults.name.as_deref(),
            defaults.emoji.as_deref(),
            PersonaChange::Set(persona),
        )?,
    )
    .await?;
    Ok(())
}

pub async fn clear_remote_agent_persona(
    pool: &SshConnectionPool,
    host_id: &str,
    agent_id: &str,
) -> Result<(), String> {
    let (_config_path, _raw, cfg) =
        crate::commands::remote_read_openclaw_config_text_and_json(pool, host_id)
            .await
            .map_err(|error| format!("Failed to parse config: {error}"))?;
    let identity_path = resolve_remote_identity_path(pool, host_id, &cfg, agent_id).await?;
    let explicit_defaults = resolve_identity_explicit_defaults(&cfg, agent_id)?;
    let defaults = resolve_identity_defaults(&cfg, agent_id)?;
    let remote_workspace = identity_path
        .strip_suffix("/IDENTITY.md")
        .ok_or_else(|| "Failed to resolve remote identity directory".to_string())?;
    pool.exec(
        host_id,
        &format!("mkdir -p {}", shell_escape(remote_workspace)),
    )
    .await?;
    let existing = match pool.sftp_read(host_id, &identity_path).await {
        Ok(text) => Some(text),
        Err(error) if error.contains("No such file") || error.contains("not found") => None,
        Err(error) => return Err(error),
    };
    pool.sftp_write(
        host_id,
        &identity_path,
        &upsert_persona_content(
            existing.as_deref(),
            explicit_defaults.name.as_deref(),
            explicit_defaults.emoji.as_deref(),
            defaults.name.as_deref(),
            defaults.emoji.as_deref(),
            PersonaChange::Clear,
        )?,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{set_local_agent_persona, write_local_agent_identity};
    use crate::cli_runner::{
        lock_active_override_test_state, set_active_clawpal_data_override,
        set_active_openclaw_home_override,
    };
    use crate::models::resolve_paths;
    use serde_json::json;
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn write_local_agent_identity_creates_identity_file_from_config_workspace() {
        let _override_guard = lock_active_override_test_state();
        let temp_root = std::env::temp_dir().join(format!("clawpal-identity-{}", Uuid::new_v4()));
        let openclaw_home = temp_root.join("home");
        let clawpal_data = temp_root.join("data");
        let openclaw_dir = openclaw_home.join(".openclaw");
        let workspace = temp_root.join("workspace").join("lobster");
        fs::create_dir_all(&openclaw_dir).expect("create openclaw dir");
        fs::create_dir_all(&clawpal_data).expect("create clawpal data dir");
        fs::write(
            openclaw_dir.join("openclaw.json"),
            serde_json::to_string_pretty(&json!({
                "agents": {
                    "list": [
                        {
                            "id": "lobster",
                            "workspace": workspace.to_string_lossy(),
                        }
                    ]
                }
            }))
            .expect("serialize config"),
        )
        .expect("write config");

        set_active_openclaw_home_override(Some(openclaw_home.to_string_lossy().to_string()))
            .expect("set openclaw override");
        set_active_clawpal_data_override(Some(clawpal_data.to_string_lossy().to_string()))
            .expect("set clawpal override");

        let result = write_local_agent_identity(
            &resolve_paths(),
            "lobster",
            Some("Lobster"),
            Some("🦞"),
            Some("You help triage crabby incidents."),
        );

        set_active_openclaw_home_override(None).expect("clear openclaw override");
        set_active_clawpal_data_override(None).expect("clear clawpal override");

        assert!(result.is_ok());
        assert_eq!(
            fs::read_to_string(workspace.join("IDENTITY.md")).expect("read identity file"),
            "- Name: Lobster\n- Emoji: 🦞\n\n## Persona\nYou help triage crabby incidents.\n"
        );

        let _ = fs::remove_dir_all(temp_root);
    }

    #[test]
    fn write_local_agent_identity_preserves_name_and_emoji_when_updating_persona_only() {
        let _override_guard = lock_active_override_test_state();
        let temp_root = std::env::temp_dir().join(format!("clawpal-identity-{}", Uuid::new_v4()));
        let openclaw_home = temp_root.join("home");
        let clawpal_data = temp_root.join("data");
        let openclaw_dir = openclaw_home.join(".openclaw");
        let workspace = temp_root.join("workspace").join("lobster");
        fs::create_dir_all(&openclaw_dir).expect("create openclaw dir");
        fs::create_dir_all(&clawpal_data).expect("create clawpal data dir");
        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::write(
            workspace.join("IDENTITY.md"),
            "- Name: Lobster\n- Emoji: 🦞\n\n## Persona\nOld persona.\n",
        )
        .expect("write identity seed");
        fs::write(
            openclaw_dir.join("openclaw.json"),
            serde_json::to_string_pretty(&json!({
                "agents": {
                    "list": [
                        {
                            "id": "lobster",
                            "workspace": workspace.to_string_lossy(),
                        }
                    ]
                }
            }))
            .expect("serialize config"),
        )
        .expect("write config");

        set_active_openclaw_home_override(Some(openclaw_home.to_string_lossy().to_string()))
            .expect("set openclaw override");
        set_active_clawpal_data_override(Some(clawpal_data.to_string_lossy().to_string()))
            .expect("set clawpal override");

        let result = write_local_agent_identity(
            &resolve_paths(),
            "lobster",
            None,
            None,
            Some("New persona."),
        );

        set_active_openclaw_home_override(None).expect("clear openclaw override");
        set_active_clawpal_data_override(None).expect("clear clawpal override");

        assert!(result.is_ok());
        assert_eq!(
            fs::read_to_string(workspace.join("IDENTITY.md")).expect("read identity file"),
            "- Name: Lobster\n- Emoji: 🦞\n\n## Persona\nNew persona.\n"
        );

        let _ = fs::remove_dir_all(temp_root);
    }

    #[test]
    fn write_local_agent_identity_updates_existing_agent_dir_identity_when_workspace_missing() {
        let _override_guard = lock_active_override_test_state();
        let temp_root = std::env::temp_dir().join(format!("clawpal-identity-{}", Uuid::new_v4()));
        let openclaw_home = temp_root.join("home");
        let clawpal_data = temp_root.join("data");
        let openclaw_dir = openclaw_home.join(".openclaw");
        let agent_dir = openclaw_dir.join("agents").join("main").join("agent");
        fs::create_dir_all(&agent_dir).expect("create agent dir");
        fs::create_dir_all(&clawpal_data).expect("create clawpal data dir");
        fs::write(
            agent_dir.join("IDENTITY.md"),
            "- Name: Main Agent\n- Emoji: 🤖\n\n## Persona\nOld persona.\n",
        )
        .expect("write identity seed");
        fs::write(
            openclaw_dir.join("openclaw.json"),
            serde_json::to_string_pretty(&json!({
                "agents": {
                    "list": [
                        {
                            "id": "main",
                            "model": "anthropic/claude-sonnet-4-20250514",
                        }
                    ]
                }
            }))
            .expect("serialize config"),
        )
        .expect("write config");

        set_active_openclaw_home_override(Some(openclaw_home.to_string_lossy().to_string()))
            .expect("set openclaw override");
        set_active_clawpal_data_override(Some(clawpal_data.to_string_lossy().to_string()))
            .expect("set clawpal override");

        let result =
            write_local_agent_identity(&resolve_paths(), "main", None, None, Some("New persona."));

        set_active_openclaw_home_override(None).expect("clear openclaw override");
        set_active_clawpal_data_override(None).expect("clear clawpal override");

        assert!(result.is_ok());
        assert_eq!(
            fs::read_to_string(agent_dir.join("IDENTITY.md")).expect("read identity file"),
            "- Name: Main Agent\n- Emoji: 🤖\n\n## Persona\nNew persona.\n"
        );

        let _ = fs::remove_dir_all(temp_root);
    }

    #[test]
    fn write_local_agent_identity_uses_agent_id_when_identity_file_is_missing() {
        let _override_guard = lock_active_override_test_state();
        let temp_root = std::env::temp_dir().join(format!("clawpal-identity-{}", Uuid::new_v4()));
        let openclaw_home = temp_root.join("home");
        let clawpal_data = temp_root.join("data");
        let openclaw_dir = openclaw_home.join(".openclaw");
        let workspace = temp_root.join("workspace").join("test-agent");
        fs::create_dir_all(&openclaw_dir).expect("create openclaw dir");
        fs::create_dir_all(&clawpal_data).expect("create clawpal data dir");
        fs::write(
            openclaw_dir.join("openclaw.json"),
            serde_json::to_string_pretty(&json!({
                "agents": {
                    "list": [
                        {
                            "id": "test-agent",
                            "workspace": workspace.to_string_lossy(),
                        }
                    ]
                }
            }))
            .expect("serialize config"),
        )
        .expect("write config");

        set_active_openclaw_home_override(Some(openclaw_home.to_string_lossy().to_string()))
            .expect("set openclaw override");
        set_active_clawpal_data_override(Some(clawpal_data.to_string_lossy().to_string()))
            .expect("set clawpal override");

        let result = write_local_agent_identity(
            &resolve_paths(),
            "test-agent",
            None,
            None,
            Some("New persona."),
        );

        set_active_openclaw_home_override(None).expect("clear openclaw override");
        set_active_clawpal_data_override(None).expect("clear clawpal override");

        assert!(result.is_ok());
        assert_eq!(
            fs::read_to_string(workspace.join("IDENTITY.md")).expect("read identity file"),
            "- Name: test-agent\n\n## Persona\nNew persona.\n"
        );

        let _ = fs::remove_dir_all(temp_root);
    }

    #[test]
    fn set_local_agent_persona_rewrites_openclaw_identity_template_with_explicit_defaults() {
        let _override_guard = lock_active_override_test_state();
        let temp_root = std::env::temp_dir().join(format!("clawpal-identity-{}", Uuid::new_v4()));
        let openclaw_home = temp_root.join("home");
        let clawpal_data = temp_root.join("data");
        let openclaw_dir = openclaw_home.join(".openclaw");
        let workspace = temp_root.join("workspace").join("ops-bot");
        fs::create_dir_all(&openclaw_dir).expect("create openclaw dir");
        fs::create_dir_all(&clawpal_data).expect("create clawpal data dir");
        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::write(
            workspace.join("IDENTITY.md"),
            "# IDENTITY.md - Who Am I?\n\n_Fill this in during your first conversation._\n",
        )
        .expect("write identity seed");
        fs::write(
            openclaw_dir.join("openclaw.json"),
            serde_json::to_string_pretty(&json!({
                "agents": {
                    "list": [
                        {
                            "id": "ops-bot",
                            "workspace": workspace.to_string_lossy(),
                            "identity": {
                                "name": "Ops Bot",
                                "emoji": "🛰️"
                            }
                        }
                    ]
                }
            }))
            .expect("serialize config"),
        )
        .expect("write config");

        set_active_openclaw_home_override(Some(openclaw_home.to_string_lossy().to_string()))
            .expect("set openclaw override");
        set_active_clawpal_data_override(Some(clawpal_data.to_string_lossy().to_string()))
            .expect("set clawpal override");

        let result = set_local_agent_persona(&resolve_paths(), "ops-bot", "Keep systems green.");

        set_active_openclaw_home_override(None).expect("clear openclaw override");
        set_active_clawpal_data_override(None).expect("clear clawpal override");

        assert!(result.is_ok());
        assert_eq!(
            fs::read_to_string(workspace.join("IDENTITY.md")).expect("read identity file"),
            "- Name: Ops Bot\n- Emoji: 🛰️\n\n## Persona\nKeep systems green.\n"
        );

        let _ = fs::remove_dir_all(temp_root);
    }

    #[test]
    fn set_local_agent_persona_preserves_non_clawpal_identity_header() {
        let _override_guard = lock_active_override_test_state();
        let temp_root = std::env::temp_dir().join(format!("clawpal-identity-{}", Uuid::new_v4()));
        let openclaw_home = temp_root.join("home");
        let clawpal_data = temp_root.join("data");
        let openclaw_dir = openclaw_home.join(".openclaw");
        let workspace = temp_root.join("workspace").join("ops-bot");
        fs::create_dir_all(&openclaw_dir).expect("create openclaw dir");
        fs::create_dir_all(&clawpal_data).expect("create clawpal data dir");
        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::write(
            workspace.join("IDENTITY.md"),
            "# Ops Bot\n\nOpenClaw managed identity header.\n",
        )
        .expect("write identity seed");
        fs::write(
            openclaw_dir.join("openclaw.json"),
            serde_json::to_string_pretty(&json!({
                "agents": {
                    "list": [
                        {
                            "id": "ops-bot",
                            "workspace": workspace.to_string_lossy(),
                        }
                    ]
                }
            }))
            .expect("serialize config"),
        )
        .expect("write config");

        set_active_openclaw_home_override(Some(openclaw_home.to_string_lossy().to_string()))
            .expect("set openclaw override");
        set_active_clawpal_data_override(Some(clawpal_data.to_string_lossy().to_string()))
            .expect("set clawpal override");

        let result = set_local_agent_persona(&resolve_paths(), "ops-bot", "Keep systems green.");

        set_active_openclaw_home_override(None).expect("clear openclaw override");
        set_active_clawpal_data_override(None).expect("clear clawpal override");

        assert!(result.is_ok());
        assert_eq!(
            fs::read_to_string(workspace.join("IDENTITY.md")).expect("read identity file"),
            "# Ops Bot\n\nOpenClaw managed identity header.\n\n## Persona\nKeep systems green.\n"
        );

        let _ = fs::remove_dir_all(temp_root);
    }
}
