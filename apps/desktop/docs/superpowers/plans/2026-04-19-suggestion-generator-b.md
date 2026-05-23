# Suggestion Generator (B 阶段) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `suggestion_generator` LLM step between `push_decider` and overlay delivery so push payload is a 3-suggestion bundle (first one shown, all 3 stored) instead of raw proposition text. Failures degrade to current behavior — never block push.

**Architecture:** New `services/suggestion_generator.rs` (pure service, mockable LLM). `push_pipeline::handle_event` calls it after `PushDecision::Push`. Suggestions persist in new `suggestions` table; `notification_log.suggestion_id` links each push to its surfaced suggestion (NULL = degraded). Schema bumps v6 → v7. Front-end zero changes except adding a third Prompt-Debug textarea for the new `suggest` prompt override.

**Tech Stack:** Rust (Tauri 2) backend, rusqlite + r2d2, tokio + async-trait, `LlmProvider` trait + `MockLlm` for tests, `insta` for prompt snapshots, React 19 + TypeScript front-end (tiny patch only).

**Reference spec:** [`docs/superpowers/specs/2026-04-19-suggestion-generator-design.md`](../specs/2026-04-19-suggestion-generator-design.md)

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `src-tauri/src/db/migrations/007_suggestions.sql` | Create | DDL for `suggestions` + add `notification_log.suggestion_id` |
| `src-tauri/src/db/migrations.rs` | Modify | Register v7, bump `LATEST_VERSION = 7`, add `apply_v7` |
| `src-tauri/src/db/repos/suggestions.rs` | Create | `SuggestionRepo` trait + `SqliteSuggestionRepo` impl |
| `src-tauri/src/db/repos/mod.rs` | Modify | Re-export `suggestions` module |
| `src-tauri/src/db/repos/notification_log.rs` | Modify | Add `suggestion_id` field to `NewNotificationLog` + `NotificationLogEntry` + insert SQL |
| `src-tauri/prompts/suggest.md` | Create | LLM prompt template |
| `src-tauri/src/services/user_model/prompts.rs` | Modify | Add `SuggestInput`, `SuggestRelated`, `render_suggest` |
| `src-tauri/src/services/suggestion_generator.rs` | Create | Pure service: retrieve G → render → LLM → insert; returns `Result<SuggestionBundle>` |
| `src-tauri/src/services/mod.rs` | Modify | Re-export `suggestion_generator` module |
| `src-tauri/src/services/push_pipeline.rs` | Modify | Call generator after `Push` decision; build payload from bundle or fallback |
| `src-tauri/src/domain/config.rs` | Modify | Add `PromptDebugConfig::suggest_override` + normalize |
| `src-tauri/src/lib.rs` | Modify | Construct `SuggestionRepo`, build `SuggestionGenerator`, pass to `push_pipeline::spawn_default` |
| `src-tauri/tests/migrations_v7.rs` | Create | Verify v6→v7 step creates table + adds column |
| `src-tauri/tests/suggestions_repo.rs` | Create | Repo unit tests (insert_batch transaction, by_id, by_proposition_id) |
| `src-tauri/tests/notification_log_repo.rs` | Modify | Cover insert with `suggestion_id` |
| `src-tauri/tests/prompt_snapshots.rs` | Modify | Add `suggest_basic` + `suggest_empty_related` snapshots |
| `src-tauri/tests/suggestion_generator.rs` | Create | Service unit tests (happy + 3 error paths) |
| `src-tauri/tests/suggestion_generator_end_to_end.rs` | Create | Real SQLite + MockLlm + push_pipeline E2E |
| `src-tauri/tests/push_pipeline_end_to_end.rs` | Modify | Add suggestion-happy + suggestion-fallback cases |
| `src/lib/types.ts` | Modify | Add `suggest_override` field to `PromptDebugConfig` |
| `src/pages/settings/sections/prompt-debug-section.tsx` | Modify | Third textarea (mirror `summary_override` pattern) |
| `src/lib/config-tauri.test.ts` | Modify | Add `suggest_override: null` to test fixture |

---

## Task 1: v7 migration — schema + register + test

**Files:**
- Create: `src-tauri/src/db/migrations/007_suggestions.sql`
- Modify: `src-tauri/src/db/migrations.rs:8-11, 45-48, register apply_v7`
- Create: `src-tauri/tests/migrations_v7.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/tests/migrations_v7.rs`:

```rust
use corivo_app_lib::db::{
    migrations::{apply_migrations, LATEST_VERSION},
    pool::test_in_memory_pool,
};

const SCHEMA_V1: &str = include_str!("../src/db/schema.sql");
const SCHEMA_V5: &str = include_str!("../src/db/migrations/005_gum_user_model.sql");
const SCHEMA_V6: &str = include_str!("../src/db/migrations/006_propositions_fts.sql");

fn seed_v6() -> r2d2::Pool<r2d2_sqlite::SqliteConnectionManager> {
    let pool = test_in_memory_pool().expect("pool");
    let conn = pool.get().unwrap();
    conn.execute_batch(SCHEMA_V1).unwrap();
    conn.execute_batch(SCHEMA_V5).unwrap();
    conn.execute_batch(SCHEMA_V6).unwrap();
    pool
}

#[test]
fn v7_creates_suggestions_table_and_adds_notification_log_column() {
    let pool = seed_v6();
    let conn = pool.get().unwrap();
    apply_migrations(&conn).expect("migrations apply");

    let v: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, LATEST_VERSION);
    assert_eq!(LATEST_VERSION, 7);

    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='suggestions'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(table_exists, 1);

    let column_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('notification_log') WHERE name='suggestion_id'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(column_exists, 1);
}

#[test]
fn v7_is_idempotent_when_run_twice() {
    let pool = seed_v6();
    let conn = pool.get().unwrap();
    apply_migrations(&conn).expect("first apply");
    apply_migrations(&conn).expect("second apply must be a no-op");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --test migrations_v7`
Expected: FAIL — `LATEST_VERSION` is still `6`, no `suggestions` table.

- [ ] **Step 3: Create the v7 SQL file**

Create `src-tauri/src/db/migrations/007_suggestions.sql`:

```sql
-- v7: 给 push pipeline 引入 suggestion_generator —— LLM 把命题翻译成"建议"，
-- 每次推送生成 3 条候选，第一条作为 overlay body 推出去，3 条都入库。
-- notification_log 加 suggestion_id 外键标记本次推送是哪条建议（NULL = 降级路径，
-- 推的是原命题文本）。

CREATE TABLE suggestions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    proposition_id  INTEGER NOT NULL REFERENCES propositions(id) ON DELETE CASCADE,
    text            TEXT    NOT NULL,
    reasoning       TEXT    NOT NULL,
    position        INTEGER NOT NULL CHECK (position >= 0 AND position <= 2),
    created_at      TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_suggestions_proposition_id ON suggestions(proposition_id);

ALTER TABLE notification_log
    ADD COLUMN suggestion_id INTEGER NULL REFERENCES suggestions(id) ON DELETE SET NULL;

INSERT OR IGNORE INTO schema_version (version) VALUES (7);
```

- [ ] **Step 4: Register v7 in migrations.rs**

Edit `src-tauri/src/db/migrations.rs`:

Replace lines 7-11 with:

```rust
const SCHEMA_V1: &str = include_str!("schema.sql");
const SCHEMA_V5: &str = include_str!("migrations/005_gum_user_model.sql");
const SCHEMA_V6: &str = include_str!("migrations/006_propositions_fts.sql");
const SCHEMA_V7: &str = include_str!("migrations/007_suggestions.sql");

pub const LATEST_VERSION: i64 = 7;
```

After the `current_version < 6` block (around line 48), insert:

```rust
    let current_version = get_current_version(conn)?;
    if current_version < 7 {
        apply_v7(conn)?;
    }
```

After `fn apply_v6(...)`, insert:

```rust
fn apply_v7(conn: &DbConnection) -> Result<()> {
    conn.execute_batch(SCHEMA_V7)
        .map_err(|error| CorivoError::Internal(format!("V7 migration failed: {error}")))?;
    tracing::info!(schema_version = 7, "applied v7 migration");
    Ok(())
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd src-tauri && cargo test --test migrations_v7`
Expected: PASS — both tests pass.

Also run `cd src-tauri && cargo test --test migrations_v6` to confirm no regression.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db/migrations/007_suggestions.sql \
        src-tauri/src/db/migrations.rs \
        src-tauri/tests/migrations_v7.rs
git commit -m "feat(db): add v7 migration for suggestions table"
```

---

## Task 2: SuggestionRepo trait + Sqlite impl

**Files:**
- Create: `src-tauri/src/db/repos/suggestions.rs`
- Modify: `src-tauri/src/db/repos/mod.rs`
- Create: `src-tauri/tests/suggestions_repo.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/tests/suggestions_repo.rs`:

```rust
use chrono::Utc;
use corivo_app_lib::db::{
    migrations::apply_migrations,
    pool::test_in_memory_pool,
    repos::{
        propositions::{NewProposition, PropositionRepo, SqlitePropositionRepo},
        suggestions::{NewSuggestion, SqliteSuggestionRepo, SuggestionRepo},
    },
};

