# Notification Decision Log Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add session-scoped, append-only audit logging for screenshot notification decisions without changing the user-visible notification product behavior.

**Architecture:** Introduce a dedicated Rust service that writes one JSONL record per processed batch into the capture session directory. Thread that logger into `MvpPipeline`, refactor judgment handling to return structured outcomes instead of only side effects, and verify both happy-path and failure-path logging via focused cargo tests.

**Tech Stack:** Rust, Tokio filesystem APIs, Serde/serde_json, Tauri service wiring, integration tests in `src-tauri/tests`

---

## File Structure

- `docs/superpowers/specs/2026-04-14-notification-decision-log-design.md`
  Source-of-truth spec for field names, enum values, failure policy, and scope boundaries.
- `src-tauri/src/services/notification_decision_log.rs`
  New logger module: log entry structs, enum values, JSONL serialization, append writer, and logger-focused unit tests.
- `src-tauri/src/services/capture_store.rs`
  Add a shared session-path helper so the pipeline/logger does not hardcode `captures/<session_id>`.
- `src-tauri/src/services/mod.rs`
  Export the new logger module.
- `src-tauri/src/services/mvp_pipeline.rs`
  Main integration point. Build the log entry, capture related-memory filtering and judgment parsing details, and write logs on send/skip/failure branches.
- `src-tauri/src/lib.rs`
  Construct the logger during app setup and inject it into `MvpPipeline`.
- `src-tauri/tests/mvp_pipeline.rs`
  Extend the integration-style test harness to verify JSONL output for sent, skipped, parse-failed, and notifier-failed paths.

## Task 1: Add Session Path Helper And JSONL Logger

**Files:**
- Create: `src-tauri/src/services/notification_decision_log.rs`
- Modify: `src-tauri/src/services/capture_store.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/notification_decision_log.rs`

- [ ] **Step 1: Write the failing logger unit tests**

Add `#[cfg(test)]` tests in `src-tauri/src/services/notification_decision_log.rs` for:

```rust
#[tokio::test]
async fn append_writes_one_json_line_into_the_session_directory() {
    let logger = NotificationDecisionLogger::new(temp_root.clone());
    let entry = sample_log_entry("session-1");

    logger.append(&entry).await.unwrap();

    let log_path = temp_root.join("captures").join("session-1").join("notification-decisions.jsonl");
    let content = tokio::fs::read_to_string(log_path).await.unwrap();
    let lines = content.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
}

#[tokio::test]
async fn append_returns_error_when_session_directory_is_missing() {
    let logger = NotificationDecisionLogger::new(temp_root.clone());
    let error = logger.append(&sample_log_entry("missing-session")).await.unwrap_err();
    assert!(error.to_string().contains("session"));
}
```

- [ ] **Step 2: Run the logger tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml notification_decision_log --lib`
Expected: FAIL because the new module/types do not exist yet.

- [ ] **Step 3: Implement the session-path helper in `CaptureStore`**

Add a non-async helper that centralizes session directory resolution:

```rust
pub fn session_dir(&self, session_id: &str) -> PathBuf {
    self.captures_root.join(session_id)
}
```

Keep `captures_root()` intact for existing callers. Do not move path logic into the pipeline directly.

- [ ] **Step 4: Implement the logger data model and append writer**

Create `src-tauri/src/services/notification_decision_log.rs` with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationDecisionLogEntry {
    pub schema_version: u32,
    pub pipeline: String,
    pub decision_id: String,
    pub session_id: String,
    pub segment_id: i64,
    pub captured_at: String,
    pub logged_at: String,
    pub input: NotificationDecisionInput,
    pub retrieval: NotificationDecisionRetrieval,
    pub judgment: NotificationDecisionJudgment,
    pub decision: NotificationDecisionResult,
    pub notification: NotificationDecisionNotification,
    pub errors: Vec<NotificationDecisionError>,
}

pub struct NotificationDecisionLogger {
    captures_root: PathBuf,
}

impl NotificationDecisionLogger {
    pub async fn append(&self, entry: &NotificationDecisionLogEntry) -> Result<()> {
        let session_dir = self.captures_root.join(&entry.session_id);
        let log_path = session_dir.join("notification-decisions.jsonl");
        let line = format!("{}\n", serde_json::to_string(entry)?);
        tokio::fs::metadata(&session_dir).await?;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .await?;
        tokio::io::AsyncWriteExt::write_all(&mut file, line.as_bytes()).await?;
        Ok(())
    }
}
```

Implementation requirements:
- Use the exact enum strings from the spec for `filtered_out.reason`, `decision_reason`, `skip_reason`, and `send_result`.
- Keep the logger pure I/O plus serialization; do not let it know anything about LLMs or notifications.
- Return `crate::error::Result<()>` with readable error messages so the pipeline can trace them.

- [ ] **Step 5: Export the logger service**

Update `src-tauri/src/services/mod.rs`:

```rust
pub mod notification_decision_log;
```

