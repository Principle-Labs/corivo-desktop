//! `WorkflowStore` — filesystem definitions + SQLite runtime state.
//!
//! Two layers:
//!
//! * **Definitions** live under `$APPDATA/corivo/workflows/<slug>/WORKFLOW.md`.
//!   Frontmatter holds metadata (`name` / `description` / `tool_whitelist` /
//!   `max_turns`); the body is the system-prompt template. The
//!   filesystem is the source of truth — `scan_definitions` rebuilds
//!   from disk on each call.
//!
//! * **Runtime state** lives in `workflow_schedules` + `workflow_runs`.
//!   A schedule binds one trigger to one slug; runs are append-only
//!   audit rows.
//!
//! `list_and_advance_due` is the Ticker's hot path — selects due
//! schedules and advances their `next_run_at` (or disables the
//! schedule for one-shot triggers) in a single transaction so two
//! ticks racing on the same row can never double-fire.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use ulid::Ulid;

use crate::db::pool::{run_blocking, DbPool};
use crate::db::time::DbInstant;
use crate::domain::workflow::{
    Trigger, TriggerKind, WorkflowDefinition, WorkflowRun, WorkflowRunStatus, WorkflowSchedule,
    WorkflowScheduleSource,
};
use crate::error::{CorivoError, Result};

const WORKFLOWS_SUBDIR: &str = "corivo/workflows";
const WORKFLOW_MANIFEST: &str = "WORKFLOW.md";
const DEFAULT_MAX_TURNS: u32 = 12;

/// Carrier the Ticker uses to dispatch a due schedule. Slim by design
/// — we keep the loaded `WorkflowDefinition` separate so a missing
/// `WORKFLOW.md` doesn't poison the Ticker (it just logs + skips).
#[derive(Debug, Clone, Serialize)]
pub struct DispatchedSchedule {
    pub slug: String,
    pub trigger: Trigger,
    /// Updated `next_run_at` after the Ticker advanced the row.
    /// `None` for one-shot (`Once`) triggers that auto-disable.
    pub next_run_at: Option<DateTime<Utc>>,
}

/// Input to `WorkflowStore::upsert_schedule`. `source` +
/// `created_by_thread_id` are only honoured on INSERT — the ON
/// CONFLICT path preserves whatever was already on the row.
#[derive(Debug, Clone)]
pub struct UpsertSchedule {
    pub slug: String,
    pub trigger: Trigger,
    pub enabled: bool,
    pub source: WorkflowScheduleSource,
    pub created_by_thread_id: Option<String>,
}

pub struct WorkflowStore {
    pool: DbPool,
    root: PathBuf,
    /// In-memory set of slugs currently enqueued or running. Used by
    /// the Ticker + run_now IPC to refuse duplicate dispatches when
    /// the user spam-clicks "立即运行" or the cron fires faster than
    /// the previous run finished. **NOT persisted** — if the process
    /// restarts mid-run, the slug is naturally released; the next
    /// dispatch attempt simply re-claims it. Lock contention is
    /// trivial (only touched at enqueue / consume_output boundary).
    pending_slugs: Arc<StdMutex<HashSet<String>>>,
    /// Tauri AppHandle for emitting `workflow:completed` events from
    /// non-runner code paths (`on_dispatch_aborted` etc.). Optional
    /// because tests construct a store without a Tauri context.
    app: Option<tauri::AppHandle<tauri::Wry>>,
}

impl WorkflowStore {
    /// `app_data_dir` is `Tauri::Manager::app_data_dir()` — we suffix
    /// with `corivo/workflows/` so the existing top-level dir layout
    /// (`captures/`, `corivo-agent-sessions/`, ...) stays unbroken.
    pub fn new(pool: DbPool, app_data_dir: &Path) -> Arc<Self> {
        Arc::new(Self {
            pool,
            root: app_data_dir.join(WORKFLOWS_SUBDIR),
            pending_slugs: Arc::new(StdMutex::new(HashSet::new())),
            app: None,
        })
    }

