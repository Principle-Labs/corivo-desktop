//! `notes` repo (memory-system-spec §3).
//!
//! The four lifecycle commands the rest of the codebase needs:
//!
//! * `create` — write a fresh note. `save_note` native tool uses it
//!   inline during a user turn; `commands::memory::create_note`
//!   uses it from the Settings UI.
//! * `list` — filtered enumerator. Used by both the persistent-block
//!   loader (`scope=global`, `status=active`) and the Settings list.
//! * `update` — patch content / status / confidence (e.g. promote a
//!   `suggested` note to `active`).
//! * `delete` — hard delete. The user controls the persistent prompt
//!   surface; archiving stays as a soft option via `update`.

use async_trait::async_trait;
use rusqlite::{params, OptionalExtension};
use ulid::Ulid;

use crate::{
    db::{
        pool::{run_blocking, DbPool},
        time::DbInstant,
    },
    domain::note::{Note, NoteScope, NoteSourceType, NoteStatus},
    error::{CorivoError, Result},
};

#[derive(Debug, Clone)]
pub struct NewNote {
    pub content: String,
    pub scope: NoteScope,
    pub scope_ref: Option<String>,
    pub source_type: NoteSourceType,
    pub source_message_id: Option<String>,
    pub source_thread_id: Option<String>,
    /// `None` → default by source (1.0 for user_explicit, 0.6 for
    /// agent_inferred).
    pub confidence: Option<f32>,
    /// `None` → default by source (active for user_explicit, suggested
    /// for agent_inferred).
    pub status: Option<NoteStatus>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct ListNotesOptions {
    pub scope: Option<NoteScope>,
    pub scope_ref: Option<String>,
    pub status: Option<NoteStatus>,
    pub source_type: Option<NoteSourceType>,
    /// `None` → unbounded.
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateNote {
    pub content: Option<String>,
    pub status: Option<NoteStatus>,
    pub confidence: Option<f32>,
    pub superseded_by: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NoteHit {
    pub note: Note,
    /// Raw BM25 score (already negated so higher is better; the recall
    /// layer multiplies recency / scope boosts on top).
    pub score: f32,
}

#[async_trait]
pub trait NotesRepo: Send + Sync {
    async fn create(&self, new: NewNote) -> Result<Note>;
    async fn list(&self, opts: ListNotesOptions) -> Result<Vec<Note>>;
    async fn by_id(&self, id: &str) -> Result<Option<Note>>;
    async fn update(&self, id: &str, patch: UpdateNote) -> Result<Note>;
    async fn delete(&self, id: &str) -> Result<()>;
    /// Stamp `last_referenced_at = now()` — used by the recall layer
    /// when a note is surfaced to the prompt so we can later compute
    /// "things you told me but never came up again".
    async fn touch_referenced(&self, id: &str) -> Result<()>;
    /// FTS5 over `notes_fts`. `fts_query` is the already-tokenized,
    /// FTS5-syntax-safe expression (typically built by
    /// `services::tokenize::tokenize_for_query`). `status_filter` /
    /// `scope_filter` apply application-side filters before scoring.
    async fn fts_search(
        &self,
        fts_query: &str,
        status_filter: Option<NoteStatus>,
        scope_filter: Option<NoteScope>,
        limit: u32,
    ) -> Result<Vec<NoteHit>>;
}

#[derive(Clone)]
pub struct SqliteNotesRepo {
    pool: DbPool,
}

impl SqliteNotesRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

const NOTE_COLUMNS: &str = "id, content, scope, scope_ref, source_type, \
     source_message_id, source_thread_id, confidence, status, superseded_by, \
     created_at, updated_at, last_referenced_at, expires_at";

fn derive_search_tokens(content: &str) -> String {
    crate::services::tokenize::tokenize_for_index(content)
}

#[async_trait]
impl NotesRepo for SqliteNotesRepo {
    async fn create(&self, new: NewNote) -> Result<Note> {
        let id = Ulid::new().to_string();
        let now = DbInstant::now();
        let return_id = id.clone();
        let confidence = new.confidence.unwrap_or(match new.source_type {
            NoteSourceType::UserExplicit => 1.0,
            NoteSourceType::AgentInferred => 0.6,
        });
        let status = new.status.unwrap_or(match new.source_type {
            NoteSourceType::UserExplicit => NoteStatus::Active,
            NoteSourceType::AgentInferred => NoteStatus::Suggested,
        });
        let scope = new.scope.as_str().to_string();
        let source_type = new.source_type.as_str().to_string();
        let status_str = status.as_str().to_string();
        let expires_at = new.expires_at.map(DbInstant::from);

        let search_tokens = derive_search_tokens(&new.content);
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "INSERT INTO notes
                   (id, content, search_tokens, scope, scope_ref, source_type,
                    source_message_id, source_thread_id, confidence,
                    status, created_at, updated_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?12)",
                params![
                    id,
                    new.content,
                    search_tokens,
                    scope,
                    new.scope_ref,
                    source_type,
                    new.source_message_id,
                    new.source_thread_id,
                    confidence,
                    status_str,
                    now,
                    expires_at,
                ],
            )
            .map_err(|error| CorivoError::Internal(format!("INSERT note 失败: {error}")))?;
            let sql = format!("SELECT {NOTE_COLUMNS} FROM notes WHERE id = ?1");
            conn.query_row(&sql, params![return_id], row_to_note)
                .map_err(|error| CorivoError::Internal(format!("回读 note 失败: {error}")))
        })
        .await
    }

