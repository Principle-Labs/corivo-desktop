//! Frames repo — CRUD over the v3 SSOT table (spec §五).
//!
//! Phase 1 surface:
//!   * insert            — write a new row, returning the inserted Frame
//!   * by_id             — single-row lookup by ULID
//!   * recent            — paginated time-desc list (used by /timeline)
//!   * list              — filtered list (from / to / app)
//!   * last_in_session   — most recent frame in the same capture session;
//!                          dedup uses this to compare against the
//!                          previous frame.
//!   * touch_still_present — bump still_present_until without writing a
//!                            new row when the dedup hash matches.
//!
//! Phase 3 will add fts_search / by_content_hash; not in this repo yet.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use ulid::Ulid;

use crate::{
    db::{
        pool::{run_blocking, DbPool},
        time::DbInstant,
    },
    domain::frame::{Frame, NewFrame},
    error::{CorivoError, Result},
};

#[derive(Debug, Clone, Default)]
pub struct ListFramesOptions {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub app_bundle_id: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

#[async_trait]
pub trait FrameRepo: Send + Sync {
    async fn insert(&self, new: NewFrame) -> Result<Frame>;
    async fn by_id(&self, id: &str) -> Result<Option<Frame>>;
    async fn recent(&self, limit: u32) -> Result<Vec<Frame>>;
    async fn list(&self, options: ListFramesOptions) -> Result<Vec<Frame>>;
    async fn last_in_session(&self, capture_session_id: &str) -> Result<Option<Frame>>;
    async fn touch_still_present(&self, id: &str, until: DateTime<Utc>) -> Result<()>;
    async fn count(&self) -> Result<i64>;
    /// Drop every frame captured strictly before `cutoff`. FK cascades
    /// take `frame_embeddings` rows along with them.
    /// Returns the number of frames removed.
    async fn delete_older_than(&self, cutoff: DateTime<Utc>) -> Result<i64>;
    /// Drop every frame captured at or after `cutoff` — the inverse of
    /// `delete_older_than`. Used by the "delete data from the last N
    /// minutes" UX in the privacy popover. Returns the screenshot paths
    /// of the rows that were removed so the caller can clean up the
    /// on-disk files (which live outside SQLite). FK cascades take
    /// `frame_embeddings` rows along with them.
    async fn delete_since(&self, cutoff: DateTime<Utc>) -> Result<Vec<String>>;
    /// FTS5 search over `frames_fts` (BM25 ranked). Phase 3 surface for
    /// the `/ask` recall layer; Phase 4 swaps in hybrid (FTS + vector RRF).
    /// Filters mirror `ListFramesOptions` so the LLM can scope a query
    /// by time and app.
    async fn fts_search(&self, query: &str, options: ListFramesOptions) -> Result<Vec<Frame>>;
    /// Per-app frame counts in a time range. Backs the `list_apps_used`
    /// recall tool.
    async fn list_apps_used(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<AppUsageRow>>;
    /// `(trigger, count)` pairs for frames captured in the last
    /// `minutes_ago` minutes. Backs `capture_diagnostic_status` so the
    /// frontend can answer "is the event driver actually firing real
    /// events?" at a glance. Always returns descending-by-count rows.
    async fn trigger_counts_since_minutes_ago(
        &self,
        minutes_ago: u32,
    ) -> Result<Vec<(String, i64)>>;
    /// `MAX(captured_at)` across the whole table — the timestamp of the
    /// most recent frame, ever. Useful for "last activity" diagnostics.
    async fn most_recent_captured_at(&self) -> Result<Option<DateTime<Utc>>>;
}

/// Aggregate row produced by `list_apps_used`. Shape mirrors the spec
/// §八 tool definition.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AppUsageRow {
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub frame_count: i64,
}

#[derive(Clone)]
pub struct SqliteFrameRepo {
    pool: DbPool,
}

impl SqliteFrameRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

// v1500: 在 ocr_text 之后插入 ax_text_pii_spans。所有列号都向后位移 1，
// 与下面 row_to_frame 中的 row.get(N) 必须保持同步。
const FRAME_COLUMNS: &str = "id, captured_at, device_id, capture_session_id,
        app_bundle_id, app_name, window_title, url,
        screenshot_path, screenshot_hash, screenshot_size_bytes,
        ax_text, ocr_text, ax_text_pii_spans, adapter_name, adapter_payload,
        extraction_strategy, extraction_duration_ms, fallback_reason,
        trigger,
        content_hash, derived_from_frame_id, still_present_until,
        exclusion_match, search_tokens, created_at";

#[async_trait]
impl FrameRepo for SqliteFrameRepo {
    async fn insert(&self, mut new: NewFrame) -> Result<Frame> {
        let id = Ulid::new().to_string();
        let captured_at = DbInstant::from(new.captured_at);
        let created_at = DbInstant::now();
        let return_id = id.clone();
        // If the caller didn't pre-compute search_tokens (snapshot
        // consumer always does; tests + future backfills may not),
        // derive them from the text columns here so the FTS index
        // never sees an empty row for an otherwise-meaningful frame.
        if new.search_tokens.is_none() {
            new.search_tokens = derive_search_tokens(
                new.app_name.as_deref(),
                new.window_title.as_deref(),
                new.url.as_deref(),
                new.ax_text.as_deref(),
                new.ocr_text.as_deref(),
            );
        }
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "INSERT INTO frames (
                    id, captured_at, device_id, capture_session_id,
                    app_bundle_id, app_name, window_title, url,
                    screenshot_path, screenshot_hash, screenshot_size_bytes,
                    ax_text, ocr_text, ax_text_pii_spans, adapter_name, adapter_payload,
                    extraction_strategy, extraction_duration_ms, fallback_reason,
                    trigger,
                    content_hash, exclusion_match, search_tokens, created_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                    ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
                 )",
                params![
                    id,
                    captured_at,
                    new.device_id,
                    new.capture_session_id,
                    new.app_bundle_id,
                    new.app_name,
                    new.window_title,
                    new.url,
                    new.screenshot_path,
                    new.screenshot_hash,
                    new.screenshot_size_bytes,
                    new.ax_text,
                    new.ocr_text,
                    new.ax_text_pii_spans,
                    new.adapter_name,
                    new.adapter_payload,
                    new.extraction_strategy,
                    new.extraction_duration_ms,
                    new.fallback_reason,
                    new.trigger,
                    new.content_hash,
                    new.exclusion_match,
                    new.search_tokens,
                    created_at,
                ],
            )
            .map_err(|error| CorivoError::Internal(format!("插入 frame 失败: {error}")))?;