    /// Same as [`Self::new`] but attaches an AppHandle so the store
    /// can emit `workflow:completed` events on background failure
    /// paths. The handle is `Option<>` on the struct so test
    /// fixtures (which build a store with `test_in_memory_pool`) can
    /// skip the Tauri runtime entirely.
    pub fn with_app(
        pool: DbPool,
        app_data_dir: &Path,
        app: tauri::AppHandle<tauri::Wry>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            root: app_data_dir.join(WORKFLOWS_SUBDIR),
            pending_slugs: Arc::new(StdMutex::new(HashSet::new())),
            app: Some(app),
        })
    }

    /// Tauri AppHandle, if the store was constructed via
    /// [`Self::with_app`]. `None` only in test fixtures.
    pub fn app_handle(&self) -> Option<&tauri::AppHandle<tauri::Wry>> {
        self.app.as_ref()
    }

    /// Try to claim a slug for dispatch. Returns `true` if the caller
    /// got the claim (proceed with enqueue), `false` if the slug was
    /// already pending (caller should refuse / log + skip).
    ///
    /// Pair with [`Self::release_slug`] in the task's
    /// `consume_output` so a finished run unblocks the next attempt.
    pub fn try_claim_slug(&self, slug: &str) -> bool {
        match self.pending_slugs.lock() {
            Ok(mut set) => set.insert(slug.to_string()),
            Err(poisoned) => {
                // Poisoning means another thread panicked while
                // holding the lock. Recover by taking the inner
                // value; an extra "claimed twice" on the next
                // dispatch is fine.
                let mut set = poisoned.into_inner();
                set.insert(slug.to_string())
            }
        }
    }

    /// Release a slug after the run completes (success OR failure).
    /// Idempotent — releasing an unclaimed slug is a no-op.
    pub fn release_slug(&self, slug: &str) {
        if let Ok(mut set) = self.pending_slugs.lock() {
            set.remove(slug);
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Test-only DB-pool accessor — used by `schedule_task_handler`
    /// tests to seed FK parents (`chat_threads` rows) before exercising
    /// the agent's `created_by_thread_id` path. Don't lean on this
    /// outside `#[cfg(test)]`; route real production reads/writes
    /// through the dedicated methods on this struct.
    #[cfg(test)]
    pub(crate) fn pool_for_tests(&self) -> DbPool {
        self.pool.clone()
    }

    /// Create the workflows root if it doesn't exist yet. Called at
    /// boot before the first scan so a fresh install behaves like a
    /// well-formed empty repo.
    pub fn ensure_root(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.root)
    }

    // ------------------------------------------------------------------
    // Filesystem layer
    // ------------------------------------------------------------------

    /// Re-read every `WORKFLOW.md` under the root. Best-effort: a
    /// malformed file logs a warning and gets skipped rather than
    /// failing the whole scan.
    pub fn scan_definitions(&self) -> Vec<WorkflowDefinition> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest = path.join(WORKFLOW_MANIFEST);
            if !manifest.is_file() {
                continue;
            }
            let slug = entry.file_name().to_string_lossy().to_string();
            if slug.starts_with('.') {
                continue;
            }
            match read_definition(&manifest, &slug) {
                Ok(def) => out.push(def),
                Err(error) => {
                    tracing::warn!(slug = %slug, %error, "scheduled_workflows.scan_skip");
                }
            }
        }
        out.sort_by(|a, b| a.slug.cmp(&b.slug));
        out
    }

    pub fn load_definition(&self, slug: &str) -> Option<WorkflowDefinition> {
        let path = self.root.join(slug).join(WORKFLOW_MANIFEST);
        read_definition(&path, slug).ok()
    }

    /// Write the canonical `WORKFLOW.md` for a definition. Creates
    /// the `<slug>/` dir if needed. Used by `workflows_save` and by
    /// the first-boot preset seeder (PR4).
    pub fn write_definition(&self, def: &WorkflowDefinition) -> Result<()> {
        let dir = self.root.join(&def.slug);
        fs::create_dir_all(&dir)
            .map_err(|e| CorivoError::Internal(format!("create workflow dir {}: {e}", def.slug)))?;
        let manifest = dir.join(WORKFLOW_MANIFEST);
        let text = render_workflow_md(def);
        fs::write(&manifest, text)
            .map_err(|e| CorivoError::Internal(format!("write {}: {e}", manifest.display())))?;
        Ok(())
    }

    /// Delete the `<slug>/` directory. Idempotent — missing dir is OK.
    pub fn delete_definition(&self, slug: &str) -> Result<()> {
        let dir = self.root.join(slug);
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|e| {
                CorivoError::Internal(format!("delete workflow dir {slug}: {e}"))
            })?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // SQLite — schedules
    // ------------------------------------------------------------------

    pub async fn list_schedules(&self) -> Result<Vec<WorkflowSchedule>> {
        run_blocking(self.pool.clone(), move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT slug, trigger_kind, trigger_expr, enabled,
                            last_run_at, next_run_at, last_status,
                            source, created_by_thread_id,
                            created_at, updated_at
                       FROM workflow_schedules
                       ORDER BY slug",
                )
                .map_err(|e| CorivoError::Internal(format!("list_schedules prepare: {e}")))?;
            let rows = stmt
                .query_map([], row_to_schedule)
                .map_err(|e| CorivoError::Internal(format!("list_schedules query: {e}")))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| CorivoError::Internal(format!("list_schedules collect: {e}")))
        })
        .await
    }

    pub async fn get_schedule(&self, slug: &str) -> Result<Option<WorkflowSchedule>> {
        let slug = slug.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row(
                "SELECT slug, trigger_kind, trigger_expr, enabled,
                        last_run_at, next_run_at, last_status,
                        source, created_by_thread_id,
                        created_at, updated_at
                   FROM workflow_schedules WHERE slug = ?1",
                params![slug],
                row_to_schedule,
            )
            .optional()
            .map_err(|e| CorivoError::Internal(format!("get_schedule: {e}")))
        })
        .await
    }

    /// Insert-or-update. Recomputes `next_run_at` from `trigger` against
    /// the current clock so toggling enabled or changing the trigger
    /// can never leave a stale firing time on the row.
    ///
    /// `source` + `created_by_thread_id` are only applied on INSERT;
    /// on conflict we explicitly preserve the original row's values so
    /// a user fine-tuning an agent-created schedule doesn't lose its
    /// provenance.
    pub async fn upsert_schedule(&self, spec: UpsertSchedule) -> Result<WorkflowSchedule> {
        let now = DbInstant::now();
        let next_run = spec
            .enabled
            .then(|| spec.trigger.next_after(now.as_datetime()))
            .flatten();
        let trigger_kind = TriggerKind::from_trigger(&spec.trigger);
        let trigger_expr = serde_json::to_string(&spec.trigger)
            .map_err(|e| CorivoError::Internal(format!("trigger serialize: {e}")))?;
        let next_run_db = next_run.map(DbInstant::from);
        let UpsertSchedule {
            slug,
            enabled,
            source,
            created_by_thread_id,
            ..
        } = spec;
        let pool = self.pool.clone();
        let stored_slug = slug.clone();

        run_blocking(pool, move |conn| {
            conn.execute(
                "INSERT INTO workflow_schedules
                   (slug, trigger_kind, trigger_expr, enabled, next_run_at,
                    source, created_by_thread_id,
                    created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                 ON CONFLICT(slug) DO UPDATE SET
                   trigger_kind  = excluded.trigger_kind,
                   trigger_expr  = excluded.trigger_expr,
                   enabled       = excluded.enabled,
                   next_run_at   = excluded.next_run_at,
                   updated_at    = excluded.updated_at",
                params![
                    stored_slug,
                    trigger_kind.as_str(),
                    trigger_expr,
                    enabled as i32,
                    next_run_db,
                    source.as_str(),
                    created_by_thread_id,
                    now,
                ],
            )
            .map_err(|e| CorivoError::Internal(format!("upsert_schedule: {e}")))?;
            Ok(())
        })
        .await?;

        // Round-trip read so callers get the canonical row (including
        // any DB-side defaults).
        self.get_schedule(&slug).await?.ok_or_else(|| {
            CorivoError::Internal(format!("upsert_schedule lost row {slug}"))
        })
    }

    pub async fn set_enabled(&self, slug: String, enabled: bool) -> Result<()> {
        let current = self
            .get_schedule(&slug)
            .await?
            .ok_or_else(|| CorivoError::Internal(format!("no schedule for slug {slug}")))?;
        // Preserve source on toggle — the upsert below would re-supply
        // it anyway, but explicit is friendlier to future grep.
        self.upsert_schedule(UpsertSchedule {
            slug,
            trigger: current.trigger,
            enabled,
            source: current.source,
            created_by_thread_id: current.created_by_thread_id,
        })
        .await
        .map(|_| ())
    }

    pub async fn delete_schedule(&self, slug: String) -> Result<()> {
        run_blocking(self.pool.clone(), move |mut conn| {
            // Two-table delete in one transaction. v1432 had a FK
            // `workflow_runs.slug REFERENCES workflow_schedules(slug)
            //  ON DELETE CASCADE` that did this implicitly, but v1433
            // dropped the FK (so run-now on a schedule-less definition
            // could persist its row); the run cleanup is now explicit.
            let tx = conn
                .transaction()
                .map_err(|e| CorivoError::Internal(format!("delete_schedule begin: {e}")))?;
            tx.execute(
                "DELETE FROM workflow_runs WHERE slug = ?1",
                params![slug],
            )
            .map_err(|e| CorivoError::Internal(format!("delete_schedule runs: {e}")))?;
            tx.execute(
                "DELETE FROM workflow_schedules WHERE slug = ?1",
                params![slug],
            )
            .map_err(|e| CorivoError::Internal(format!("delete_schedule schedule: {e}")))?;
            tx.commit()
                .map_err(|e| CorivoError::Internal(format!("delete_schedule commit: {e}")))?;
            Ok(())
        })
        .await
    }

    /// Atomically: select due schedules, advance their `next_run_at`
    /// (or `enabled=0` for `Once`), return them for the Ticker to
    /// dispatch. Single transaction so two ticks racing on the same
    /// minute boundary can never both pull the same row.
    pub async fn list_and_advance_due(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<DispatchedSchedule>> {
        let now_db = DbInstant::from(now);
        run_blocking(self.pool.clone(), move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| CorivoError::Internal(format!("dispatch tx begin: {e}")))?;
            let mut stmt = tx
                .prepare(
                    "SELECT slug, trigger_expr
                       FROM workflow_schedules
                      WHERE enabled = 1
                        AND next_run_at IS NOT NULL
                        AND next_run_at <= ?1
                      ORDER BY next_run_at",
                )
                .map_err(|e| CorivoError::Internal(format!("dispatch prepare: {e}")))?;
            let mut due: Vec<(String, Trigger)> = Vec::new();
            let rows = stmt
                .query_map(params![now_db], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| CorivoError::Internal(format!("dispatch query: {e}")))?;
            for row in rows {
                let (slug, expr) = row
                    .map_err(|e| CorivoError::Internal(format!("dispatch row: {e}")))?;
                let trigger: Trigger = serde_json::from_str(&expr)
                    .map_err(|e| CorivoError::Internal(format!("trigger parse {slug}: {e}")))?;
                due.push((slug, trigger));
            }
            drop(stmt);

            let updated_at = DbInstant::from(now);
            let mut out = Vec::with_capacity(due.len());
            for (slug, trigger) in due {
                let next = trigger.next_after(now);
                let enabled = !matches!(trigger, Trigger::Once { .. });
                tx.execute(
                    "UPDATE workflow_schedules
                        SET next_run_at = ?2,
                            enabled     = ?3,
                            updated_at  = ?4
                      WHERE slug = ?1",
                    params![
                        slug,
                        next.map(DbInstant::from),
                        enabled as i32,
                        updated_at,
                    ],
                )
                .map_err(|e| CorivoError::Internal(format!("dispatch advance: {e}")))?;
                out.push(DispatchedSchedule {
                    slug,
                    trigger,
                    next_run_at: next,
                });
            }
            tx.commit()
                .map_err(|e| CorivoError::Internal(format!("dispatch commit: {e}")))?;
            Ok(out)
        })
        .await
    }

    /// Called by the task once a run finishes — records the audit row
    /// in `workflow_runs` and bumps `last_run_at` / `last_status` on
    /// the schedule. `next_run_at` was already advanced by the Ticker
    /// at dispatch time.
    pub async fn record_run(&self, run: WorkflowRun) -> Result<()> {
        let finished_db = DbInstant::from(run.finished_at);
        let status = run.status;
        let slug = run.slug.clone();
        run_blocking(self.pool.clone(), move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| CorivoError::Internal(format!("record_run tx: {e}")))?;
            tx.execute(
                "INSERT INTO workflow_runs
                   (id, slug, thread_id, status, started_at, finished_at,
                    error_message, summary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    run.id,
                    run.slug,
                    run.thread_id,
                    status.as_str(),
                    DbInstant::from(run.started_at),
                    finished_db,
                    run.error_message,
                    run.summary,
                ],
            )
            .map_err(|e| CorivoError::Internal(format!("record_run insert: {e}")))?;
            tx.execute(
                "UPDATE workflow_schedules
                    SET last_run_at = ?2,
                        last_status = ?3,
                        updated_at  = ?2
                  WHERE slug = ?1",
                params![slug, finished_db, status.as_str()],
            )
            .map_err(|e| CorivoError::Internal(format!("record_run touch schedule: {e}")))?;
            tx.commit()
                .map_err(|e| CorivoError::Internal(format!("record_run commit: {e}")))?;
            Ok(())
        })
        .await
    }

    pub async fn list_runs(
        &self,
        slug: Option<String>,
        limit: u32,
    ) -> Result<Vec<WorkflowRun>> {
        let limit = limit.clamp(1, 200) as i64;
        run_blocking(self.pool.clone(), move |conn| match slug {
            Some(s) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, slug, thread_id, status, started_at, finished_at,
                                error_message, summary
                           FROM workflow_runs WHERE slug = ?1
                           ORDER BY started_at DESC LIMIT ?2",
                    )
                    .map_err(|e| CorivoError::Internal(format!("list_runs prepare: {e}")))?;
                let rows = stmt
                    .query_map(params![s, limit], row_to_run)
                    .map_err(|e| CorivoError::Internal(format!("list_runs query: {e}")))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|e| CorivoError::Internal(format!("list_runs collect: {e}")))
            }
            None => {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, slug, thread_id, status, started_at, finished_at,
                                error_message, summary
                           FROM workflow_runs ORDER BY started_at DESC LIMIT ?1",
                    )
                    .map_err(|e| CorivoError::Internal(format!("list_runs prepare: {e}")))?;
                let rows = stmt
                    .query_map(params![limit], row_to_run)
                    .map_err(|e| CorivoError::Internal(format!("list_runs query: {e}")))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|e| CorivoError::Internal(format!("list_runs collect: {e}")))
            }
        })
        .await
    }

    pub async fn get_run(&self, id: String) -> Result<Option<WorkflowRun>> {
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row(
                "SELECT id, slug, thread_id, status, started_at, finished_at,
                        error_message, summary
                   FROM workflow_runs WHERE id = ?1",
                params![id],
                row_to_run,
            )
            .optional()
            .map_err(|e| CorivoError::Internal(format!("get_run: {e}")))
        })
        .await
    }

    /// Issue a fresh ULID for a new run row.
    pub fn new_run_id() -> String {
        Ulid::new().to_string()
    }

    /// Derive a kebab-case slug from a human-readable name and append a
    /// short suffix if it collides with an existing definition. Used by
    /// the agent's `schedule_task` tool so the LLM doesn't have to invent
    /// — and validate — slugs on the fly.
    ///
    /// Strategy: kebab the name (ASCII-only; CJK gets stripped → fall
    /// back to `task` + suffix). Try the bare slug first; on collision
    /// append `-<4-char-hex>` from a fresh ULID's tail.
    pub async fn generate_unique_slug(&self, name: &str) -> Result<String> {
        let base = kebab_from_name(name);
        if self.get_schedule(&base).await?.is_none()
            && self.load_definition(&base).is_none()
        {
            return Ok(base);
        }
        // Up to 5 retries — collision space is 16^4 = 65k, so two
        // failures is astronomically rare unless the name is hot.
        for _ in 0..5 {
            let id = Ulid::new().to_string();
            let suffix = id[id.len() - 4..].to_lowercase();
            let candidate = format!("{base}-{suffix}");
            if self.get_schedule(&candidate).await?.is_none()
                && self.load_definition(&candidate).is_none()
            {
                return Ok(candidate);
            }
        }
        Err(CorivoError::Internal(
            "could not generate unique workflow slug after 5 attempts".into(),
        ))
    }
}

