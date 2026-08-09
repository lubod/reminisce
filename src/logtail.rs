//! In-process log retention and rotating file sink for the "observability in the
//! app" UI. This replaces the external Loki/Promtail sidecar:
//!   - a bounded in-memory ring of recent structured log events (fast UI tail
//!     + error-rate alerts), and
//!   - rotating JSON-lines on disk (`log_dir`, hourly, bounded file count) that
//!     survive restarts and are served by `GET /api/admin/logs?full=1`.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tracing::{field, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::Layer;

pub const RING_CAPACITY: usize = 5000;
pub const FILE_PREFIX: &str = "reminisce.log";
pub const MAX_LOG_FILES: usize = 14;

#[derive(Clone, Debug, Serialize)]
pub struct LogEntry {
    pub timestamp: u64,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: serde_json::Value,
}

#[derive(Default)]
pub struct LogStore {
    entries: VecDeque<LogEntry>,
}

impl LogStore {
    pub fn push(&mut self, entry: LogEntry) {
        self.entries.push_back(entry);
        while self.entries.len() > RING_CAPACITY {
            self.entries.pop_front();
        }
    }

    /// Newest-first entries whose level rank is >= `min_level`.
    pub fn query(&self, min_level: u8, limit: usize) -> Vec<LogEntry> {
        self.entries
            .iter()
            .rev()
            .filter(|e| level_rank(&e.level) >= min_level)
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn count_since(&self, since_epoch: u64, min_level: u8) -> usize {
        self.entries
            .iter()
            .filter(|e| e.timestamp >= since_epoch && level_rank(&e.level) >= min_level)
            .count()
    }
}

static RING: OnceLock<Arc<Mutex<LogStore>>> = OnceLock::new();
static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();
static FILE_GUARD: Mutex<Option<WorkerGuard>> = Mutex::new(None);

pub fn ring() -> Arc<Mutex<LogStore>> {
    RING.get_or_init(|| Arc::new(Mutex::new(LogStore::default())))
        .clone()
}

/// Arm the (one-shot) dir once so `read_file_history` knows where to look.
pub fn set_log_dir(dir: Option<&str>) {
    let _ = LOG_DIR.set(dir.map(PathBuf::from).unwrap_or_default());
    if let Some(dir) = dir {
        let _ = std::fs::create_dir_all(dir);
    }
}

/// Install once a panic hook that also records panics into the ring buffer so
/// they appear in the in-app error list. The default handler still prints.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("{}", info);
        if let Ok(mut store) = ring().lock() {
            store.push(LogEntry {
                timestamp: now_epoch(),
                level: "PANIC".to_string(),
                target: "panic".to_string(),
                message: msg.clone(),
                fields: serde_json::Value::Null,
            });
        }
        eprintln!(
            "thread panicked at {}: {}",
            info.location().map(|l| l.to_string()).unwrap_or_default(),
            msg
        );
    }));
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn level_rank(level: &str) -> u8 {
    match level {
        "TRACE" => 0,
        "DEBUG" => 1,
        "INFO" => 2,
        "WARN" | "WARNING" => 3,
        "ERROR" => 4,
        "PANIC" => 5,
        _ => 2,
    }
}

/// Map a query/min level string ("error", "warn", "info", "debug", "trace")
/// to its rank so it can be compared against entry levels.
pub fn level_from_str(level: &str) -> u8 {
    match level.trim().to_ascii_lowercase().as_str() {
        "trace" => 0,
        "debug" => 1,
        "info" => 2,
        "warn" | "warning" => 3,
        "error" => 4,
        "panic" => 5,
        _ => 2,
    }
}

/// tracing Layer that records every event into the ring buffer as a LogEntry.
#[derive(Clone)]
pub struct RingLayer {
    store: Arc<Mutex<LogStore>>,
}

impl RingLayer {
    pub fn new(store: Arc<Mutex<LogStore>>) -> Self {
        Self { store }
    }
}

impl<S: Subscriber> Layer<S> for RingLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        if let Some(entry) = capture_event(event) {
            if let Ok(mut store) = self.store.lock() {
                store.push(entry);
            }
        }
    }
}

fn capture_event(event: &tracing::Event<'_>) -> Option<LogEntry> {
    let meta = event.metadata();
    let mut visitor = FieldCapture {
        message: None,
        fields: serde_json::Map::new(),
    };
    event.record(&mut visitor);
    Some(LogEntry {
        timestamp: now_epoch(),
        level: meta.level().as_str().to_string(),
        target: meta.target().to_string(),
        message: visitor.message.unwrap_or_default(),
        fields: serde_json::Value::Object(visitor.fields),
    })
}

struct FieldCapture {
    message: Option<String>,
    fields: serde_json::Map<String, serde_json::Value>,
}

impl FieldCapture {
    fn set(&mut self, field: &field::Field, value: serde_json::Value) {
        if field.name() == "message" {
            if let serde_json::Value::String(s) = value {
                self.message = Some(s);
            }
            return;
        }
        self.fields.insert(field.name().to_string(), value);
    }
}

