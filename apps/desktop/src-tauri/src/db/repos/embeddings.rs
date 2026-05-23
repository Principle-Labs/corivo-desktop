//! `frame_embeddings` repo (Phase 4).
//!
//! Three operations the indexer + recall layer need:
//!   - `unembedded_frames(limit)` — batch input for the background
//!     indexer; finds frames with text but no embedding row yet.
//!   - `upsert(new)` — write or replace the embedding (re-embedding a
//!     frame is rare but valid when the model upgrades).
//!   - `all_with_filter(filter)` — full-table scan over (frame_id, model,
//!     vector) for the cosine pass; the caller scores in Rust because
//!     SQLite has no native cosine UDF.

use async_trait::async_trait;
use rusqlite::params;

use crate::{
    db::{
        pool::{run_blocking, DbPool},
        time::DbInstant,
    },
    domain::{
        embedding::{decode_vector, encode_vector, FrameEmbedding, NewFrameEmbedding},
        frame::Frame,
    },
    error::{CorivoError, Result},
};

/// A frame the indexer needs to embed: id + the text we'll feed the
/// embedder. Picks `ax_text` first, falls back to `ocr_text` — same
/// preference the FTS search uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnembeddedFrame {
    pub id: String,
    pub text: String,
}

/// Filter for the cosine-scan path: restrict by time / app to match
/// what `frames.fts_search` does, so hybrid_search hits a comparable
/// candidate set on both sides of the RRF fusion.
#[derive(Debug, Clone, Default)]
pub struct EmbeddingScanOptions {
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    pub app_bundle_id: Option<String>,
    /// If set, restrict to embeddings produced by this exact model.
    pub model: Option<String>,
}

/// One row materialised for cosine scoring. Frame metadata is included
/// so the caller can avoid a second round-trip per hit.
#[derive(Debug, Clone)]
pub struct EmbeddingScanRow {
    pub frame: Frame,
    pub vector: Vec<f32>,
}

#[async_trait]
pub trait FrameEmbeddingRepo: Send + Sync {
    async fn upsert(&self, new: NewFrameEmbedding) -> Result<FrameEmbedding>;
    async fn by_frame_id(&self, frame_id: &str) -> Result<Option<FrameEmbedding>>;
    async fn unembedded_frames(&self, limit: u32) -> Result<Vec<UnembeddedFrame>>;
    async fn scan_with_vectors(
        &self,
        options: EmbeddingScanOptions,
    ) -> Result<Vec<EmbeddingScanRow>>;
    async fn count(&self) -> Result<i64>;
}

#[derive(Clone)]
pub struct SqliteFrameEmbeddingRepo {
    pool: DbPool,
}

impl SqliteFrameEmbeddingRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

// v1500: 镜像 repos::frames::FRAME_COLUMNS 的列序（在 ocr_text 之后
// 插入 ax_text_pii_spans）。两份常量要保持同步 —— 后续抽到 db/repos/mod.rs
// 的共享 helper 里是 TODO。
const FRAME_COLUMNS: &str = "id, captured_at, device_id, capture_session_id,
        app_bundle_id, app_name, window_title, url,
        screenshot_path, screenshot_hash, screenshot_size_bytes,
        ax_text, ocr_text, ax_text_pii_spans, adapter_name, adapter_payload,
        extraction_strategy, extraction_duration_ms, fallback_reason,
        trigger,
        content_hash, derived_from_frame_id, still_present_until,
        exclusion_match, search_tokens, created_at";