/// Lower-case ASCII kebab — strips anything outside `[a-z0-9-]`,
/// collapses runs, trims hyphens. Empty / pure-CJK input falls back
/// to `task` so the suffix path always has something to append to.
fn kebab_from_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = true;
    for ch in name.chars() {
        let mapped = if ch.is_ascii_alphabetic() {
            ch.to_ascii_lowercase()
        } else if ch.is_ascii_digit() {
            ch
        } else {
            '-'
        };
        if mapped == '-' {
            if !last_dash {
                out.push('-');
                last_dash = true;
            }
        } else {
            out.push(mapped);
            last_dash = false;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "task".to_string()
    } else if trimmed.len() > 48 {
        trimmed.chars().take(48).collect::<String>().trim_end_matches('-').to_string()
    } else {
        trimmed
    }
}

fn row_to_schedule(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowSchedule> {
    let trigger_expr: String = row.get("trigger_expr")?;
    let trigger: Trigger = serde_json::from_str(&trigger_expr).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(e),
        )
    })?;
    let last_status_raw: Option<String> = row.get("last_status")?;
    let last_status = last_status_raw.and_then(|s| WorkflowRunStatus::parse(&s));
    let source_raw: String = row.get("source")?;
    let source = WorkflowScheduleSource::parse(&source_raw).unwrap_or(WorkflowScheduleSource::User);
    Ok(WorkflowSchedule {
        slug: row.get("slug")?,
        trigger,
        enabled: row.get::<_, i32>("enabled")? == 1,
        last_run_at: row
            .get::<_, Option<DbInstant>>("last_run_at")?
            .map(DbInstant::into_inner),
        next_run_at: row
            .get::<_, Option<DbInstant>>("next_run_at")?
            .map(DbInstant::into_inner),
        last_status,
        source,
        created_by_thread_id: row.get("created_by_thread_id")?,
        created_at: row
            .get::<_, DbInstant>("created_at")?
            .into_inner(),
        updated_at: row
            .get::<_, DbInstant>("updated_at")?
            .into_inner(),
    })
}

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowRun> {
    let status_raw: String = row.get("status")?;
    let status = WorkflowRunStatus::parse(&status_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("bad workflow_runs.status: {status_raw}"),
            )),
        )
    })?;
    Ok(WorkflowRun {
        id: row.get("id")?,
        slug: row.get("slug")?,
        thread_id: row.get("thread_id")?,
        status,
        started_at: row
            .get::<_, DbInstant>("started_at")?
            .into_inner(),
        finished_at: row
            .get::<_, DbInstant>("finished_at")?
            .into_inner(),
        error_message: row.get("error_message")?,
        summary: row.get("summary")?,
    })
}