async fn seed_proposition(pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>) -> i64 {
    let repo = SqlitePropositionRepo::new(pool.clone());
    repo.insert(NewProposition {
        text: "用户偏好凌晨工作".into(),
        reasoning: "多次观察到 0:00-3:00 仍在 IDE".into(),
        confidence: Some(7),
        decay: Some(6),
        revision_group: "g1".into(),
        version: 1,
        contradicts_proposition_id: None,
    })
    .await
    .unwrap()
    .id
}

#[tokio::test]
async fn insert_batch_persists_three_rows_with_position_index() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop_id = seed_proposition(&pool).await;

    let repo = SqliteSuggestionRepo::new(pool);
    let inserted = repo
        .insert_batch(
            prop_id,
            vec![
                NewSuggestion {
                    text: "你最近常熬夜，要不要明早把会挪一挪？".into(),
                    reasoning: "对凌晨工作的轻量补偿提议".into(),
                },
                NewSuggestion {
                    text: "看起来这周凌晨工作的节奏在加快。".into(),
                    reasoning: "情境式陈述，不带提议".into(),
                },
                NewSuggestion {
                    text: "要不要 Corivo 帮你统计一下本周凌晨时段？".into(),
                    reasoning: "提供可观察的数据回看".into(),
                },
            ],
        )
        .await
        .unwrap();

    assert_eq!(inserted.len(), 3);
    assert_eq!(inserted[0].position, 0);
    assert_eq!(inserted[1].position, 1);
    assert_eq!(inserted[2].position, 2);
    assert!(inserted.iter().all(|s| s.proposition_id == prop_id));
}

#[tokio::test]
async fn by_id_returns_inserted_suggestion() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop_id = seed_proposition(&pool).await;

    let repo = SqliteSuggestionRepo::new(pool);
    let inserted = repo
        .insert_batch(
            prop_id,
            vec![NewSuggestion {
                text: "唯一一条".into(),
                reasoning: "因为本测试只插一条".into(),
            }],
        )
        .await
        .unwrap();
    let id = inserted[0].id;

    let fetched = repo.by_id(id).await.unwrap().expect("should exist");
    assert_eq!(fetched.text, "唯一一条");
    assert_eq!(fetched.position, 0);
}

#[tokio::test]
async fn by_proposition_id_returns_in_position_order() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop_id = seed_proposition(&pool).await;

    let repo = SqliteSuggestionRepo::new(pool);
    repo.insert_batch(
        prop_id,
        vec![
            NewSuggestion { text: "first".into(), reasoning: "r1".into() },
            NewSuggestion { text: "second".into(), reasoning: "r2".into() },
            NewSuggestion { text: "third".into(), reasoning: "r3".into() },
        ],
    )
    .await
    .unwrap();

    let listed = repo.by_proposition_id(prop_id).await.unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].text, "first");
    assert_eq!(listed[1].text, "second");
    assert_eq!(listed[2].text, "third");
}

#[tokio::test]
async fn insert_batch_rejects_more_than_three_items() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop_id = seed_proposition(&pool).await;

    let repo = SqliteSuggestionRepo::new(pool);
    let result = repo
        .insert_batch(
            prop_id,
            vec![
                NewSuggestion { text: "1".into(), reasoning: "1".into() },
                NewSuggestion { text: "2".into(), reasoning: "2".into() },
                NewSuggestion { text: "3".into(), reasoning: "3".into() },
                NewSuggestion { text: "4".into(), reasoning: "4".into() }, // > MAX
            ],
        )
        .await;
    assert!(result.is_err(), "must reject > 3 items so position CHECK never fires");
}

#[tokio::test]
async fn insert_batch_rejects_empty_input() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop_id = seed_proposition(&pool).await;

    let repo = SqliteSuggestionRepo::new(pool);
    let result = repo.insert_batch(prop_id, vec![]).await;
    assert!(result.is_err(), "empty batch is meaningless — fail loudly");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --test suggestions_repo`
Expected: FAIL — module `suggestions` does not exist.

- [ ] **Step 3: Create the suggestions repo**

Create `src-tauri/src/db/repos/suggestions.rs`:

```rust
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{
    db::{
        pool::{run_blocking, DbPool},
        types::parse_datetime,
    },
    error::{CorivoError, Result},
};

const MAX_BATCH: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Suggestion {
    pub id: i64,
    pub proposition_id: i64,
    pub text: String,
    pub reasoning: String,
    pub position: u8,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewSuggestion {
    pub text: String,
    pub reasoning: String,
}

#[async_trait]
pub trait SuggestionRepo: Send + Sync {
    /// Insert exactly 1..=3 suggestions for a proposition in a single
    /// transaction. `position` is auto-assigned 0/1/2 by `items` order.
    async fn insert_batch(
        &self,
        proposition_id: i64,
        items: Vec<NewSuggestion>,
    ) -> Result<Vec<Suggestion>>;
    async fn by_id(&self, id: i64) -> Result<Option<Suggestion>>;
    async fn by_proposition_id(&self, proposition_id: i64) -> Result<Vec<Suggestion>>;
}

#[derive(Clone)]
pub struct SqliteSuggestionRepo {
    pool: DbPool,
}

impl SqliteSuggestionRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

const SELECT_COLUMNS: &str =
    "id, proposition_id, text, reasoning, position, created_at";

#[async_trait]
impl SuggestionRepo for SqliteSuggestionRepo {
    async fn insert_batch(
        &self,
        proposition_id: i64,
        items: Vec<NewSuggestion>,
    ) -> Result<Vec<Suggestion>> {
        if items.is_empty() {
            return Err(CorivoError::Internal(
                "suggestions.insert_batch: empty batch".to_string(),
            ));
        }
        if items.len() > MAX_BATCH {
            return Err(CorivoError::Internal(format!(
                "suggestions.insert_batch: batch size {} exceeds max {}",
                items.len(),
                MAX_BATCH
            )));
        }
        run_blocking(self.pool.clone(), move |conn| {
            let tx = conn.unchecked_transaction().map_err(|error| {
                CorivoError::Internal(format!("suggestions begin tx 失败: {error}"))
            })?;
            let mut inserted = Vec::with_capacity(items.len());
            for (idx, item) in items.iter().enumerate() {
                let row = tx
                    .query_row(
                        &format!(
                            "INSERT INTO suggestions (proposition_id, text, reasoning, position)
                             VALUES (?1, ?2, ?3, ?4)
                             RETURNING {SELECT_COLUMNS}"
                        ),
                        params![
                            proposition_id,
                            item.text,
                            item.reasoning,
                            idx as i64,
                        ],
                        row_to_suggestion,
                    )
                    .map_err(|error| {
                        CorivoError::Internal(format!("插入 suggestion 失败: {error}"))
                    })?;
                inserted.push(row);
            }
            tx.commit().map_err(|error| {
                CorivoError::Internal(format!("suggestions commit 失败: {error}"))
            })?;
            Ok(inserted)
        })
        .await
    }

    async fn by_id(&self, id: i64) -> Result<Option<Suggestion>> {
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row(
                &format!("SELECT {SELECT_COLUMNS} FROM suggestions WHERE id = ?1"),
                params![id],
                row_to_suggestion,
            )
            .optional()
            .map_err(|error| CorivoError::Internal(format!("查询 suggestion 失败: {error}")))
        })
        .await
    }

    async fn by_proposition_id(&self, proposition_id: i64) -> Result<Vec<Suggestion>> {
        run_blocking(self.pool.clone(), move |conn| {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {SELECT_COLUMNS} FROM suggestions
                     WHERE proposition_id = ?1
                     ORDER BY position ASC"
                ))
                .map_err(|error| {
                    CorivoError::Internal(format!("prepare suggestions list 失败: {error}"))
                })?;
            let rows = stmt
                .query_map(params![proposition_id], row_to_suggestion)
                .map_err(|error| {
                    CorivoError::Internal(format!("query suggestions list 失败: {error}"))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
                CorivoError::Internal(format!("collect suggestions list 失败: {error}"))
            })
        })
        .await
    }
}

fn row_to_suggestion(row: &rusqlite::Row<'_>) -> rusqlite::Result<Suggestion> {
    let created_at = parse_datetime(&row.get::<_, String>(5)?)?;
    Ok(Suggestion {
        id: row.get(0)?,
        proposition_id: row.get(1)?,
        text: row.get(2)?,
        reasoning: row.get(3)?,
        position: row.get::<_, i64>(4)? as u8,
        created_at,
    })
}
```

- [ ] **Step 4: Register module**

Edit `src-tauri/src/db/repos/mod.rs` and add `pub mod suggestions;` next to the other `pub mod` lines (alphabetical position: between `propositions` and any later module).

- [ ] **Step 5: Run test to verify it passes**

Run: `cd src-tauri && cargo test --test suggestions_repo`
Expected: PASS — all 5 tests green.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db/repos/suggestions.rs \
        src-tauri/src/db/repos/mod.rs \
        src-tauri/tests/suggestions_repo.rs
git commit -m "feat(db): add SuggestionRepo with insert_batch transaction"
```

