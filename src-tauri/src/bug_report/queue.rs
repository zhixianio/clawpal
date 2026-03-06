use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use uuid::Uuid;

use super::reporter::{send_report, BugReportEvent};
use super::settings::BugReportSettings;
use crate::models::resolve_paths;

const MAX_PENDING: usize = 100;
const MAX_DEAD_LETTER: usize = 20;
const RETRY_LIMIT: u32 = 3;
const RETENTION_DAYS: i64 = 30;

const PENDING_FILE: &str = "pending.jsonl";
const DEAD_LETTER_FILE: &str = "dead_letter.jsonl";
const SENT_FILE: &str = "sent.jsonl";
const FAILURES_FILE: &str = "failures.jsonl";

/// Process-level lock serialising all mutations to `pending.jsonl` and
/// `dead_letter.jsonl`. Prevents races between the async sender thread
/// (`mark_sent`) and startup `flush` / hot-path `enqueue`.
fn pending_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingEntry {
    pub id: String,
    pub ts: String,
    pub level: String,
    pub message: String,
    pub stack_trace: Option<String>,
    pub attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SentEntry {
    id: String,
    ts: String,
    level: String,
    message: String,
    stack_trace: Option<String>,
    attempts: u32,
    sent_at: String,
}

impl SentEntry {
    fn from_pending(entry: PendingEntry) -> Self {
        Self {
            id: entry.id,
            ts: entry.ts,
            level: entry.level,
            message: entry.message,
            stack_trace: entry.stack_trace,
            attempts: entry.attempts,
            sent_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FailureEntry {
    ts: String,
    error: String,
}

#[derive(Debug, Clone, Default)]
pub struct QueueStats {
    pub persisted_pending: u64,
    pub dead_letter_count: u64,
}

#[derive(Debug, Clone)]
struct QueueStore {
    root: PathBuf,
}

impl QueueStore {
    fn app_data_dir() -> PathBuf {
        resolve_paths().clawpal_dir
    }

    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn default_store() -> Self {
        Self::new(Self::app_data_dir().join("bug_report"))
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn ensure_dir(&self) -> Result<(), String> {
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())
    }

    fn read_jsonl<T: DeserializeOwned>(&self, path: &Path) -> Result<Vec<T>, String> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(path).map_err(|e| e.to_string())?;
        let reader = BufReader::new(file);
        let mut out = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|e| e.to_string())?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<T>(trimmed) {
                out.push(value);
            }
        }
        Ok(out)
    }

    fn write_jsonl<T: Serialize>(&self, path: &Path, entries: &[T]) -> Result<(), String> {
        let mut file = fs::File::create(path).map_err(|e| e.to_string())?;
        for entry in entries {
            serde_json::to_writer(&mut file, entry).map_err(|e| e.to_string())?;
            file.write_all(b"\n").map_err(|e| e.to_string())?;
        }
        file.flush().map_err(|e| e.to_string())
    }

    fn append_jsonl<T: Serialize>(&self, path: &Path, entry: &T) -> Result<(), String> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut file, entry).map_err(|e| e.to_string())?;
        file.write_all(b"\n").map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())
    }

    fn enqueue(
        &self,
        level: &str,
        message: &str,
        stack_trace: Option<&str>,
    ) -> Result<String, String> {
        self.ensure_dir()?;
        let _guard = pending_lock().lock().map_err(|e| e.to_string())?;
        let path = self.path(PENDING_FILE);
        let mut entries: Vec<PendingEntry> = self.read_jsonl(&path)?;
        let entry = PendingEntry {
            id: Uuid::new_v4().to_string(),
            ts: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            level: level.to_string(),
            message: message.to_string(),
            stack_trace: stack_trace.map(str::to_string),
            attempts: 0,
        };
        entries.push(entry.clone());
        if entries.len() > MAX_PENDING {
            let drop_count = entries.len().saturating_sub(MAX_PENDING);
            entries.drain(0..drop_count);
        }
        self.write_jsonl(&path, &entries)?;
        Ok(entry.id)
    }

    fn mark_sent(&self, id: &str) -> Result<(), String> {
        self.ensure_dir()?;
        let _guard = pending_lock().lock().map_err(|e| e.to_string())?;
        let pending_path = self.path(PENDING_FILE);
        let sent_path = self.path(SENT_FILE);
        let mut pending: Vec<PendingEntry> = self.read_jsonl(&pending_path)?;
        let Some(index) = pending.iter().position(|entry| entry.id == id) else {
            return Ok(());
        };
        let entry = pending.remove(index);
        let mut sent: Vec<SentEntry> = self.read_jsonl(&sent_path)?;
        sent.push(SentEntry::from_pending(entry));
        self.write_jsonl(&pending_path, &pending)?;
        self.write_jsonl(&sent_path, &sent)?;
        Ok(())
    }

    fn flush(&self, settings: &BugReportSettings) -> Result<(), String> {
        if !settings.enabled {
            return Ok(());
        }
        self.ensure_dir()?;
        let _guard = pending_lock().lock().map_err(|e| e.to_string())?;
        let pending_path = self.path(PENDING_FILE);
        let dead_letter_path = self.path(DEAD_LETTER_FILE);
        let sent_path = self.path(SENT_FILE);
        let pending: Vec<PendingEntry> = self.read_jsonl(&pending_path)?;
        if pending.is_empty() {
            return Ok(());
        }

        let mut keep_pending = Vec::new();
        let mut dead_letter: Vec<PendingEntry> = self.read_jsonl(&dead_letter_path)?;
        let mut sent: Vec<SentEntry> = self.read_jsonl(&sent_path)?;
        for mut entry in pending {
            let event = bug_report_event_from_pending(&entry);
            match send_report(settings, &event) {
                Ok(()) => sent.push(SentEntry::from_pending(entry)),
                Err(err) => {
                    if let Err(log_err) = self.log_send_failure_unlocked(&err) {
                        eprintln!("[bug-report] failure log write failed: {log_err}");
                    }
                    entry.attempts = entry.attempts.saturating_add(1);
                    if entry.attempts >= RETRY_LIMIT {
                        dead_letter.push(entry);
                    } else {
                        keep_pending.push(entry);
                    }
                }
            }
        }
        if dead_letter.len() > MAX_DEAD_LETTER {
            let drop_count = dead_letter.len().saturating_sub(MAX_DEAD_LETTER);
            dead_letter.drain(0..drop_count);
        }

        self.write_jsonl(&pending_path, &keep_pending)?;
        self.write_jsonl(&dead_letter_path, &dead_letter)?;
        self.write_jsonl(&sent_path, &sent)?;
        Ok(())
    }

    fn log_send_failure(&self, err: &str) -> Result<(), String> {
        self.ensure_dir()?;
        self.log_send_failure_unlocked(err)
    }

    fn log_send_failure_unlocked(&self, err: &str) -> Result<(), String> {
        let entry = FailureEntry {
            ts: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            error: err.to_string(),
        };
        self.append_jsonl(&self.path(FAILURES_FILE), &entry)
    }

    fn cleanup_old_logs(&self) -> Result<(), String> {
        self.ensure_dir()?;
        {
            let _guard = pending_lock().lock().map_err(|e| e.to_string())?;
            self.cleanup_by_retention(&self.path(SENT_FILE), |entry: &SentEntry| {
                parse_timestamp_utc(&entry.sent_at)
            })?;
        }
        self.cleanup_by_retention(&self.path(FAILURES_FILE), |entry: &FailureEntry| {
            parse_timestamp_utc(&entry.ts)
        })?;
        Ok(())
    }

    fn cleanup_by_retention<T, F>(&self, path: &Path, get_ts: F) -> Result<(), String>
    where
        T: DeserializeOwned + Serialize,
        F: Fn(&T) -> Option<DateTime<Utc>>,
    {
        let cutoff = Utc::now() - Duration::days(RETENTION_DAYS);
        let entries: Vec<T> = self.read_jsonl(path)?;
        let retained: Vec<T> = entries
            .into_iter()
            .filter(|entry| match get_ts(entry) {
                Some(ts) => ts >= cutoff,
                None => false,
            })
            .collect();
        self.write_jsonl(path, &retained)
    }

    fn stats(&self) -> QueueStats {
        let pending_count = self
            .read_jsonl::<PendingEntry>(&self.path(PENDING_FILE))
            .map(|entries| entries.len() as u64)
            .unwrap_or(0);
        let dead_letter_count = self
            .read_jsonl::<PendingEntry>(&self.path(DEAD_LETTER_FILE))
            .map(|entries| entries.len() as u64)
            .unwrap_or(0);
        QueueStats {
            persisted_pending: pending_count,
            dead_letter_count,
        }
    }
}