    async fn list(&self, opts: ListNotesOptions) -> Result<Vec<Note>> {
        run_blocking(self.pool.clone(), move |conn| {
            let mut sql = format!("SELECT {NOTE_COLUMNS} FROM notes WHERE 1=1");
            let mut params_dyn: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            if let Some(scope) = opts.scope {
                sql.push_str(" AND scope = ?");
                params_dyn.push(Box::new(scope.as_str().to_string()));
            }
            if let Some(scope_ref) = opts.scope_ref {
                sql.push_str(" AND scope_ref = ?");
                params_dyn.push(Box::new(scope_ref));
            }
            if let Some(status) = opts.status {
                sql.push_str(" AND status = ?");
                params_dyn.push(Box::new(status.as_str().to_string()));
            }
            if let Some(source_type) = opts.source_type {
                sql.push_str(" AND source_type = ?");
                params_dyn.push(Box::new(source_type.as_str().to_string()));
            }
            sql.push_str(" ORDER BY created_at ASC");
            if let Some(limit) = opts.limit {
                sql.push_str(" LIMIT ?");
                params_dyn.push(Box::new(limit.max(1) as i64));
            }
            let mut stmt = conn.prepare(&sql).map_err(|error| {
                CorivoError::Internal(format!("prepare notes list 失败: {error}"))
            })?;
            let refs: Vec<&dyn rusqlite::ToSql> = params_dyn.iter().map(|b| b.as_ref()).collect();
            let rows = stmt
                .query_map(refs.as_slice(), row_to_note)
                .map_err(|error| {
                    CorivoError::Internal(format!("query notes list 失败: {error}"))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| CorivoError::Internal(format!("collect notes list 失败: {error}")))
        })
        .await
    }

    async fn by_id(&self, id: &str) -> Result<Option<Note>> {
        let id = id.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            let sql = format!("SELECT {NOTE_COLUMNS} FROM notes WHERE id = ?1");
            conn.query_row(&sql, params![id], row_to_note)
                .optional()
                .map_err(|error| CorivoError::Internal(format!("查询 note 失败: {error}")))
        })
        .await
    }

    async fn update(&self, id: &str, patch: UpdateNote) -> Result<Note> {
        let id = id.to_string();
        let return_id = id.clone();
        let now = DbInstant::now();
        run_blocking(self.pool.clone(), move |conn| {
            let mut sets: Vec<&str> = Vec::new();
            let mut params_dyn: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            if let Some(content) = patch.content {
                let tokens = derive_search_tokens(&content);
                sets.push("content = ?");
                params_dyn.push(Box::new(content));
                sets.push("search_tokens = ?");
                params_dyn.push(Box::new(tokens));
            }
            if let Some(status) = patch.status {
                sets.push("status = ?");
                params_dyn.push(Box::new(status.as_str().to_string()));
            }
            if let Some(confidence) = patch.confidence {
                sets.push("confidence = ?");
                params_dyn.push(Box::new(confidence));
            }
            if let Some(superseded_by) = patch.superseded_by {
                sets.push("superseded_by = ?");
                params_dyn.push(Box::new(superseded_by));
            }
            if sets.is_empty() {
                let sql = format!("SELECT {NOTE_COLUMNS} FROM notes WHERE id = ?1");
                return conn
                    .query_row(&sql, params![id], row_to_note)
                    .map_err(|error| CorivoError::Internal(format!("查询 note 失败: {error}")));
            }
            sets.push("updated_at = ?");
            params_dyn.push(Box::new(now));
            params_dyn.push(Box::new(id));
            let sql = format!("UPDATE notes SET {} WHERE id = ?", sets.join(", "));
            let refs: Vec<&dyn rusqlite::ToSql> = params_dyn.iter().map(|b| b.as_ref()).collect();
            let updated = conn
                .execute(&sql, refs.as_slice())
                .map_err(|error| CorivoError::Internal(format!("UPDATE note 失败: {error}")))?;
            if updated == 0 {
                return Err(CorivoError::Internal(format!("note {return_id} 不存在")));
            }
            let sql = format!("SELECT {NOTE_COLUMNS} FROM notes WHERE id = ?1");
            conn.query_row(&sql, params![return_id], row_to_note)
                .map_err(|error| CorivoError::Internal(format!("回读 note 失败: {error}")))
        })
        .await
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute("DELETE FROM notes WHERE id = ?1", params![id])
                .map_err(|error| CorivoError::Internal(format!("DELETE note 失败: {error}")))?;
            Ok(())
        })
        .await
    }

    async fn fts_search(
        &self,
        fts_query: &str,
        status_filter: Option<NoteStatus>,
        scope_filter: Option<NoteScope>,
        limit: u32,
    ) -> Result<Vec<NoteHit>> {
        let q = fts_query.to_string();
        let status = status_filter.map(|s| s.as_str().to_string());
        let scope = scope_filter.map(|s| s.as_str().to_string());
        let limit = limit.max(1).min(50) as i64;
        run_blocking(self.pool.clone(), move |conn| {
            let qualified = NOTE_COLUMNS
                .split(',')
                .map(|c| format!("notes.{}", c.trim()))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT {qualified}, -bm25(notes_fts) AS score
                 FROM notes_fts
                 JOIN notes ON notes.rowid = notes_fts.rowid
                 WHERE notes_fts MATCH ?1
                   AND (?2 IS NULL OR notes.status = ?2)
                   AND (?3 IS NULL OR notes.scope = ?3)
                 ORDER BY score DESC
                 LIMIT ?4"
            );
            let mut stmt = conn.prepare(&sql).map_err(|error| {
                CorivoError::Internal(format!("prepare notes_fts 失败: {error}"))
            })?;
            let rows = stmt
                .query_map(params![q, status, scope, limit], |row| {
                    let note = row_to_note(row)?;
                    let score: f64 = row.get(14)?;
                    Ok(NoteHit {
                        note,
                        score: score as f32,
                    })
                })
                .map_err(|error| CorivoError::Internal(format!("query notes_fts 失败: {error}")))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| CorivoError::Internal(format!("collect notes_fts 失败: {error}")))
        })
        .await
    }

    async fn touch_referenced(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        let now = DbInstant::now();
        run_blocking(self.pool.clone(), move |conn| {
            conn.execute(
                "UPDATE notes SET last_referenced_at = ?1 WHERE id = ?2",
                params![now, id],
            )
            .map_err(|error| CorivoError::Internal(format!("touch note 失败: {error}")))?;
            Ok(())
        })
        .await
    }
}

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    let scope_raw: String = row.get(2)?;
    let scope = NoteScope::parse(&scope_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown scope: {scope_raw}"),
            )),
        )
    })?;
    let source_raw: String = row.get(4)?;
    let source_type = NoteSourceType::parse(&source_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown source_type: {source_raw}"),
            )),
        )
    })?;
    let status_raw: String = row.get(8)?;
    let status = NoteStatus::parse(&status_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            8,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown status: {status_raw}"),
            )),
        )
    })?;
    Ok(Note {
        id: row.get(0)?,
        content: row.get(1)?,
        scope,
        scope_ref: row.get(3)?,
        source_type,
        source_message_id: row.get(5)?,
        source_thread_id: row.get(6)?,
        confidence: row.get::<_, f64>(7)? as f32,
        status,
        superseded_by: row.get(9)?,
        created_at: row.get::<_, DbInstant>(10)?.into_inner(),
        updated_at: row.get::<_, DbInstant>(11)?.into_inner(),
        last_referenced_at: row
            .get::<_, Option<DbInstant>>(12)?
            .map(DbInstant::into_inner),
        expires_at: row
            .get::<_, Option<DbInstant>>(13)?
            .map(DbInstant::into_inner),
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

    fn fake(content: &str, source: NoteSourceType) -> NewNote {
        NewNote {
            content: content.into(),
            scope: NoteScope::Global,
            scope_ref: None,
            source_type: source,
            source_message_id: None,
            source_thread_id: None,
            confidence: None,
            status: None,
            expires_at: None,
        }
    }

    #[tokio::test]
    async fn create_defaults_active_for_user_explicit() {
        let pool = init_pool();
        let repo = SqliteNotesRepo::new(pool);
        let n = repo
            .create(fake("用中文回我", NoteSourceType::UserExplicit))
            .await
            .unwrap();
        assert_eq!(n.status, NoteStatus::Active);
        assert!((n.confidence - 1.0).abs() < f32::EPSILON);
        assert_eq!(n.source_type, NoteSourceType::UserExplicit);
    }

    #[tokio::test]
    async fn create_defaults_suggested_for_agent_inferred() {
        let pool = init_pool();
        let repo = SqliteNotesRepo::new(pool);
        let n = repo
            .create(fake("可能喜欢 jj", NoteSourceType::AgentInferred))
            .await
            .unwrap();
        assert_eq!(n.status, NoteStatus::Suggested);
        assert!((n.confidence - 0.6).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn list_filters_by_scope_and_status() {
        let pool = init_pool();
        let repo = SqliteNotesRepo::new(pool);
        repo.create(fake("a", NoteSourceType::UserExplicit))
            .await
            .unwrap();
        repo.create(fake("b", NoteSourceType::AgentInferred))
            .await
            .unwrap();
        let mut session = fake("c", NoteSourceType::UserExplicit);
        session.scope = NoteScope::Session;
        session.scope_ref = Some("T1".into());
        repo.create(session).await.unwrap();

        let globals = repo
            .list(ListNotesOptions {
                scope: Some(NoteScope::Global),
                status: Some(NoteStatus::Active),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(globals.len(), 1);
        assert_eq!(globals[0].content, "a");

        let suggested = repo
            .list(ListNotesOptions {
                status: Some(NoteStatus::Suggested),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(suggested.len(), 1);
        assert_eq!(suggested[0].content, "b");

        let by_ref = repo
            .list(ListNotesOptions {
                scope: Some(NoteScope::Session),
                scope_ref: Some("T1".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(by_ref.len(), 1);
        assert_eq!(by_ref[0].content, "c");
    }

    #[tokio::test]
    async fn update_promotes_suggested_to_active() {
        let pool = init_pool();
        let repo = SqliteNotesRepo::new(pool);
        let n = repo
            .create(fake("learn me", NoteSourceType::AgentInferred))
            .await
            .unwrap();
        assert_eq!(n.status, NoteStatus::Suggested);

        let updated = repo
            .update(
                &n.id,
                UpdateNote {
                    status: Some(NoteStatus::Active),
                    confidence: Some(0.95),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.status, NoteStatus::Active);
        assert!((updated.confidence - 0.95).abs() < f32::EPSILON);
        assert!(updated.updated_at >= n.updated_at);
    }

    #[tokio::test]
    async fn fts_search_returns_match_with_score() {
        let pool = init_pool();
        let repo = SqliteNotesRepo::new(pool);
        repo.create(NewNote {
            content: "用 4 空格缩进 Python".into(),
            scope: NoteScope::Global,
            scope_ref: None,
            source_type: NoteSourceType::UserExplicit,
            source_message_id: None,
            source_thread_id: None,
            confidence: None,
            status: None,
            expires_at: None,
        })
        .await
        .unwrap();
        repo.create(fake("用中文回我", NoteSourceType::UserExplicit))
            .await
            .unwrap();
        let q = crate::services::tokenize::tokenize_for_query("Python 缩进").unwrap();
        let hits = repo.fts_search(&q, None, None, 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].note.content.contains("Python"));
        assert!(hits[0].score > 0.0);
    }

    #[tokio::test]
    async fn delete_then_lookup_returns_none() {
        let pool = init_pool();
        let repo = SqliteNotesRepo::new(pool);
        let n = repo
            .create(fake("doomed", NoteSourceType::UserExplicit))
            .await
            .unwrap();
        repo.delete(&n.id).await.unwrap();
        assert!(repo.by_id(&n.id).await.unwrap().is_none());
    }
}
