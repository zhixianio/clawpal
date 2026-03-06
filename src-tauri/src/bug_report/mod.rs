pub mod collector;
pub mod os_info;
pub mod queue;
pub mod reporter;
pub mod sanitize;
pub mod settings;

use std::sync::Once;

use collector::BugReportStats;
use settings::BugReportSeverity;

#[tauri::command]
pub fn get_bug_report_stats() -> Result<BugReportStats, String> {
    Ok(collector::get_stats())
}

#[tauri::command]
pub fn test_bug_report_connection() -> Result<bool, String> {
    collector::send_test_report().map(|_| true)
}

#[tauri::command]
pub fn capture_frontend_error(
    message: String,
    stack: Option<String>,
    level: Option<String>,
) -> Result<(), String> {
    use settings::BugReportSeverity;
    let severity = match level.as_deref() {
        Some("critical") => BugReportSeverity::Critical,
        Some("warn") => BugReportSeverity::Warn,
        Some("info") => BugReportSeverity::Info,
        _ => BugReportSeverity::Error,
    };
    collector::capture(severity, &message, stack.as_deref());
    Ok(())
}

pub fn install_panic_hook() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let payload = if let Some(msg) = panic_info.payload().downcast_ref::<&str>() {
                (*msg).to_string()
            } else if let Some(msg) = panic_info.payload().downcast_ref::<String>() {
                msg.clone()
            } else {
                "panic".to_string()
            };
            let location = panic_info
                .location()
                .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
                .unwrap_or_else(|| "unknown".to_string());
            let message = format!("panic at {location}: {payload}");

            collector::capture(BugReportSeverity::Critical, &message, None);
            previous(panic_info);
        }));
    });
}
