use std::collections::VecDeque;
use std::sync::{
    mpsc::{self, Sender},
    Mutex, OnceLock,
};

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use uuid::Uuid;

use super::reporter::{send_report, BugReportEvent};
use super::sanitize::{sanitize_optional_text, sanitize_text};
use super::settings::{BugReportSettings, BugReportSeverity};
use super::{os_info, queue};
use crate::commands::preferences::load_bug_report_settings_from_paths;
use crate::models::resolve_paths;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BugReportStats {
    pub session_id: String,
    pub total_sent: u64,
    pub sent_last_hour: u64,
    pub dropped_rate_limited: u64,
    pub send_failures: u64,
    pub last_sent_at: Option<String>,
    pub persisted_pending: u64,
    pub dead_letter_count: u64,
}

#[derive(Debug)]
struct CollectorState {
    session_id: String,
    sent_timestamps: VecDeque<i64>,
    pending_reports: u32,
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
            pending_reports: 0,
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

#[derive(Debug)]
struct QueuedBugReport {
    settings: BugReportSettings,
    event: BugReportEvent,
    now_epoch_secs: i64,
    sent_at: String,
    queue_entry_id: Option<String>,
}

fn sender() -> &'static Sender<QueuedBugReport> {
    static SENDER: OnceLock<Sender<QueuedBugReport>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<QueuedBugReport>();
        std::thread::spawn(move || {
            for queued in rx {
                let result = send_report(&queued.settings, &queued.event);
                if let Ok(mut guard) = state().lock() {
                    guard.pending_reports = guard.pending_reports.saturating_sub(1);
                    match result {
                        Ok(()) => {
                            guard.sent_timestamps.push_back(queued.now_epoch_secs);
                            guard.total_sent += 1;
                            guard.last_sent_at = Some(queued.sent_at);
                            if let Some(id) = queued.queue_entry_id.as_deref() {
                                if let Err(err) = queue::mark_sent(id) {
                                    eprintln!("[bug-report] queue mark_sent failed: {err}");
                                }
                            }
                        }
                        Err(err) => {
                            guard.send_failures += 1;
                            eprintln!("[bug-report] send failed: {err}");
                            if let Err(log_err) = queue::log_send_failure(&err) {
                                eprintln!("[bug-report] failure log write failed: {log_err}");
                            }
                        }
                    }
                }
            }
        });
        tx
    })
}

fn build_event(
    level: BugReportSeverity,
    message: &str,
    stack_trace: Option<&str>,
) -> BugReportEvent {
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
        os_version: os_info::os_version_string(),
    }
}

fn record_send_failure() {
    if let Ok(mut guard) = state().lock() {
        guard.send_failures += 1;
    }
}

fn prepare_event(
    level: BugReportSeverity,
    message: &str,
    stack_trace: Option<&str>,
    queue_entry_id: Option<String>,
    ignore_rate_limit: bool,
    ignore_enabled: bool,
    ignore_threshold: bool,
) -> Result<Option<QueuedBugReport>, String> {
    let settings = load_bug_report_settings_from_paths(&resolve_paths());
    if !ignore_enabled && !settings.enabled {
        return Ok(None);
    }
    if !ignore_threshold && !level.meets_threshold(&settings.severity_threshold) {
        return Ok(None);
    }

    let now = Utc::now();
    let now_epoch_secs = now.timestamp();
    {
        let mut guard = state().lock().map_err(|e| e.to_string())?;
        guard.prune(now_epoch_secs);
        let sent_last_hour = guard.sent_timestamps.len() as u32 + guard.pending_reports;
        if !ignore_rate_limit && sent_last_hour >= settings.max_reports_per_hour {
            guard.dropped_rate_limited += 1;
            return Ok(None);
        }
        guard.pending_reports += 1;
    }

    let event = build_event(level, message, stack_trace);
    Ok(Some(QueuedBugReport {
        settings,
        sent_at: event.timestamp.clone(),
        event,
        now_epoch_secs,
        queue_entry_id,
    }))
}