---

## Task 3: Add `suggestion_id` to NotificationLog repo

**Files:**
- Modify: `src-tauri/src/db/repos/notification_log.rs:25-87`
- Modify: `src-tauri/tests/notification_log_repo.rs`

- [ ] **Step 1: Write the failing test**

Append to `src-tauri/tests/notification_log_repo.rs`:

```rust
#[tokio::test]
async fn insert_with_suggestion_id_round_trips() {
    use chrono::Utc;
    use corivo_app_lib::db::{
        migrations::apply_migrations,
        pool::test_in_memory_pool,
        repos::{
            notification_log::{NewNotificationLog, NotificationLogRepo, SqliteNotificationLogRepo},
            propositions::{NewProposition, PropositionRepo, SqlitePropositionRepo},
            suggestions::{NewSuggestion, SqliteSuggestionRepo, SuggestionRepo},
        },
    };

    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();

    let prop = SqlitePropositionRepo::new(pool.clone())
        .insert(NewProposition {
            text: "p".into(),
            reasoning: "r".into(),
            confidence: Some(5),
            decay: Some(5),
            revision_group: "g".into(),
            version: 1,
            contradicts_proposition_id: None,
        })
        .await
        .unwrap();
    let suggestions = SqliteSuggestionRepo::new(pool.clone())
        .insert_batch(
            prop.id,
            vec![NewSuggestion { text: "t".into(), reasoning: "rr".into() }],
        )
        .await
        .unwrap();
    let suggestion_id = suggestions[0].id;

    let log_repo = SqliteNotificationLogRepo::new(pool);
    let entry = log_repo
        .insert(NewNotificationLog {
            revision_group: "g".into(),
            proposition_id: Some(prop.id),
            pushed_at: Utc::now(),
            suggestion_id: Some(suggestion_id),
        })
        .await
        .unwrap();

    assert_eq!(entry.suggestion_id, Some(suggestion_id));
}

#[tokio::test]
async fn insert_with_null_suggestion_id_marks_fallback_path() {
    use chrono::Utc;
    use corivo_app_lib::db::{
        migrations::apply_migrations,
        pool::test_in_memory_pool,
        repos::notification_log::{
            NewNotificationLog, NotificationLogRepo, SqliteNotificationLogRepo,
        },
    };

    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let log_repo = SqliteNotificationLogRepo::new(pool);

    let entry = log_repo
        .insert(NewNotificationLog {
            revision_group: "g".into(),
            proposition_id: None,
            pushed_at: Utc::now(),
            suggestion_id: None,
        })
        .await
        .unwrap();
    assert!(entry.suggestion_id.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --test notification_log_repo insert_with_suggestion_id_round_trips`
Expected: FAIL — `NewNotificationLog` has no `suggestion_id` field; `NotificationLogEntry` has no `suggestion_id` field.

- [ ] **Step 3: Update NotificationLog struct + insert SQL**

Edit `src-tauri/src/db/repos/notification_log.rs`:

Replace lines 25-40 with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotificationLogEntry {
    pub id: i64,
    pub revision_group: String,
    pub proposition_id: Option<i64>,
    pub pushed_at: DateTime<Utc>,
    pub outcome: Option<String>,
    pub outcome_at: Option<DateTime<Utc>>,
    pub suggestion_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NewNotificationLog {
    pub revision_group: String,
    pub proposition_id: Option<i64>,
    pub pushed_at: DateTime<Utc>,
    pub suggestion_id: Option<i64>,
}
```

Replace the `insert` method body (lines 72-87) with:

```rust
    async fn insert(&self, new: NewNotificationLog) -> Result<NotificationLogEntry> {
        let pushed_at = datetime_to_sql(&new.pushed_at);
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row(
                "INSERT INTO notification_log (revision_group, proposition_id, pushed_at, suggestion_id)
                 VALUES (?1, ?2, ?3, ?4)
                 RETURNING id, revision_group, proposition_id, pushed_at, outcome, outcome_at, suggestion_id",
                params![
                    new.revision_group,
                    new.proposition_id,
                    pushed_at,
                    new.suggestion_id,
                ],
                row_to_entry,
            )
            .map_err(|error| {
                CorivoError::Internal(format!("插入 notification_log 失败: {error}"))
            })
        })
        .await
    }
```

Update each remaining `SELECT ... FROM notification_log` query (in `last_in_group`, `recent`) to also return `suggestion_id` as the 7th column. Specifically:
- `last_in_group` SQL (line 93): `SELECT id, revision_group, proposition_id, pushed_at, outcome, outcome_at, suggestion_id`
- `recent` SQL (line 181): same change

Replace `row_to_entry` (lines 202-213) with:

```rust
fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<NotificationLogEntry> {
    let pushed_at = parse_datetime(&row.get::<_, String>(3)?)?;
    let outcome_at = parse_optional_datetime(row.get::<_, Option<String>>(5)?)?;
    Ok(NotificationLogEntry {
        id: row.get(0)?,
        revision_group: row.get(1)?,
        proposition_id: row.get(2)?,
        pushed_at,
        outcome: row.get(4)?,
        outcome_at,
        suggestion_id: row.get(6)?,
    })
}
```

- [ ] **Step 4: Run tests to verify pass + no regression**

Run: `cd src-tauri && cargo test --test notification_log_repo`
Expected: PASS — both new tests pass; the pre-existing tests in this file still pass (the new `suggestion_id` field is `None` by default and unused by older callers — but **all callers of `NewNotificationLog` will currently fail to compile**).

If the build errors out at this point, that's expected for Task 4 onward — the next tasks update the call sites. For now, the test under `--test notification_log_repo` should compile and pass. If it doesn't, fix the test file's struct literals.

Run: `cd src-tauri && cargo build 2>&1 | grep "error\[" | head` to see what's broken downstream — it should be `push_pipeline.rs` and `tests/push_pipeline_end_to_end.rs`. Leave them for Tasks 9 & 10; **do not** commit until the build is green.

Defer commit — the next task fixes a chunk of those errors.

- [ ] **Step 5: Patch the obvious downstream call sites enough to build**

Just to keep `cargo test` green during the next tasks, edit `src-tauri/src/services/push_pipeline.rs` line 125-132:

```rust
    let entry = deps
        .notification_log
        .insert(NewNotificationLog {
            revision_group: proposition.revision_group.clone(),
            proposition_id: Some(proposition.id),
            pushed_at: deps.clock.now(),
            suggestion_id: None, // Task 9 will populate this from the bundle.
        })
        .await?;
```

And `src-tauri/tests/push_pipeline_end_to_end.rs` — find every `NewNotificationLog { ... }` literal and add `suggestion_id: None,`. (The grep `grep -n "NewNotificationLog" src-tauri/tests/push_pipeline_end_to_end.rs` will list them.)

- [ ] **Step 6: Verify build + tests green**

Run: `cd src-tauri && cargo build && cargo test --test notification_log_repo --test push_pipeline_end_to_end`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/db/repos/notification_log.rs \
        src-tauri/src/services/push_pipeline.rs \
        src-tauri/tests/notification_log_repo.rs \
        src-tauri/tests/push_pipeline_end_to_end.rs
git commit -m "feat(db): add suggestion_id to notification_log repo"
```

---

## Task 4: Prompt template + render_suggest + snapshots

**Files:**
- Create: `src-tauri/prompts/suggest.md`
- Modify: `src-tauri/src/services/user_model/prompts.rs`
- Modify: `src-tauri/tests/prompt_snapshots.rs`

- [ ] **Step 1: Create prompt template**

Create `src-tauri/prompts/suggest.md`:

```
<!-- version: 1 | GUM §4.3.1 Discovering Suggestions (B 阶段) -->

你是 Corivo 的贴心观察者。Corivo 是一个长期观察用户行为、为用户建立"通用用户模型"的助手。
现在 Corivo 对用户产生了一条新的判断（命题），它打算用一句温和、自然的话告诉用户。

新命题：
- 内容：{{proposition_text}}
- 依据：{{proposition_reasoning}}
- 置信度：{{proposition_confidence}}/10

相关的已有命题（按相关性排序，作为情境上下文）：
{{related_propositions}}

请生成 3 条候选建议文本，要求：
- 用第二人称对用户说话（"你最近……"），口吻温和、不评判
- 描述 Corivo 观察到的情境，可以轻量提议（"要不要……"），但不许声称替用户做事
- 每条不超过 60 字
- 避免健康 / 关系 / 政治等敏感话题
- reasoning 字段写一句话解释你为什么这么建议（中文，30 字内）

仅输出 JSON 数组，恰好 3 个对象：

```json
[
  {"text": "...", "reasoning": "..."},
  {"text": "...", "reasoning": "..."},
  {"text": "...", "reasoning": "..."}
]
```

不要 Markdown 包裹，不要任何额外文字。
```