fn render_workflow_md(def: &WorkflowDefinition) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("name: {}\n", quote_yaml_scalar(&def.name)));
    if let Some(desc) = &def.description {
        out.push_str(&format!("description: {}\n", quote_yaml_scalar(desc)));
    }
    out.push_str("tool_whitelist:\n");
    for tool in &def.tool_whitelist {
        out.push_str(&format!("  - {}\n", quote_yaml_scalar(tool)));
    }
    out.push_str(&format!("max_turns: {}\n", def.max_turns));
    out.push_str("---\n");
    // Body — exactly the system_prompt; the reader's `trim_start`
    // tolerates trailing/leading whitespace either way.
    out.push_str(def.system_prompt.trim_end());
    out.push('\n');
    out
}

/// Wrap a scalar in double quotes when it contains characters our
/// hand-rolled YAML reader doesn't tolerate (colons, `#`, leading
/// whitespace). Pure ASCII / CJK without those characters stays
/// unquoted so the file remains human-editable.
fn quote_yaml_scalar(s: &str) -> String {
    let needs_quotes = s.is_empty()
        || s.starts_with(char::is_whitespace)
        || s.contains(':')
        || s.contains('#')
        || s.contains('\n')
        || s.contains('"');
    if needs_quotes {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn read_definition(manifest: &Path, slug: &str) -> std::io::Result<WorkflowDefinition> {
    let text = fs::read_to_string(manifest)?;
    let (front, body) = split_frontmatter(&text).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "missing or malformed frontmatter",
        )
    })?;

    let name = pick_string(&front, "name").unwrap_or_else(|| slug.to_string());
    let description = pick_string(&front, "description");
    let max_turns = pick_string(&front, "max_turns")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(DEFAULT_MAX_TURNS);
    let tool_whitelist = pick_list(&front, "tool_whitelist");

    Ok(WorkflowDefinition {
        slug: slug.to_string(),
        name,
        description,
        tool_whitelist,
        max_turns,
        system_prompt: body.trim_start().to_string(),
    })
}

