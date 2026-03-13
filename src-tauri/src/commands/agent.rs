use super::*;

fn resolve_openclaw_default_workspace(cfg: &Value) -> Option<String> {
    cfg.pointer("/agents/defaults/workspace")
        .or_else(|| cfg.pointer("/agents/default/workspace"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            collect_agent_overviews_from_config(cfg)
                .into_iter()
                .find_map(|agent| agent.workspace.filter(|value| !value.trim().is_empty()))
        })
}

fn expand_local_workspace_path(workspace: &str) -> String {
    shellexpand::tilde(workspace).to_string()
}

#[tauri::command]
pub async fn remote_setup_agent_identity(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    agent_id: String,
    name: String,
    emoji: Option<String>,
) -> Result<bool, String> {
    let agent_id = agent_id.trim().to_string();
    let name = name.trim().to_string();
    if agent_id.is_empty() {
        return Err("Agent ID is required".into());
    }
    if name.is_empty() {
        return Err("Name is required".into());
    }
    crate::agent_identity::write_remote_agent_identity(
        pool.inner(),
        &host_id,
        &agent_id,
        Some(&name),
        emoji.as_deref(),
        None,
    )
    .await?;
    Ok(true)
}

#[tauri::command]
pub async fn remote_chat_via_openclaw(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    agent_id: String,
    message: String,
    session_id: Option<String>,
) -> Result<Value, String> {
    let escaped_msg = message.replace('\'', "'\\''");
    let escaped_agent = agent_id.replace('\'', "'\\''");
    let mut cmd = format!(
        "openclaw agent --local --agent '{}' --message '{}' --json --no-color",
        escaped_agent, escaped_msg
    );
    if let Some(sid) = session_id {
        let escaped_sid = sid.replace('\'', "'\\''");
        cmd.push_str(&format!(" --session-id '{}'", escaped_sid));
    }
    let result = pool.exec_login(&host_id, &cmd).await?;
    // Try to extract JSON from stdout first — even on non-zero exit the
    // command may have produced valid output (e.g. bash job-control warnings
    // in stderr cause exit 1 but the actual command succeeded).
    if let Some(json_str) = clawpal_core::doctor::extract_json_from_output(&result.stdout) {
        return serde_json::from_str(json_str)
            .map_err(|e| format!("Failed to parse remote chat response: {e}"));
    }
    if result.exit_code != 0 {
        return Err(format!(
            "Remote chat failed (exit {}): {}",
            result.exit_code, result.stderr
        ));
    }
    Err(format!(
        "No JSON in remote openclaw output: {}",
        result.stdout
    ))
}

#[tauri::command]
pub fn create_agent(
    agent_id: String,
    model_value: Option<String>,
    independent: Option<bool>,
) -> Result<AgentOverview, String> {
    let agent_id = agent_id.trim().to_string();
    if agent_id.is_empty() {
        return Err("Agent ID is required".into());
    }
    if !agent_id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Agent ID may only contain letters, numbers, hyphens, and underscores".into());
    }

    let paths = resolve_paths();
    let cfg = read_openclaw_config(&paths)?;

    let existing_ids = collect_agent_ids(&cfg);
    if existing_ids
        .iter()
        .any(|id| id.eq_ignore_ascii_case(&agent_id))
    {
        return Err(format!("Agent '{}' already exists", agent_id));
    }

    let model_display = model_value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let _ = independent;
    let workspace = resolve_openclaw_default_workspace(&cfg).ok_or_else(|| {
        "OpenClaw default workspace could not be resolved for non-interactive agent creation"
            .to_string()
    })?;
    let workspace = expand_local_workspace_path(&workspace);

    let mut args = vec![
        "agents".to_string(),
        "add".to_string(),
        agent_id.clone(),
        "--non-interactive".to_string(),
        "--workspace".to_string(),
        workspace,
    ];
    if let Some(model_value) = &model_display {
        args.push("--model".to_string());
        args.push(model_value.clone());
    }
    let arg_refs: Vec<&str> = args.iter().map(|value| value.as_str()).collect();
    run_openclaw_raw(&arg_refs)?;

    let updated = read_openclaw_config(&paths)?;
    collect_agent_overviews_from_config(&updated)
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .ok_or_else(|| "Created agent was not found after OpenClaw refresh".to_string())
}

#[tauri::command]
pub fn delete_agent(agent_id: String) -> Result<bool, String> {
    let agent_id = agent_id.trim().to_string();
    if agent_id.is_empty() {
        return Err("Agent ID is required".into());
    }
    if agent_id == "main" {
        return Err("Cannot delete the main agent".into());
    }

    let paths = resolve_paths();
    let mut cfg = read_openclaw_config(&paths)?;
    let current = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;

    let list = cfg
        .pointer_mut("/agents/list")
        .and_then(Value::as_array_mut)
        .ok_or("agents.list not found")?;

    let before = list.len();
    list.retain(|agent| agent.get("id").and_then(Value::as_str) != Some(&agent_id));

    if list.len() == before {
        return Err(format!("Agent '{}' not found", agent_id));
    }

    // Reset any bindings that reference this agent back to "main" (default)
    // so the channel doesn't lose its binding entry entirely.
    if let Some(bindings) = cfg.pointer_mut("/bindings").and_then(Value::as_array_mut) {
        for b in bindings.iter_mut() {
            if b.get("agentId").and_then(Value::as_str) == Some(&agent_id) {
                if let Some(obj) = b.as_object_mut() {
                    obj.insert("agentId".into(), Value::String("main".into()));
                }
            }
        }
    }

    write_config_with_snapshot(&paths, &current, &cfg, "delete-agent")?;
    Ok(true)
}

#[tauri::command]
pub fn setup_agent_identity(
    agent_id: String,
    name: String,
    emoji: Option<String>,
) -> Result<bool, String> {
    let agent_id = agent_id.trim().to_string();
    let name = name.trim().to_string();
    if agent_id.is_empty() {
        return Err("Agent ID is required".into());
    }
    if name.is_empty() {
        return Err("Name is required".into());
    }

    let paths = resolve_paths();
    crate::agent_identity::write_local_agent_identity(
        &paths,
        &agent_id,
        Some(&name),
        emoji.as_deref(),
        None,
    )?;
    Ok(true)
}

#[tauri::command]
pub async fn chat_via_openclaw(
    agent_id: String,
    message: String,
    session_id: Option<String>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let paths = resolve_paths();
        if let Err(err) = sync_main_auth_for_active_config(&paths) {
            eprintln!("Warning: pre-chat main auth sync failed: {err}");
        }
        let mut args = vec![
            "agent".to_string(),
            "--local".to_string(),
            "--agent".to_string(),
            agent_id,
            "--message".to_string(),
            message,
            "--json".to_string(),
            "--no-color".to_string(),
        ];
        if let Some(sid) = session_id {
            args.push("--session-id".to_string());
            args.push(sid);
        }

        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let output = run_openclaw_raw(&arg_refs)?;
        let json_str = clawpal_core::doctor::extract_json_from_output(&output.stdout)
            .ok_or_else(|| format!("No JSON in openclaw output: {}", output.stdout))?;
        serde_json::from_str(json_str).map_err(|e| format!("Parse openclaw response failed: {}", e))
    })
    .await
    .map_err(|e| format!("Task join failed: {}", e))?
}