impl field::Visit for FieldCapture {
    fn record_str(&mut self, field: &field::Field, value: &str) {
        self.set(field, serde_json::Value::String(value.to_string()));
    }
    fn record_bool(&mut self, field: &field::Field, value: bool) {
        self.set(field, serde_json::Value::Bool(value));
    }
    fn record_i64(&mut self, field: &field::Field, value: i64) {
        self.set(field, serde_json::Value::Number(value.into()));
    }
    fn record_u64(&mut self, field: &field::Field, value: u64) {
        self.set(field, serde_json::Value::Number(value.into()));
    }
    fn record_f64(&mut self, field: &field::Field, value: f64) {
        self.set(field, serde_json::Value::Number(serde_json::Number::from_f64(value).unwrap_or(serde_json::Number::from(0))));
    }
    fn record_debug(&mut self, field: &field::Field, value: &dyn std::fmt::Debug) {
        self.set(field, serde_json::Value::String(format!("{:?}", value)));
    }
    fn record_error(&mut self, field: &field::Field, value: &(dyn std::error::Error + 'static)) {
        self.set(field, serde_json::Value::String(value.to_string()));
    }
}

/// Reader implementing `tracing_subscriber::fmt::MakeWriter`: stdout (docker
/// logs) plus an optional non-blocking rotating file appender.
pub struct MultiWriter {
    stdout: io::Stdout,
    file: Option<tracing_appender::non_blocking::NonBlocking>,
}

impl Write for MultiWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stdout.write_all(buf)?;
        if let Some(f) = self.file.as_mut() {
            let _ = f.write(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stdout.flush()?;
        if let Some(f) = self.file.as_mut() {
            let _ = f.flush();
        }
        Ok(())
    }
}

pub struct MultiWriterMaker {
    file: Option<tracing_appender::non_blocking::NonBlocking>,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for MultiWriterMaker {
    type Writer = MultiWriter;

    fn make_writer(&'a self) -> Self::Writer {
        MultiWriter {
            stdout: io::stdout(),
            file: self.file.clone(),
        }
    }
}

/// Open (once) the rotating file sink under `log_dir` and keep its worker guard
/// alive for the process lifetime. `stdout_only` skips file writes entirely.
pub fn file_writer(stdout_only: bool) -> MultiWriterMaker {
    let some = match LOG_DIR.get() {
        Some(d) if !d.as_os_str().is_empty() && !stdout_only => {
            let builder = tracing_appender::rolling::Builder::new()
                .rotation(tracing_appender::rolling::Rotation::HOURLY)
                .max_log_files(MAX_LOG_FILES)
                .filename_prefix(FILE_PREFIX);
            let appender = builder
                .build(d)
                .expect("failed to create rotating log appender");
            let (nb, guard) = tracing_appender::non_blocking(appender);
            *FILE_GUARD.lock().unwrap() = Some(guard);
            Some(nb)
        }
        _ => None,
    };
    MultiWriterMaker { file: some }
}

/// Read the tail of the newest rotating log files (newest first), parse the JSON
/// lines, and return up to `limit` entries at `min_level` severity or higher.
pub fn read_file_history(min_level: u8, limit: usize) -> Vec<LogEntry> {
    let dir = match LOG_DIR.get() {
        Some(d) if !d.as_os_str().is_empty() => d.clone(),
        _ => return Vec::new(),
    };
    if limit == 0 {
        return Vec::new();
    }

    let mut files: Vec<PathBuf> = match read_dir_names(&dir) {
        Some(files) => files,
        None => return Vec::new(),
    };
    files.sort();

    let mut out: Vec<LogEntry> = Vec::new();
    for path in files.into_iter().rev() {
        if out.len() >= limit {
            break;
        }
        let entries = read_file_entries(&path, min_level);
        for e in entries.into_iter().rev() {
            out.push(e);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

fn read_dir_names(dir: &Path) -> Option<Vec<PathBuf>> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut files = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == FILE_PREFIX || (name.starts_with(FILE_PREFIX) && name.contains('.')) {
            files.push(path);
        }
    }
    Some(files)
}

fn read_file_entries(path: &Path, min_level: u8) -> Vec<LogEntry> {
    let mut out = Vec::new();
    let f = match File::open(path) {
        Ok(f) => f,
        Err(_) => return out,
    };
    let mut reader = io::BufReader::new(f);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim_end();
                if let Some(entry) = parse_json_line(trimmed) {
                    if level_rank(&entry.level) >= min_level {
                        out.push(entry);
                    }
                }
            }
            Err(_) => break,
        }
    }
    out
}

/// Parse a JSON-lines entry emitted by the tracing_subscriber json fmt layer.
fn parse_json_line(line: &str) -> Option<LogEntry> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let obj = v.as_object()?;
    let level = obj.get("level").and_then(|l| l.as_str()).unwrap_or("INFO").to_string();
    let target = obj.get("target").and_then(|t| t.as_str()).unwrap_or("").to_string();
    let message = obj.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
    let mut fields = obj.clone();
    fields.remove("timestamp");
    fields.remove("level");
    fields.remove("target");
    fields.remove("message");
    let timestamp = obj
        .get("timestamp")
        .and_then(|t| t.as_str())
        .and_then(parse_iso_epoch)
        .unwrap_or(now_epoch());
    Some(LogEntry {
        timestamp,
        level,
        target,
        message,
        fields: serde_json::Value::Object(fields),
    })
}

fn parse_iso_epoch(iso: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|dt| dt.timestamp().max(0) as u64)
}