/// Returns `(frontmatter_body, post_frontmatter_body)` if the file
/// opens with a `---`-fenced block. Frontmatter delimiters are
/// stripped; the post-frontmatter body is everything after the
/// closing fence verbatim.
fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let stripped = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n"))?;
    let close = stripped.find("\n---")?;
    let front = &stripped[..close];
    let mut rest = &stripped[close + 4..]; // skip "\n---"
    if let Some(after) = rest.strip_prefix('\n') {
        rest = after;
    } else if let Some(after) = rest.strip_prefix("\r\n") {
        rest = after;
    }
    Some((front, rest))
}

/// Pick a top-level scalar field from minimalist YAML frontmatter.
/// Supports `key: value` on a single line; surrounding quotes are
/// stripped. Lists / nested maps go through `pick_list` instead.
fn pick_string(front: &str, key: &str) -> Option<String> {
    for line in front.lines() {
        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.strip_prefix(&format!("{key}:")) {
            let value = rest.trim();
            if value.is_empty() {
                return None;
            }
            return Some(
                value
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            );
        }
    }
    None
}

/// Pick a YAML list from frontmatter. Supports two shapes:
///
/// * Block style:
///   ```yaml
///   tool_whitelist:
///     - foo
///     - bar
///   ```
///
/// * Inline comma-separated (one-line convenience):
///   ```yaml
///   tool_whitelist: foo, bar
///   ```
fn pick_list(front: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let prefix = format!("{key}:");
    let mut lines = front.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();
        let Some(rest) = trimmed.strip_prefix(&prefix) else {
            continue;
        };
        let inline = rest.trim();
        if !inline.is_empty() {
            // Inline comma-separated form.
            for part in inline.split(',') {
                let value = part
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .trim_matches('[')
                    .trim_matches(']')
                    .trim();
                if !value.is_empty() {
                    out.push(value.to_string());
                }
            }
            return out;
        }
        // Block style — consume subsequent `  - item` lines until we
        // hit a non-indented line or another top-level key.
        while let Some(peek) = lines.peek() {
            let peek_trim = peek.trim_end();
            if peek_trim.is_empty() {
                lines.next();
                continue;
            }
            if !peek_trim.starts_with(char::is_whitespace) {
                break;
            }
            let item = peek_trim.trim_start();
            if let Some(value) = item.strip_prefix("- ") {
                out.push(
                    value
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string(),
                );
                lines.next();
            } else {
                break;
            }
        }
        return out;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::apply_migrations;
    use crate::db::pool::test_in_memory_pool;
    use tempfile::TempDir;

    fn fresh_store() -> (TempDir, Arc<WorkflowStore>) {
        let pool = test_in_memory_pool().unwrap();
        // Apply the full schema so workflow_schedules / workflow_runs
        // exist (the in-memory pool starts empty).
        let conn = pool.get().unwrap();
        apply_migrations(&conn).unwrap();
        drop(conn);
        let dir = TempDir::new().unwrap();
        let store = WorkflowStore::new(pool, dir.path());
        store.ensure_root().unwrap();
        (dir, store)
    }

    fn user_spec(slug: &str, trigger: Trigger, enabled: bool) -> UpsertSchedule {
        UpsertSchedule {
            slug: slug.to_string(),
            trigger,
            enabled,
            source: WorkflowScheduleSource::User,
            created_by_thread_id: None,
        }
    }

    #[tokio::test]
    async fn upsert_schedule_computes_next_run_at() {
        let (_dir, store) = fresh_store();
        let saved = store
            .upsert_schedule(user_spec(
                "demo",
                Trigger::Interval { minutes: 30 },
                true,
            ))
            .await
            .unwrap();
        assert!(saved.enabled);
        let next = saved.next_run_at.expect("interval trigger must produce next");
        let now = chrono::Utc::now();
        // Within 30m +/- a second of now.
        let delta = (next - now).num_seconds();
        assert!(
            (1700..=1850).contains(&delta),
            "expected ~30m ahead, got {delta}s"
        );
    }

    #[tokio::test]
    async fn upsert_with_enabled_false_skips_next_run_at() {
        let (_dir, store) = fresh_store();
        let saved = store
            .upsert_schedule(user_spec(
                "paused",
                Trigger::Interval { minutes: 5 },
                false,
            ))
            .await
            .unwrap();
        assert!(!saved.enabled);
        assert!(saved.next_run_at.is_none(), "disabled → no next firing");
    }

    #[tokio::test]
    async fn list_and_advance_picks_due_and_pushes_next() {
        let (_dir, store) = fresh_store();
        // Past `at` for a Once trigger so it's due immediately.
        let past = chrono::Utc::now() - chrono::Duration::minutes(5);
        store
            .upsert_schedule(user_spec(
                "due-once",
                Trigger::Once { at: past },
                true,
            ))
            .await
            .unwrap();
        // The upsert won't actually set next_run_at for an
        // already-past Once trigger (Trigger::next_after returns
        // None). Inject one by hand so we can exercise the dispatch
        // path.
        crate::db::pool::run_blocking(store.pool.clone(), move |conn| {
            conn.execute(
                "UPDATE workflow_schedules SET next_run_at = ?1 WHERE slug = ?2",
                rusqlite::params![DbInstant::from(past), "due-once"],
            )
            .map_err(|e| crate::error::CorivoError::Internal(format!("inject: {e}")))?;
            Ok(())
        })
        .await
        .unwrap();

        let dispatched = store
            .list_and_advance_due(chrono::Utc::now())
            .await
            .unwrap();
        assert_eq!(dispatched.len(), 1);
        assert_eq!(dispatched[0].slug, "due-once");
        // Once auto-disables.
        let after = store.get_schedule("due-once").await.unwrap().unwrap();
        assert!(!after.enabled);
    }

    #[tokio::test]
    async fn record_run_writes_audit_row_and_touches_schedule() {
        let (_dir, store) = fresh_store();
        store
            .upsert_schedule(user_spec(
                "demo",
                Trigger::Interval { minutes: 60 },
                true,
            ))
            .await
            .unwrap();
        let now = chrono::Utc::now();
        // Use thread_id=None to skip the FK to chat_threads — the
        // runner-side path always supplies a real thread, but here
        // we're exercising the audit-row + last_run_at update path
        // in isolation.
        let run = WorkflowRun {
            id: WorkflowStore::new_run_id(),
            slug: "demo".to_string(),
            thread_id: None,
            status: WorkflowRunStatus::Success,
            started_at: now - chrono::Duration::seconds(10),
            finished_at: now,
            error_message: None,
            summary: Some("test run".to_string()),
        };
        store.record_run(run.clone()).await.unwrap();
        let runs = store.list_runs(Some("demo".to_string()), 10).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert!(runs[0].thread_id.is_none());
        let schedule = store.get_schedule("demo").await.unwrap().unwrap();
        assert_eq!(schedule.last_status, Some(WorkflowRunStatus::Success));
        assert!(schedule.last_run_at.is_some());
    }

    #[test]
    fn render_workflow_md_round_trips_through_reader() {
        let def = WorkflowDefinition {
            slug: "round-trip".to_string(),
            name: "回归测试 workflow".to_string(),
            description: Some("一段:带冒号的描述".to_string()),
            tool_whitelist: vec!["save_note".into(), "memory_search".into()],
            max_turns: 9,
            system_prompt: "请按提示工作\n第二行".to_string(),
        };
        let rendered = render_workflow_md(&def);
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("WORKFLOW.md");
        fs::write(&manifest, &rendered).unwrap();
        let parsed = read_definition(&manifest, "round-trip").unwrap();
        assert_eq!(parsed.name, def.name);
        assert_eq!(parsed.description, def.description);
        assert_eq!(parsed.tool_whitelist, def.tool_whitelist);
        assert_eq!(parsed.max_turns, def.max_turns);
        assert_eq!(parsed.system_prompt.trim(), def.system_prompt.trim());
    }

    #[test]
    fn split_frontmatter_handles_basic_doc() {
        let doc = "---\nname: foo\n---\nbody here\n";
        let (front, body) = split_frontmatter(doc).unwrap();
        assert_eq!(front, "name: foo");
        assert_eq!(body, "body here\n");
    }

    #[test]
    fn pick_list_block_style() {
        let front = "name: x\ntool_whitelist:\n  - foo\n  - bar\nmax_turns: 5\n";
        assert_eq!(
            pick_list(front, "tool_whitelist"),
            vec!["foo".to_string(), "bar".to_string()]
        );
    }

    #[test]
    fn pick_list_inline_style() {
        let front = "tool_whitelist: foo, bar, baz\n";
        assert_eq!(
            pick_list(front, "tool_whitelist"),
            vec!["foo".to_string(), "bar".to_string(), "baz".to_string()]
        );
    }

    #[test]
    fn pick_string_strips_quotes() {
        let front = "name: \"每日回顾\"\n";
        assert_eq!(pick_string(front, "name"), Some("每日回顾".to_string()));
    }
}