- [ ] **Step 2: Write the failing snapshot tests**

Append to `src-tauri/tests/prompt_snapshots.rs`:

```rust
use corivo_app_lib::services::user_model::prompts::{
    render_suggest, SuggestInput, SuggestRelated,
};

#[test]
fn suggest_basic() {
    let input = SuggestInput {
        proposition_text: "用户倾向凌晨工作".into(),
        proposition_reasoning: "多次观察到 0:00-3:00 仍在 IDE 编码".into(),
        proposition_confidence: 7,
        related: vec![
            SuggestRelated {
                text: "用户偏好长会议被打断后走动一会儿".into(),
                confidence: 6,
            },
            SuggestRelated {
                text: "用户上午不喜欢被打扰".into(),
                confidence: 8,
            },
        ],
    };
    insta::assert_snapshot!(render_suggest(&input));
}

#[test]
fn suggest_with_empty_related() {
    let input = SuggestInput {
        proposition_text: "用户偏好把音乐放在右侧屏幕".into(),
        proposition_reasoning: "Spotify 频繁出现在右屏".into(),
        proposition_confidence: 4,
        related: vec![],
    };
    insta::assert_snapshot!(render_suggest(&input));
}
```

- [ ] **Step 3: Run snapshot tests to verify they fail**

Run: `cd src-tauri && cargo test --test prompt_snapshots suggest`
Expected: FAIL — `render_suggest` and types don't exist.

- [ ] **Step 4: Implement render_suggest**

Edit `src-tauri/src/services/user_model/prompts.rs`. Add to the top:

```rust
const SUGGEST_TEMPLATE: &str = include_str!("../../../prompts/suggest.md");
```

Append the input types and renderer at the end of the file:

```rust
/// Input for the SUGGEST stage (B-phase suggestion_generator).
#[derive(Debug, Clone)]
pub struct SuggestInput {
    pub proposition_text: String,
    pub proposition_reasoning: String,
    pub proposition_confidence: i32,
    pub related: Vec<SuggestRelated>,
}

/// One related proposition row carried into SUGGEST as context.
#[derive(Debug, Clone)]
pub struct SuggestRelated {
    pub text: String,
    pub confidence: i32,
}

pub fn render_suggest(input: &SuggestInput) -> String {
    let related_block = if input.related.is_empty() {
        "（暂无相关命题）".to_string()
    } else {
        input
            .related
            .iter()
            .map(|p| format!("- {}（置信度 {}）", p.text, p.confidence))
            .collect::<Vec<_>>()
            .join("\n")
    };
    SUGGEST_TEMPLATE
        .replace("{{proposition_text}}", &input.proposition_text)
        .replace("{{proposition_reasoning}}", &input.proposition_reasoning)
        .replace(
            "{{proposition_confidence}}",
            &input.proposition_confidence.to_string(),
        )
        .replace("{{related_propositions}}", &related_block)
}
```

- [ ] **Step 5: Accept snapshots**

Run: `cd src-tauri && cargo test --test prompt_snapshots suggest`
Expected: tests fail with "snapshot file not found" or pending snapshot.

Then accept the new snapshots:

```bash
cd src-tauri && cargo insta accept --quiet
```

Re-run: `cargo test --test prompt_snapshots suggest`
Expected: PASS.

Manually inspect the two snapshot files just created under `src-tauri/tests/snapshots/` to make sure substitutions look right (no leftover `{{placeholder}}`, related list rendered correctly, empty case shows `（暂无相关命题）`).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/prompts/suggest.md \
        src-tauri/src/services/user_model/prompts.rs \
        src-tauri/tests/prompt_snapshots.rs \
        src-tauri/tests/snapshots/
git commit -m "feat(prompts): add suggest.md template + render_suggest"
```

---

## Task 5: SuggestionGenerator service — happy path

**Files:**
- Create: `src-tauri/src/services/suggestion_generator.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Create: `src-tauri/tests/suggestion_generator.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/tests/suggestion_generator.rs`:

```rust
use std::sync::Arc;

use corivo_app_lib::{
    db::{
        migrations::apply_migrations,
        pool::test_in_memory_pool,
        repos::{
            propositions::{NewProposition, Proposition, PropositionRepo, SqlitePropositionRepo},
            suggestions::{SqliteSuggestionRepo, SuggestionRepo},
        },
    },
    domain::config::RetrievalConfig,
    providers::llm::{mock::MockLlm, LlmProvider},
    services::suggestion_generator::SuggestionGenerator,
};

fn default_retrieval_cfg() -> RetrievalConfig {
    RetrievalConfig {
        w_confidence: 1.0,
        w_decay: 1.0,
        k_decay_days: 14.0,
        limit_multiplier: 3,
    }
}

async fn seed_proposition(
    pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
) -> Proposition {
    let repo = SqlitePropositionRepo::new(pool.clone());
    repo.insert(NewProposition {
        text: "用户倾向凌晨工作".into(),
        reasoning: "多次观察到 0:00-3:00 仍在 IDE".into(),
        confidence: Some(7),
        decay: Some(6),
        revision_group: "g1".into(),
        version: 1,
        contradicts_proposition_id: None,
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn happy_path_generates_three_suggestions_and_returns_first() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop = seed_proposition(&pool).await;

    let llm_response = serde_json::json!([
        { "text": "看起来你这周凌晨工作的节奏在加快。", "reasoning": "情境式回声" },
        { "text": "要不要明早把 10 点前的会挪一挪？", "reasoning": "轻量补偿提议" },
        { "text": "Corivo 可以帮你统计本周凌晨时段。", "reasoning": "提供数据回看" }
    ])
    .to_string();
    let llm: Arc<dyn LlmProvider> =
        Arc::new(MockLlm::with_responses(vec![llm_response]));

    let generator = SuggestionGenerator::new(
        Arc::new(SqlitePropositionRepo::new(pool.clone())),
        Arc::new(SqliteSuggestionRepo::new(pool.clone())),
        llm,
        default_retrieval_cfg(),
        None, // no prompt override
    );
    let bundle = generator.generate(&prop).await.unwrap();

    assert_eq!(bundle.surfaced.position, 0);
    assert_eq!(bundle.surfaced.text, "看起来你这周凌晨工作的节奏在加快。");
    assert_eq!(bundle.all.len(), 3);

    let stored = SqliteSuggestionRepo::new(pool)
        .by_proposition_id(prop.id)
        .await
        .unwrap();
    assert_eq!(stored.len(), 3);
    assert_eq!(stored[1].text, "要不要明早把 10 点前的会挪一挪？");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --test suggestion_generator`
Expected: FAIL — module `suggestion_generator` does not exist.

- [ ] **Step 3: Implement the service**

Create `src-tauri/src/services/suggestion_generator.rs`:

```rust
//! B-phase suggestion generator (GUM §4.3.1 "Discovering Suggestions").
//!
//! For a proposition that just passed `push_decider`, retrieve a small pool of
//! related propositions, ask the LLM for 3 candidate suggestion texts, persist
//! all 3, and return the first as the one to surface. Callers are expected to
//! degrade to the original `proposition.text` when this returns `Err`.

use std::sync::Arc;

use serde::Deserialize;

use crate::{
    db::repos::{
        propositions::{Proposition, PropositionRepo},
        suggestions::{NewSuggestion, Suggestion, SuggestionRepo},
    },
    domain::config::RetrievalConfig,
    error::{CorivoError, Result},
    providers::llm::{LlmProvider, LlmProviderExt, LlmRequest},
    services::user_model::{
        prompts::{render_suggest, SuggestInput, SuggestRelated},
        retrieval::{query, RetrievalArgs},
    },
};

const SUGGESTION_COUNT: usize = 3;
const RETRIEVAL_LIMIT: u32 = 5;

#[derive(Debug, Deserialize)]
struct RawSuggestion {
    text: String,
    reasoning: String,
}

/// Bundle returned to the caller. `surfaced` is the suggestion to send into
/// the overlay; `all` carries everything that was persisted (3 rows including
/// `surfaced`).
#[derive(Debug, Clone)]
pub struct SuggestionBundle {
    pub surfaced: Suggestion,
    pub all: Vec<Suggestion>,
}

pub struct SuggestionGenerator {
    proposition_repo: Arc<dyn PropositionRepo>,
    suggestion_repo: Arc<dyn SuggestionRepo>,
    llm: Arc<dyn LlmProvider>,
    retrieval_cfg: RetrievalConfig,
    prompt_override: Option<String>,
}

impl SuggestionGenerator {
    pub fn new(
        proposition_repo: Arc<dyn PropositionRepo>,
        suggestion_repo: Arc<dyn SuggestionRepo>,
        llm: Arc<dyn LlmProvider>,
        retrieval_cfg: RetrievalConfig,
        prompt_override: Option<String>,
    ) -> Self {
        Self {
            proposition_repo,
            suggestion_repo,
            llm,
            retrieval_cfg,
            prompt_override,
        }
    }

    pub async fn generate(&self, proposition: &Proposition) -> Result<SuggestionBundle> {
        let related = self.retrieve_related(proposition).await?;
        let prompt = self.render_prompt(proposition, &related);
        let raw = self.call_llm(prompt).await?;
        let items = raw
            .into_iter()
            .map(|r| NewSuggestion { text: r.text, reasoning: r.reasoning })
            .collect();
        let inserted = self
            .suggestion_repo
            .insert_batch(proposition.id, items)
            .await?;
        let surfaced = inserted
            .first()
            .cloned()
            .ok_or_else(|| CorivoError::Internal(
                "suggestion_generator: insert_batch returned 0 rows".to_string(),
            ))?;
        Ok(SuggestionBundle { surfaced, all: inserted })
    }

    async fn retrieve_related(&self, prop: &Proposition) -> Result<Vec<SuggestRelated>> {
        let scored = query(
            self.proposition_repo.as_ref(),
            &self.retrieval_cfg,
            RetrievalArgs {
                text: Some(prop.text.clone()),
                limit: RETRIEVAL_LIMIT,
                start_time: None,
                end_time: None,
                include_observations: false,
            },
        )
        .await
        .map_err(|error| CorivoError::Internal(format!(
            "suggestion_generator: retrieval failed: {error}"
        )))?;
        let related = scored
            .into_iter()
            .filter(|s| s.proposition.revision_group != prop.revision_group)
            .map(|s| SuggestRelated {
                text: s.proposition.text,
                confidence: s.proposition.confidence.unwrap_or(0),
            })
            .collect();
        Ok(related)
    }

    fn render_prompt(&self, prop: &Proposition, related: &[SuggestRelated]) -> String {
        if let Some(template) = self.prompt_override.as_deref() {
            // Override path: same placeholder substitution, but applied to
            // the user-supplied template instead of the built-in one. We do it
            // inline rather than calling render_suggest to avoid a second
            // include_str! detour.
            let related_block = if related.is_empty() {
                "（暂无相关命题）".to_string()
            } else {
                related
                    .iter()
                    .map(|p| format!("- {}（置信度 {}）", p.text, p.confidence))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            template
                .replace("{{proposition_text}}", &prop.text)
                .replace("{{proposition_reasoning}}", &prop.reasoning)
                .replace(
                    "{{proposition_confidence}}",
                    &prop.confidence.unwrap_or(0).to_string(),
                )
                .replace("{{related_propositions}}", &related_block)
        } else {
            render_suggest(&SuggestInput {
                proposition_text: prop.text.clone(),
                proposition_reasoning: prop.reasoning.clone(),
                proposition_confidence: prop.confidence.unwrap_or(0),
                related: related.to_vec(),
            })
        }
    }

    async fn call_llm(&self, prompt: String) -> Result<Vec<RawSuggestion>> {
        let req = LlmRequest::text(prompt);
        let parsed: Vec<RawSuggestion> =
            self.llm.complete_json::<Vec<RawSuggestion>>(&req).await?;
        if parsed.len() != SUGGESTION_COUNT {
            return Err(CorivoError::InvalidResponse(format!(
                "suggestion_generator: expected {SUGGESTION_COUNT} suggestions, got {}",
                parsed.len()
            )));
        }
        Ok(parsed)
    }
}
```

> **Note:** the existing `SuggestRelated` type is `Clone`-able (it derives `Clone`). Confirm that derive is present in Task 4's edit; if not, add `#[derive(Debug, Clone)]` so this `.to_vec()` compiles.

- [ ] **Step 4: Register module**

Edit `src-tauri/src/services/mod.rs` and add `pub mod suggestion_generator;` (alphabetical order; after `push_pipeline` and before `user_model` is fine).

- [ ] **Step 5: Run test to verify pass**

Run: `cd src-tauri && cargo test --test suggestion_generator happy_path_generates_three_suggestions_and_returns_first`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/services/suggestion_generator.rs \
        src-tauri/src/services/mod.rs \
        src-tauri/tests/suggestion_generator.rs
git commit -m "feat(services): add SuggestionGenerator with happy-path test"
```

---

## Task 6: SuggestionGenerator — error paths

**Files:**
- Modify: `src-tauri/tests/suggestion_generator.rs`

All tests added here should already pass against the implementation written in Task 5 (the implementation classifies these errors). If any fails, it points to a real bug in Task 5's code — fix the implementation, do not weaken the test.

- [ ] **Step 1: Add error-path tests**

Append to `src-tauri/tests/suggestion_generator.rs`:

```rust
#[tokio::test]
async fn invalid_json_response_returns_err() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop = seed_proposition(&pool).await;

    let llm: Arc<dyn LlmProvider> =
        Arc::new(MockLlm::with_responses(vec!["this is not json at all".into()]));

    let generator = SuggestionGenerator::new(
        Arc::new(SqlitePropositionRepo::new(pool.clone())),
        Arc::new(SqliteSuggestionRepo::new(pool)),
        llm,
        default_retrieval_cfg(),
        None,
    );
    let result = generator.generate(&prop).await;
    assert!(result.is_err(), "non-JSON response must surface as Err");
}

#[tokio::test]
async fn fewer_than_three_items_returns_err() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop = seed_proposition(&pool).await;

    let llm_response = serde_json::json!([
        { "text": "only one", "reasoning": "lazy LLM" }
    ])
    .to_string();
    let llm: Arc<dyn LlmProvider> =
        Arc::new(MockLlm::with_responses(vec![llm_response]));

    let generator = SuggestionGenerator::new(
        Arc::new(SqlitePropositionRepo::new(pool.clone())),
        Arc::new(SqliteSuggestionRepo::new(pool.clone())),
        llm,
        default_retrieval_cfg(),
        None,
    );
    let result = generator.generate(&prop).await;
    assert!(result.is_err(), "wrong count must surface as Err");

    let stored = SqliteSuggestionRepo::new(pool)
        .by_proposition_id(prop.id)
        .await
        .unwrap();
    assert!(stored.is_empty(), "must not partially persist on count mismatch");
}

#[tokio::test]
async fn llm_queue_exhausted_returns_err() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop = seed_proposition(&pool).await;

    let llm: Arc<dyn LlmProvider> = Arc::new(MockLlm::with_responses(vec![])); // empty queue

    let generator = SuggestionGenerator::new(
        Arc::new(SqlitePropositionRepo::new(pool.clone())),
        Arc::new(SqliteSuggestionRepo::new(pool)),
        llm,
        default_retrieval_cfg(),
        None,
    );
    let result = generator.generate(&prop).await;
    assert!(result.is_err(), "LLM transport failure must surface as Err");
}

#[tokio::test]
async fn empty_related_block_still_works() {
    // Fresh pool; only the seeded proposition exists. Retrieval will return
    // it, the filter will drop it (same revision_group), and `related` will
    // be empty. The generator must still call the LLM with an empty list.
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop = seed_proposition(&pool).await;

    let llm_response = serde_json::json!([
        { "text": "a", "reasoning": "ra" },
        { "text": "b", "reasoning": "rb" },
        { "text": "c", "reasoning": "rc" }
    ])
    .to_string();
    let llm = Arc::new(MockLlm::with_responses(vec![llm_response]));
    let llm_handle = llm.clone();
    let llm_provider: Arc<dyn LlmProvider> = llm;

    let generator = SuggestionGenerator::new(
        Arc::new(SqlitePropositionRepo::new(pool.clone())),
        Arc::new(SqliteSuggestionRepo::new(pool)),
        llm_provider,
        default_retrieval_cfg(),
        None,
    );
    let bundle = generator.generate(&prop).await.unwrap();
    assert_eq!(bundle.all.len(), 3);

    let last_prompt = llm_handle.last_prompt().unwrap();
    assert!(
        last_prompt.contains("（暂无相关命题）"),
        "empty related block sentinel must appear in the prompt"
    );
}
```

- [ ] **Step 2: Run all suggestion_generator tests**

Run: `cd src-tauri && cargo test --test suggestion_generator`
Expected: PASS — all 4 new tests + 1 from Task 5 = 5 green.

If `empty_related_block_still_works` fails because retrieval doesn't return the seeded proposition (no observations / FTS empty), inspect what `retrieval::query` produces and adjust the test setup (you can verify by adding `dbg!(scored.len())` temporarily). The intent is to assert the empty-related branch — if retrieval returns 0 rows naturally for this seeded data, the test still validates the path.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/tests/suggestion_generator.rs
git commit -m "test(suggestion_generator): cover invalid JSON, count mismatch, exhausted LLM, empty related"
```