- [ ] **Step 6: Run the logger tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml notification_decision_log --lib`
Expected: PASS with the new logger tests green.

- [ ] **Step 7: Commit Task 1**

```bash
git add src-tauri/src/services/notification_decision_log.rs src-tauri/src/services/capture_store.rs src-tauri/src/services/mod.rs
git commit -m "feat: add notification decision logger service"
```

## Task 2: Thread The Logger Through App Setup And Happy-Path Pipeline Logging

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/services/mvp_pipeline.rs`
- Test: `src-tauri/tests/mvp_pipeline.rs`

- [ ] **Step 1: Write the failing integration test for the sent-notification path**

Extend `src-tauri/tests/mvp_pipeline.rs` with a helper to create a capture root and read the JSONL file:

```rust
async fn read_log_lines(captures_root: &Path, session_id: &str) -> Vec<serde_json::Value> {
    let path = captures_root.join(session_id).join("notification-decisions.jsonl");
    let content = tokio::fs::read_to_string(path).await.unwrap();
    content
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect()
}

#[tokio::test]
async fn process_batch_writes_decision_log_when_notification_is_sent() {
    // arrange pipeline with a real temp captures root and a fake notifier that succeeds
    // assert a single JSONL entry with should_notify=true and send_result="sent"
}
```

Assert at minimum:
- one log line exists
- `decision.should_notify == true`
- `notification.send_result == "sent"`
- `judgment.raw_output` contains the fenced JSON from the fake LLM
- `retrieval.filtered_result_count == 1`

- [ ] **Step 2: Run the focused pipeline test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test mvp_pipeline process_batch_writes_decision_log_when_notification_is_sent`
Expected: FAIL because `MvpPipeline` does not accept a logger or write a log yet.

- [ ] **Step 3: Inject the logger at app startup**

Update `src-tauri/src/lib.rs` to construct the logger from the existing `CaptureStore`:

```rust
let decision_logger = Arc::new(NotificationDecisionLogger::new(
    capture_store.captures_root().clone(),
));

let mvp_pipeline = Arc::new(MvpPipeline::new(
    llm_service.clone(),
    memory_service.clone(),
    notification_service.clone(),
    memory_event_emitter,
    decision_logger,
    db.clone(),
));
```

Keep the logger out of `AppState` unless another caller genuinely needs it.

- [ ] **Step 4: Refactor `MvpPipeline` to build a structured happy-path log entry**

In `src-tauri/src/services/mvp_pipeline.rs`:

1. Add a new field:

```rust
decision_logger: Arc<NotificationDecisionLogger>,
```

2. Expand `MvpPipeline::new(...)` to accept the logger.
3. Add small internal types or helper functions so `handle_judgment` no longer only logs side effects.

Recommended shape:

```rust
struct ParsedJudgment {
    should_push: bool,
    title: Option<String>,
    body: Option<String>,
    raw_output: String,
    cleaned_output: String,
    parse_error: Option<String>,
}

struct NotificationAttemptResult {
    attempted: bool,
    title: Option<String>,
    body: Option<String>,
    send_result: &'static str,
    send_error: Option<String>,
}
```

4. Build `NotificationDecisionLogEntry` after:
- summary is available
- canonical memory id is known
- search results are filtered
- judgment is parsed
- notifier send result is known

5. Append the log asynchronously before returning from the branch. If append fails, only `tracing::error!`.

- [ ] **Step 5: Run the focused pipeline test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test mvp_pipeline process_batch_writes_decision_log_when_notification_is_sent`
Expected: PASS and the JSONL assertions succeed.

- [ ] **Step 6: Commit Task 2**

```bash
git add src-tauri/src/lib.rs src-tauri/src/services/mvp_pipeline.rs src-tauri/tests/mvp_pipeline.rs
git commit -m "feat: log notification decisions for sent batches"
```

## Task 3: Capture Skip Branches And Judgment Parse Failures

**Files:**
- Modify: `src-tauri/src/services/mvp_pipeline.rs`
- Modify: `src-tauri/tests/mvp_pipeline.rs`
- Test: `src-tauri/tests/mvp_pipeline.rs`

- [ ] **Step 1: Write the failing skip-branch tests**

Add these tests to `src-tauri/tests/mvp_pipeline.rs`:

```rust
#[tokio::test]
async fn process_batch_logs_skip_when_search_returns_no_related_memories() {}

#[tokio::test]
async fn process_batch_logs_skip_when_all_related_results_are_filtered_out() {}

#[tokio::test]
async fn process_batch_logs_parse_error_when_judgment_returns_invalid_json() {}
```

Assertions:
- no-related case: `decision.skip_reason == "no_related_memories"` and `judgment.prompt == ""`
- filtered-empty case: `decision.skip_reason == "all_related_filtered_out"`
- parse-error case: `decision.skip_reason == "judgment_parse_error"` and `judgment.parse_error` is non-empty

