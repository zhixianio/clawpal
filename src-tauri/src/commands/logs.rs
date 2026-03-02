use super::*;

fn clamp_lines(lines: Option<usize>) -> usize {
    lines.unwrap_or(200).clamp(1, 400)
}

fn log_dev_enabled() -> bool {
    if cfg!(debug_assertions) {
        return true;
    }
    match std::env::var("CLAWPAL_DEV_LOG") {
        Ok(value) => {
            let value = value.to_ascii_lowercase();
            value == "1" || value == "true" || value == "yes" || value == "on"
        }
        Err(_) => false,
    }
}

pub fn log_dev(message: impl AsRef<str>) {
    if log_dev_enabled() {
        eprintln!("[dev] {}", message.as_ref());
    }
}

fn log_debug(message: &str) {
    log_dev(format!("[dev][logs] {message}"));
}

#[tauri::command]
pub async fn remote_read_app_log(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    lines: Option<usize>,
) -> Result<String, String> {
    let n = clamp_lines(lines);
    let cmd = format!("tail -n {n} ~/.clawpal/logs/app.log 2>/dev/null || echo ''");
    log_debug(&format!("remote_read_app_log start host_id={host_id} lines={n} cmd={cmd}"));
    let result = pool.exec(&host_id, &cmd).await.map_err(|error| {
        log_debug(&format!(
            "remote_read_app_log failed host_id={host_id} error={error}"
        ));
        error
    })?;
    Ok(result.stdout)
}

#[tauri::command]
pub async fn remote_read_error_log(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    lines: Option<usize>,
) -> Result<String, String> {
    let n = clamp_lines(lines);
    let cmd = format!("tail -n {n} ~/.clawpal/logs/error.log 2>/dev/null || echo ''");
    log_debug(&format!("remote_read_error_log start host_id={host_id} lines={n} cmd={cmd}"));
    let result = pool.exec(&host_id, &cmd).await.map_err(|error| {
        log_debug(&format!(
            "remote_read_error_log failed host_id={host_id} error={error}"
        ));
        error
    })?;
    Ok(result.stdout)
}

#[tauri::command]
pub async fn remote_read_gateway_log(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    lines: Option<usize>,
) -> Result<String, String> {
    let n = clamp_lines(lines);
    let cmd = format!("tail -n {n} ~/.openclaw/logs/gateway.log 2>/dev/null || echo ''");
    log_debug(&format!("remote_read_gateway_log start host_id={host_id} lines={n} cmd={cmd}"));
    let result = pool.exec(&host_id, &cmd).await.map_err(|error| {
        log_debug(&format!(
            "remote_read_gateway_log failed host_id={host_id} error={error}"
        ));
        error
    })?;
    Ok(result.stdout)
}

#[tauri::command]
pub async fn remote_read_gateway_error_log(
    pool: State<'_, SshConnectionPool>,
    host_id: String,
    lines: Option<usize>,
) -> Result<String, String> {
    let n = clamp_lines(lines);
    let cmd = format!("tail -n {n} ~/.openclaw/logs/gateway.err.log 2>/dev/null || echo ''");
    log_debug(&format!(
        "remote_read_gateway_error_log start host_id={host_id} lines={n} cmd={cmd}"
    ));
    let result = pool.exec(&host_id, &cmd).await.map_err(|error| {
        log_debug(&format!(
            "remote_read_gateway_error_log failed host_id={host_id} error={error}"
        ));
        error
    })?;
    Ok(result.stdout)
}