#[async_trait]
impl FrameEmbeddingRepo for SqliteFrameEmbeddingRepo {
    async fn upsert(&self, new: NewFrameEmbedding) -> Result<FrameEmbedding> {
        let now = DbInstant::now();
        let dimensions = new.vector.len() as i64;
        let bytes = encode_vector(&new.vector);
        let frame_id = new.frame_id.clone();
        let model = new.model.clone();
        let return_id = frame_id.clone();
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "INSERT INTO frame_embeddings
                   (frame_id, model, dimensions, vector, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(frame_id) DO UPDATE SET
                   model = excluded.model,
                   dimensions = excluded.dimensions,
                   vector = excluded.vector,
                   created_at = excluded.created_at",
                params![frame_id, model, dimensions, bytes, now],
            )
            .map_err(|error| {
                CorivoError::Internal(format!("upsert frame_embedding 失败: {error}"))
            })?;
            conn.query_row(
                "SELECT frame_id, model, dimensions, vector, created_at
                 FROM frame_embeddings WHERE frame_id = ?1",
                params![return_id],
                row_to_embedding,
            )
            .map_err(|error| CorivoError::Internal(format!("回读 frame_embedding 失败: {error}")))
        })
        .await
    }

    async fn by_frame_id(&self, frame_id: &str) -> Result<Option<FrameEmbedding>> {
        let id = frame_id.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row(
                "SELECT frame_id, model, dimensions, vector, created_at
                 FROM frame_embeddings WHERE frame_id = ?1",
                params![id],
                row_to_embedding,
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(CorivoError::Internal(format!(
                    "查询 frame_embedding 失败: {other}"
                ))),
            })
        })
        .await
    }

    async fn unembedded_frames(&self, limit: u32) -> Result<Vec<UnembeddedFrame>> {
        let limit = limit.max(1) as i64;
        run_blocking(self.pool.clone(), move |conn| {
            // LEFT JOIN + IS NULL is the standard "anti-join" pattern.
            // Restrict to frames that actually have text — there's
            // nothing to embed for `Skipped` strategies.
            let mut stmt = conn
                .prepare(
                    "SELECT f.id,
                            COALESCE(NULLIF(f.ax_text, ''), f.ocr_text) AS text
                     FROM frames f
                     LEFT JOIN frame_embeddings e ON e.frame_id = f.id
                     WHERE e.frame_id IS NULL
                       AND COALESCE(NULLIF(f.ax_text, ''), f.ocr_text) IS NOT NULL
                       AND f.extraction_strategy != 'skipped'
                     ORDER BY f.captured_at ASC
                     LIMIT ?1",
                )
                .map_err(|error| {
                    CorivoError::Internal(format!("prepare unembedded_frames 失败: {error}"))
                })?;
            let rows = stmt
                .query_map(params![limit], |row| {
                    Ok(UnembeddedFrame {
                        id: row.get(0)?,
                        text: row.get::<_, String>(1)?,
                    })
                })
                .map_err(|error| {
                    CorivoError::Internal(format!("query unembedded_frames 失败: {error}"))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
                CorivoError::Internal(format!("collect unembedded_frames 失败: {error}"))
            })
        })
        .await
    }

    async fn scan_with_vectors(
        &self,
        options: EmbeddingScanOptions,
    ) -> Result<Vec<EmbeddingScanRow>> {
        run_blocking(self.pool.clone(), move |conn| {
            // Both `frames` and `frame_embeddings` have `created_at` —
            // qualify each frame column so SQLite doesn't reject the
            // join with "ambiguous column name".
            let qualified = FRAME_COLUMNS
                .split(',')
                .map(|c| format!("frames.{}", c.trim()))
                .collect::<Vec<_>>()
                .join(", ");
            let mut sql = format!(
                "SELECT {qualified}, e.vector
                 FROM frame_embeddings e
                 JOIN frames ON frames.id = e.frame_id
                 WHERE 1=1"
            );
            let mut bindings: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            if let Some(model) = options.model {
                sql.push_str(" AND e.model = ?");
                bindings.push(Box::new(model));
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

            let mut stmt = conn.prepare(&sql).map_err(|error| {
                CorivoError::Internal(format!("prepare scan_with_vectors 失败: {error}"))
            })?;
            let params_iter: Vec<&dyn rusqlite::ToSql> =
                bindings.iter().map(|b| b.as_ref()).collect();
            let rows = stmt
                .query_map(params_iter.as_slice(), row_to_scan_row)
                .map_err(|error| {
                    CorivoError::Internal(format!("query scan_with_vectors 失败: {error}"))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
                CorivoError::Internal(format!("collect scan_with_vectors 失败: {error}"))
            })
        })
        .await
    }

    async fn count(&self) -> Result<i64> {
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row("SELECT COUNT(*) FROM frame_embeddings", [], |row| {
                row.get(0)
            })
            .map_err(|error| CorivoError::Internal(format!("count frame_embeddings 失败: {error}")))
        })
        .await
    }
}

fn row_to_embedding(row: &rusqlite::Row<'_>) -> rusqlite::Result<FrameEmbedding> {
    let frame_id: String = row.get(0)?;
    let model: String = row.get(1)?;
    let _dimensions: i64 = row.get(2)?;
    let bytes: Vec<u8> = row.get(3)?;
    let vector = decode_vector(&bytes).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Blob,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "frame_embedding vector blob is not a multiple of 4 bytes",
            )),
        )
    })?;
    let created_at = row.get::<_, DbInstant>(4)?.into_inner();
    Ok(FrameEmbedding {
        frame_id,
        model,
        vector,
        created_at,
    })
}

