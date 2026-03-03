use super::*;

#[tauri::command]
pub async fn remote_list_discord_guild_channels(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<DiscordGuildChannel>, String> {
    let output = crate::cli_runner::run_openclaw_remote(
        &pool,
        &host_id,
        &["config", "get", "channels.discord", "--json"],
    )
    .await?;
    let discord_section = if output.exit_code == 0 {
        crate::cli_runner::parse_json_output(&output).unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let bindings_output = crate::cli_runner::run_openclaw_remote(
        &pool,
        &host_id,
        &["config", "get", "bindings", "--json"],
    )
    .await?;
    let bindings_section = if bindings_output.exit_code == 0 {
        crate::cli_runner::parse_json_output(&bindings_output)
            .unwrap_or_else(|_| Value::Array(Vec::new()))
    } else {
        Value::Array(Vec::new())
    };
    // Wrap to match existing code expectations (rest of function uses cfg.get("channels").and_then(|c| c.get("discord")))
    let cfg = serde_json::json!({
        "channels": { "discord": discord_section },
        "bindings": bindings_section
    });

    let discord_cfg = cfg.get("channels").and_then(|c| c.get("discord"));
    let configured_single_guild_id = discord_cfg
        .and_then(|d| d.get("guilds"))
        .and_then(Value::as_object)
        .and_then(|guilds| {
            if guilds.len() == 1 {
                guilds.keys().next().cloned()
            } else {
                None
            }
        });

    // Extract bot token: top-level first, then fall back to first account token
    let bot_token = discord_cfg
        .and_then(|d| d.get("botToken").or_else(|| d.get("token")))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .or_else(|| {
            discord_cfg
                .and_then(|d| d.get("accounts"))
                .and_then(Value::as_object)
                .and_then(|accounts| {
                    accounts.values().find_map(|acct| {
                        acct.get("token")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                    })
                })
        });
    let mut guild_name_fallback_map = pool
        .sftp_read(&host_id, "~/.clawpal/discord-guild-channels.json")
        .await
        .ok()
        .map(|text| parse_discord_cache_guild_name_fallbacks(&text))
        .unwrap_or_default();
    guild_name_fallback_map.extend(collect_discord_config_guild_name_fallbacks(discord_cfg));

    let core_channels = clawpal_core::discovery::parse_guild_channels(&cfg.to_string())?;
    let mut entries: Vec<DiscordGuildChannel> = core_channels
        .iter()
        .map(|c| DiscordGuildChannel {
            guild_id: c.guild_id.clone(),
            guild_name: c.guild_name.clone(),
            channel_id: c.channel_id.clone(),
            channel_name: c.channel_name.clone(),
            default_agent_id: None,
        })
        .collect();
    let mut channel_ids: Vec<String> = entries.iter().map(|e| e.channel_id.clone()).collect();
    let mut unresolved_guild_ids: Vec<String> = entries
        .iter()
        .filter(|e| e.guild_name == e.guild_id)
        .map(|e| e.guild_id.clone())
        .collect();
    unresolved_guild_ids.sort();
    unresolved_guild_ids.dedup();

    // Fallback A: if we have token + guild ids, fetch channels from Discord REST directly.
    // This avoids hard-failing when CLI rejects config due non-critical schema drift.
    if channel_ids.is_empty() {
        let configured_guild_ids = collect_discord_config_guild_ids(discord_cfg);
        if let Some(token) = bot_token.clone() {
            let rest_entries = tokio::task::spawn_blocking(move || {
                let mut out: Vec<DiscordGuildChannel> = Vec::new();
                for guild_id in configured_guild_ids {
                    if let Ok(channels) = fetch_discord_guild_channels(&token, &guild_id) {
                        for (channel_id, channel_name) in channels {
                            if out
                                .iter()
                                .any(|e| e.guild_id == guild_id && e.channel_id == channel_id)
                            {
                                continue;
                            }
                            out.push(DiscordGuildChannel {
                                guild_id: guild_id.clone(),
                                guild_name: guild_id.clone(),
                                channel_id,
                                channel_name,
                                default_agent_id: None,
                            });
                        }
                    }
                }
                out
            })
            .await
            .unwrap_or_default();
            for entry in rest_entries {
                if entries
                    .iter()
                    .any(|e| e.guild_id == entry.guild_id && e.channel_id == entry.channel_id)
                {
                    continue;
                }
                channel_ids.push(entry.channel_id.clone());
                entries.push(entry);
            }
        }
    }

    // Fallback B: query channel ids from directory and keep compatibility
    // with existing cache shape when config has no explicit channel map.
    if channel_ids.is_empty() {
        let cmd = "openclaw directory groups list --channel discord --json";
        if let Ok(r) = pool.exec_login(&host_id, cmd).await {
            if r.exit_code == 0 && !r.stdout.trim().is_empty() {
                for channel_id in parse_directory_group_channel_ids(&r.stdout) {
                    if entries.iter().any(|e| e.channel_id == channel_id) {
                        continue;
                    }
                    let (guild_id, guild_name) =
                        if let Some(gid) = configured_single_guild_id.clone() {
                            (gid.clone(), gid)
                        } else {
                            ("discord".to_string(), "Discord".to_string())
                        };
                    channel_ids.push(channel_id.clone());
                    entries.push(DiscordGuildChannel {
                        guild_id,
                        guild_name,
                        channel_id: channel_id.clone(),
                        channel_name: channel_id,
                        default_agent_id: None,
                    });
                }
            }
        }
    }

    // Resolve channel names via openclaw CLI on remote
    if !channel_ids.is_empty() {
        let ids_arg = channel_ids.join(" ");
        let cmd = format!(
            "openclaw channels resolve --json --channel discord --kind auto {}",
            ids_arg
        );
        if let Ok(r) = pool.exec_login(&host_id, &cmd).await {
            if r.exit_code == 0 && !r.stdout.trim().is_empty() {
                if let Some(name_map) = parse_resolve_name_map(&r.stdout) {
                    for entry in &mut entries {
                        if let Some(name) = name_map.get(&entry.channel_id) {
                            entry.channel_name = name.clone();
                        }
                    }
                }
            }
        }
    }

    // Resolve guild names via Discord REST API (guild names can't be resolved by openclaw CLI)
    // Must use spawn_blocking because reqwest::blocking panics in async context
    if let Some(token) = bot_token {
        if !unresolved_guild_ids.is_empty() {
            let guild_name_map = tokio::task::spawn_blocking(move || {
                let mut map = std::collections::HashMap::new();
                for gid in &unresolved_guild_ids {
                    if let Ok(name) = fetch_discord_guild_name(&token, gid) {
                        map.insert(gid.clone(), name);
                    }
                }
                map
            })
            .await
            .unwrap_or_default();
            for entry in &mut entries {
                if let Some(name) = guild_name_map.get(&entry.guild_id) {
                    entry.guild_name = name.clone();
                }
            }
        }
    }
    for entry in &mut entries {
        if entry.guild_name == entry.guild_id {
            if let Some(name) = guild_name_fallback_map.get(&entry.guild_id) {
                entry.guild_name = name.clone();
            }
        }
    }

    // Resolve default agent per guild from account config + bindings (remote)
    {
        // Build account_id -> default agent_id from bindings (account-level, no peer)
        let mut account_agent_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        if let Some(bindings) = cfg.get("bindings").and_then(Value::as_array) {
            for b in bindings {
                let m = match b.get("match") {
                    Some(m) => m,
                    None => continue,
                };
                if m.get("channel").and_then(Value::as_str) != Some("discord") {
                    continue;
                }
                let account_id = match m.get("accountId").and_then(Value::as_str) {
                    Some(s) => s,
                    None => continue,
                };
                if m.get("peer").and_then(|p| p.get("id")).is_some() {
                    continue;
                } // skip channel-specific
                if let Some(agent_id) = b.get("agentId").and_then(Value::as_str) {
                    account_agent_map
                        .entry(account_id.to_string())
                        .or_insert_with(|| agent_id.to_string());
                }
            }
        }
        // Build guild_id -> default agent from account->guild mapping
        let mut guild_default_agent: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        if let Some(accounts) = discord_cfg
            .and_then(|d| d.get("accounts"))
            .and_then(Value::as_object)
        {
            for (account_id, account_val) in accounts {
                let agent = account_agent_map
                    .get(account_id)
                    .cloned()
                    .unwrap_or_else(|| account_id.clone());
                if let Some(guilds) = account_val.get("guilds").and_then(Value::as_object) {
                    for guild_id in guilds.keys() {
                        guild_default_agent
                            .entry(guild_id.clone())
                            .or_insert(agent.clone());
                    }
                }
            }
        }
        for entry in &mut entries {
            if entry.default_agent_id.is_none() {
                if let Some(agent_id) = guild_default_agent.get(&entry.guild_id) {
                    entry.default_agent_id = Some(agent_id.clone());
                }
            }
        }
    }

    // Persist to remote cache
    if !entries.is_empty() {
        let json = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
        let _ = pool
            .sftp_write(&host_id, "~/.clawpal/discord-guild-channels.json", &json)
            .await;
    }

    Ok(entries)
}

#[tauri::command]
pub async fn remote_list_bindings(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<Value>, String> {
    let output = crate::cli_runner::run_openclaw_remote(
        &pool,
        &host_id,
        &["config", "get", "bindings", "--json"],
    )
    .await?;
    // "bindings" may not exist yet — treat non-zero exit with "not found" as empty
    if output.exit_code != 0 {
        let msg = format!("{} {}", output.stderr, output.stdout).to_lowercase();
        if msg.contains("not found") {
            return Ok(Vec::new());
        }
    }
    let json = crate::cli_runner::parse_json_output(&output)?;
    clawpal_core::discovery::parse_bindings(&json.to_string())
}

#[tauri::command]
pub async fn remote_list_channels_minimal(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<ChannelNode>, String> {
    let output = crate::cli_runner::run_openclaw_remote(
        &pool,
        &host_id,
        &["config", "get", "channels", "--json"],
    )
    .await?;
    // channels key might not exist yet
    if output.exit_code != 0 {
        let msg = format!("{} {}", output.stderr, output.stdout).to_lowercase();
        if msg.contains("not found") {
            return Ok(Vec::new());
        }
        return Err(format!(
            "openclaw config get channels failed: {}",
            output.stderr
        ));
    }
    let channels_val = crate::cli_runner::parse_json_output(&output).unwrap_or(Value::Null);
    // Wrap in top-level object with "channels" key so collect_channel_nodes works
    let cfg = serde_json::json!({ "channels": channels_val });
    Ok(collect_channel_nodes(&cfg))
}

#[tauri::command]
pub async fn remote_list_agents_overview(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Vec<AgentOverview>, String> {
    let output =
        run_openclaw_remote_with_autofix(&pool, &host_id, &["agents", "list", "--json"]).await?;
    if output.exit_code != 0 {
        let details = format!("{}\n{}", output.stderr.trim(), output.stdout.trim());
        return Err(format!(
            "openclaw agents list failed ({}): {}",
            output.exit_code,
            details.trim()
        ));
    }
    let json = crate::cli_runner::parse_json_output(&output)?;
    // Check which agents have sessions remotely (single command, batch check)
    // Lists agents whose sessions.json is larger than 2 bytes (not just "{}")
    let online_set = match pool.exec_login(
        &host_id,
        "for d in ~/.openclaw/agents/*/sessions/sessions.json; do [ -f \"$d\" ] && [ $(wc -c < \"$d\") -gt 2 ] && basename $(dirname $(dirname \"$d\")); done",
    ).await {
        Ok(result) => {
            result.stdout.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect::<std::collections::HashSet<String>>()
        }
        Err(_) => std::collections::HashSet::new(), // fallback: all offline
    };
    parse_agents_cli_output(&json, Some(&online_set))
}

#[tauri::command]
pub async fn list_channels() -> Result<Vec<ChannelNode>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let paths = resolve_paths();
        let cfg = read_openclaw_config(&paths)?;
        let mut nodes = collect_channel_nodes(&cfg);
        enrich_channel_display_names(&paths, &cfg, &mut nodes)?;
        Ok(nodes)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_channels_minimal(
    cache: tauri::State<'_, crate::cli_runner::CliCache>,
) -> Result<Vec<ChannelNode>, String> {
    let cache_key = local_cli_cache_key("channels-minimal");
    let ttl = Some(std::time::Duration::from_secs(30));
    if let Some(cached) = cache.get(&cache_key, ttl) {
        return serde_json::from_str(&cached).map_err(|e| e.to_string());
    }
    let cache = cache.inner().clone();
    let cache_key_cloned = cache_key.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let output = crate::cli_runner::run_openclaw(&["config", "get", "channels", "--json"])
            .map_err(|e| format!("Failed to run openclaw: {e}"))?;
        if output.exit_code != 0 {
            let msg = format!("{} {}", output.stderr, output.stdout).to_lowercase();
            if msg.contains("not found") {
                return Ok(Vec::new());
            }
            // Fallback: direct read
            let paths = resolve_paths();
            let cfg = read_openclaw_config(&paths)?;
            let result = collect_channel_nodes(&cfg);
            if let Ok(serialized) = serde_json::to_string(&result) {
                cache.set(cache_key_cloned, serialized);
            }
            return Ok(result);
        }
        let channels_val = crate::cli_runner::parse_json_output(&output).unwrap_or(Value::Null);
        let cfg = serde_json::json!({ "channels": channels_val });
        let result = collect_channel_nodes(&cfg);
        if let Ok(serialized) = serde_json::to_string(&result) {
            cache.set(cache_key_cloned, serialized);
        }
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn list_discord_guild_channels() -> Result<Vec<DiscordGuildChannel>, String> {
    let paths = resolve_paths();
    let cache_file = paths.clawpal_dir.join("discord-guild-channels.json");
    if cache_file.exists() {
        let text = fs::read_to_string(&cache_file).map_err(|e| e.to_string())?;
        let entries: Vec<DiscordGuildChannel> = serde_json::from_str(&text).unwrap_or_default();
        return Ok(entries);
    }
    Ok(Vec::new())
}

#[tauri::command]
pub async fn refresh_discord_guild_channels() -> Result<Vec<DiscordGuildChannel>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let paths = resolve_paths();
        ensure_dirs(&paths)?;
        let cfg = read_openclaw_config(&paths)?;

        let discord_cfg = cfg.get("channels").and_then(|c| c.get("discord"));
        let configured_single_guild_id = discord_cfg
            .and_then(|d| d.get("guilds"))
            .and_then(Value::as_object)
            .and_then(|guilds| {
                if guilds.len() == 1 {
                    guilds.keys().next().cloned()
                } else {
                    None
                }
            });

        // Extract bot token: top-level first, then fall back to first account token
        let bot_token = discord_cfg
            .and_then(|d| d.get("botToken").or_else(|| d.get("token")))
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .or_else(|| {
                discord_cfg
                    .and_then(|d| d.get("accounts"))
                    .and_then(Value::as_object)
                    .and_then(|accounts| {
                        accounts.values().find_map(|acct| {
                            acct.get("token")
                                .and_then(Value::as_str)
                                .filter(|s| !s.is_empty())
                                .map(|s| s.to_string())
                        })
                    })
            });
        let cache_file = paths.clawpal_dir.join("discord-guild-channels.json");
        let mut guild_name_fallback_map = fs::read_to_string(&cache_file)
            .ok()
            .map(|text| parse_discord_cache_guild_name_fallbacks(&text))
            .unwrap_or_default();
        guild_name_fallback_map.extend(collect_discord_config_guild_name_fallbacks(discord_cfg));

        let mut entries: Vec<DiscordGuildChannel> = Vec::new();
        let mut channel_ids: Vec<String> = Vec::new();
        let mut unresolved_guild_ids: Vec<String> = Vec::new();

        // Helper: collect guilds from a guilds object
        let mut collect_guilds = |guilds: &serde_json::Map<String, Value>| {
            for (guild_id, guild_val) in guilds {
                let guild_name = guild_val
                    .get("slug")
                    .or_else(|| guild_val.get("name"))
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| guild_id.clone());

                if guild_name == *guild_id && !unresolved_guild_ids.contains(guild_id) {
                    unresolved_guild_ids.push(guild_id.clone());
                }

                if let Some(channels) = guild_val.get("channels").and_then(Value::as_object) {
                    for (channel_id, _channel_val) in channels {
                        // Skip glob/wildcard patterns (e.g. "*") — not real channel IDs
                        if channel_id.contains('*') || channel_id.contains('?') {
                            continue;
                        }
                        if entries
                            .iter()
                            .any(|e| e.guild_id == *guild_id && e.channel_id == *channel_id)
                        {
                            continue;
                        }
                        channel_ids.push(channel_id.clone());
                        entries.push(DiscordGuildChannel {
                            guild_id: guild_id.clone(),
                            guild_name: guild_name.clone(),
                            channel_id: channel_id.clone(),
                            channel_name: channel_id.clone(),
                            default_agent_id: None,
                        });
                    }
                }
            }
        };

        // Collect from channels.discord.guilds (top-level structured config)
        if let Some(guilds) = discord_cfg
            .and_then(|d| d.get("guilds"))
            .and_then(Value::as_object)
        {
            collect_guilds(guilds);
        }

        // Collect from channels.discord.accounts.<accountId>.guilds (multi-account config)
        if let Some(accounts) = discord_cfg
            .and_then(|d| d.get("accounts"))
            .and_then(Value::as_object)
        {
            for (_account_id, account_val) in accounts {
                if let Some(guilds) = account_val.get("guilds").and_then(Value::as_object) {
                    collect_guilds(guilds);
                }
            }
        }

        drop(collect_guilds); // Release mutable borrows before bindings section

        // Also collect from bindings array (users may only have bindings, no guilds map)
        if let Some(bindings) = cfg.get("bindings").and_then(Value::as_array) {
            for b in bindings {
                let m = match b.get("match") {
                    Some(m) => m,
                    None => continue,
                };
                if m.get("channel").and_then(Value::as_str) != Some("discord") {
                    continue;
                }
                let guild_id = match m.get("guildId") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Number(n)) => n.to_string(),
                    _ => continue,
                };
                let channel_id = match m.pointer("/peer/id") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Number(n)) => n.to_string(),
                    _ => continue,
                };
                // Skip if already collected from guilds map
                if entries
                    .iter()
                    .any(|e| e.guild_id == guild_id && e.channel_id == channel_id)
                {
                    continue;
                }
                if !unresolved_guild_ids.contains(&guild_id) {
                    unresolved_guild_ids.push(guild_id.clone());
                }
                channel_ids.push(channel_id.clone());
                entries.push(DiscordGuildChannel {
                    guild_id: guild_id.clone(),
                    guild_name: guild_id.clone(),
                    channel_id: channel_id.clone(),
                    channel_name: channel_id.clone(),
                    default_agent_id: None,
                });
            }
        }

        // Fallback A: fetch channels from Discord REST for guilds that have no entries yet.
        // Build a guild_id -> token mapping so each guild uses the correct bot token.
        {
            let mut guild_token_map: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();

            // Map guilds from accounts to their respective tokens
            if let Some(accounts) = discord_cfg
                .and_then(|d| d.get("accounts"))
                .and_then(Value::as_object)
            {
                for (_acct_id, acct_val) in accounts {
                    let acct_token = acct_val
                        .get("token")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string());
                    if let Some(token) = acct_token {
                        if let Some(guilds) = acct_val.get("guilds").and_then(Value::as_object) {
                            for guild_id in guilds.keys() {
                                guild_token_map
                                    .entry(guild_id.clone())
                                    .or_insert_with(|| token.clone());
                            }
                        }
                    }
                }
            }

            // Also map top-level guilds to the top-level bot token
            if let Some(token) = &bot_token {
                let configured_guild_ids = collect_discord_config_guild_ids(discord_cfg);
                for guild_id in &configured_guild_ids {
                    guild_token_map
                        .entry(guild_id.clone())
                        .or_insert_with(|| token.clone());
                }
            }

            for (guild_id, token) in &guild_token_map {
                // Skip guilds that already have entries from config/bindings
                if entries.iter().any(|e| e.guild_id == *guild_id) {
                    continue;
                }
                if let Ok(channels) = fetch_discord_guild_channels(token, guild_id) {
                    for (channel_id, channel_name) in channels {
                        if entries
                            .iter()
                            .any(|e| e.guild_id == *guild_id && e.channel_id == channel_id)
                        {
                            continue;
                        }
                        channel_ids.push(channel_id.clone());
                        entries.push(DiscordGuildChannel {
                            guild_id: guild_id.clone(),
                            guild_name: guild_id.clone(),
                            channel_id,
                            channel_name,
                            default_agent_id: None,
                        });
                    }
                }
            }
        }

        // Fallback B: query channel ids from directory and keep compatibility
        // with existing cache shape when config has no explicit channel map.
        if channel_ids.is_empty() {
            if let Ok(output) = run_openclaw_raw(&[
                "directory",
                "groups",
                "list",
                "--channel",
                "discord",
                "--json",
            ]) {
                for channel_id in parse_directory_group_channel_ids(&output.stdout) {
                    if entries.iter().any(|e| e.channel_id == channel_id) {
                        continue;
                    }
                    let (guild_id, guild_name) =
                        if let Some(gid) = configured_single_guild_id.clone() {
                            (gid.clone(), gid)
                        } else {
                            ("discord".to_string(), "Discord".to_string())
                        };
                    channel_ids.push(channel_id.clone());
                    entries.push(DiscordGuildChannel {
                        guild_id,
                        guild_name,
                        channel_id: channel_id.clone(),
                        channel_name: channel_id,
                        default_agent_id: None,
                    });
                }
            }
        }

        if entries.is_empty() {
            return Ok(Vec::new());
        }

        // Resolve channel names via openclaw CLI
        if !channel_ids.is_empty() {
            let mut args = vec![
                "channels",
                "resolve",
                "--json",
                "--channel",
                "discord",
                "--kind",
                "auto",
            ];
            let id_refs: Vec<&str> = channel_ids.iter().map(String::as_str).collect();
            args.extend_from_slice(&id_refs);

            if let Ok(output) = run_openclaw_raw(&args) {
                if let Some(name_map) = parse_resolve_name_map(&output.stdout) {
                    for entry in &mut entries {
                        if let Some(name) = name_map.get(&entry.channel_id) {
                            entry.channel_name = name.clone();
                        }
                    }
                }
            }
        }

        // Resolve guild names via Discord REST API
        if let Some(token) = &bot_token {
            if !unresolved_guild_ids.is_empty() {
                let mut guild_name_map: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                for gid in &unresolved_guild_ids {
                    if let Ok(name) = fetch_discord_guild_name(token, gid) {
                        guild_name_map.insert(gid.clone(), name);
                    }
                }
                for entry in &mut entries {
                    if let Some(name) = guild_name_map.get(&entry.guild_id) {
                        entry.guild_name = name.clone();
                    }
                }
            }
        }
        for entry in &mut entries {
            if entry.guild_name == entry.guild_id {
                if let Some(name) = guild_name_fallback_map.get(&entry.guild_id) {
                    entry.guild_name = name.clone();
                }
            }
        }

        // Resolve default agent per guild from account config + bindings
        {
            // Build account_id -> default agent_id from bindings (account-level, no peer)
            let mut account_agent_map: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            if let Some(bindings) = cfg.get("bindings").and_then(Value::as_array) {
                for b in bindings {
                    let m = match b.get("match") {
                        Some(m) => m,
                        None => continue,
                    };
                    if m.get("channel").and_then(Value::as_str) != Some("discord") {
                        continue;
                    }
                    let account_id = match m.get("accountId").and_then(Value::as_str) {
                        Some(s) => s,
                        None => continue,
                    };
                    if m.get("peer").and_then(|p| p.get("id")).is_some() {
                        continue;
                    }
                    if let Some(agent_id) = b.get("agentId").and_then(Value::as_str) {
                        account_agent_map
                            .entry(account_id.to_string())
                            .or_insert_with(|| agent_id.to_string());
                    }
                }
            }
            let mut guild_default_agent: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            if let Some(accounts) = discord_cfg
                .and_then(|d| d.get("accounts"))
                .and_then(Value::as_object)
            {
                for (account_id, account_val) in accounts {
                    let agent = account_agent_map
                        .get(account_id)
                        .cloned()
                        .unwrap_or_else(|| account_id.clone());
                    if let Some(guilds) = account_val.get("guilds").and_then(Value::as_object) {
                        for guild_id in guilds.keys() {
                            guild_default_agent
                                .entry(guild_id.clone())
                                .or_insert(agent.clone());
                        }
                    }
                }
            }
            for entry in &mut entries {
                if entry.default_agent_id.is_none() {
                    if let Some(agent_id) = guild_default_agent.get(&entry.guild_id) {
                        entry.default_agent_id = Some(agent_id.clone());
                    }
                }
            }
        }

        // Persist to cache
        let json = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
        write_text(&cache_file, &json)?;

        Ok(entries)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_bindings(
    cache: tauri::State<'_, crate::cli_runner::CliCache>,
) -> Result<Vec<Value>, String> {
    let cache_key = local_cli_cache_key("bindings");
    if let Some(cached) = cache.get(&cache_key, None) {
        return serde_json::from_str(&cached).map_err(|e| e.to_string());
    }
    let cache = cache.inner().clone();
    let cache_key_cloned = cache_key.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let output = crate::cli_runner::run_openclaw(&["config", "get", "bindings", "--json"])?;
        // "bindings" may not exist yet — treat "not found" as empty
        if output.exit_code != 0 {
            let msg = format!("{} {}", output.stderr, output.stdout).to_lowercase();
            if msg.contains("not found") {
                return Ok(Vec::new());
            }
        }
        let json = crate::cli_runner::parse_json_output(&output)?;
        let result = json.as_array().cloned().unwrap_or_default();
        if let Ok(serialized) = serde_json::to_string(&result) {
            cache.set(cache_key_cloned, serialized);
        }
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_agents_overview(
    cache: tauri::State<'_, crate::cli_runner::CliCache>,
) -> Result<Vec<AgentOverview>, String> {
    let cache_key = local_cli_cache_key("agents-list");
    if let Some(cached) = cache.get(&cache_key, None) {
        return serde_json::from_str(&cached).map_err(|e| e.to_string());
    }
    let cache = cache.inner().clone();
    let cache_key_cloned = cache_key.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let output = crate::cli_runner::run_openclaw(&["agents", "list", "--json"])?;
        let json = crate::cli_runner::parse_json_output(&output)?;
        let result = parse_agents_cli_output(&json, None)?;
        if let Ok(serialized) = serde_json::to_string(&result) {
            cache.set(cache_key_cloned, serialized);
        }
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}
