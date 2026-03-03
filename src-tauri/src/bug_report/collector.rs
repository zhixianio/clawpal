use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use uuid::Uuid;

use super::reporter::{send_report, BugReportEvent};
use super::sanitize::{sanitize_optional_text, sanitize_text};
use super::settings::{load_bug_report_settings, BugReportSeverity};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BugReportStats {
    pub session_id: String,
    pub total_sent: u64,
    pub sent_last_hour: u64,
    pub dropped_rate_limited: u64,
    pub send_failures: u64,
    pub last_sent_at: Option<String>,
}

#[derive(Debug)]
struct CollectorState {
    session_id: String,
    sent_timestamps: VecDeque<i64>,
    total_sent: u64,
    dropped_rate_limited: u64,
    send_failures: u64,
    last_sent_at: Option<String>,
}

impl CollectorState {
    fn new() -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            sent_timestamps: VecDeque::new(),
            total_sent: 0,
            dropped_rate_limited: 0,
            send_failures: 0,
            last_sent_at: None,
        }
    }

    fn prune(&mut self, now_epoch_secs: i64) {
        while let Some(oldest) = self.sent_timestamps.front() {
            if now_epoch_secs - *oldest >= 3_600 {
                self.sent_timestamps.pop_front();
            } else {
                break;
            }
        }
    }
}

fn state() -> &'static Mutex<CollectorState> {
    static STATE: OnceLock<Mutex<CollectorState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(CollectorState::new()))
}

fn os_version_string() -> String {
    std::env::var("OSTYPE")
        .or_else(|_| std::env::var("OS"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn build_event(level: BugReportSeverity, message: &str, stack_trace: Option<&str>) -> BugReportEvent {
    let session_id = state()
        .lock()
        .map(|guard| guard.session_id.clone())
        .unwrap_or_else(|_| "unknown-session".to_string());
    let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    BugReportEvent {
        message: sanitize_text(message),
        stack_trace: sanitize_optional_text(stack_trace),
        level: level.as_str().to_string(),
        timestamp,
        session_id,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os_type: std::env::consts::OS.to_string(),
        os_version: os_version_string(),
    }
}

fn send_event(
    level: BugReportSeverity,
    message: &str,
    stack_trace: Option<&str>,
    ignore_rate_limit: bool,
    ignore_enabled: bool,
    ignore_threshold: bool,
) -> Result<(), String> {
    let settings = load_bug_report_settings();
    if !ignore_enabled && !settings.enabled {
        return Ok(());
    }
    if !ignore_threshold && !level.meets_threshold(&settings.severity_threshold) {
        return Ok(());
    }

    let now = Utc::now();
    let now_epoch_secs = now.timestamp();
    {
        let mut guard = state().lock().map_err(|e| e.to_string())?;
        guard.prune(now_epoch_secs);
        let sent_last_hour = guard.sent_timestamps.len() as u32;
        if !ignore_rate_limit && sent_last_hour >= settings.max_reports_per_hour {
            guard.dropped_rate_limited += 1;
            return Ok(());
        }
    }

    let event = build_event(level, message, stack_trace);
    let sent_at = event.timestamp.clone();
    match send_report(&settings, &event) {
        Ok(()) => {
            let mut guard = state().lock().map_err(|e| e.to_string())?;
            guard.sent_timestamps.push_back(now_epoch_secs);
            guard.total_sent += 1;
            guard.last_sent_at = Some(sent_at);
            Ok(())
        }
        Err(err) => {
            if let Ok(mut guard) = state().lock() {
                guard.send_failures += 1;
            }
            Err(err)
        }
    }
}

pub fn capture(level: BugReportSeverity, message: &str, stack_trace: Option<&str>) {
    if let Err(err) = send_event(level, message, stack_trace, false, false, false) {
        eprintln!("[bug-report] send failed: {err}");
    }
}

pub fn capture_error(message: &str) {
    capture(BugReportSeverity::Error, message, None);
}

pub fn send_test_report() -> Result<(), String> {
    send_event(
        BugReportSeverity::Error,
        "Bug report connection test from ClawPal",
        Some("test_stack: simulated"),
        true,
        true,
        true,
    )
}

pub fn get_stats() -> BugReportStats {
    let now_epoch_secs = Utc::now().timestamp();
    let mut guard = match state().lock() {
        Ok(value) => value,
        Err(_) => {
            return BugReportStats {
                session_id: "unknown-session".to_string(),
                total_sent: 0,
                sent_last_hour: 0,
                dropped_rate_limited: 0,
                send_failures: 0,
                last_sent_at: None,
            };
        }
    };
    guard.prune(now_epoch_secs);
    BugReportStats {
        session_id: guard.session_id.clone(),
        total_sent: guard.total_sent,
        sent_last_hour: guard.sent_timestamps.len() as u64,
        dropped_rate_limited: guard.dropped_rate_limited,
        send_failures: guard.send_failures,
        last_sent_at: guard.last_sent_at.clone(),
    }
}