fn enqueue_event(
    level: BugReportSeverity,
    message: &str,
    stack_trace: Option<&str>,
    queue_entry_id: Option<String>,
    ignore_rate_limit: bool,
    ignore_enabled: bool,
    ignore_threshold: bool,
) -> Result<(), String> {
    let prepared = prepare_event(
        level,
        message,
        stack_trace,
        queue_entry_id,
        ignore_rate_limit,
        ignore_enabled,
        ignore_threshold,
    )?;
    let Some(queued) = prepared else {
        return Ok(());
    };
    if let Err(err) = sender().send(queued) {
        if let Ok(mut guard) = state().lock() {
            guard.pending_reports = guard.pending_reports.saturating_sub(1);
        }
        return Err(err.to_string());
    }
    Ok(())
}

fn send_event_sync(
    level: BugReportSeverity,
    message: &str,
    stack_trace: Option<&str>,
    queue_entry_id: Option<String>,
    ignore_rate_limit: bool,
    ignore_enabled: bool,
    ignore_threshold: bool,
) -> Result<(), String> {
    let prepared = prepare_event(
        level,
        message,
        stack_trace,
        queue_entry_id.clone(),
        ignore_rate_limit,
        ignore_enabled,
        ignore_threshold,
    )?;
    let Some(queued) = prepared else {
        return Ok(());
    };
    match send_report(&queued.settings, &queued.event) {
        Ok(()) => {
            let mut guard = state().lock().map_err(|e| e.to_string())?;
            guard.pending_reports = guard.pending_reports.saturating_sub(1);
            guard.sent_timestamps.push_back(queued.now_epoch_secs);
            guard.total_sent += 1;
            guard.last_sent_at = Some(queued.sent_at);
            if let Some(id) = queue_entry_id.as_deref() {
                if let Err(err) = queue::mark_sent(id) {
                    eprintln!("[bug-report] queue mark_sent failed: {err}");
                }
            }
            Ok(())
        }
        Err(err) => {
            if let Ok(mut guard) = state().lock() {
                guard.pending_reports = guard.pending_reports.saturating_sub(1);
            }
            record_send_failure();
            if let Err(log_err) = queue::log_send_failure(&err) {
                eprintln!("[bug-report] failure log write failed: {log_err}");
            }
            Err(err)
        }
    }
}

pub fn capture(level: BugReportSeverity, message: &str, stack_trace: Option<&str>) {
    // Sanitise once here so both the disk copy (queue) and the network copy
    // (Sentry/backend) are produced from the same sanitised source.
    let sanitised_message = sanitize_text(message);
    let sanitised_stack = sanitize_optional_text(stack_trace);
    let persisted_id = match queue::enqueue(
        level.as_str(),
        &sanitised_message,
        sanitised_stack.as_deref(),
    ) {
        Ok(id) => Some(id),
        Err(err) => {
            eprintln!("[bug-report] queue enqueue failed: {err}");
            None
        }
    };
    if let Err(err) = enqueue_event(
        level,
        &sanitised_message,
        sanitised_stack.as_deref(),
        persisted_id,
        false,
        false,
        false,
    ) {
        eprintln!("[bug-report] send failed: {err}");
    }
}

pub fn capture_error(message: &str) {
    capture(BugReportSeverity::Error, message, None);
}

pub fn send_test_report() -> Result<(), String> {
    send_event_sync(
        BugReportSeverity::Error,
        "Bug report connection test from ClawPal",
        Some("test_stack: simulated"),
        None,
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
                persisted_pending: 0,
                dead_letter_count: 0,
            };
        }
    };
    guard.prune(now_epoch_secs);
    let queue_stats = queue::stats();
    BugReportStats {
        session_id: guard.session_id.clone(),
        total_sent: guard.total_sent,
        sent_last_hour: guard.sent_timestamps.len() as u64,
        dropped_rate_limited: guard.dropped_rate_limited,
        send_failures: guard.send_failures,
        last_sent_at: guard.last_sent_at.clone(),
        persisted_pending: queue_stats.persisted_pending,
        dead_letter_count: queue_stats.dead_letter_count,
    }
}