fn parse_timestamp_utc(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn bug_report_event_from_pending(entry: &PendingEntry) -> BugReportEvent {
    BugReportEvent {
        message: entry.message.clone(),
        stack_trace: entry.stack_trace.clone(),
        level: entry.level.clone(),
        timestamp: entry.ts.clone(),
        session_id: format!("queued-{}", entry.id),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os_type: std::env::consts::OS.to_string(),
        os_version: super::os_info::os_version_string(),
    }
}

pub fn enqueue(level: &str, message: &str, stack_trace: Option<&str>) -> Result<String, String> {
    QueueStore::default_store().enqueue(level, message, stack_trace)
}

pub fn mark_sent(id: &str) -> Result<(), String> {
    QueueStore::default_store().mark_sent(id)
}

pub fn flush(settings: &BugReportSettings) -> Result<(), String> {
    QueueStore::default_store().flush(settings)
}

pub fn cleanup_old_logs() -> Result<(), String> {
    QueueStore::default_store().cleanup_old_logs()
}

pub fn log_send_failure(err: &str) -> Result<(), String> {
    QueueStore::default_store().log_send_failure(err)
}

pub fn stats() -> QueueStats {
    QueueStore::default_store().stats()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn make_store(name: &str) -> QueueStore {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before unix epoch")
            .as_nanos();
        QueueStore::new(std::env::temp_dir().join(format!("clawpal-bug-report-{name}-{nonce}")))
    }

    #[test]
    fn enqueue_caps_pending_size() {
        let store = make_store("enqueue-cap");
        for i in 0..(MAX_PENDING + 5) {
            let message = format!("msg-{i}");
            store
                .enqueue("error", &message, None)
                .expect("enqueue should succeed");
        }
        let pending: Vec<PendingEntry> = store
            .read_jsonl(&store.path(PENDING_FILE))
            .expect("read pending");
        assert_eq!(pending.len(), MAX_PENDING);
        assert_eq!(
            pending.first().map(|entry| entry.message.as_str()),
            Some("msg-5")
        );
        if let Err(err) = fs::remove_dir_all(&store.root) {
            eprintln!("cleanup test dir failed: {err}");
        }
    }

    #[test]
    fn cleanup_prunes_old_failure_logs() {
        let store = make_store("cleanup");
        store.ensure_dir().expect("ensure dir");
        let old = FailureEntry {
            ts: "2000-01-01T00:00:00.000Z".to_string(),
            error: "old".to_string(),
        };
        let fresh = FailureEntry {
            ts: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            error: "fresh".to_string(),
        };
        store
            .write_jsonl(&store.path(FAILURES_FILE), &[old, fresh.clone()])
            .expect("seed failures");
        store.cleanup_old_logs().expect("cleanup");
        let failures: Vec<FailureEntry> = store
            .read_jsonl(&store.path(FAILURES_FILE))
            .expect("read failures");
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].error, fresh.error);
        if let Err(err) = fs::remove_dir_all(&store.root) {
            eprintln!("cleanup test dir failed: {err}");
        }
    }

    #[test]
    fn mark_sent_is_idempotent_for_unknown_id() {
        let store = make_store("mark-sent-idempotent");
        store
            .mark_sent("nonexistent-id")
            .expect("mark_sent should not error for unknown id");
        if let Err(err) = fs::remove_dir_all(&store.root) {
            eprintln!("cleanup test dir failed: {err}");
        }
    }
}
