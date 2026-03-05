use super::*;
use clawpal_core::ssh::diagnostic::{from_any_error, SshIntent, SshStage};

#[tauri::command]
pub async fn remote_run_doctor(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<Value, String> {
    let result = pool
        .exec_login(
            &host_id,
            "openclaw doctor --json 2>/dev/null || openclaw doctor 2>&1",
        )
        .await?;
    // Try to parse as JSON first
    if let Ok(json) = serde_json::from_str::<Value>(&result.stdout) {
        return Ok(json);
    }
    // Fallback: return raw output as a simple report
    Ok(serde_json::json!({
        "ok": result.exit_code == 0,
        "score": if result.exit_code == 0 { 100 } else { 0 },
        "issues": [],
        "rawOutput": result.stdout,
    }))
}

#[tauri::command]
pub async fn remote_fix_issues(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    ids: Vec<String>,
) -> Result<FixResult, String> {
    let (config_path, raw, _cfg) =
        remote_read_openclaw_config_text_and_json(&pool, &host_id).await?;
    let mut cfg = clawpal_core::doctor::parse_json5_document_or_default(&raw);
    let applied = clawpal_core::doctor::apply_issue_fixes(&mut cfg, &ids)?;

    if !applied.is_empty() {
        remote_write_config_with_snapshot(&pool, &host_id, &config_path, &raw, &cfg, "doctor-fix")
            .await?;
    }

    let remaining: Vec<String> = ids.into_iter().filter(|id| !applied.contains(id)).collect();
    Ok(FixResult {
        ok: true,
        applied,
        remaining_issues: remaining,
    })
}

#[tauri::command]
pub async fn remote_get_system_status(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<StatusLight, String> {
    // Tier 1: fast, essential — health check + agents config (2 SSH calls in parallel)
    let (config_res, pgrep_res) = tokio::join!(
        run_openclaw_remote_with_autofix(&pool, &host_id, &["config", "get", "agents", "--json"]),
        pool.exec(&host_id, "pgrep -f '[o]penclaw-gateway' >/dev/null 2>&1"),
    );

    let config_ok = matches!(&config_res, Ok(output) if output.exit_code == 0);
    let ssh_diagnostic = match (&config_res, &pgrep_res) {
        (Err(error), _) => Some(from_any_error(
            SshStage::RemoteExec,
            SshIntent::HealthCheck,
            error.clone(),
        )),
        (_, Err(error)) => Some(from_any_error(
            SshStage::RemoteExec,
            SshIntent::HealthCheck,
            error.clone(),
        )),
        _ => None,
    };

    let (active_agents, global_default_model, fallback_models) = match config_res {
        Ok(ref output) if output.exit_code == 0 => {
            let cfg: Value = crate::cli_runner::parse_json_output(output).unwrap_or(Value::Null);
            let explicit = cfg
                .pointer("/list")
                .and_then(Value::as_array)
                .map(|a| a.len() as u32)
                .unwrap_or(0);
            let agents = if explicit == 0 { 1 } else { explicit };
            let model = cfg
                .pointer("/defaults/model")
                .and_then(|v| read_model_value(v))
                .or_else(|| {
                    cfg.pointer("/default/model")
                        .and_then(|v| read_model_value(v))
                });
            let fallbacks = cfg
                .pointer("/defaults/model/fallbacks")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            (agents, model, fallbacks)
        }
        _ => (0, None, Vec::new()),
    };

    // Avoid false negatives from transient SSH exec failures:
    // if health probe fails but config fetch in the same cycle succeeded,
    // keep health as true instead of flipping to unhealthy.
    let healthy = match pgrep_res {
        Ok(r) => r.exit_code == 0,
        Err(_) if config_ok => true,
        Err(_) => false,
    };

    Ok(StatusLight {
        healthy,
        active_agents,
        global_default_model,
        fallback_models,
        ssh_diagnostic,
    })
}

#[tauri::command]
pub async fn remote_get_status_extra(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
) -> Result<StatusExtra, String> {
    let detect_duplicates_script = concat!(
        "seen=''; for p in $(which -a openclaw 2>/dev/null) ",
        "\"$HOME/.npm-global/bin/openclaw\" \"/usr/local/bin/openclaw\" \"/opt/homebrew/bin/openclaw\"; do ",
        "[ -x \"$p\" ] || continue; ",
        "rp=$(readlink -f \"$p\" 2>/dev/null || echo \"$p\"); ",
        "echo \"$seen\" | grep -qF \"$rp\" && continue; ",
        "seen=\"$seen $rp\"; ",
        "v=$($p --version 2>/dev/null || echo 'unknown'); ",
        "echo \"$p: $v\"; ",
        "done"
    );

    let (version_res, dup_res) = tokio::join!(
        pool.exec_login(&host_id, "openclaw --version"),
        pool.exec_login(&host_id, detect_duplicates_script),
    );

    let openclaw_version = match version_res {
        Ok(r) if r.exit_code == 0 => Some(r.stdout.trim().to_string()),
        Ok(r) => {
            let trimmed = r.stdout.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        Err(_) => None,
    };

    let duplicate_installs = match dup_res {
        Ok(r) => {
            let entries: Vec<String> = r
                .stdout
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            if entries.len() > 1 {
                entries
            } else {
                Vec::new()
            }
        }
        Err(_) => Vec::new(),
    };

    Ok(StatusExtra {
        openclaw_version,
        duplicate_installs,
    })
}

#[tauri::command]
pub async fn get_status_light() -> Result<StatusLight, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let paths = resolve_paths();
        let cfg = read_openclaw_config(&paths)?;
        let local_health = clawpal_core::health::check_instance(&local_health_instance())
            .map_err(|e| e.to_string())?;
        let explicit_count = cfg
            .get("agents")
            .and_then(|a| a.get("list"))
            .and_then(|a| a.as_array())
            .map(|a| a.len() as u32)
            .unwrap_or(0);
        // At least 1 agent (implicit "main") when agents section exists
        let active_agents = if explicit_count == 0 && cfg.get("agents").is_some() {
            1
        } else {
            explicit_count
        };
        let global_default_model = cfg
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

        Ok(StatusLight {
            healthy: local_health.healthy,
            active_agents: if local_health.active_agents == 0 {
                active_agents
            } else {
                local_health.active_agents
            },
            global_default_model,
            fallback_models,
            ssh_diagnostic: None,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_status_extra() -> Result<StatusExtra, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let openclaw_version = {
            let mut cache = OPENCLAW_VERSION_CACHE.lock().unwrap();
            if cache.is_none() {
                let version = clawpal_core::health::check_instance(&local_health_instance())
                    .ok()
                    .and_then(|status| status.version);
                *cache = Some(version);
            }
            cache.as_ref().unwrap().clone()
        };
        Ok(StatusExtra {
            openclaw_version,
            duplicate_installs: Vec::new(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn get_system_status() -> Result<SystemStatus, String> {
    let paths = resolve_paths();
    ensure_dirs(&paths)?;
    let cfg = read_openclaw_config(&paths)?;
    let active_agents = cfg
        .get("agents")
        .and_then(|a| a.get("list"))
        .and_then(|a| a.as_array())
        .map(|a| a.len() as u32)
        .unwrap_or(0);
    let snapshots = list_snapshots(&paths.metadata_path)
        .unwrap_or_default()
        .items
        .len();
    let model_summary = collect_model_summary(&cfg);
    let channel_summary = collect_channel_summary(&cfg);
    let memory = collect_memory_overview(&paths.base_dir);
    let sessions = collect_session_overview(&paths.base_dir);
    let openclaw_version = resolve_openclaw_version();
    let openclaw_update =
        check_openclaw_update_cached(&paths, false).unwrap_or_else(|_| OpenclawUpdateCheck {
            installed_version: openclaw_version.clone(),
            latest_version: None,
            upgrade_available: false,
            channel: None,
            details: Some("update status unavailable".into()),
            source: "unknown".into(),
            checked_at: format_timestamp_from_unix(unix_timestamp_secs()),
        });
    Ok(SystemStatus {
        healthy: true,
        config_path: paths.config_path.to_string_lossy().to_string(),
        openclaw_dir: paths.openclaw_dir.to_string_lossy().to_string(),
        clawpal_dir: paths.clawpal_dir.to_string_lossy().to_string(),
        openclaw_version,
        active_agents,
        snapshots,
        channels: channel_summary,
        models: model_summary,
        memory,
        sessions,
        openclaw_update,
    })
}

#[tauri::command]
pub fn run_doctor_command() -> Result<DoctorReport, String> {
    let paths = resolve_paths();
    Ok(run_doctor(&paths))
}

#[tauri::command]
pub fn fix_issues(ids: Vec<String>) -> Result<FixResult, String> {
    let paths = resolve_paths();
    let issues = run_doctor(&paths);
    let mut fixable = Vec::new();
    for issue in issues.issues {
        if ids.contains(&issue.id) && issue.auto_fixable {
            fixable.push(issue.id);
        }
    }
    let auto_applied = apply_auto_fixes(&paths, &fixable);
    let mut remaining = Vec::new();
    let mut applied = Vec::new();
    for id in ids {
        if fixable.contains(&id) && auto_applied.iter().any(|x| x == &id) {
            applied.push(id);
        } else {
            remaining.push(id);
        }
    }
    Ok(FixResult {
        ok: true,
        applied,
        remaining_issues: remaining,
    })
}