- [ ] **Step 2: Run the skip-branch tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test mvp_pipeline process_batch_logs_`
Expected: FAIL because only the happy path is logged so far.

- [ ] **Step 3: Implement structured skip/failure population in the pipeline**

Update `src-tauri/src/services/mvp_pipeline.rs` so each early-return branch builds a complete log entry:

```rust
if related.is_empty() {
    entry.decision.should_notify = false;
    entry.decision.decision_reason = "skipped_before_judgment".to_string();
    entry.decision.skip_reason = Some("no_related_memories".to_string());
    self.try_append_decision_log(&entry).await;
    return;
}
```

Also implement:
- `all_related_filtered_out`
- `judgment_returned_false`
- `judgment_parse_error`

Do not duplicate large entry-construction code in every branch. Introduce helpers like:

```rust
fn base_log_entry(...) -> NotificationDecisionLogEntry
fn record_skip(entry: &mut NotificationDecisionLogEntry, reason: &str)
fn record_judgment(entry: &mut NotificationDecisionLogEntry, parsed: &ParsedJudgment)
```

- [ ] **Step 4: Run the skip-branch tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test mvp_pipeline process_batch_logs_`
Expected: PASS with the three new skip/parse tests green.

- [ ] **Step 5: Commit Task 3**

```bash
git add src-tauri/src/services/mvp_pipeline.rs src-tauri/tests/mvp_pipeline.rs
git commit -m "feat: record skipped notification decisions"
```

## Task 4: Capture Judgment Call Failures, Notifier Failures, And Logging Fault Tolerance

**Files:**
- Modify: `src-tauri/src/services/mvp_pipeline.rs`
- Modify: `src-tauri/tests/mvp_pipeline.rs`
- Test: `src-tauri/tests/mvp_pipeline.rs`

- [ ] **Step 1: Write the failing failure-path tests**

Add these tests:

```rust
#[tokio::test]
async fn process_batch_logs_judgment_call_failure() {}

#[tokio::test]
async fn process_batch_logs_notification_send_failure() {}

#[tokio::test]
async fn process_batch_keeps_running_when_decision_log_write_fails() {}
```

Implementation hints for the test harness:
- Create a `FailingNotifier` that returns `Err(CorivoError::Provider("boom".into()))`.
- Trigger judgment-call failure by giving `FakeLlm` only one response, so the second LLM call returns `"missing fake response"`.
- Trigger log-write failure by constructing the logger with a path that cannot contain the session directory, for example a temp file instead of a directory, or by explicitly deleting the session directory before append.

Assertions:
- judgment-call failure: `skip_reason == "judgment_call_failed"` and `errors[0].stage == "judgment_call"`
- notifier failure: `notification.send_result == "failed"` and `skip_reason == "notification_send_failed"`
- log-write failure: notification still sends, and the pipeline test does not fail or panic

- [ ] **Step 2: Run the failure-path tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test mvp_pipeline process_batch_logs_`
Expected: FAIL because these failure branches are not fully logged yet.

- [ ] **Step 3: Implement failure-path logging and non-fatal append behavior**

In `src-tauri/src/services/mvp_pipeline.rs`:
- Wrap `judge_push()` failures into `errors.push(NotificationDecisionError { stage: "judgment_call".into(), ... })`
- Convert notifier errors into:

```rust
entry.notification.attempted = true;
entry.notification.send_result = "failed".to_string();
entry.notification.send_error = Some(error.to_string());
entry.decision.should_notify = false;
entry.decision.decision_reason = "llm_should_push_true".to_string();
entry.decision.skip_reason = Some("notification_send_failed".to_string());
```

- Introduce a small helper for append failures:

```rust
async fn try_append_decision_log(&self, entry: &NotificationDecisionLogEntry) {
    if let Err(error) = self.decision_logger.append(entry).await {
        tracing::error!("写入通知决策日志失败: {:?}", error);
    }
}
```

This helper must never return `Err`.

- [ ] **Step 4: Run the full pipeline test file**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test mvp_pipeline`
Expected: PASS for existing pipeline tests plus all new notification-decision-log coverage.

- [ ] **Step 5: Run the full Rust test suite for final verification**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS. If unrelated existing failures appear, document them explicitly before proceeding.

- [ ] **Step 6: Commit Task 4**

```bash
git add src-tauri/src/services/mvp_pipeline.rs src-tauri/tests/mvp_pipeline.rs
git commit -m "test: cover notification decision logging failures"
```

## Notes For The Implementer

- Reuse the exact string enums from `docs/superpowers/specs/2026-04-14-notification-decision-log-design.md`. Do not invent near-duplicate spellings during implementation.
- Keep JSONL append behavior simple. Do not introduce log rotation, compression, or a second index file in this change.
- Do not move notification business rules into the logger. The logger is serialization plus file append only.
- Prefer adding small helper structs/functions inside `mvp_pipeline.rs` over making `process_batch()` even more monolithic.
- Preserve existing behavior for users: this change is observability-first, not a product-logic rewrite.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-14-notification-decision-log.md`. Two execution options:

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