---

## Task 7: PromptDebugConfig.suggest_override — backend

**Files:**
- Modify: `src-tauri/src/domain/config.rs:91-96, 372-377`

- [ ] **Step 1: Write the failing test**

Append to `src-tauri/tests/config_validate.rs` (or create if absent — look for the file with `grep -l "PromptDebugConfig" src-tauri/tests/`):

```rust
#[test]
fn prompt_debug_config_default_has_suggest_override_none() {
    use corivo_app_lib::domain::config::PromptDebugConfig;
    let cfg = PromptDebugConfig::default();
    assert!(cfg.suggest_override.is_none());
}

#[test]
fn prompt_debug_config_normalize_blank_suggest_override_to_none() {
    use corivo_app_lib::domain::config::Config;
    let mut cfg = Config::default();
    cfg.prompt_debug.suggest_override = Some("   \n  ".to_string());
    let normalized = cfg.normalize();
    assert!(normalized.prompt_debug.suggest_override.is_none());
}
```

If `tests/config_validate.rs` doesn't exist, create it with the above plus `use corivo_app_lib::domain::config::*;`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test prompt_debug_config_default_has_suggest_override_none`
Expected: FAIL — `suggest_override` field doesn't exist.

- [ ] **Step 3: Add the field**

Edit `src-tauri/src/domain/config.rs` lines 91-96:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PromptDebugConfig {
    pub summary_override: Option<String>,
    pub push_judgment_override: Option<String>,
    pub suggest_override: Option<String>,
}
```

Edit the `normalize` function — find the existing block at lines 374-377:

```rust
        self.prompt_debug.summary_override =
            normalize_optional_prompt_override(self.prompt_debug.summary_override);
        self.prompt_debug.push_judgment_override =
            normalize_optional_prompt_override(self.prompt_debug.push_judgment_override);
```

Add immediately after:

```rust
        self.prompt_debug.suggest_override =
            normalize_optional_prompt_override(self.prompt_debug.suggest_override);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test prompt_debug_config`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/domain/config.rs src-tauri/tests/config_validate.rs
git commit -m "feat(config): add PromptDebugConfig.suggest_override"
```

---

## Task 8: PromptDebugConfig.suggest_override — frontend

**Files:**
- Modify: `src/lib/types.ts:54-57`
- Modify: `src/lib/config-tauri.test.ts`
- Modify: `src/pages/settings/sections/prompt-debug-section.tsx`

- [ ] **Step 1: Update TypeScript type**

Edit `src/lib/types.ts` lines 54-57:

```typescript
export interface PromptDebugConfig {
  summary_override: string | null;
  push_judgment_override: string | null;
  suggest_override: string | null;
}
```

- [ ] **Step 2: Update TS test fixture**

Edit `src/lib/config-tauri.test.ts` — find the existing `prompt_debug:` literal in the test (search for `summary_override`) and add `suggest_override: null` next to it:

```typescript
prompt_debug: {
  summary_override: null,
  push_judgment_override: null,
  suggest_override: null,
},
```

- [ ] **Step 3: Run TS type-check**

Run: `pnpm exec tsc --noEmit`
Expected: PASS — no type errors.

- [ ] **Step 4: Add textarea to prompt-debug-section**

Edit `src/pages/settings/sections/prompt-debug-section.tsx`:

Update the `PromptKey` type (line 74):

```typescript
type PromptKey =
  | "summary_override"
  | "push_judgment_override"
  | "suggest_override";
```

Add a new `DEFAULT_SUGGEST_PROMPT` constant near the other defaults — copy the current contents of `src-tauri/prompts/suggest.md` verbatim into a backtick string. Place it after `DEFAULT_PUSH_JUDGMENT_PROMPT`:

```typescript
const DEFAULT_SUGGEST_PROMPT = `<!-- version: 1 | GUM §4.3.1 Discovering Suggestions (B 阶段) -->

你是 Corivo 的贴心观察者。Corivo 是一个长期观察用户行为、为用户建立"通用用户模型"的助手。
现在 Corivo 对用户产生了一条新的判断（命题），它打算用一句温和、自然的话告诉用户。

新命题：
- 内容：{{proposition_text}}
- 依据：{{proposition_reasoning}}
- 置信度：{{proposition_confidence}}/10

相关的已有命题（按相关性排序，作为情境上下文）：
{{related_propositions}}

请生成 3 条候选建议文本，要求：
- 用第二人称对用户说话（"你最近……"），口吻温和、不评判
- 描述 Corivo 观察到的情境，可以轻量提议（"要不要……"），但不许声称替用户做事
- 每条不超过 60 字
- 避免健康 / 关系 / 政治等敏感话题
- reasoning 字段写一句话解释你为什么这么建议（中文，30 字内）

仅输出 JSON 数组，恰好 3 个对象：

\`\`\`json
[
  {"text": "...", "reasoning": "..."},
  {"text": "...", "reasoning": "..."},
  {"text": "...", "reasoning": "..."}
]
\`\`\`

不要 Markdown 包裹，不要任何额外文字。`;
```

In the component, add a third `useState` hook and a third `useEffect` initializer:

```typescript
const [suggestDraft, setSuggestDraft] = useState("");
```

In the `useEffect` (lines 85-91), append:

```typescript
setSuggestDraft(config.prompt_debug.suggest_override ?? "");
```

In the `resetOverride` function (lines 112-126), extend the if/else:

```typescript
const resetOverride = (key: PromptKey) => {
  if (key === "summary_override") {
    setSummaryDraft("");
  } else if (key === "push_judgment_override") {
    setPushJudgmentDraft("");
  } else {
    setSuggestDraft("");
  }
  // ... existing update() call unchanged
};
```

Add a `suggestSource` near `pushJudgmentSource`:

```typescript
const suggestSource = sourceLabel(config.prompt_debug.suggest_override);
```

After the existing `<FieldGroup title="推送判断 Prompt">` block (which ends around line 261), add a new `<FieldGroup>` mirroring its structure for the suggest override:

```tsx
<FieldGroup title="建议生成 Prompt">
  <div className="flex flex-wrap items-center gap-2">
    <Badge variant="outline">{suggestSource}</Badge>
    <p className="text-xs text-muted-foreground">
      影响 push 决策通过后，LLM 把命题翻译成 3 条候选建议的提示词。
    </p>
  </div>

  <div className="space-y-2">
    <Label htmlFor="suggest-override">覆盖值</Label>
    <Textarea
      id="suggest-override"
      name="suggest-override"
      rows={14}
      value={suggestDraft}
      placeholder="留空表示继续使用内置默认 prompt。"
      onChange={(event) => setSuggestDraft(event.target.value)}
    />
  </div>

  <div className="flex flex-wrap gap-2">
    <Button
      onClick={() => saveOverride("suggest_override", suggestDraft)}
      disabled={isSaving}
    >
      保存
    </Button>
    <Button
      variant="outline"
      onClick={() => resetOverride("suggest_override")}
      disabled={isSaving}
    >
      恢复默认
    </Button>
    <Button
      variant="outline"
      onClick={() =>
        void copyEffectivePrompt(
          config.prompt_debug.suggest_override,
          DEFAULT_SUGGEST_PROMPT,
        )
      }
      disabled={isSaving}
    >
      复制当前生效版本
    </Button>
  </div>

  <div className="rounded-xl border border-border bg-muted/20 p-4">
    <div className="mb-2 text-xs font-medium text-muted-foreground">
      当前生效版本预览
    </div>
    <pre className="whitespace-pre-wrap break-words text-xs leading-6 text-foreground">
      {config.prompt_debug.suggest_override ?? DEFAULT_SUGGEST_PROMPT}
    </pre>
  </div>
</FieldGroup>
```

- [ ] **Step 5: Verify build + visual smoke**

Run: `pnpm exec tsc --noEmit`
Expected: PASS.

If you can spin up the app: `pnpm tauri dev`, navigate to Settings → Prompt 调试, confirm the new "建议生成 Prompt" group renders with the default text. Click 保存 with empty textarea — should still be 默认值. Type something, save, reload — value persists.

If you can't run the app right now, that's fine — the TS check + the existing pattern parity is enough confidence.

- [ ] **Step 6: Commit**

```bash
git add src/lib/types.ts \
        src/lib/config-tauri.test.ts \
        src/pages/settings/sections/prompt-debug-section.tsx
