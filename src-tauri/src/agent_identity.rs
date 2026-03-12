use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::config_io::read_openclaw_config;
use crate::models::OpenClawPaths;
use crate::ssh::SshConnectionPool;

fn identity_content(name: &str, emoji: Option<&str>) -> String {
    let mut content = format!("- Name: {}\n", name.trim());
    if let Some(emoji) = emoji.map(str::trim).filter(|value| !value.is_empty()) {
        content.push_str(&format!("- Emoji: {}\n", emoji));
    }
    content
}

fn resolve_workspace(
    cfg: &Value,
    agent_id: &str,
    default_workspace: Option<&str>,
) -> Result<String, String> {
    clawpal_core::doctor::resolve_agent_workspace_from_config(cfg, agent_id, default_workspace)
}

pub fn write_local_agent_identity(
    paths: &OpenClawPaths,
    agent_id: &str,
    name: &str,
    emoji: Option<&str>,
) -> Result<(), String> {
    let cfg = read_openclaw_config(paths)?;
    let workspace = resolve_workspace(&cfg, agent_id, None)
        .map(|path| shellexpand::tilde(&path).to_string())?;
    let workspace_path = Path::new(&workspace);
    fs::create_dir_all(workspace_path)
        .map_err(|error| format!("Failed to create workspace dir: {}", error))?;
    fs::write(
        workspace_path.join("IDENTITY.md"),
        identity_content(name, emoji),
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
    name: &str,
    emoji: Option<&str>,
) -> Result<(), String> {
    let (_config_path, _raw, cfg) =
        crate::commands::remote_read_openclaw_config_text_and_json(pool, host_id)
            .await
            .map_err(|error| format!("Failed to parse config: {error}"))?;

    let workspace = resolve_workspace(&cfg, agent_id, Some("~/.openclaw/agents"))?;
    let remote_workspace = if workspace.starts_with("~/") {
        workspace
    } else {
        format!("~/{workspace}")
    };
    pool.exec(
        host_id,
        &format!("mkdir -p {}", shell_escape(&remote_workspace)),
    )
    .await?;
    pool.sftp_write(
        host_id,
        &format!("{remote_workspace}/IDENTITY.md"),
        &identity_content(name, emoji),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_local_agent_identity;
    use crate::cli_runner::{set_active_clawpal_data_override, set_active_openclaw_home_override};
    use crate::models::resolve_paths;
    use serde_json::json;
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn write_local_agent_identity_creates_identity_file_from_config_workspace() {
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

        let result = write_local_agent_identity(&resolve_paths(), "lobster", "Lobster", Some("🦞"));

        set_active_openclaw_home_override(None).expect("clear openclaw override");
        set_active_clawpal_data_override(None).expect("clear clawpal override");

        assert!(result.is_ok());
        assert_eq!(
            fs::read_to_string(workspace.join("IDENTITY.md")).expect("read identity file"),
            "- Name: Lobster\n- Emoji: 🦞\n"
        );

        let _ = fs::remove_dir_all(temp_root);
    }
}