            let sql = format!("SELECT {FRAME_COLUMNS} FROM frames WHERE id = ?1");
            conn.query_row(&sql, params![return_id], row_to_frame)
                .map_err(|error| CorivoError::Internal(format!("回读 frame 失败: {error}")))
        })
        .await
    }

    async fn by_id(&self, id: &str) -> Result<Option<Frame>> {
        let id = id.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            let sql = format!("SELECT {FRAME_COLUMNS} FROM frames WHERE id = ?1");
            conn.query_row(&sql, params![id], row_to_frame)
                .optional()
                .map_err(|error| CorivoError::Internal(format!("查询 frame 失败: {error}")))
        })
        .await
    }

    async fn recent(&self, limit: u32) -> Result<Vec<Frame>> {
        self.list(ListFramesOptions {
            limit,
            ..Default::default()
        })
        .await
    }

    async fn list(&self, options: ListFramesOptions) -> Result<Vec<Frame>> {
        run_blocking(self.pool.clone(), move |conn| {
            let mut sql = format!("SELECT {FRAME_COLUMNS} FROM frames WHERE 1=1");
            let mut bindings: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            if let Some(from) = options.from {
                sql.push_str(" AND captured_at >= ?");
                bindings.push(Box::new(DbInstant::from(from)));
            }
            if let Some(to) = options.to {
                sql.push_str(" AND captured_at < ?");
                bindings.push(Box::new(DbInstant::from(to)));
            }
            if let Some(app) = options.app_bundle_id {
                sql.push_str(" AND app_bundle_id = ?");
                bindings.push(Box::new(app));
            }
            sql.push_str(" ORDER BY captured_at DESC LIMIT ? OFFSET ?");
            let limit = options.limit.max(1) as i64;
            bindings.push(Box::new(limit));
            bindings.push(Box::new(options.offset as i64));

            let mut statement = conn.prepare(&sql).map_err(|error| {
                CorivoError::Internal(format!("prepare frames list 失败: {error}"))
            })?;
            let params_iter: Vec<&dyn rusqlite::ToSql> =
                bindings.iter().map(|b| b.as_ref()).collect();
            let rows = statement
                .query_map(params_iter.as_slice(), row_to_frame)
                .map_err(|error| {
                    CorivoError::Internal(format!("query frames list 失败: {error}"))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
                CorivoError::Internal(format!("collect frames list 失败: {error}"))
            })
        })
        .await
    }

    async fn last_in_session(&self, capture_session_id: &str) -> Result<Option<Frame>> {
        let session_id = capture_session_id.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            let sql = format!(
                "SELECT {FRAME_COLUMNS} FROM frames
                 WHERE capture_session_id = ?1
                 ORDER BY captured_at DESC LIMIT 1"
            );
            conn.query_row(&sql, params![session_id], row_to_frame)
                .optional()
                .map_err(|error| {
                    CorivoError::Internal(format!("查询 last_in_session 失败: {error}"))
                })
        })
        .await
    }

    async fn touch_still_present(&self, id: &str, until: DateTime<Utc>) -> Result<()> {
        let id = id.to_string();
        let until = DbInstant::from(until);
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "UPDATE frames SET still_present_until = ?1 WHERE id = ?2",
                params![until, id],
            )
            .map_err(|error| {
                CorivoError::Internal(format!("更新 still_present_until 失败: {error}"))
            })?;
            Ok(())
        })
        .await
    }

    async fn count(&self) -> Result<i64> {
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row("SELECT COUNT(*) FROM frames", [], |row| row.get(0))
                .map_err(|error| CorivoError::Internal(format!("count frames 失败: {error}")))
        })
        .await
    }

    async fn delete_older_than(&self, cutoff: DateTime<Utc>) -> Result<i64> {
        let cutoff_db = DbInstant::from(cutoff);
        run_blocking(self.pool.clone(), move |conn| {
            let removed = conn
                .execute(
                    "DELETE FROM frames WHERE captured_at < ?1",
                    params![cutoff_db],
                )
                .map_err(|error| {
                    CorivoError::Internal(format!("delete_older_than 失败: {error}"))
                })?;
            Ok(removed as i64)
        })
        .await
    }

    async fn delete_since(&self, cutoff: DateTime<Utc>) -> Result<Vec<String>> {
        let cutoff_db = DbInstant::from(cutoff);
        run_blocking(self.pool.clone(), move |mut conn| {
            // Two-step inside a transaction: collect the screenshot paths
            // we're about to lose, then delete the rows. Doing it in one
            // transaction means a concurrent reader never sees half-deleted
            // frames, and the caller gets a stable list of files to remove
            // from disk afterwards (the file cleanup happens outside SQLite,
            // outside this lock — see commands::settings::data_delete_range).
            let tx = conn.transaction().map_err(|error| {
                CorivoError::Internal(format!("delete_since begin tx 失败: {error}"))
            })?;

            let mut paths: Vec<String> = Vec::new();
            {
                let mut stmt = tx
                    .prepare(
                        "SELECT screenshot_path FROM frames \
                           WHERE captured_at >= ?1 AND screenshot_path IS NOT NULL",
                    )
                    .map_err(|error| {
                        CorivoError::Internal(format!("delete_since prepare 失败: {error}"))
                    })?;
                let rows = stmt
                    .query_map(params![cutoff_db], |row| row.get::<_, Option<String>>(0))
                    .map_err(|error| {
                        CorivoError::Internal(format!("delete_since query 失败: {error}"))
                    })?;
                for r in rows {
                    if let Some(p) = r.map_err(|error| {
                        CorivoError::Internal(format!("delete_since row 失败: {error}"))
                    })? {
                        paths.push(p);
                    }
                }
            }

            tx.execute(
                "DELETE FROM frames WHERE captured_at >= ?1",
                params![cutoff_db],
            )
            .map_err(|error| {
                CorivoError::Internal(format!("delete_since execute 失败: {error}"))
            })?;

            tx.commit().map_err(|error| {
                CorivoError::Internal(format!("delete_since commit 失败: {error}"))
            })?;
            Ok(paths)
        })
        .await
    }

    async fn trigger_counts_since_minutes_ago(
        &self,
        minutes_ago: u32,
    ) -> Result<Vec<(String, i64)>> {
        let cutoff = crate::db::time::now_utc() - chrono::Duration::minutes(minutes_ago as i64);
        let cutoff_db = DbInstant::from(cutoff);
        run_blocking(self.pool.clone(), move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT trigger, COUNT(*) FROM frames
                       WHERE captured_at >= ?1
                       GROUP BY trigger
                       ORDER BY COUNT(*) DESC",
                )
                .map_err(|error| {
                    CorivoError::Internal(format!("trigger_counts prepare 失败: {error}"))
                })?;
            let rows = stmt
                .query_map(params![cutoff_db], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(|error| {
                    CorivoError::Internal(format!("trigger_counts query 失败: {error}"))
                })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.map_err(|error| {
                    CorivoError::Internal(format!("trigger_counts row 失败: {error}"))
                })?);
            }
            Ok(out)
        })
        .await
    }

    async fn most_recent_captured_at(&self) -> Result<Option<DateTime<Utc>>> {
        run_blocking(self.pool.clone(), move |conn| {
            let inst = conn
                .query_row("SELECT MAX(captured_at) FROM frames", [], |row| {
                    row.get::<_, Option<DbInstant>>(0)
                })
                .map_err(|error| {
                    CorivoError::Internal(format!("most_recent_captured_at 失败: {error}"))
                })?;
            Ok(inst.map(DateTime::<Utc>::from))
        })
        .await
    }

    async fn fts_search(&self, query: &str, options: ListFramesOptions) -> Result<Vec<Frame>> {
        let query_owned = query.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            // FTS5 expects a MATCH expression; we sanitise to a phrase
            // so the LLM can pass user-facing strings without learning
            // the FTS5 mini-syntax. Empty / whitespace-only queries
            // degrade to "list all" by skipping the MATCH filter.
            let trimmed = query_owned.trim();
            // `frames` and `frames_fts` share several column names
            // (app_name, window_title, …); qualify the SELECT with the
            // base-table prefix so the join doesn't trip over them.
            let qualified = FRAME_COLUMNS
                .split(',')
                .map(|c| format!("frames.{}", c.trim()))
                .collect::<Vec<_>>()
                .join(", ");
            let mut sql = format!(
                "SELECT {qualified} FROM frames
                 JOIN frames_fts ON frames_fts.rowid = frames.rowid
                 WHERE 1=1"
            );
            let mut bindings: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            // jieba-segment the query into an OR-joined phrase
            // expression. Falling back to the legacy single-phrase
            // form when jieba returns nothing (rare — only for inputs
            // that are pure punctuation) keeps the bind nonempty so
            // FTS5 doesn't blow up on `MATCH ''`.
            let match_expr = if trimmed.is_empty() {
                None
            } else {
                Some(
                    crate::services::tokenize::tokenize_for_query(trimmed)
                        .unwrap_or_else(|| fts_phrase(trimmed)),
                )
            };
            if let Some(expr) = match_expr.as_ref() {
                sql.push_str(" AND frames_fts MATCH ?");
                bindings.push(Box::new(expr.clone()));
            }
            if let Some(from) = options.from {
                sql.push_str(" AND frames.captured_at >= ?");
                bindings.push(Box::new(DbInstant::from(from)));
            }
            if let Some(to) = options.to {
                sql.push_str(" AND frames.captured_at < ?");
                bindings.push(Box::new(DbInstant::from(to)));
            }
            if let Some(app) = options.app_bundle_id {
                sql.push_str(" AND frames.app_bundle_id = ?");
                bindings.push(Box::new(app));
            }
            // BM25 score: lower is better, so ASC.
            if !trimmed.is_empty() {
                sql.push_str(" ORDER BY bm25(frames_fts) ASC LIMIT ? OFFSET ?");
            } else {
                sql.push_str(" ORDER BY frames.captured_at DESC LIMIT ? OFFSET ?");
            }
            let limit = options.limit.max(1) as i64;
            bindings.push(Box::new(limit));
            bindings.push(Box::new(options.offset as i64));

            let mut stmt = conn.prepare(&sql).map_err(|error| {
                CorivoError::Internal(format!("prepare frames fts search 失败: {error}"))
            })?;
            let params_iter: Vec<&dyn rusqlite::ToSql> =
                bindings.iter().map(|b| b.as_ref()).collect();
            let rows = stmt
                .query_map(params_iter.as_slice(), row_to_frame)
                .map_err(|error| {
                    CorivoError::Internal(format!("query frames fts search 失败: {error}"))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
                CorivoError::Internal(format!("collect frames fts search 失败: {error}"))
            })
        })
        .await
    }

    async fn list_apps_used(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<AppUsageRow>> {
        let from = DbInstant::from(from);
        let to = DbInstant::from(to);
        run_blocking(self.pool.clone(), move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT app_bundle_id, MAX(app_name) AS app_name, COUNT(*) AS frame_count
                     FROM frames
                     WHERE captured_at >= ?1 AND captured_at < ?2
                       AND app_bundle_id IS NOT NULL
                     GROUP BY app_bundle_id
                     ORDER BY frame_count DESC",
                )
                .map_err(|error| {
                    CorivoError::Internal(format!("prepare list_apps_used 失败: {error}"))
                })?;
            let rows = stmt
                .query_map(rusqlite::params![from, to], |row| {
                    Ok(AppUsageRow {
                        app_bundle_id: row.get(0)?,
                        app_name: row.get(1)?,
                        frame_count: row.get(2)?,
                    })
                })
                .map_err(|error| {
                    CorivoError::Internal(format!("query list_apps_used 失败: {error}"))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
                CorivoError::Internal(format!("collect list_apps_used 失败: {error}"))
            })
        })
        .await
    }
}