git commit -m "feat(settings): add suggest_override textarea to prompt-debug page"
```

---

## Task 9: Wire SuggestionGenerator into push_pipeline + lib.rs

**Files:**
- Modify: `src-tauri/src/services/push_pipeline.rs`
- Modify: `src-tauri/src/lib.rs:332-389`

- [ ] **Step 1: Extend PushPipelineDeps and refactor handle_event**

Edit `src-tauri/src/services/push_pipeline.rs`:

Add to imports at the top:

```rust
use crate::services::suggestion_generator::{SuggestionBundle, SuggestionGenerator};
```

Update `PushPipelineDeps` (lines 32-39):

```rust
pub struct PushPipelineDeps {
    pub revised_rx: broadcast::Receiver<PropositionRevised>,
    pub proposition_repo: Arc<dyn PropositionRepo>,
    pub notification_log: Arc<dyn NotificationLogRepo>,
    pub notification_service: Arc<NotificationService>,
    pub decider: Arc<dyn PushDecider>,
    pub clock: Arc<dyn Clock>,
    pub suggestion_generator: Arc<SuggestionGenerator>,
}
```

Replace the body of `handle_event` from after the `match decision` block down through `notification_service.send` (lines 117-158) with:

```rust
    let decision = deps.decider.decide(&proposition, &op).await?;
    match decision {
        PushDecision::Skip(reason) => {
            log_decision_skip(&event, &reason);
            return Ok(());
        }
        PushDecision::Push => {}
    }

    let bundle: Option<SuggestionBundle> =
        match deps.suggestion_generator.generate(&proposition).await {
            Ok(bundle) => Some(bundle),
            Err(error) => {
                tracing::warn!(
                    suggestion_failed = true,
                    %error,
                    proposition_id = proposition.id,
                    "push_pipeline: suggestion_generator failed; degrading to proposition text"
                );
                None
            }
        };

    let suggestion_id = bundle.as_ref().map(|b| b.surfaced.id);
    let entry = deps
        .notification_log
        .insert(NewNotificationLog {
            revision_group: proposition.revision_group.clone(),
            proposition_id: Some(proposition.id),
            pushed_at: deps.clock.now(),
            suggestion_id,
        })
        .await?;

    let confidence = proposition.confidence.map(|c| c.clamp(0, u8::MAX as i32) as u8);
    tracing::info!(
        outcome = "push",
        revision_group = %proposition.revision_group,
        proposition_id = proposition.id,
        notification_log_id = entry.id,
        suggestion_id = ?suggestion_id,
        "push.decided"
    );

    let (title, body) = match bundle.as_ref() {
        Some(b) => ("Corivo 想跟你说一句".to_string(), b.surfaced.text.clone()),
        None => (
            "Corivo 学到了一条新判断".to_string(),
            proposition.text.clone(),
        ),
    };

    if let Err(error) = deps.notification_service.send(NotificationPayload {
        title,
        body,
        level: NotificationLevel::Info,
        duration_ms: None,
        notification_log_id: Some(entry.id),
        proposition_id: Some(proposition.id),
        revision_group: Some(proposition.revision_group.clone()),
        reasoning: Some(proposition.reasoning.clone()),
        confidence,
    }) {
        tracing::warn!(
            %error,
            proposition_id = proposition.id,
            "push_pipeline: notification_service.send failed"
        );
    }

    Ok(())
}
```

Update `spawn_default` (lines 211-234) to take and forward the generator:

```rust
pub fn spawn_default(
    revised_rx: broadcast::Receiver<PropositionRevised>,
    proposition_repo: Arc<dyn PropositionRepo>,
    notification_log: Arc<dyn NotificationLogRepo>,
    notification_service: Arc<NotificationService>,
    suggestion_generator: Arc<SuggestionGenerator>,
    cfg: PushConfig,
    clock: Arc<dyn Clock>,
) {
    let decider: Arc<dyn PushDecider> = Arc::new(
        crate::services::push_decider::DefaultPushDecider::new(
            notification_log.clone(),
            cfg,
            Arc::new(RuntimeClock::new(clock.clone())),
        ),
    );
    spawn(PushPipelineDeps {
        revised_rx,
        proposition_repo,
        notification_log,
        notification_service,
        decider,
        clock,
        suggestion_generator,
    });
}
```

- [ ] **Step 2: Wire into lib.rs**

Edit `src-tauri/src/lib.rs`. Around line 337 where `notification_log` is constructed, add right after:

```rust
            let suggestion_repo: std::sync::Arc<dyn crate::db::repos::suggestions::SuggestionRepo> =
                Arc::new(crate::db::repos::suggestions::SqliteSuggestionRepo::new(db.pool()));
