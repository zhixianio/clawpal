use super::*;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceConfigSnapshot {
    pub global_default_model: Option<String>,
    pub fallback_models: Vec<String>,
    pub agents: Vec<AgentOverview>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceRuntimeSnapshot {
    pub status: StatusLight,
    pub agents: Vec<AgentOverview>,
    pub global_default_model: Option<String>,
    pub fallback_models: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelsConfigSnapshot {
    pub channels: Vec<ChannelNode>,
    pub bindings: Vec<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelsRuntimeSnapshot {
    pub channels: Vec<ChannelNode>,
    pub bindings: Vec<Value>,
    pub agents: Vec<AgentOverview>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronConfigSnapshot {
    pub jobs: Vec<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronRuntimeSnapshot {
    pub jobs: Vec<Value>,
    pub watchdog: Value,
}

fn extract_default_model_and_fallbacks(cfg: &Value) -> (Option<String>, Vec<String>) {
    let default_model = cfg
        .pointer("/agents/defaults/model")
        .and_then(read_model_value)
        .or_else(|| {
            cfg.pointer("/agents/default/model")
                .and_then(read_model_value)
        });
    let fallback_models = cfg
        .pointer("/agents/defaults/model/fallbacks")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    (default_model, fallback_models)
}

pub(crate) fn collect_agent_overviews_from_config(cfg: &Value) -> Vec<AgentOverview> {
    cfg.pointer("/agents/list")
        .and_then(Value::as_array)
        .map(|agents| {
            agents
                .iter()
                .filter_map(|agent| {
                    let id = agent.get("id").and_then(Value::as_str)?.trim().to_string();
                    if id.is_empty() {
                        return None;
                    }
                    Some(AgentOverview {
                        id,
                        name: agent
                            .get("identityName")
                            .or_else(|| agent.get("name"))
                            .and_then(Value::as_str)
                            .map(|value| value.to_string()),
                        emoji: agent
                            .get("identityEmoji")
                            .or_else(|| agent.get("emoji"))
                            .and_then(Value::as_str)
                            .map(|value| value.to_string()),
                        model: agent.get("model").and_then(read_model_value),
                        channels: Vec::new(),
                        online: false,
                        workspace: agent
                            .get("workspace")
                            .and_then(Value::as_str)
                            .map(|value| value.to_string()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_bindings_from_config(cfg: &Value) -> Result<Vec<Value>, String> {
    let bindings = cfg
        .get("bindings")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let parsed = clawpal_core::discovery::parse_bindings(&bindings.to_string())?;
    serde_json::to_value(parsed)
        .map_err(|error| error.to_string())?
        .as_array()
        .cloned()
        .ok_or_else(|| "bindings payload is not an array".to_string())
}

fn extract_instance_config_snapshot(cfg: &Value) -> InstanceConfigSnapshot {
    let (global_default_model, fallback_models) = extract_default_model_and_fallbacks(cfg);
    InstanceConfigSnapshot {
        global_default_model,
        fallback_models,
        agents: collect_agent_overviews_from_config(cfg),
    }
}

fn extract_channels_config_snapshot(cfg: &Value) -> Result<ChannelsConfigSnapshot, String> {
    Ok(ChannelsConfigSnapshot {
        channels: collect_channel_nodes(cfg),
        bindings: extract_bindings_from_config(cfg)?,
    })
}

fn parse_remote_watchdog_value(raw: Value) -> Value {
    match raw {
        Value::Object(_) => raw,
        _ => serde_json::json!({
            "alive": false,
            "deployed": false,
        }),
    }
}

async fn remote_instance_runtime_snapshot_impl(
    pool: &SshConnectionPool,
    host_id: &str,
) -> Result<InstanceRuntimeSnapshot, String> {
    let (config_res, agents_res, pgrep_res, online_res) = tokio::join!(
        crate::cli_runner::run_openclaw_remote(pool, host_id, &["config", "get", "agents", "--json"]),
        crate::cli_runner::run_openclaw_remote(pool, host_id, &["agents", "list", "--json"]),
        pool.exec(host_id, "pgrep -f '[o]penclaw-gateway' >/dev/null 2>&1"),
        pool.exec_login(
            host_id,
            "for d in ~/.openclaw/agents/*/sessions/sessions.json; do [ -f \"$d\" ] && [ $(wc -c < \"$d\") -gt 2 ] && basename $(dirname $(dirname \"$d\")); done",
        ),
    );

    let config_output = config_res?;
    let agents_output = agents_res?;
    let pgrep_output = pgrep_res?;

    let config_json = if config_output.exit_code == 0 {
        crate::cli_runner::parse_json_output(&config_output)?
    } else {
        Value::Null
    };
    let agents_json = if agents_output.exit_code == 0 {
        crate::cli_runner::parse_json_output(&agents_output)?
    } else {
        Value::Null
    };

    let online_set = online_res
        .ok()
        .map(|result| {
            result
                .stdout
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect::<std::collections::HashSet<String>>()
        })
        .unwrap_or_default();
    let agents = parse_agents_cli_output(&agents_json, Some(&online_set))?;
    let active_agents = count_agent_entries_from_cli_json(&agents_json).unwrap_or(0);
    let (global_default_model, fallback_models) = extract_default_model_and_fallbacks(&config_json);

    let ssh_diagnostic = if config_output.exit_code != 0 {
        Some(from_any_error(
            SshStage::RemoteExec,
            SshIntent::HealthCheck,
            format!(
                "openclaw config get agents failed ({}): {} {}",
                config_output.exit_code, config_output.stderr, config_output.stdout
            ),
        ))
    } else if agents_output.exit_code != 0 {
        Some(from_any_error(
            SshStage::RemoteExec,
            SshIntent::HealthCheck,
            format!(
                "openclaw agents list failed ({}): {} {}",
                agents_output.exit_code, agents_output.stderr, agents_output.stdout
            ),
        ))
    } else {
        None
    };

    Ok(InstanceRuntimeSnapshot {
        status: StatusLight {
            healthy: pgrep_output.exit_code == 0
                || (config_output.exit_code == 0 && ssh_diagnostic.is_none()),
            active_agents,
            global_default_model: global_default_model.clone(),
            fallback_models: fallback_models.clone(),
            ssh_diagnostic,
        },
        agents,
        global_default_model,
        fallback_models,
    })
}

async fn remote_channels_runtime_snapshot_impl(
    pool: &SshConnectionPool,
    host_id: &str,
) -> Result<ChannelsRuntimeSnapshot, String> {
    let (channels_res, bindings_res, agents_res, online_res) = tokio::join!(
        crate::cli_runner::run_openclaw_remote(pool, host_id, &["config", "get", "channels", "--json"]),
        crate::cli_runner::run_openclaw_remote(pool, host_id, &["config", "get", "bindings", "--json"]),
        crate::cli_runner::run_openclaw_remote(pool, host_id, &["agents", "list", "--json"]),
        pool.exec_login(
            host_id,
            "for d in ~/.openclaw/agents/*/sessions/sessions.json; do [ -f \"$d\" ] && [ $(wc -c < \"$d\") -gt 2 ] && basename $(dirname $(dirname \"$d\")); done",
        ),
    );

    let channels_output = channels_res?;
    let bindings_output = bindings_res?;
    let agents_output = agents_res?;
    let online_set = online_res
        .ok()
        .map(|result| {
            result
                .stdout
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect::<std::collections::HashSet<String>>()
        })
        .unwrap_or_default();

    let channels = if channels_output.exit_code == 0 {
        let channels_val = crate::cli_runner::parse_json_output(&channels_output)?;
        let cfg = serde_json::json!({ "channels": channels_val });
        collect_channel_nodes(&cfg)
    } else {
        Vec::new()
    };

    let bindings = if bindings_output.exit_code == 0 {
        let bindings_json = crate::cli_runner::parse_json_output(&bindings_output)?;
        serde_json::to_value(clawpal_core::discovery::parse_bindings(
            &bindings_json.to_string(),
        )?)
        .map_err(|error| error.to_string())?
        .as_array()
        .cloned()
        .unwrap_or_default()
    } else {
        Vec::new()
    };

    let agents = if agents_output.exit_code == 0 {
        let agents_json = crate::cli_runner::parse_json_output(&agents_output)?;
        parse_agents_cli_output(&agents_json, Some(&online_set))?
    } else {
        Vec::new()
    };

    Ok(ChannelsRuntimeSnapshot {
        channels,
        bindings,
        agents,
    })
}

#[tauri::command]
pub async fn get_instance_config_snapshot() -> Result<InstanceConfigSnapshot, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let cfg = read_openclaw_config(&resolve_paths())?;
        Ok(extract_instance_config_snapshot(&cfg))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn remote_get_instance_config_snapshot(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<InstanceConfigSnapshot, String> {
    let (_, _, cfg) = remote_read_openclaw_config_text_and_json(&pool, &host_id).await?;
    Ok(extract_instance_config_snapshot(&cfg))
}

#[tauri::command]
pub async fn get_instance_runtime_snapshot(
    cache: tauri::State<'_, crate::cli_runner::CliCache>,
) -> Result<InstanceRuntimeSnapshot, String> {
    let status = get_status_light().await?;
    let agents = list_agents_overview(cache).await?;
    Ok(InstanceRuntimeSnapshot {
        global_default_model: status.global_default_model.clone(),
        fallback_models: status.fallback_models.clone(),
        status,
        agents,
    })
}

#[tauri::command]
pub async fn remote_get_instance_runtime_snapshot(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<InstanceRuntimeSnapshot, String> {
    remote_instance_runtime_snapshot_impl(&pool, &host_id).await
}

#[tauri::command]
pub async fn get_channels_config_snapshot() -> Result<ChannelsConfigSnapshot, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let cfg = read_openclaw_config(&resolve_paths())?;
        extract_channels_config_snapshot(&cfg)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn remote_get_channels_config_snapshot(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<ChannelsConfigSnapshot, String> {
    let (_, _, cfg) = remote_read_openclaw_config_text_and_json(&pool, &host_id).await?;
    extract_channels_config_snapshot(&cfg)
}

#[tauri::command]
pub async fn get_channels_runtime_snapshot(
    cache: tauri::State<'_, crate::cli_runner::CliCache>,
) -> Result<ChannelsRuntimeSnapshot, String> {
    let channels = list_channels_minimal(cache.clone()).await?;
    let bindings = list_bindings(cache.clone()).await?;
    let agents = list_agents_overview(cache).await?;
    let bindings = serde_json::to_value(bindings)
        .map_err(|error| error.to_string())?
        .as_array()
        .cloned()
        .unwrap_or_default();
    Ok(ChannelsRuntimeSnapshot {
        channels,
        bindings,
        agents,
    })
}

#[tauri::command]
pub async fn remote_get_channels_runtime_snapshot(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<ChannelsRuntimeSnapshot, String> {
    remote_channels_runtime_snapshot_impl(&pool, &host_id).await
}

#[tauri::command]
pub fn get_cron_config_snapshot() -> Result<CronConfigSnapshot, String> {
    let jobs = list_cron_jobs()?;
    let jobs = jobs.as_array().cloned().unwrap_or_default();
    Ok(CronConfigSnapshot { jobs })
}

#[tauri::command]
pub async fn remote_get_cron_config_snapshot(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<CronConfigSnapshot, String> {
    let jobs = remote_list_cron_jobs(pool, host_id).await?;
    let jobs = jobs.as_array().cloned().unwrap_or_default();
    Ok(CronConfigSnapshot { jobs })
}

#[tauri::command]
pub async fn get_cron_runtime_snapshot() -> Result<CronRuntimeSnapshot, String> {
    let jobs = list_cron_jobs()?;
    let watchdog = get_watchdog_status().await?;
    let jobs = jobs.as_array().cloned().unwrap_or_default();
    Ok(CronRuntimeSnapshot { jobs, watchdog })
}

#[tauri::command]
pub async fn remote_get_cron_runtime_snapshot(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<CronRuntimeSnapshot, String> {
    let jobs = remote_list_cron_jobs(pool.clone(), host_id.clone()).await?;
    let watchdog = remote_get_watchdog_status(pool, host_id).await?;
    let jobs = jobs.as_array().cloned().unwrap_or_default();
    Ok(CronRuntimeSnapshot {
        jobs,
        watchdog: parse_remote_watchdog_value(watchdog),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_config_snapshot_extracts_defaults_and_agents() {
        let cfg = serde_json::json!({
            "agents": {
                "defaults": {
                    "model": {
                        "primary": "openai/gpt-5.3-codex",
                        "fallbacks": ["openai/gpt-5-mini"]
                    }
                },
                "list": [
                    { "id": "main", "name": "Main", "emoji": "🦀", "model": "openai/gpt-5.3-codex" }
                ]
            }
        });

        let snapshot = extract_instance_config_snapshot(&cfg);

        assert_eq!(
            snapshot.global_default_model.as_deref(),
            Some("openai/gpt-5.3-codex")
        );
        assert_eq!(snapshot.fallback_models, vec!["openai/gpt-5-mini"]);
        assert_eq!(snapshot.agents.len(), 1);
        assert_eq!(snapshot.agents[0].id, "main");
        assert!(!snapshot.agents[0].online);
    }

    #[test]
    fn agent_overviews_from_config_accept_identity_fields() {
        let cfg = serde_json::json!({
            "agents": {
                "list": [
                    {
                        "id": "helper",
                        "identityName": "Helper",
                        "identityEmoji": "🛟",
                        "model": "openai/gpt-4o"
                    }
                ]
            }
        });

        let agents = collect_agent_overviews_from_config(&cfg);

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "helper");
        assert_eq!(agents[0].name.as_deref(), Some("Helper"));
        assert_eq!(agents[0].emoji.as_deref(), Some("🛟"));
    }

    #[test]
    fn channels_config_snapshot_extracts_bindings_and_nodes() {
        let cfg = serde_json::json!({
            "channels": {
                "discord": {
                    "guilds": {
                        "guild-1": {
                            "channels": {
                                "peer-1": { "type": "discord" }
                            }
                        }
                    }
                }
            },
            "bindings": [
                {
                    "agentId": "main",
                    "match": {
                        "channel": "discord",
                        "peer": { "kind": "channel", "id": "peer-1" }
                    }
                }
            ]
        });

        let snapshot = extract_channels_config_snapshot(&cfg).expect("snapshot");

        assert!(snapshot
            .channels
            .iter()
            .any(|node| node.path == "channels.discord.guilds.guild-1.channels.peer-1"));
        assert_eq!(snapshot.bindings.len(), 1);
    }
}