/// Wrap user input as an FTS5 phrase, escaping embedded quotes. Keeps
/// the LLM out of FTS5 mini-syntax (NEAR / OR / column filters) — Phase
/// 3 wants stupid + safe, Phase 4 can layer richer parsing on top.
fn fts_phrase(raw: &str) -> String {
    let escaped = raw.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

/// Compose the FTS5 input from a frame's text-bearing fields and pass
/// it through jieba so CJK queries match through unicode61. Mirrors
/// `services::snapshot_consumer::local::build_search_tokens` for the
/// "caller didn't precompute" code path.
fn derive_search_tokens(
    app_name: Option<&str>,
    window_title: Option<&str>,
    url: Option<&str>,
    ax_text: Option<&str>,
    ocr_text: Option<&str>,
) -> Option<String> {
    let mut buf = String::new();
    let mut push = |s: &str| {
        let t = s.trim();
        if t.is_empty() {
            return;
        }
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(t);
    };
    if let Some(s) = app_name {
        push(s);
    }
    if let Some(s) = window_title {
        push(s);
    }
    if let Some(s) = url {
        push(s);
    }
    if let Some(s) = ax_text {
        push(s);
    }
    if let Some(s) = ocr_text {
        push(s);
    }
    if buf.is_empty() {
        return None;
    }
    let tokens = crate::services::tokenize::tokenize_for_index(&buf);
    if tokens.is_empty() {
        None
    } else {
        Some(tokens)
    }
}

fn row_to_frame(row: &rusqlite::Row<'_>) -> rusqlite::Result<Frame> {
    // 列序参见上方 FRAME_COLUMNS；v1500 在 ocr_text 之后插入
    // ax_text_pii_spans，13 之后全部 +1。
    Ok(Frame {
        id: row.get(0)?,
        captured_at: row.get::<_, DbInstant>(1)?.into_inner(),
        device_id: row.get(2)?,
        capture_session_id: row.get(3)?,
        app_bundle_id: row.get(4)?,
        app_name: row.get(5)?,
        window_title: row.get(6)?,
        url: row.get(7)?,
        screenshot_path: row.get(8)?,
        screenshot_hash: row.get(9)?,
        screenshot_size_bytes: row.get(10)?,
        ax_text: row.get(11)?,
        ocr_text: row.get(12)?,
        ax_text_pii_spans: row.get(13)?,
        adapter_name: row.get(14)?,
        adapter_payload: row.get(15)?,
        extraction_strategy: row.get(16)?,
        extraction_duration_ms: row.get(17)?,
        fallback_reason: row.get(18)?,
        trigger: row.get(19)?,
        content_hash: row.get(20)?,
        derived_from_frame_id: row.get(21)?,
        still_present_until: row.get::<_, Option<DbInstant>>(22)?.map(|d| d.into_inner()),
        exclusion_match: row.get(23)?,
        search_tokens: row.get(24)?,
        created_at: row.get::<_, DbInstant>(25)?.into_inner(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::test_in_memory_pool;

    const SCHEMA_SQL: &str = include_str!("../schema.sql");

    fn init_pool() -> DbPool {
        let pool = test_in_memory_pool().unwrap();
        let conn = pool.get().unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
        pool
    }

    fn sample_new_frame() -> NewFrame {
        NewFrame {
            captured_at: Utc::now(),
            device_id: "dev-1".into(),
            capture_session_id: "sess-1".into(),
            app_bundle_id: Some("com.example".into()),
            app_name: Some("Example".into()),
            window_title: Some("hello".into()),
            url: None,
            screenshot_path: Some("captures/2026/04/27/abc.jpg".into()),
            screenshot_hash: Some("sha256:abc".into()),
            screenshot_size_bytes: Some(1234),
            ax_text: None,
            ocr_text: Some("hello world".into()),
            ax_text_pii_spans: None,
            adapter_name: None,
            adapter_payload: None,
            extraction_strategy: "ocr".into(),
            extraction_duration_ms: Some(87),
            fallback_reason: None,
            trigger: NewFrame::default_trigger().to_string(),
            content_hash: Some("hash-1".into()),
            exclusion_match: None,
            search_tokens: None,
        }
    }

    #[tokio::test]
    async fn insert_and_round_trip_via_by_id() {
        let pool = init_pool();
        let repo = SqliteFrameRepo::new(pool);
        let inserted = repo.insert(sample_new_frame()).await.unwrap();
        assert_eq!(inserted.app_name.as_deref(), Some("Example"));

        let fetched = repo.by_id(&inserted.id).await.unwrap().unwrap();
        assert_eq!(fetched, inserted);
    }

    #[tokio::test]
    async fn recent_returns_descending_by_captured_at() {
        let pool = init_pool();
        let repo = SqliteFrameRepo::new(pool);

        let earlier = repo.insert(sample_new_frame()).await.unwrap();
        let mut later_input = sample_new_frame();
        later_input.captured_at = earlier.captured_at + chrono::Duration::seconds(10);
        let later = repo.insert(later_input).await.unwrap();

        let rows = repo.recent(10).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, later.id);
        assert_eq!(rows[1].id, earlier.id);
    }

    #[tokio::test]
    async fn list_filters_by_app_bundle_id() {
        let pool = init_pool();
        let repo = SqliteFrameRepo::new(pool);

        repo.insert(sample_new_frame()).await.unwrap();
        let mut other = sample_new_frame();
        other.app_bundle_id = Some("com.other".into());
        repo.insert(other).await.unwrap();

        let opts = ListFramesOptions {
            app_bundle_id: Some("com.other".into()),
            limit: 10,
            ..Default::default()
        };
        let rows = repo.list(opts).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].app_bundle_id.as_deref(), Some("com.other"));
    }

    #[tokio::test]
    async fn last_in_session_picks_most_recent_in_session() {
        let pool = init_pool();
        let repo = SqliteFrameRepo::new(pool);

        let first = repo.insert(sample_new_frame()).await.unwrap();
        let mut later_input = sample_new_frame();
        later_input.captured_at = first.captured_at + chrono::Duration::seconds(5);
        let later = repo.insert(later_input).await.unwrap();

        let mut other_session = sample_new_frame();
        other_session.capture_session_id = "sess-other".into();
        other_session.captured_at = first.captured_at + chrono::Duration::seconds(99);
        repo.insert(other_session).await.unwrap();

        let last = repo.last_in_session("sess-1").await.unwrap().unwrap();
        assert_eq!(last.id, later.id);
    }

    #[tokio::test]
    async fn touch_still_present_updates_column_in_place() {
        let pool = init_pool();
        let repo = SqliteFrameRepo::new(pool);
        let frame = repo.insert(sample_new_frame()).await.unwrap();
        assert!(frame.still_present_until.is_none());

        let until = frame.captured_at + chrono::Duration::seconds(30);
        repo.touch_still_present(&frame.id, until).await.unwrap();
        let after = repo.by_id(&frame.id).await.unwrap().unwrap();
        assert!(after.still_present_until.is_some());
    }

    #[tokio::test]
    async fn count_reflects_inserts() {
        let pool = init_pool();
        let repo = SqliteFrameRepo::new(pool);
        assert_eq!(repo.count().await.unwrap(), 0);
        repo.insert(sample_new_frame()).await.unwrap();
        repo.insert(sample_new_frame()).await.unwrap();
        assert_eq!(repo.count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn fts_search_matches_token_in_ocr_text() {
        let pool = init_pool();
        let repo = SqliteFrameRepo::new(pool);

        let mut a = sample_new_frame();
        a.ocr_text = Some("hello kitchen world".into());
        let mut b = sample_new_frame();
        b.ocr_text = Some("ignore me".into());
        repo.insert(a).await.unwrap();
        repo.insert(b).await.unwrap();

        let opts = ListFramesOptions {
            limit: 10,
            ..Default::default()
        };
        let hits = repo.fts_search("kitchen", opts).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0]
            .ocr_text
            .as_deref()
            .unwrap_or_default()
            .contains("kitchen"));
    }

    #[tokio::test]
    async fn fts_search_matches_chinese_substring_via_jieba() {
        let pool = init_pool();
        let repo = SqliteFrameRepo::new(pool);

        let mut a = sample_new_frame();
        a.ocr_text = Some("用户处理事务时倾向于先关注沟通信息".into());
        let mut b = sample_new_frame();
        b.ocr_text = Some("无关的中文文本".into());
        repo.insert(a).await.unwrap();
        repo.insert(b).await.unwrap();

        let opts = ListFramesOptions {
            limit: 10,
            ..Default::default()
        };
        // Without jieba, unicode61 sees the whole CJK string as one
        // token, so a substring query like "沟通" misses entirely.
        // With jieba writing space-separated tokens to search_tokens,
        // it finds the right row.
        let hits = repo.fts_search("沟通", opts).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0]
            .ocr_text
            .as_deref()
            .unwrap_or_default()
            .contains("沟通"));
    }

    #[tokio::test]
    async fn fts_search_with_empty_query_falls_back_to_recent() {
        let pool = init_pool();
        let repo = SqliteFrameRepo::new(pool);
        repo.insert(sample_new_frame()).await.unwrap();
        repo.insert(sample_new_frame()).await.unwrap();
        let opts = ListFramesOptions {
            limit: 10,
            ..Default::default()
        };
        let hits = repo.fts_search("   ", opts).await.unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[tokio::test]
    async fn list_apps_used_groups_by_bundle_id_descending() {
        let pool = init_pool();
        let repo = SqliteFrameRepo::new(pool);

        // 3x com.example, 1x com.other, 1x null bundle id.
        for _ in 0..3 {
            repo.insert(sample_new_frame()).await.unwrap();
        }
        let mut other = sample_new_frame();
        other.app_bundle_id = Some("com.other".into());
        repo.insert(other).await.unwrap();
        let mut anon = sample_new_frame();
        anon.app_bundle_id = None;
        repo.insert(anon).await.unwrap();

        let from = Utc::now() - chrono::Duration::hours(1);
        let to = Utc::now() + chrono::Duration::hours(1);
        let rows = repo.list_apps_used(from, to).await.unwrap();
        assert_eq!(rows.len(), 2, "null bundle ids must be excluded");
        assert_eq!(rows[0].app_bundle_id.as_deref(), Some("com.example"));
        assert_eq!(rows[0].frame_count, 3);
        assert_eq!(rows[1].app_bundle_id.as_deref(), Some("com.other"));
    }

    #[tokio::test]
    async fn delete_since_drops_recent_frames_and_returns_paths() {
        let pool = init_pool();
        let repo = SqliteFrameRepo::new(pool);

        // Three frames straddling the cutoff: two at-or-after, one before.
        let now = Utc::now();
        let mut old = sample_new_frame();
        old.captured_at = now - chrono::Duration::minutes(30);
        old.screenshot_path = Some("captures/old.jpg".into());
        repo.insert(old).await.unwrap();

        let mut a = sample_new_frame();
        a.captured_at = now - chrono::Duration::minutes(2);
        a.screenshot_path = Some("captures/a.jpg".into());
        repo.insert(a).await.unwrap();

        let mut b = sample_new_frame();
        b.captured_at = now;
        b.screenshot_path = Some("captures/b.jpg".into());
        repo.insert(b).await.unwrap();

        let cutoff = now - chrono::Duration::minutes(5);
        let paths = repo.delete_since(cutoff).await.unwrap();

        assert_eq!(paths.len(), 2, "two frames are within the window");
        assert!(paths.contains(&"captures/a.jpg".to_string()));
        assert!(paths.contains(&"captures/b.jpg".to_string()));
        assert_eq!(repo.count().await.unwrap(), 1, "the old frame stays");
    }

    #[tokio::test]
    async fn delete_since_with_far_future_cutoff_keeps_everything() {
        let pool = init_pool();
        let repo = SqliteFrameRepo::new(pool);
        repo.insert(sample_new_frame()).await.unwrap();
        repo.insert(sample_new_frame()).await.unwrap();

        let paths = repo
            .delete_since(Utc::now() + chrono::Duration::days(7))
            .await
            .unwrap();
        assert!(paths.is_empty());
        assert_eq!(repo.count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn delete_since_with_far_past_cutoff_removes_everything() {
        let pool = init_pool();
        let repo = SqliteFrameRepo::new(pool);
        repo.insert(sample_new_frame()).await.unwrap();
        repo.insert(sample_new_frame()).await.unwrap();

        let paths = repo
            .delete_since(Utc::now() - chrono::Duration::days(365))
            .await
            .unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(repo.count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn delete_since_omits_null_screenshot_paths_from_returned_list() {
        // AX-only frames (or future no-screenshot pipelines) leave
        // `screenshot_path` NULL. The delete still has to drop the row,
        // but the returned paths list must not contain NULL/empty —
        // it's fed directly into a `fs::remove_file` loop.
        let pool = init_pool();
        let repo = SqliteFrameRepo::new(pool);

        let now = Utc::now();
        let mut with_shot = sample_new_frame();
        with_shot.captured_at = now;
        with_shot.screenshot_path = Some("captures/with.jpg".into());
        repo.insert(with_shot).await.unwrap();

        let mut without_shot = sample_new_frame();
        without_shot.captured_at = now;
        without_shot.screenshot_path = None;
        repo.insert(without_shot).await.unwrap();

        let paths = repo
            .delete_since(now - chrono::Duration::minutes(1))
            .await
            .unwrap();

        assert_eq!(paths, vec!["captures/with.jpg".to_string()]);
        assert_eq!(repo.count().await.unwrap(), 0, "both rows are gone");
    }
}
