//! Per-run plain-text log file for background agent tasks
//! (memory-system-spec §11.5).
//!
//! The sidecar already writes a full NDJSON trace to
//! `${sessions_dir}/<thread_id>.jsonl`, and the daily tracing log
//! captures every `tracing::info!` / `warn!` from this module. Neither
//! is great for *reading* — JSONL needs parsing, the daily log
//! interleaves every subsystem in the app.
//!
//! This module adds a third surface that's optimized for "I want to
//! read one background task run cover-to-cover": one file per task
//! run, plain text, timestamped, one line per event.
//!
//! Path: `<app_data_dir>/logs/background-tasks/<YYYY-MM-DD>/<task>-<thread_id>.log`
//!
//! Why a third surface instead of just filtering the daily log:
//!
//! * One file per run = no grep / no interleaving with chat / capture
//!   noise.
//! * Persona distill / session learner are infrequent (≤ 1 per day +
//!   ≤ 1 per idle thread); the disk cost is trivial.
//! * The daily tracing log can still surface "summary" lines (task
//!   started / completed / failed) — see TaskLogger::summary_event.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Local;

use crate::domain::chat::SystemTaskKind;

const LOG_SUBDIR: &str = "background-tasks";
/// Cap on how many characters of a piece of payload (system prompt,
/// initial user message, assistant text, tool args) we record verbatim.
/// Anything past this gets `…` truncated. Keeps the log file readable
/// even when the agent dumps a 50 KB chat_thread_get response.
const PAYLOAD_PREVIEW_CHARS: usize = 4_000;

/// Per-run handle. Cheap to construct; opens the file lazily on the
/// first write so a task that fails before the first event still
/// doesn't leave an empty file lying around.
pub struct TaskLogger {
    path: PathBuf,
    file: Mutex<Option<File>>,
    start: std::time::Instant,
}

impl TaskLogger {
    pub fn open(app_data_dir: &std::path::Path, kind: SystemTaskKind, thread_id: &str) -> Self {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let dir = app_data_dir.join("logs").join(LOG_SUBDIR).join(&today);
        let path = dir.join(format!("{}-{}.log", kind.as_str(), thread_id));
        Self {
            path,
            file: Mutex::new(None),
            start: std::time::Instant::now(),
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Write a single line. Failures are logged via tracing and then
    /// swallowed — a broken log file should never abort a task.
    pub fn line(&self, level: &str, message: impl AsRef<str>) {
        let ts = Local::now().format("%H:%M:%S%.3f");
        let formatted = format!("[{ts}] [{level}] {}\n", message.as_ref());
        if let Err(error) = self.append(&formatted) {
            tracing::warn!(?error, path = %self.path.display(), "task_logger.write_failed");
        }
    }

    /// Convenience: append a header section with a multiline body. Body
    /// is preview-truncated and indented so it stands out.
    pub fn block(&self, title: &str, body: &str) {
        let ts = Local::now().format("%H:%M:%S%.3f");
        let preview = preview(body);
        let indented: String = preview
            .lines()
            .map(|l| format!("    {l}"))
            .collect::<Vec<_>>()
            .join("\n");
        let block = format!("[{ts}] [section] {title}\n{indented}\n");
        if let Err(error) = self.append(&block) {
            tracing::warn!(?error, "task_logger.block_write_failed");
        }
    }

    /// Time elapsed since `open()`, in milliseconds. Used at task
    /// completion to record run duration.
    pub fn elapsed_ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }

    fn append(&self, payload: &str) -> std::io::Result<()> {
        let mut guard = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            if let Some(parent) = self.path.parent() {
                fs::create_dir_all(parent)?;
            }
            let f = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            *guard = Some(f);
        }
        if let Some(file) = guard.as_mut() {
            file.write_all(payload.as_bytes())?;
        }
        Ok(())
    }
}

fn preview(text: &str) -> String {
    let count = text.chars().count();
    if count <= PAYLOAD_PREVIEW_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(PAYLOAD_PREVIEW_CHARS).collect();
    format!("{head}…(truncated, total {count} chars)")
}

/// Render a single sidecar JSON event line into a human-readable log
/// entry. Returns `None` if the line is uninteresting (e.g. duplicate
/// status pings) — caller skips logging.
pub fn render_sidecar_event(raw_line: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(raw_line).ok()?;
    let kind = value.get("type").and_then(|v| v.as_str())?;
    match kind {
        // text-delta fires hundreds of times per response — we log the
        // accumulated text once at the end via TaskLogger::block. Don't
        // produce a per-delta line.
        "text-delta" => None,
        "tool-call" => {
            let name = value.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let args = value
                .get("args")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "{}".to_string());
            Some(format!("tool-call {name} args={}", preview(&args)))
        }
        "tool-result" => {
            let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let result = value
                .get("result")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "(none)".to_string());
            Some(format!("tool-result id={id} result={}", preview(&result)))
        }
        "status" => {
            let detail = value.get("detail").and_then(|v| v.as_str()).unwrap_or("");
            let tag = value.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
            Some(format!("status {tag}: {detail}"))
        }
        "error" => {
            let msg = value
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("(no message)");
            Some(format!("error: {msg}"))
        }
        "finish" => {
            let reason = value.get("reason").and_then(|v| v.as_str()).unwrap_or("?");
            Some(format!("finish reason={reason}"))
        }
        "cited-frames" => {
            let ids = value
                .get("frame_ids")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            Some(format!("cited-frames [{ids}]"))
        }
        _ => Some(format!("event type={kind}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn writes_one_line_per_call() {
        let dir = TempDir::new().unwrap();
        let logger = TaskLogger::open(dir.path(), SystemTaskKind::SessionMemoryLearning, "T1");
        logger.line("info", "hello world");
        logger.line("warn", "second line");
        let body = std::fs::read_to_string(logger.path()).unwrap();
        assert!(body.contains("hello world"));
        assert!(body.contains("second line"));
        assert_eq!(body.lines().count(), 2);
    }

    #[test]
    fn block_indents_multiline_body() {
        let dir = TempDir::new().unwrap();
        let logger = TaskLogger::open(dir.path(), SystemTaskKind::PersonaDistill, "T2");
        logger.block("system prompt", "line1\nline2");
        let body = std::fs::read_to_string(logger.path()).unwrap();
        assert!(body.contains("section] system prompt"));
        assert!(body.contains("    line1"));
        assert!(body.contains("    line2"));
    }

    #[test]
    fn preview_truncates_long_text() {
        let big: String = "x".repeat(PAYLOAD_PREVIEW_CHARS + 50);
        let out = preview(&big);
        assert!(out.contains("(truncated"));
        assert!(out.chars().count() < big.chars().count());
    }

    #[test]
    fn render_sidecar_event_skips_text_delta() {
        let line = r#"{"type":"text-delta","text":"hi"}"#;
        assert_eq!(render_sidecar_event(line), None);
    }

    #[test]
    fn render_sidecar_event_summarizes_tool_call() {
        let line = r#"{"type":"tool-call","name":"note_list","args":{"scope":"global"}}"#;
        let rendered = render_sidecar_event(line).unwrap();
        assert!(rendered.contains("note_list"));
        assert!(rendered.contains("scope"));
    }

    #[test]
    fn task_file_path_contains_kind_and_thread_id() {
        let dir = TempDir::new().unwrap();
        let logger = TaskLogger::open(dir.path(), SystemTaskKind::SessionMemoryLearning, "01HXYZ");
        let p = logger.path().to_string_lossy().to_string();
        assert!(p.contains("background-tasks"));
        assert!(p.contains("session_memory_learning-01HXYZ.log"));
    }
}