```

Above the `tauri::async_runtime::block_on(async {` block that calls `push_pipeline::spawn_default` (around line 376), construct the generator:

```rust
            let suggestion_generator = std::sync::Arc::new(
                crate::services::suggestion_generator::SuggestionGenerator::new(
                    proposition_repo.clone(),
                    suggestion_repo.clone(),
                    llm_provider.clone(),
                    config.user_model.retrieval.clone(),
                    config.prompt_debug.suggest_override.clone(),
                ),
            );
```

Update the `push_pipeline::spawn_default(...)` call to pass it as the new 5th arg (matches the signature change in Step 1):

```rust
                push_pipeline::spawn_default(
                    user_model.revised_receiver(),
                    proposition_repo.clone(),
                    notification_log.clone(),
                    notification_service.clone(),
                    suggestion_generator.clone(),
                    crate::services::push_decider::PushConfig::from_domain(
                        &config.user_model.push,
                    ),
                    clock,
                );
```

- [ ] **Step 3: Run cargo build**

Run: `cd src-tauri && cargo build`
Expected: success — no type errors.

If errors mention `llm_provider` no longer being usable after `.clone()`, capture it once into a variable before both `UserModel::spawn` and `SuggestionGenerator::new`. The existing wiring already does this on line 338-339; just confirm the order of moves works out. Worst case: bind `let llm_provider = llm_service.active_provider();` once and use `.clone()` for both consumers.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/services/push_pipeline.rs src-tauri/src/lib.rs
git commit -m "feat(push_pipeline): integrate SuggestionGenerator with degrade fallback"
```

---

## Task 10: Update push_pipeline_end_to_end tests

**Files:**
- Modify: `src-tauri/tests/push_pipeline_end_to_end.rs`

- [ ] **Step 1: Patch the test harness to construct a SuggestionGenerator**

First enumerate the touched call sites:

```bash
grep -n "push_pipeline::spawn\|PushPipelineDeps {" src-tauri/tests/push_pipeline_end_to_end.rs
```

For each `PushPipelineDeps { ... }` literal, add a `suggestion_generator: ...` field; for each spawn call construct a generator. If the file uses `push_pipeline::spawn(PushPipelineDeps { ... })` directly with a custom decider, add to imports:

If the file uses `push_pipeline::spawn` directly with a custom decider, add to imports:

```rust
use std::sync::Arc;
use corivo_app_lib::{
    db::repos::suggestions::{SqliteSuggestionRepo, SuggestionRepo},
    domain::config::RetrievalConfig,
    providers::llm::{mock::MockLlm, LlmProvider},
    services::suggestion_generator::SuggestionGenerator,
};
```

In each test that spawns the pipeline, add (with concrete LLM responses chosen per test):

```rust
let llm: Arc<dyn LlmProvider> = Arc::new(MockLlm::with_responses(vec![
    serde_json::json!([
        {"text": "test surface", "reasoning": "rs"},
        {"text": "alt 1", "reasoning": "ra1"},
        {"text": "alt 2", "reasoning": "ra2"},
    ])
    .to_string(),
]));
let suggestion_repo: Arc<dyn SuggestionRepo> =
    Arc::new(SqliteSuggestionRepo::new(pool.clone()));
let suggestion_generator = Arc::new(SuggestionGenerator::new(
    proposition_repo.clone(),
    suggestion_repo,
    llm,
    RetrievalConfig {
        w_confidence: 1.0,
        w_decay: 1.0,
        k_decay_days: 14.0,
        limit_multiplier: 3,
    },
    None,
));
```

And add `suggestion_generator` to the `PushPipelineDeps { ... }` literal.

For any existing assertion that read `payload.body == proposition.text`, update it:
- For the **happy** test: assert `payload.title == "Corivo 想跟你说一句"` and `payload.body == "test surface"`.
- For tests where the LLM should fail (build a `MockLlm::with_responses(vec![])`), assert `payload.title == "Corivo 学到了一条新判断"` and `payload.body == proposition.text`.

- [ ] **Step 2: Add an explicit fallback test**

Append a new `#[tokio::test]` that constructs an empty MockLlm queue (forcing failure) and asserts:
1. `notification_service` still receives a payload
2. `payload.body == proposition.text` (degraded)
3. The corresponding `notification_log` row's `suggestion_id` is `None`

Use the existing `RecordingProvider` pattern from this file as the notification sink so you can read `provider.sent.lock().unwrap()`.

```rust
#[tokio::test]
async fn fallback_path_uses_proposition_text_and_null_suggestion_id() {
    // ... reuse the test harness; pass MockLlm::with_responses(vec![]) ...
    // After triggering one PropositionRevised event:
    let sent = provider.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].title, "Corivo 学到了一条新判断");
    assert_eq!(sent[0].body, proposition.text); // captured from your seed

    let log = log_repo.recent(1).await.unwrap();
    assert_eq!(log[0].suggestion_id, None);
}
```

- [ ] **Step 3: Add an explicit happy-path test**

Mirror the fallback test but with a real LLM response queued. Assert:
1. `payload.title == "Corivo 想跟你说一句"`
2. `payload.body == "test surface"` (or whatever the queued response's first item was)
3. `notification_log.suggestion_id == Some(<inserted_id>)`
4. The `suggestions` table contains 3 rows for this proposition_id

- [ ] **Step 4: Run all push_pipeline tests**

Run: `cd src-tauri && cargo test --test push_pipeline_end_to_end`
Expected: PASS — all pre-existing tests + 2 new ones.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/tests/push_pipeline_end_to_end.rs
git commit -m "test(push_pipeline): cover suggestion happy + fallback paths"
```

---

## Task 11: End-to-end integration test

**Files:**
- Create: `src-tauri/tests/suggestion_generator_end_to_end.rs`

- [ ] **Step 1: Write the integration test**

Create `src-tauri/tests/suggestion_generator_end_to_end.rs`:

```rust
//! End-to-end: real SQLite + MockLlm + push_pipeline. Verifies the full
//! PropositionRevised → push_decider → suggestion_generator → notification_log
//! → notification_service path lands data in every table and surfaces a
//! payload that carries the first generated suggestion text.

use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use chrono::Utc;
use corivo_app_lib::{
    db::{
        migrations::apply_migrations,
        pool::test_in_memory_pool,
        repos::{
            notification_log::{NotificationLogRepo, SqliteNotificationLogRepo},
            propositions::{NewProposition, PropositionRepo, SqlitePropositionRepo},
            suggestions::{SqliteSuggestionRepo, SuggestionRepo},
        },
    },
    domain::config::{NotificationConfig, NotificationProviderKind, RetrievalConfig},
    events::{PropositionRevised, RevisionOpTag},
    providers::{
        llm::{mock::MockLlm, LlmProvider},
        notification::{NotificationPayload, NotificationProvider},
    },
    services::{
        notification_service::{NotificationProviderFactory, NotificationService},
        push_decider::{Clock, DefaultPushDecider, PushConfig, PushDecider},
        push_pipeline::{self, PushPipelineDeps},
        suggestion_generator::SuggestionGenerator,
    },
};
use tokio::sync::broadcast;

#[derive(Default)]
struct RecordingProvider {
    sent: Mutex<Vec<NotificationPayload>>,
}

impl NotificationProvider for RecordingProvider {
    fn send(
        &self,
        payload: NotificationPayload,
    ) -> corivo_app_lib::providers::notification::Result<()> {
        self.sent.lock().unwrap().push(payload);
        Ok(())
    }
}

struct RecordingFactory {
    provider: Arc<RecordingProvider>,
}

impl NotificationProviderFactory for RecordingFactory {
    fn create(
        &self,
        _provider_kind: &NotificationProviderKind,
        _config: &NotificationConfig,
    ) -> corivo_app_lib::error::Result<Arc<dyn NotificationProvider>> {
        Ok(self.provider.clone())
    }
}

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> chrono::DateTime<Utc> {
        Utc::now()
    }
}

#[tokio::test]
async fn end_to_end_push_with_three_suggestions() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();

    let proposition_repo = Arc::new(SqlitePropositionRepo::new(pool.clone()));
    let suggestion_repo: Arc<dyn SuggestionRepo> =
        Arc::new(SqliteSuggestionRepo::new(pool.clone()));
    let notification_log = Arc::new(SqliteNotificationLogRepo::new(pool.clone()));

    // Seed a proposition that will pass push_decider's gates.
    let prop = proposition_repo
        .insert(NewProposition {
            text: "用户偏好凌晨工作".into(),
            reasoning: "0:00-3:00 仍在 IDE".into(),
            confidence: Some(8),
            decay: Some(7),
            revision_group: "g1".into(),
            version: 1,
            contradicts_proposition_id: None,
        })
        .await
        .unwrap();

    let llm_response = serde_json::json!([
        {"text": "看起来你这周凌晨工作的节奏在加快。", "reasoning": "情境式回声"},
        {"text": "要不要明早把会挪一挪？", "reasoning": "轻量提议"},
        {"text": "Corivo 可以帮你统计本周凌晨时段。", "reasoning": "数据回看"}
    ])
    .to_string();
    let llm: Arc<dyn LlmProvider> =
        Arc::new(MockLlm::with_responses(vec![llm_response]));
    let suggestion_generator = Arc::new(SuggestionGenerator::new(
        proposition_repo.clone() as Arc<dyn PropositionRepo>,
        suggestion_repo.clone(),
        llm,
        RetrievalConfig {
            w_confidence: 1.0,
            w_decay: 1.0,
            k_decay_days: 14.0,
            limit_multiplier: 3,
        },
        None,
    ));

    // Notification sink that records what was sent.
    let recording = Arc::new(RecordingProvider::default());
    let factory = Arc::new(RecordingFactory { provider: recording.clone() });
    let notification_service = Arc::new(NotificationService::new(
        NotificationProviderKind::Overlay,
        NotificationConfig::default(),
        factory,
    ));

    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let decider: Arc<dyn PushDecider> = Arc::new(DefaultPushDecider::new(
        notification_log.clone(),
        PushConfig::default(),
        Arc::new(push_pipeline::RuntimeClock::new(clock.clone())),
    ));

    let (tx, rx) = broadcast::channel::<PropositionRevised>(8);
    push_pipeline::spawn(PushPipelineDeps {
        revised_rx: rx,
        proposition_repo: proposition_repo.clone(),
        notification_log: notification_log.clone(),
        notification_service,
        decider,
        clock,
        suggestion_generator,
    });

    tx.send(PropositionRevised {
        proposition_id: prop.id,
        revision_group: prop.revision_group.clone(),
        op: RevisionOpTag::Update,
    })
    .unwrap();

    // Drive the spawned task forward.
    tokio::time::sleep(StdDuration::from_millis(150)).await;

    let sent = recording.sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "exactly one notification expected");
    assert_eq!(sent[0].title, "Corivo 想跟你说一句");
    assert_eq!(sent[0].body, "看起来你这周凌晨工作的节奏在加快。");

    let stored = suggestion_repo
        .by_proposition_id(prop.id)
        .await
        .unwrap();
    assert_eq!(stored.len(), 3);

    let log = notification_log.recent(1).await.unwrap();
    assert!(log[0].suggestion_id.is_some(), "log should reference the surfaced suggestion");
}
```

- [ ] **Step 2: Run the test**

Run: `cd src-tauri && cargo test --test suggestion_generator_end_to_end`
Expected: PASS.

If `tokio::time::sleep` is too short for the spawned consumer to drain the broadcast, bump to 300ms. If `Default::default()` for `PushConfig` doesn't exist, look up the actual constructor — likely `PushConfig::default()` or `PushConfig::from_domain(&Config::default().user_model.push)`.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/tests/suggestion_generator_end_to_end.rs
git commit -m "test(suggestion_generator): add end-to-end PropositionRevised → overlay test"
```

---

## Task 12: Final regression sweep

- [ ] **Step 1: Run the full test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS — every test green.

- [ ] **Step 2: Run frontend type-check + tests**

Run: `pnpm exec tsc --noEmit`
Run: `pnpm test`
Expected: PASS for `tsc`. `pnpm test` may have pre-existing flakes unrelated to this change (per the earlier `window is not defined` issue) — confirm any failures are not in `prompt-debug-section.test.tsx` (if it exists) or `config-tauri.test.ts`.

- [ ] **Step 3: Run a manual smoke if possible**

If you can run the app: `pnpm tauri dev`. Open Settings → Prompt 调试, confirm the new "建议生成 Prompt" group appears. Trigger a real notification by waiting for a proposition to be created (or using the test notification command in Settings → 测试). Observe whether the overlay title says "Corivo 想跟你说一句" — that confirms the new payload path is live.

- [ ] **Step 4: Final commit (if anything left)**

If any small fixes came out of the regression sweep:

```bash
git add -p
git commit -m "fix: regression sweep adjustments after suggestion_generator landing"
```

---

## Done Criteria

- All 12 tasks committed.
- `cd src-tauri && cargo test` is green.
- `pnpm exec tsc --noEmit` is clean.
- A real `PropositionRevised` event causes a notification with title `Corivo 想跟你说一句` (or, on LLM failure, the legacy title), and the `suggestions` + `notification_log` tables both gain rows.
- C-phase roadmap remains untouched in [`docs/superpowers/plans/2026-04-18-proactive-push-roadmap.md`](2026-04-18-proactive-push-roadmap.md).