fn row_to_scan_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EmbeddingScanRow> {
    // 与 repos::frames::row_to_frame 列序一致 —— v1500 在 ocr_text 之后
    // 插入 ax_text_pii_spans，13 之后全部 +1，e.vector 从 25 移到 26。
    let frame = crate::domain::frame::Frame {
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
    };
    let vector_bytes: Vec<u8> = row.get(26)?;
    let vector = decode_vector(&vector_bytes).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            26,
            rusqlite::types::Type::Blob,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "scan_with_vectors vector blob is not a multiple of 4 bytes",
            )),
        )
    })?;
    Ok(EmbeddingScanRow { frame, vector })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repos::frames::FrameRepo;
    use crate::db::{pool::test_in_memory_pool, repos::frames::SqliteFrameRepo};
    use crate::domain::frame::NewFrame;
    use chrono::Utc;

    const SCHEMA_SQL: &str = include_str!("../schema.sql");

    fn init_pool() -> DbPool {
        let pool = test_in_memory_pool().unwrap();
        let conn = pool.get().unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
        pool
    }

    fn sample_new_frame(text: &str) -> NewFrame {
        NewFrame {
            captured_at: Utc::now(),
            device_id: "dev".into(),
            capture_session_id: "sess".into(),
            app_bundle_id: Some("com.example".into()),
            app_name: Some("Example".into()),
            window_title: Some("hi".into()),
            url: None,
            screenshot_path: None,
            screenshot_hash: None,
            screenshot_size_bytes: None,
            ax_text: None,
            ocr_text: Some(text.into()),
            ax_text_pii_spans: None,
            adapter_name: None,
            adapter_payload: None,
            extraction_strategy: "ocr".into(),
            extraction_duration_ms: Some(50),
            fallback_reason: None,
            trigger: NewFrame::default_trigger().to_string(),
            content_hash: None,
            exclusion_match: None,
            search_tokens: None,
        }
    }

    #[tokio::test]
    async fn upsert_round_trips_via_by_frame_id() {
        let pool = init_pool();
        let frames = SqliteFrameRepo::new(pool.clone());
        let repo = SqliteFrameEmbeddingRepo::new(pool);

        let f = frames.insert(sample_new_frame("hello")).await.unwrap();
        let v = vec![0.1_f32, 0.2, -0.3, 0.4];
        let inserted = repo
            .upsert(NewFrameEmbedding {
                frame_id: f.id.clone(),
                model: "test-model".into(),
                vector: v.clone(),
            })
            .await
            .unwrap();
        assert_eq!(inserted.frame_id, f.id);
        assert_eq!(inserted.vector, v);

        let fetched = repo.by_frame_id(&f.id).await.unwrap().unwrap();
        assert_eq!(fetched, inserted);
    }

    #[tokio::test]
    async fn upsert_replaces_existing_row() {
        let pool = init_pool();
        let frames = SqliteFrameRepo::new(pool.clone());
        let repo = SqliteFrameEmbeddingRepo::new(pool);

        let f = frames.insert(sample_new_frame("hello")).await.unwrap();
        repo.upsert(NewFrameEmbedding {
            frame_id: f.id.clone(),
            model: "old".into(),
            vector: vec![1.0, 2.0],
        })
        .await
        .unwrap();
        repo.upsert(NewFrameEmbedding {
            frame_id: f.id.clone(),
            model: "new".into(),
            vector: vec![3.0, 4.0, 5.0],
        })
        .await
        .unwrap();

        let after = repo.by_frame_id(&f.id).await.unwrap().unwrap();
        assert_eq!(after.model, "new");
        assert_eq!(after.vector, vec![3.0, 4.0, 5.0]);
        assert_eq!(repo.count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn unembedded_frames_skips_already_embedded_and_skipped() {
        let pool = init_pool();
        let frames = SqliteFrameRepo::new(pool.clone());
        let repo = SqliteFrameEmbeddingRepo::new(pool);

        let f1 = frames.insert(sample_new_frame("a")).await.unwrap();
        let f2 = frames.insert(sample_new_frame("b")).await.unwrap();
        let mut skipped = sample_new_frame("");
        skipped.ocr_text = None;
        skipped.extraction_strategy = "skipped".into();
        let _f3 = frames.insert(skipped).await.unwrap();

        // Mark f1 as already embedded.
        repo.upsert(NewFrameEmbedding {
            frame_id: f1.id.clone(),
            model: "m".into(),
            vector: vec![0.0],
        })
        .await
        .unwrap();

        let pending = repo.unembedded_frames(10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, f2.id);
    }

    #[tokio::test]
    async fn scan_with_vectors_returns_frame_plus_vector() {
        let pool = init_pool();
        let frames = SqliteFrameRepo::new(pool.clone());
        let repo = SqliteFrameEmbeddingRepo::new(pool);

        let f = frames.insert(sample_new_frame("hello")).await.unwrap();
        repo.upsert(NewFrameEmbedding {
            frame_id: f.id.clone(),
            model: "test".into(),
            vector: vec![0.1, 0.2, 0.3],
        })
        .await
        .unwrap();

        let rows = repo
            .scan_with_vectors(EmbeddingScanOptions::default())
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].frame.id, f.id);
        assert_eq!(rows[0].vector, vec![0.1, 0.2, 0.3]);
    }
}
