# Proactivity Scorer (C 阶段) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the 4-gate hard-threshold `push_decider` with an LLM-scored proactivity system (P_S 1-5 + user-tunable θ), with one safety rule (dismissed revision_group = permanent block). Persist every scoring attempt for future evaluation.

**Architecture:** New `ProactivityScorer` service runs after a dismiss-block check and before `suggestion_generator`. Score persists to a new `proactivity_scores` table; `notification_log.proactivity_score_id` links pushed entries to the score that passed. `PushConfig` collapses from 4 fields to 1 (`proactivity_threshold: u8`, default 4). `push_decider.rs` is deleted; `Clock`/`SystemClock`/`MockClock` move to a new `services/clock.rs`.

**Tech Stack:** Rust (Tauri 2) backend, rusqlite + r2d2 + async-trait, `LlmProvider` trait + `MockLlm`, `insta` snapshots, React 19 + TypeScript front-end.

**Reference spec:** [`docs/superpowers/specs/2026-04-19-proactivity-scorer-design.md`](../specs/2026-04-19-proactivity-scorer-design.md)

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `src-tauri/src/db/migrations/008_proactivity.sql` | Create | DDL: create `proactivity_scores` + add `notification_log.proactivity_score_id` |
| `src-tauri/src/db/migrations.rs` | Modify | Register v8, bump `LATEST_VERSION = 8`, add `apply_v8` |
| `src-tauri/src/db/repos/proactivity_scores.rs` | Create | `ProactivityScoreRepo` trait + SQLite impl |
| `src-tauri/src/db/repos/mod.rs` | Modify | Re-export `proactivity_scores` |
| `src-tauri/src/db/repos/notification_log.rs` | Modify | Add `proactivity_score_id` field; new methods `group_ever_dismissed` / `recent_in_group` / `recent_window` |
| `src-tauri/src/services/clock.rs` | Create | Move `Clock` trait + `SystemClock` + `MockClock` here (split from `push_decider`) |
| `src-tauri/src/services/user_model/retrieval.rs` | Modify | Add public `related_for_proposition()` helper |
| `src-tauri/src/services/user_model/prompts.rs` | Modify | Add `render_score` + `ScoreInput` + helper types |
| `src-tauri/prompts/score.md` | Create | C-stage proactivity prompt |
| `src-tauri/src/services/proactivity_scorer.rs` | Create | Pure service: retrieve → render → LLM → persist |
| `src-tauri/src/services/suggestion_generator.rs` | Modify | Replace private `retrieve_related` with `related_for_proposition` call |
| `src-tauri/src/services/push_pipeline.rs` | Modify | Rewrite `handle_event`; drop `PushDecider`, add `ProactivityScorer` dep; relocate `spawn_default` signature |
| `src-tauri/src/services/push_decider.rs` | Delete | Replaced by hard-rule-in-handle_event + scorer |
| `src-tauri/src/services/mod.rs` | Modify | Add `clock` and `proactivity_scorer`, remove `push_decider` |
| `src-tauri/src/domain/config.rs` | Modify | `PushConfig` simplification + `PromptDebugConfig.score_override` + `validate()` |
| `src-tauri/src/lib.rs` | Modify | Wire `ProactivityScorer` + construct `SystemClock` from new location |
| `src-tauri/tests/migrations_v8.rs` | Create | Verify v7→v8 step |
| `src-tauri/tests/proactivity_scores_repo.rs` | Create | Repo unit tests |
| `src-tauri/tests/proactivity_scorer.rs` | Create | Scorer unit tests (happy + error paths) |
| `src-tauri/tests/push_pipeline_proactivity.rs` | Create | Scorer-aware end-to-end tests |
| `src-tauri/tests/prompt_snapshots.rs` | Modify | Add 4 `score_*` snapshots |
| `src-tauri/tests/notification_log_repo.rs` | Modify | Tests for `group_ever_dismissed` / `recent_in_group` / `recent_window`; `proactivity_score_id` field |
| `src-tauri/tests/config_validate.rs` | Modify | `proactivity_threshold` range + `score_override` normalize |
| `src-tauri/tests/push_decider.rs` | Delete | Targets deleted service |
| `src-tauri/tests/push_pipeline_dismiss_penalty.rs` | Delete | Concept removed with `dismiss_penalty_secs` |
| `src-tauri/tests/push_pipeline_end_to_end.rs` | Modify | Drop `second_push_in_same_group_is_skipped_within_cooldown`; add scorer mocks to remaining cases |
| `src-tauri/tests/suggestion_generator.rs` | Modify | Update any `PushConfig` fixtures (grep) |
| `src-tauri/tests/suggestion_generator_end_to_end.rs` | Modify | Same: grep for `PushConfig` fixtures |
| `src/lib/types.ts` | Modify | `PushConfig` becomes `{proactivity_threshold}`; `PromptDebugConfig.score_override` |
| `src/lib/config-tauri.test.ts` | Modify | Fixture updates |
| `src/pages/settings/sections/general-section.tsx` | Modify | Remove `push_on_contradict` ToggleRow; add proactivity slider |
| `src/pages/settings/sections/prompt-debug-section.tsx` | Modify | Add score-override textarea mirroring `suggest_override` |

---

## Task 1: v8 migration — schema + register + test

**Files:**
- Create: `src-tauri/src/db/migrations/008_proactivity.sql`
- Modify: `src-tauri/src/db/migrations.rs`
- Create: `src-tauri/tests/migrations_v8.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/tests/migrations_v8.rs`:

```rust
use corivo_app_lib::db::{
    migrations::{apply_migrations, LATEST_VERSION},
    pool::test_in_memory_pool,
};

const SCHEMA_V1: &str = include_str!("../src/db/schema.sql");
const SCHEMA_V5: &str = include_str!("../src/db/migrations/005_gum_user_model.sql");
const SCHEMA_V6: &str = include_str!("../src/db/migrations/006_propositions_fts.sql");
const SCHEMA_V7: &str = include_str!("../src/db/migrations/007_suggestions.sql");

fn seed_v7() -> r2d2::Pool<r2d2_sqlite::SqliteConnectionManager> {
    let pool = test_in_memory_pool().expect("pool");
    let conn = pool.get().unwrap();
    conn.execute_batch(SCHEMA_V1).unwrap();
    conn.execute_batch(SCHEMA_V5).unwrap();
    conn.execute_batch(SCHEMA_V6).unwrap();
    conn.execute_batch(SCHEMA_V7).unwrap();
    pool
}

#[test]
fn v8_creates_proactivity_scores_table_and_adds_notification_log_column() {
    let pool = seed_v7();
    let conn = pool.get().unwrap();
    apply_migrations(&conn).expect("migrations apply");

    let v: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, LATEST_VERSION);
    assert_eq!(LATEST_VERSION, 8);

    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='proactivity_scores'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(table_exists, 1);

    let column_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('notification_log') WHERE name='proactivity_score_id'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(column_exists, 1);

    // CHECK constraint: p_s must be 1-5
    let err = conn
        .execute(
            "INSERT INTO proactivity_scores
             (proposition_id, p_s, reasoning, threshold_at_time, scored_at)
             VALUES (1, 0, 'x', 4, '2026-04-19T10:00:00Z')",
            [],
        )
        .unwrap_err();
    assert!(format!("{err}").contains("CHECK"), "expected CHECK violation, got {err}");
}

#[test]
fn v8_is_idempotent_when_run_twice() {
    let pool = seed_v7();
    let conn = pool.get().unwrap();
    apply_migrations(&conn).expect("first apply");
    apply_migrations(&conn).expect("second apply must be a no-op");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --test migrations_v8`
Expected: FAIL — `LATEST_VERSION` is still 7.

- [ ] **Step 3: Create the v8 SQL file**

Create `src-tauri/src/db/migrations/008_proactivity.sql`:

```sql
-- v8: C 阶段第一子系统 —— 用 LLM 主动性分数 P_S 替代 4 道硬阈值 gate。
-- 每次 scorer 调用都记录（包括 P_S < θ 被 skip 的），便于后续 C+ 阶段
-- 用真实数据调 θ、算 Acc-P / MD / FD 等评估指标。
-- notification_log 加 proactivity_score_id 回指，让推送历史页能解释
-- "为什么这条被推了"。

CREATE TABLE proactivity_scores (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    proposition_id    INTEGER NOT NULL REFERENCES propositions(id) ON DELETE CASCADE,
    p_s               INTEGER NOT NULL CHECK (p_s >= 1 AND p_s <= 5),
    reasoning         TEXT    NOT NULL,
    threshold_at_time INTEGER NOT NULL,
    scored_at         TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_proactivity_scores_proposition_id ON proactivity_scores(proposition_id);
CREATE INDEX idx_proactivity_scores_scored_at ON proactivity_scores(scored_at DESC);

ALTER TABLE notification_log
    ADD COLUMN proactivity_score_id INTEGER NULL REFERENCES proactivity_scores(id) ON DELETE SET NULL;

INSERT OR IGNORE INTO schema_version (version) VALUES (8);
```

- [ ] **Step 4: Register v8 in migrations.rs**

Edit `src-tauri/src/db/migrations.rs`:

Replace lines 7-12 with:

```rust
const SCHEMA_V1: &str = include_str!("schema.sql");
const SCHEMA_V5: &str = include_str!("migrations/005_gum_user_model.sql");
const SCHEMA_V6: &str = include_str!("migrations/006_propositions_fts.sql");
const SCHEMA_V7: &str = include_str!("migrations/007_suggestions.sql");
const SCHEMA_V8: &str = include_str!("migrations/008_proactivity.sql");

pub const LATEST_VERSION: i64 = 8;
```

After the `current_version < 7` block (around line 50), insert:

```rust
    let current_version = get_current_version(conn)?;
    if current_version < 8 {
        apply_v8(conn)?;
    }
```

After `fn apply_v7(...)`, insert:

```rust
fn apply_v8(conn: &DbConnection) -> Result<()> {
    conn.execute_batch(SCHEMA_V8)
        .map_err(|error| CorivoError::Internal(format!("V8 migration failed: {error}")))?;
    tracing::info!(schema_version = 8, "applied v8 migration");
    Ok(())
}
```

- [ ] **Step 5: Update existing v7 test that asserts `LATEST_VERSION == 7`**

Edit `src-tauri/tests/migrations_v7.rs` line 30 — remove the specific version assertion so it only checks `v == LATEST_VERSION`:

Replace:
```rust
    assert_eq!(v, LATEST_VERSION);
    assert_eq!(LATEST_VERSION, 7);
```

With:
```rust
    assert_eq!(v, LATEST_VERSION);
```

- [ ] **Step 6: Run test to verify pass + no regression**

Run: `cd src-tauri && cargo test --test migrations_v8 --test migrations_v7 --test migrations_v6 --test migrations_v5`
Expected: PASS for all.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/db/migrations/008_proactivity.sql \
        src-tauri/src/db/migrations.rs \
        src-tauri/tests/migrations_v8.rs \
        src-tauri/tests/migrations_v7.rs
git commit -m "feat(db): add v8 migration for proactivity_scores table"
```

---

## Task 2: ProactivityScoreRepo trait + SQLite impl

**Files:**
- Create: `src-tauri/src/db/repos/proactivity_scores.rs`
- Modify: `src-tauri/src/db/repos/mod.rs`
- Create: `src-tauri/tests/proactivity_scores_repo.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/tests/proactivity_scores_repo.rs`:

```rust
use corivo_app_lib::db::{
    migrations::apply_migrations,
    pool::test_in_memory_pool,
    repos::{
        proactivity_scores::{
            NewProactivityScore, ProactivityScoreRepo, SqliteProactivityScoreRepo,
        },
        propositions::{NewProposition, PropositionRepo, SqlitePropositionRepo},
    },
};

async fn seed_proposition(pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>) -> i64 {
    SqlitePropositionRepo::new(pool.clone())
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
        .unwrap()
        .id
}

#[tokio::test]
async fn insert_persists_row_with_threshold_snapshot() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop_id = seed_proposition(&pool).await;

    let repo = SqliteProactivityScoreRepo::new(pool);
    let inserted = repo
        .insert(NewProactivityScore {
            proposition_id: prop_id,
            p_s: 4,
            reasoning: "用户最近没同类推送".into(),
            threshold_at_time: 4,
        })
        .await
        .unwrap();

    assert_eq!(inserted.proposition_id, prop_id);
    assert_eq!(inserted.p_s, 4);
    assert_eq!(inserted.threshold_at_time, 4);
    assert!(inserted.id > 0);
}

#[tokio::test]
async fn by_id_round_trips() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop_id = seed_proposition(&pool).await;

    let repo = SqliteProactivityScoreRepo::new(pool);
    let inserted = repo
        .insert(NewProactivityScore {
            proposition_id: prop_id,
            p_s: 3,
            reasoning: "中性".into(),
            threshold_at_time: 4,
        })
        .await
        .unwrap();

    let fetched = repo.by_id(inserted.id).await.unwrap().expect("exists");
    assert_eq!(fetched, inserted);
}

#[tokio::test]
async fn by_proposition_id_returns_in_scored_at_order() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop_id = seed_proposition(&pool).await;

    let repo = SqliteProactivityScoreRepo::new(pool);
    for p_s in [1_u8, 3, 5] {
        repo.insert(NewProactivityScore {
            proposition_id: prop_id,
            p_s,
            reasoning: format!("p_s={p_s}"),
            threshold_at_time: 4,
        })
        .await
        .unwrap();
    }

    let listed = repo.by_proposition_id(prop_id).await.unwrap();
    assert_eq!(listed.len(), 3);
    // Later inserts have later `scored_at`; list should be DESC by time.
    assert_eq!(listed[0].p_s, 5);
    assert_eq!(listed[1].p_s, 3);
    assert_eq!(listed[2].p_s, 1);
}

#[tokio::test]
async fn check_constraint_rejects_p_s_zero() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop_id = seed_proposition(&pool).await;

    let repo = SqliteProactivityScoreRepo::new(pool);
    let result = repo
        .insert(NewProactivityScore {
            proposition_id: prop_id,
            p_s: 0,
            reasoning: "invalid".into(),
            threshold_at_time: 4,
        })
        .await;
    assert!(result.is_err(), "p_s=0 must be rejected by CHECK");
}

#[tokio::test]
async fn check_constraint_rejects_p_s_six() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop_id = seed_proposition(&pool).await;

    let repo = SqliteProactivityScoreRepo::new(pool);
    let result = repo
        .insert(NewProactivityScore {
            proposition_id: prop_id,
            p_s: 6,
            reasoning: "invalid".into(),
            threshold_at_time: 4,
        })
        .await;
    assert!(result.is_err(), "p_s=6 must be rejected by CHECK");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --test proactivity_scores_repo`
Expected: FAIL — module `proactivity_scores` doesn't exist.

- [ ] **Step 3: Create the repo**

Create `src-tauri/src/db/repos/proactivity_scores.rs`:

```rust
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{
    db::{
        pool::{run_blocking, DbPool},
        types::{datetime_to_sql, parse_datetime},
    },
    error::{CorivoError, Result},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProactivityScore {
    pub id: i64,
    pub proposition_id: i64,
    pub p_s: u8,
    pub reasoning: String,
    pub threshold_at_time: u8,
    pub scored_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewProactivityScore {
    pub proposition_id: i64,
    pub p_s: u8,
    pub reasoning: String,
    pub threshold_at_time: u8,
}

#[async_trait]
pub trait ProactivityScoreRepo: Send + Sync {
    async fn insert(&self, new: NewProactivityScore) -> Result<ProactivityScore>;
    async fn by_id(&self, id: i64) -> Result<Option<ProactivityScore>>;
    async fn by_proposition_id(&self, proposition_id: i64) -> Result<Vec<ProactivityScore>>;
}

#[derive(Clone)]
pub struct SqliteProactivityScoreRepo {
    pool: DbPool,
}

impl SqliteProactivityScoreRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

const SELECT_COLUMNS: &str =
    "id, proposition_id, p_s, reasoning, threshold_at_time, scored_at";

#[async_trait]
impl ProactivityScoreRepo for SqliteProactivityScoreRepo {
    async fn insert(&self, new: NewProactivityScore) -> Result<ProactivityScore> {
        let scored_at = datetime_to_sql(&Utc::now());
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row(
                &format!(
                    "INSERT INTO proactivity_scores
                     (proposition_id, p_s, reasoning, threshold_at_time, scored_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     RETURNING {SELECT_COLUMNS}"
                ),
                params![
                    new.proposition_id,
                    new.p_s as i64,
                    new.reasoning,
                    new.threshold_at_time as i64,
                    scored_at,
                ],
                row_to_score,
            )
            .map_err(|error| {
                CorivoError::Internal(format!("插入 proactivity_score 失败: {error}"))
            })
        })
        .await
    }

    async fn by_id(&self, id: i64) -> Result<Option<ProactivityScore>> {
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row(
                &format!("SELECT {SELECT_COLUMNS} FROM proactivity_scores WHERE id = ?1"),
                params![id],
                row_to_score,
            )
            .optional()
            .map_err(|error| {
                CorivoError::Internal(format!("查询 proactivity_score 失败: {error}"))
            })
        })
        .await
    }

    async fn by_proposition_id(&self, proposition_id: i64) -> Result<Vec<ProactivityScore>> {
        run_blocking(self.pool.clone(), move |conn| {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {SELECT_COLUMNS} FROM proactivity_scores
                     WHERE proposition_id = ?1
                     ORDER BY scored_at DESC, id DESC"
                ))
                .map_err(|error| {
                    CorivoError::Internal(format!("prepare proactivity_scores list 失败: {error}"))
                })?;
            let rows = stmt
                .query_map(params![proposition_id], row_to_score)
                .map_err(|error| {
                    CorivoError::Internal(format!("query proactivity_scores list 失败: {error}"))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
                CorivoError::Internal(format!("collect proactivity_scores list 失败: {error}"))
            })
        })
        .await
    }
}

fn row_to_score(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProactivityScore> {
    let scored_at = parse_datetime(&row.get::<_, String>(5)?)?;
    Ok(ProactivityScore {
        id: row.get(0)?,
        proposition_id: row.get(1)?,
        p_s: row.get::<_, i64>(2)? as u8,
        reasoning: row.get(3)?,
        threshold_at_time: row.get::<_, i64>(4)? as u8,
        scored_at,
    })
}
```

- [ ] **Step 4: Register module**

Edit `src-tauri/src/db/repos/mod.rs` — add `pub mod proactivity_scores;` alphabetically (between `observations` and `propositions`):

```rust
pub mod notification_log;
pub mod observations;
pub mod proactivity_scores;
pub mod propositions;
pub mod screenshots;
pub mod segments;
pub mod sessions;
pub mod suggestions;
```

- [ ] **Step 5: Run test to verify pass**

Run: `cd src-tauri && cargo test --test proactivity_scores_repo`
Expected: all 5 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db/repos/proactivity_scores.rs \
        src-tauri/src/db/repos/mod.rs \
        src-tauri/tests/proactivity_scores_repo.rs
git commit -m "feat(db): add ProactivityScoreRepo with threshold snapshot"
```

---

## Task 3: Add `proactivity_score_id` + 3 new query methods to NotificationLog

**Files:**
- Modify: `src-tauri/src/db/repos/notification_log.rs`
- Modify: `src-tauri/tests/notification_log_repo.rs`

- [ ] **Step 1: Write the failing tests**

Append to `src-tauri/tests/notification_log_repo.rs`:

```rust
#[tokio::test]
async fn group_ever_dismissed_returns_true_when_any_dismissed_in_group() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let repo = SqliteNotificationLogRepo::new(pool);

    let entry = repo
        .insert(NewNotificationLog {
            revision_group: "g-1".into(),
            proposition_id: None,
            pushed_at: ts("2026-04-19T08:00:00Z"),
            suggestion_id: None,
        })
        .await
        .unwrap();
    repo.set_outcome_at(entry.id, "dismissed", ts("2026-04-19T08:05:00Z"))
        .await
        .unwrap();

    assert!(repo.group_ever_dismissed("g-1").await.unwrap());
    assert!(!repo.group_ever_dismissed("g-2").await.unwrap());
}

#[tokio::test]
async fn group_ever_dismissed_returns_false_for_acknowledged_or_ignored() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let repo = SqliteNotificationLogRepo::new(pool);

    let acked = repo
        .insert(NewNotificationLog {
            revision_group: "g-ack".into(),
            proposition_id: None,
            pushed_at: ts("2026-04-19T08:00:00Z"),
            suggestion_id: None,
        })
        .await
        .unwrap();
    repo.set_outcome_at(acked.id, "acknowledged", ts("2026-04-19T08:05:00Z"))
        .await
        .unwrap();

    let ignored = repo
        .insert(NewNotificationLog {
            revision_group: "g-ign".into(),
            proposition_id: None,
            pushed_at: ts("2026-04-19T08:00:00Z"),
            suggestion_id: None,
        })
        .await
        .unwrap();
    repo.set_outcome_at(ignored.id, "ignored", ts("2026-04-19T08:05:00Z"))
        .await
        .unwrap();

    assert!(!repo.group_ever_dismissed("g-ack").await.unwrap());
    assert!(!repo.group_ever_dismissed("g-ign").await.unwrap());
}

#[tokio::test]
async fn recent_in_group_returns_ordered_by_pushed_at_desc() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let repo = SqliteNotificationLogRepo::new(pool);

    for stamp in [
        "2026-04-19T08:00:00Z",
        "2026-04-19T09:00:00Z",
        "2026-04-19T10:00:00Z",
    ] {
        repo.insert(NewNotificationLog {
            revision_group: "g-1".into(),
            proposition_id: None,
            pushed_at: ts(stamp),
            suggestion_id: None,
        })
        .await
        .unwrap();
    }
    // Different group — must be excluded.
    repo.insert(NewNotificationLog {
        revision_group: "g-2".into(),
        proposition_id: None,
        pushed_at: ts("2026-04-19T11:00:00Z"),
        suggestion_id: None,
    })
    .await
    .unwrap();

    let got = repo.recent_in_group("g-1", 2).await.unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].pushed_at, ts("2026-04-19T10:00:00Z"));
    assert_eq!(got[1].pushed_at, ts("2026-04-19T09:00:00Z"));
}

#[tokio::test]
async fn recent_window_returns_only_entries_within_duration() {
    use chrono::Duration;
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let repo = SqliteNotificationLogRepo::new(pool);

    let now = Utc::now();
    // 30 minutes ago — inside window
    repo.insert(NewNotificationLog {
        revision_group: "recent".into(),
        proposition_id: None,
        pushed_at: now - Duration::minutes(30),
        suggestion_id: None,
    })
    .await
    .unwrap();
    // 2 hours ago — outside window
    repo.insert(NewNotificationLog {
        revision_group: "old".into(),
        proposition_id: None,
        pushed_at: now - Duration::hours(2),
        suggestion_id: None,
    })
    .await
    .unwrap();

    let got = repo.recent_window(Duration::hours(1)).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].revision_group, "recent");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --test notification_log_repo group_ever_dismissed`
Expected: FAIL — method doesn't exist.

- [ ] **Step 3: Add `proactivity_score_id` to struct + implement 3 methods**

Edit `src-tauri/src/db/repos/notification_log.rs`:

Update `NotificationLogEntry` struct (between id and joined fields, keep joined fields at end):

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
    pub proactivity_score_id: Option<i64>,
    /// Joined from `propositions.text` when the row references a still-extant
    /// proposition. Always `None` on the `insert()` path (RETURNING can't JOIN);
    /// populated by `recent()` and `last_in_group()`.
    pub proposition_text: Option<String>,
    pub proposition_reasoning: Option<String>,
    pub proposition_confidence: Option<i32>,
    /// Joined from `suggestions.text` when `suggestion_id` is `Some(_)`.
    pub suggestion_text: Option<String>,
}
```

Update `NewNotificationLog`:

```rust
#[derive(Debug, Clone)]
pub struct NewNotificationLog {
    pub revision_group: String,
    pub proposition_id: Option<i64>,
    pub pushed_at: DateTime<Utc>,
    pub suggestion_id: Option<i64>,
    pub proactivity_score_id: Option<i64>,
}
```

Update `insert` SQL:

```rust
    async fn insert(&self, new: NewNotificationLog) -> Result<NotificationLogEntry> {
        let pushed_at = datetime_to_sql(&new.pushed_at);
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row(
                "INSERT INTO notification_log (revision_group, proposition_id, pushed_at, suggestion_id, proactivity_score_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 RETURNING id, revision_group, proposition_id, pushed_at, outcome, outcome_at, suggestion_id, proactivity_score_id",
                params![new.revision_group, new.proposition_id, pushed_at, new.suggestion_id, new.proactivity_score_id],
                row_to_entry,
            )
            .map_err(|error| {
                CorivoError::Internal(format!("插入 notification_log 失败: {error}"))
            })
        })
        .await
    }
```

Update trait + add new methods:

```rust
#[async_trait]
pub trait NotificationLogRepo: Send + Sync {
    async fn insert(&self, new: NewNotificationLog) -> Result<NotificationLogEntry>;
    async fn last_in_group(&self, group: &str) -> Result<Option<NotificationLogEntry>>;
    async fn last_dismissed_in_group(&self, group: &str) -> Result<Option<DateTime<Utc>>>;
    async fn set_outcome(&self, id: i64, outcome: &str) -> Result<()>;
    async fn set_outcome_at(&self, id: i64, outcome: &str, outcome_at: DateTime<Utc>) -> Result<()>;
    async fn recent(&self, limit: u32) -> Result<Vec<NotificationLogEntry>>;
    /// C 阶段硬规则：同 revision_group 是否曾被用户 dismiss。
    async fn group_ever_dismissed(&self, group: &str) -> Result<bool>;
    /// 同组最近 N 条（新 → 旧）。
    async fn recent_in_group(&self, group: &str, limit: u32) -> Result<Vec<NotificationLogEntry>>;
    /// 全局最近一段时间内所有推送（给 scorer 判断全局频率）。
    async fn recent_window(&self, window: chrono::Duration) -> Result<Vec<NotificationLogEntry>>;
}
```

Add impls (place after `recent()` in the `impl NotificationLogRepo for SqliteNotificationLogRepo` block):

```rust
    async fn group_ever_dismissed(&self, group: &str) -> Result<bool> {
        let group = group.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            conn.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM notification_log
                     WHERE revision_group = ?1 AND outcome = 'dismissed'
                     LIMIT 1
                 )",
                params![group],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n == 1)
            .map_err(|error| {
                CorivoError::Internal(format!("group_ever_dismissed 查询失败: {error}"))
            })
        })
        .await
    }

    async fn recent_in_group(&self, group: &str, limit: u32) -> Result<Vec<NotificationLogEntry>> {
        let group = group.to_string();
        run_blocking(self.pool.clone(), move |conn| {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {ENRICHED_SELECT} {ENRICHED_FROM}
                     WHERE n.revision_group = ?1
                     ORDER BY n.pushed_at DESC, n.id DESC
                     LIMIT ?2"
                ))
                .map_err(|error| {
                    CorivoError::Internal(format!("prepare recent_in_group 失败: {error}"))
                })?;
            let rows = stmt
                .query_map(params![group, limit], row_to_entry_enriched)
                .map_err(|error| {
                    CorivoError::Internal(format!("query recent_in_group 失败: {error}"))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
                CorivoError::Internal(format!("collect recent_in_group 失败: {error}"))
            })
        })
        .await
    }

    async fn recent_window(&self, window: chrono::Duration) -> Result<Vec<NotificationLogEntry>> {
        let cutoff = datetime_to_sql(&(Utc::now() - window));
        run_blocking(self.pool.clone(), move |conn| {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {ENRICHED_SELECT} {ENRICHED_FROM}
                     WHERE n.pushed_at >= ?1
                     ORDER BY n.pushed_at DESC, n.id DESC"
                ))
                .map_err(|error| {
                    CorivoError::Internal(format!("prepare recent_window 失败: {error}"))
                })?;
            let rows = stmt
                .query_map(params![cutoff], row_to_entry_enriched)
                .map_err(|error| {
                    CorivoError::Internal(format!("query recent_window 失败: {error}"))
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
                CorivoError::Internal(format!("collect recent_window 失败: {error}"))
            })
        })
        .await
    }
```

Update SELECT lists for existing queries that return `NotificationLogEntry`. Replace the `ENRICHED_SELECT` constant:

```rust
const ENRICHED_SELECT: &str = "n.id, n.revision_group, n.proposition_id, n.pushed_at, \
     n.outcome, n.outcome_at, n.suggestion_id, n.proactivity_score_id, \
     p.text, p.reasoning, p.confidence, s.text";
```

Update both `row_to_entry` functions:

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
        proactivity_score_id: row.get(7)?,
        proposition_text: None,
        proposition_reasoning: None,
        proposition_confidence: None,
        suggestion_text: None,
    })
}

fn row_to_entry_enriched(row: &rusqlite::Row<'_>) -> rusqlite::Result<NotificationLogEntry> {
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
        proactivity_score_id: row.get(7)?,
        proposition_text: row.get(8)?,
        proposition_reasoning: row.get(9)?,
        proposition_confidence: row.get(10)?,
        suggestion_text: row.get(11)?,
    })
}
```

Fix the `insert()` SQL `RETURNING` clause to include all 8 `notification_log` columns (it doesn't JOIN, so trailing `p.*` / `s.*` positions stay `None` via `row_to_entry`):

```rust
"INSERT INTO notification_log (revision_group, proposition_id, pushed_at, suggestion_id, proactivity_score_id)
 VALUES (?1, ?2, ?3, ?4, ?5)
 RETURNING id, revision_group, proposition_id, pushed_at, outcome, outcome_at, suggestion_id, proactivity_score_id",
```

(Matches 8 columns read by `row_to_entry`.)

- [ ] **Step 4: Patch all existing `NewNotificationLog { ... }` literals**

Run: `grep -rn "NewNotificationLog {" --include="*.rs" src-tauri`

For each hit, add `proactivity_score_id: None,` before the closing brace. Files to expect (based on plan authoring grep):
- `src-tauri/src/services/push_pipeline.rs` (1 hit)
- `src-tauri/tests/push_decider.rs` (2 hits; will be deleted in Task 9 anyway — still patch to keep build green until then)
- `src-tauri/tests/notification_log_repo.rs` (9 hits)
- `src-tauri/tests/push_pipeline_dismiss_penalty.rs` (none expected, check with grep)

- [ ] **Step 5: Run tests**

Run: `cd src-tauri && cargo build --tests` first — expected PASS.
Then: `cd src-tauri && cargo test --test notification_log_repo`
Expected: all old + 4 new tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db/repos/notification_log.rs \
        src-tauri/src/services/push_pipeline.rs \
        src-tauri/tests/notification_log_repo.rs \
        src-tauri/tests/push_decider.rs
git commit -m "feat(db): add proactivity_score_id + C-stage query methods to notification_log"
```

---

## Task 4: Extract `retrieve_related` to public helper

**Files:**
- Modify: `src-tauri/src/services/user_model/retrieval.rs`
- Modify: `src-tauri/src/services/suggestion_generator.rs`

- [ ] **Step 1: Add public helper to retrieval.rs**

Edit `src-tauri/src/services/user_model/retrieval.rs`. Append (after existing `query` function):

```rust
use crate::db::repos::propositions::Proposition;

/// Shared helper used by both `SuggestionGenerator` and `ProactivityScorer` to
/// pull a pool of related propositions for LLM prompt context. Runs the
/// FTS-backed `query()` with `proposition.text` as the anchor, then filters
/// out every version in the same `revision_group` (so the LLM never sees a
/// self-reference or stale version of the same belief).
///
/// Caller chooses `limit` (typically 5 for B/C prompts). Observations are
/// never included in this path — prompts compose raw observations separately.
pub async fn related_for_proposition(
    repo: &dyn PropositionRepo,
    cfg: &RetrievalConfig,
    prop: &Proposition,
    limit: u32,
) -> Result<Vec<crate::services::user_model::prompts::SuggestRelated>> {
    let scored = query(
        repo,
        cfg,
        RetrievalArgs {
            text: Some(prop.text.clone()),
            limit,
            start_time: None,
            end_time: None,
            include_observations: false,
        },
    )
    .await?;
    Ok(scored
        .into_iter()
        .filter(|s| s.proposition.revision_group != prop.revision_group)
        .map(|s| crate::services::user_model::prompts::SuggestRelated {
            text: s.proposition.text,
            confidence: s.proposition.confidence.unwrap_or(0),
        })
        .collect())
}
```

- [ ] **Step 2: Rewrite `SuggestionGenerator::retrieve_related` to call the helper**

Edit `src-tauri/src/services/suggestion_generator.rs`. Replace the private method body (around lines 95-120) with:

```rust
    async fn retrieve_related(&self, prop: &Proposition) -> Result<Vec<SuggestRelated>> {
        crate::services::user_model::retrieval::related_for_proposition(
            self.proposition_repo.as_ref(),
            &self.retrieval_cfg,
            prop,
            RETRIEVAL_LIMIT,
        )
        .await
        .map_err(|error| {
            CorivoError::Internal(format!(
                "suggestion_generator: retrieval failed: {error}"
            ))
        })
    }
```

- [ ] **Step 3: Run regression tests**

Run: `cd src-tauri && cargo test --test suggestion_generator --test suggestion_generator_end_to_end`
Expected: all B-stage tests still PASS (behavior unchanged).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/services/user_model/retrieval.rs \
        src-tauri/src/services/suggestion_generator.rs
git commit -m "refactor(user_model): extract related_for_proposition to shared helper"
```

---

## Task 5: `render_score` + prompt template + snapshot tests

**Files:**
- Create: `src-tauri/prompts/score.md`
- Modify: `src-tauri/src/services/user_model/prompts.rs`
- Modify: `src-tauri/tests/prompt_snapshots.rs`

- [ ] **Step 1: Create the prompt template**

Create `src-tauri/prompts/score.md`:

```
<!-- version: 1 | C 阶段 Proactivity Score (P_S) — replaces hard-gate min_confidence -->

你是 Corivo 的"打扰守门人"。Corivo 长期观察用户行为，刚刚得出了一条新的判断（命题）。
你要给这条命题打一个**主动性分数 P_S（1-5 整数）**——决定它现在值不值得用刘海通知打扰用户。

打分语义（**保守倾向**——拿不准就给低）：
- 5 = 必须现在告诉用户（紧急 / 用户极可能错过 / 关键决策依据）
- 4 = 应该告诉（明显有价值，且用户当下能接收）
- 3 = 可推可不推（一般性观察、价值中性）
- 2 = 不推为宜（轻微相关、用户大概率不感兴趣）
- 1 = 绝不要推（与用户无关 / 已知重复 / 冗余）

判分时综合考虑：
- **新意**：相比"已有相关命题"，这条带来了新信息吗？还是冗余？
- **当下相关性**：从"用户最近做的事"看，现在打扰合不合适？
- **频率**：最近 1 小时已经推了几条？同主题之前推过吗？反应如何？
- **置信度**：低 confidence 命题（≤4）一般给低分；除非内容本身高价值
- **冲突**：是 contradict 类时用户可能需要裁决，倾向给高分

---

【新命题】
- 内容：{{proposition_text}}
- 依据：{{proposition_reasoning}}
- 置信度：{{proposition_confidence}}/10
- 衰减权重：{{proposition_decay}}/10
- 是否冲突命题：{{is_contradict}}{{contradicts_clause}}

【相关已有命题（按相关性排序）】
{{related_propositions}}

【用户最近的屏幕活动（最新 3 条 observations，由早到晚）】
{{recent_observations}}

【该主题（同 revision_group）最近的推送】
{{group_push_history}}

【全局最近 1 小时的推送概况】
{{global_recent_summary}}

---

仅输出 JSON，恰好一个对象：

```json
{"p_s": 4, "reasoning": "为什么打这个分（中文，30-50 字）"}
```

不要 Markdown 包裹，不要任何额外文字。p_s 必须是 1-5 之间的整数。
```

- [ ] **Step 2: Add types + render function to prompts.rs**

Edit `src-tauri/src/services/user_model/prompts.rs`. At the top, add the new `include_str!`:

```rust
const SCORE_TEMPLATE: &str = include_str!("../../../prompts/score.md");
```

Append after `render_suggest`:

```rust
/// Input for the SCORE stage (C-stage proactivity_scorer).
#[derive(Debug, Clone)]
pub struct ScoreInput {
    pub proposition_text: String,
    pub proposition_reasoning: String,
    pub proposition_confidence: i32,
    pub proposition_decay: i32,
    pub contradicts_text: Option<String>,
    pub related: Vec<SuggestRelated>,
    pub recent_observations: Vec<RecentObservation>,
    pub group_push_history: Vec<PushHistoryRow>,
    pub global_last_hour: PushFrequencySummary,
}

/// A recent `observations` row as shown to the scorer. `content` is truncated
/// to 200 chars at fill time.
#[derive(Debug, Clone)]
pub struct RecentObservation {
    pub created_at: DateTime<Utc>,
    pub content: String,
}

/// One past push in the same revision_group.
#[derive(Debug, Clone)]
pub struct PushHistoryRow {
    pub pushed_at: DateTime<Utc>,
    pub outcome: Option<String>,
}

/// Aggregated stats for the last global-scan window.
#[derive(Debug, Clone)]
pub struct PushFrequencySummary {
    pub total: u32,
    pub acknowledged: u32,
    pub dismissed: u32,
    pub no_response: u32,
}

pub fn render_score(input: &ScoreInput) -> String {
    let is_contradict_text = if input.contradicts_text.is_some() { "是" } else { "否" };
    let contradicts_clause = match input.contradicts_text.as_ref() {
        Some(t) => format!("（与命题「{t}」冲突）"),
        None => String::new(),
    };

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

    let observations_block = if input.recent_observations.is_empty() {
        "（暂无最近活动）".to_string()
    } else {
        input
            .recent_observations
            .iter()
            .map(|o| {
                let preview: String = o.content.chars().take(200).collect();
                format!("- [{}] {}", o.created_at.format("%H:%M"), preview)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let group_history_block = if input.group_push_history.is_empty() {
        "（这是该主题首次推送）".to_string()
    } else {
        input
            .group_push_history
            .iter()
            .map(|r| {
                let outcome_label = r.outcome.as_deref().unwrap_or("未响应");
                format!("- {}: {}", r.pushed_at.to_rfc3339(), outcome_label)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let global_summary = if input.global_last_hour.total == 0 {
        "过去 1 小时无推送".to_string()
    } else {
        format!(
            "过去 1 小时推送 {} 条（acknowledged {} / dismissed {} / 未响应 {}）",
            input.global_last_hour.total,
            input.global_last_hour.acknowledged,
            input.global_last_hour.dismissed,
            input.global_last_hour.no_response,
        )
    };

    SCORE_TEMPLATE
        .replace("{{proposition_text}}", &input.proposition_text)
        .replace("{{proposition_reasoning}}", &input.proposition_reasoning)
        .replace(
            "{{proposition_confidence}}",
            &input.proposition_confidence.to_string(),
        )
        .replace(
            "{{proposition_decay}}",
            &input.proposition_decay.to_string(),
        )
        .replace("{{is_contradict}}", is_contradict_text)
        .replace("{{contradicts_clause}}", &contradicts_clause)
        .replace("{{related_propositions}}", &related_block)
        .replace("{{recent_observations}}", &observations_block)
        .replace("{{group_push_history}}", &group_history_block)
        .replace("{{global_recent_summary}}", &global_summary)
}
```

- [ ] **Step 3: Write the failing snapshot tests**

Append to `src-tauri/tests/prompt_snapshots.rs`:

```rust
use chrono::TimeZone;
use corivo_app_lib::services::user_model::prompts::{
    render_score, PushFrequencySummary, PushHistoryRow, RecentObservation, ScoreInput,
    SuggestRelated,
};

fn at(h: u32, m: u32) -> chrono::DateTime<chrono::Utc> {
    chrono::Utc
        .with_ymd_and_hms(2026, 4, 19, h, m, 0)
        .unwrap()
}

#[test]
fn score_basic() {
    let input = ScoreInput {
        proposition_text: "用户倾向凌晨工作".into(),
        proposition_reasoning: "多次观察到 0:00-3:00 仍在 IDE".into(),
        proposition_confidence: 7,
        proposition_decay: 6,
        contradicts_text: None,
        related: vec![SuggestRelated {
            text: "用户偏好长会议间隙走动".into(),
            confidence: 6,
        }],
        recent_observations: vec![RecentObservation {
            created_at: at(9, 30),
            content: "在 VS Code 编辑 src/lib.rs，频繁切窗口".into(),
        }],
        group_push_history: vec![PushHistoryRow {
            pushed_at: at(7, 0),
            outcome: Some("acknowledged".into()),
        }],
        global_last_hour: PushFrequencySummary {
            total: 2,
            acknowledged: 1,
            dismissed: 0,
            no_response: 1,
        },
    };
    insta::assert_snapshot!(render_score(&input));
}

#[test]
fn score_empty_context() {
    let input = ScoreInput {
        proposition_text: "首次出现的命题".into(),
        proposition_reasoning: "唯一一条观察".into(),
        proposition_confidence: 5,
        proposition_decay: 5,
        contradicts_text: None,
        related: vec![],
        recent_observations: vec![],
        group_push_history: vec![],
        global_last_hour: PushFrequencySummary {
            total: 0,
            acknowledged: 0,
            dismissed: 0,
            no_response: 0,
        },
    };
    insta::assert_snapshot!(render_score(&input));
}

#[test]
fn score_with_contradict() {
    let input = ScoreInput {
        proposition_text: "用户偏好浅色主题".into(),
        proposition_reasoning: "最近每天早上切浅色".into(),
        proposition_confidence: 6,
        proposition_decay: 6,
        contradicts_text: Some("用户偏好深色主题".into()),
        related: vec![],
        recent_observations: vec![],
        group_push_history: vec![],
        global_last_hour: PushFrequencySummary {
            total: 0,
            acknowledged: 0,
            dismissed: 0,
            no_response: 0,
        },
    };
    insta::assert_snapshot!(render_score(&input));
}

#[test]
fn score_high_frequency() {
    let input = ScoreInput {
        proposition_text: "用户在筹备周会".into(),
        proposition_reasoning: "日历+文档活动".into(),
        proposition_confidence: 6,
        proposition_decay: 5,
        contradicts_text: None,
        related: vec![],
        recent_observations: vec![],
        group_push_history: vec![],
        global_last_hour: PushFrequencySummary {
            total: 5,
            acknowledged: 1,
            dismissed: 2,
            no_response: 2,
        },
    };
    insta::assert_snapshot!(render_score(&input));
}
```

- [ ] **Step 4: Accept snapshots**

Run: `cd src-tauri && cargo test --test prompt_snapshots score_`
Expected: tests fail with "snapshot not found" or "pending".

Auto-accept:

```bash
cd src-tauri && INSTA_UPDATE=always cargo test --test prompt_snapshots score_
```

Re-run to verify:

```bash
cd src-tauri && cargo test --test prompt_snapshots score_
```
Expected: all 4 PASS.

Manually inspect the generated `.snap` files in `src-tauri/tests/snapshots/` to sanity-check placeholder fills (no leftover `{{...}}`, correct Chinese text, correct empty sentinels).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/prompts/score.md \
        src-tauri/src/services/user_model/prompts.rs \
        src-tauri/tests/prompt_snapshots.rs \
        src-tauri/tests/snapshots/prompt_snapshots__score_basic.snap \
        src-tauri/tests/snapshots/prompt_snapshots__score_empty_context.snap \
        src-tauri/tests/snapshots/prompt_snapshots__score_with_contradict.snap \
        src-tauri/tests/snapshots/prompt_snapshots__score_high_frequency.snap
git commit -m "feat(prompts): add score.md template + render_score"
```

---

## Task 6: `ProactivityScorer` service — happy path

**Files:**
- Create: `src-tauri/src/services/proactivity_scorer.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Create: `src-tauri/tests/proactivity_scorer.rs`

- [ ] **Step 1: Write the failing happy-path test**

Create `src-tauri/tests/proactivity_scorer.rs`:

```rust
use std::sync::Arc;

use corivo_app_lib::{
    db::{
        migrations::apply_migrations,
        pool::test_in_memory_pool,
        repos::{
            notification_log::{NotificationLogRepo, SqliteNotificationLogRepo},
            observations::SqliteObservationRepo,
            proactivity_scores::{ProactivityScoreRepo, SqliteProactivityScoreRepo},
            propositions::{NewProposition, Proposition, PropositionRepo, SqlitePropositionRepo},
        },
    },
    domain::config::RetrievalConfig,
    providers::llm::{mock::MockLlm, LlmProvider},
    services::proactivity_scorer::ProactivityScorer,
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
    SqlitePropositionRepo::new(pool.clone())
        .insert(NewProposition {
            text: "用户在研究推送机制".into(),
            reasoning: "连续阅读论文和代码".into(),
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
async fn happy_path_inserts_score_and_returns_it() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop = seed_proposition(&pool).await;

    let llm_response = serde_json::json!({
        "p_s": 4,
        "reasoning": "内容新颖且用户当前正在相关上下文"
    })
    .to_string();
    let llm: Arc<dyn LlmProvider> = Arc::new(MockLlm::with_responses(vec![llm_response]));

    let proposition_repo = Arc::new(SqlitePropositionRepo::new(pool.clone()));
    let observation_repo = Arc::new(SqliteObservationRepo::new(pool.clone()));
    let notification_log = Arc::new(SqliteNotificationLogRepo::new(pool.clone()));
    let score_repo: Arc<dyn ProactivityScoreRepo> =
        Arc::new(SqliteProactivityScoreRepo::new(pool.clone()));

    let scorer = ProactivityScorer::new(
        proposition_repo,
        observation_repo,
        notification_log,
        score_repo.clone(),
        llm,
        default_retrieval_cfg(),
        4, // threshold at time
        None, // no prompt override
    );
    let score = scorer.score(&prop).await.unwrap();

    assert_eq!(score.p_s, 4);
    assert_eq!(score.threshold_at_time, 4);
    assert!(score.reasoning.contains("新颖") || !score.reasoning.is_empty());

    let persisted = score_repo.by_id(score.id).await.unwrap().expect("exists");
    assert_eq!(persisted, score);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --test proactivity_scorer happy_path_inserts_score`
Expected: FAIL — module `proactivity_scorer` doesn't exist.

- [ ] **Step 3: Create the service**

Create `src-tauri/src/services/proactivity_scorer.rs`:

```rust
//! C-phase proactivity scorer (spec §3 & §5). Runs after the hard dismiss
//! block and before `suggestion_generator`. Asks the LLM for a 1-5 score
//! summarizing "is this worth interrupting the user right now", and persists
//! every successful score (skip or push) so future calibration has data.

use std::sync::Arc;

use chrono::Duration;
use serde::Deserialize;

use crate::{
    db::repos::{
        notification_log::{NotificationLogEntry, NotificationLogRepo},
        observations::ObservationRepo,
        proactivity_scores::{NewProactivityScore, ProactivityScore, ProactivityScoreRepo},
        propositions::{Proposition, PropositionRepo},
    },
    domain::config::RetrievalConfig,
    error::{CorivoError, Result},
    providers::llm::{LlmProvider, LlmProviderExt, LlmRequest},
    services::user_model::{
        prompts::{
            render_score, PushFrequencySummary, PushHistoryRow, RecentObservation, ScoreInput,
        },
        retrieval::related_for_proposition,
    },
};

const RETRIEVAL_LIMIT: u32 = 5;
const RECENT_OBS_LIMIT: u32 = 3;
const GROUP_HISTORY_LIMIT: u32 = 3;

#[derive(Debug, Deserialize)]
struct RawScore {
    p_s: u8,
    reasoning: String,
}

pub struct ProactivityScorer {
    proposition_repo: Arc<dyn PropositionRepo>,
    observation_repo: Arc<dyn ObservationRepo>,
    notification_log: Arc<dyn NotificationLogRepo>,
    score_repo: Arc<dyn ProactivityScoreRepo>,
    llm: Arc<dyn LlmProvider>,
    retrieval_cfg: RetrievalConfig,
    threshold: u8,
    prompt_override: Option<String>,
}

impl ProactivityScorer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        proposition_repo: Arc<dyn PropositionRepo>,
        observation_repo: Arc<dyn ObservationRepo>,
        notification_log: Arc<dyn NotificationLogRepo>,
        score_repo: Arc<dyn ProactivityScoreRepo>,
        llm: Arc<dyn LlmProvider>,
        retrieval_cfg: RetrievalConfig,
        threshold: u8,
        prompt_override: Option<String>,
    ) -> Self {
        Self {
            proposition_repo,
            observation_repo,
            notification_log,
            score_repo,
            llm,
            retrieval_cfg,
            threshold,
            prompt_override,
        }
    }

    pub async fn score(&self, proposition: &Proposition) -> Result<ProactivityScore> {
        // Parallel fetch of all context inputs.
        let related_fut = related_for_proposition(
            self.proposition_repo.as_ref(),
            &self.retrieval_cfg,
            proposition,
            RETRIEVAL_LIMIT,
        );
        let obs_fut = self.observation_repo.recent(RECENT_OBS_LIMIT);
        let group_history_fut = self
            .notification_log
            .recent_in_group(&proposition.revision_group, GROUP_HISTORY_LIMIT);
        let global_fut = self.notification_log.recent_window(Duration::hours(1));

        let (related, obs, group_history, global) =
            tokio::try_join!(related_fut, obs_fut, group_history_fut, global_fut)?;

        let contradicts_text = self.resolve_contradicts_text(proposition).await?;

        let recent_observations = obs
            .into_iter()
            .rev() // recent() returns DESC; prompt wants earliest-first
            .map(|o| RecentObservation {
                created_at: o.created_at,
                content: o.content,
            })
            .collect();

        let group_push_history = group_history
            .iter()
            .map(|e| PushHistoryRow {
                pushed_at: e.pushed_at,
                outcome: e.outcome.clone(),
            })
            .collect();

        let global_last_hour = summarize_frequency(&global);

        let rendered = if let Some(template) = self.prompt_override.as_deref() {
            // Override path: inline substitution mirroring render_score exactly.
            render_score_with_template(
                template,
                &ScoreInput {
                    proposition_text: proposition.text.clone(),
                    proposition_reasoning: proposition.reasoning.clone(),
                    proposition_confidence: proposition.confidence.unwrap_or(0),
                    proposition_decay: proposition.decay.unwrap_or(0),
                    contradicts_text,
                    related,
                    recent_observations,
                    group_push_history,
                    global_last_hour,
                },
            )
        } else {
            render_score(&ScoreInput {
                proposition_text: proposition.text.clone(),
                proposition_reasoning: proposition.reasoning.clone(),
                proposition_confidence: proposition.confidence.unwrap_or(0),
                proposition_decay: proposition.decay.unwrap_or(0),
                contradicts_text,
                related,
                recent_observations,
                group_push_history,
                global_last_hour,
            })
        };

        let raw: RawScore = self
            .llm
            .complete_json::<RawScore>(&LlmRequest::text(rendered))
            .await?;

        if !(1..=5).contains(&raw.p_s) {
            return Err(CorivoError::InvalidResponse(format!(
                "proactivity_scorer: p_s out of range: got {}",
                raw.p_s
            )));
        }

        let inserted = self
            .score_repo
            .insert(NewProactivityScore {
                proposition_id: proposition.id,
                p_s: raw.p_s,
                reasoning: raw.reasoning,
                threshold_at_time: self.threshold,
            })
            .await?;
        Ok(inserted)
    }

    pub fn threshold(&self) -> u8 {
        self.threshold
    }

    async fn resolve_contradicts_text(&self, prop: &Proposition) -> Result<Option<String>> {
        let Some(id) = prop.contradicts_proposition_id else {
            return Ok(None);
        };
        Ok(self
            .proposition_repo
            .by_id(id)
            .await?
            .map(|p| p.text))
    }
}

fn summarize_frequency(rows: &[NotificationLogEntry]) -> PushFrequencySummary {
    let total = rows.len() as u32;
    let mut acknowledged = 0u32;
    let mut dismissed = 0u32;
    for r in rows {
        match r.outcome.as_deref() {
            Some("acknowledged") => acknowledged += 1,
            Some("dismissed") => dismissed += 1,
            _ => {}
        }
    }
    let no_response = total - acknowledged - dismissed;
    PushFrequencySummary {
        total,
        acknowledged,
        dismissed,
        no_response,
    }
}

/// Mirror of render_score that operates on a caller-supplied template string
/// (used when PromptDebugConfig.score_override is set). Kept close to the
/// service so the two renderers can't drift — both share the same fill logic.
fn render_score_with_template(template: &str, input: &ScoreInput) -> String {
    // Build the render by first producing every substituted block via the
    // default renderer, then swapping the template body. Simpler: copy the
    // small amount of per-field logic from prompts::render_score. Keep in
    // sync with prompts.rs if additional placeholders are added.
    let default = render_score(input);
    // If the template is identical to the bundled one, reuse the default.
    // Otherwise, re-apply substitutions directly against the override.
    let is_contradict_text = if input.contradicts_text.is_some() { "是" } else { "否" };
    let contradicts_clause = match input.contradicts_text.as_ref() {
        Some(t) => format!("（与命题「{t}」冲突）"),
        None => String::new(),
    };
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
    let observations_block = if input.recent_observations.is_empty() {
        "（暂无最近活动）".to_string()
    } else {
        input
            .recent_observations
            .iter()
            .map(|o| {
                let preview: String = o.content.chars().take(200).collect();
                format!("- [{}] {}", o.created_at.format("%H:%M"), preview)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let group_history_block = if input.group_push_history.is_empty() {
        "（这是该主题首次推送）".to_string()
    } else {
        input
            .group_push_history
            .iter()
            .map(|r| {
                let outcome_label = r.outcome.as_deref().unwrap_or("未响应");
                format!("- {}: {}", r.pushed_at.to_rfc3339(), outcome_label)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let global_summary = if input.global_last_hour.total == 0 {
        "过去 1 小时无推送".to_string()
    } else {
        format!(
            "过去 1 小时推送 {} 条（acknowledged {} / dismissed {} / 未响应 {}）",
            input.global_last_hour.total,
            input.global_last_hour.acknowledged,
            input.global_last_hour.dismissed,
            input.global_last_hour.no_response,
        )
    };

    let rendered = template
        .replace("{{proposition_text}}", &input.proposition_text)
        .replace("{{proposition_reasoning}}", &input.proposition_reasoning)
        .replace(
            "{{proposition_confidence}}",
            &input.proposition_confidence.to_string(),
        )
        .replace(
            "{{proposition_decay}}",
            &input.proposition_decay.to_string(),
        )
        .replace("{{is_contradict}}", is_contradict_text)
        .replace("{{contradicts_clause}}", &contradicts_clause)
        .replace("{{related_propositions}}", &related_block)
        .replace("{{recent_observations}}", &observations_block)
        .replace("{{group_push_history}}", &group_history_block)
        .replace("{{global_recent_summary}}", &global_summary);

    // Guard against overrides missing placeholders by falling back to the
    // default bundled prompt when the override produced zero substitution
    // activity.
    if rendered == template {
        default
    } else {
        rendered
    }
}
```

- [ ] **Step 4: Register module**

Edit `src-tauri/src/services/mod.rs` — add `pub mod proactivity_scorer;` alphabetically (after `notification_service` and before `push_decider` for now; `push_decider` will be removed in Task 9).

- [ ] **Step 5: Run test to verify pass**

Run: `cd src-tauri && cargo test --test proactivity_scorer happy_path_inserts_score`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/services/proactivity_scorer.rs \
        src-tauri/src/services/mod.rs \
        src-tauri/tests/proactivity_scorer.rs
git commit -m "feat(services): add ProactivityScorer with happy-path test"
```

---

## Task 7: `ProactivityScorer` error paths

**Files:**
- Modify: `src-tauri/tests/proactivity_scorer.rs`

- [ ] **Step 1: Add error-path tests**

Append to `src-tauri/tests/proactivity_scorer.rs`:

```rust
#[tokio::test]
async fn invalid_json_returns_err_no_db_write() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop = seed_proposition(&pool).await;

    let llm: Arc<dyn LlmProvider> =
        Arc::new(MockLlm::with_responses(vec!["not json".into()]));
    let proposition_repo = Arc::new(SqlitePropositionRepo::new(pool.clone()));
    let observation_repo = Arc::new(SqliteObservationRepo::new(pool.clone()));
    let notification_log = Arc::new(SqliteNotificationLogRepo::new(pool.clone()));
    let score_repo: Arc<dyn ProactivityScoreRepo> =
        Arc::new(SqliteProactivityScoreRepo::new(pool.clone()));

    let scorer = ProactivityScorer::new(
        proposition_repo,
        observation_repo,
        notification_log,
        score_repo.clone(),
        llm,
        default_retrieval_cfg(),
        4,
        None,
    );
    assert!(scorer.score(&prop).await.is_err());
    assert!(score_repo.by_proposition_id(prop.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn p_s_out_of_range_returns_err() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop = seed_proposition(&pool).await;

    let llm: Arc<dyn LlmProvider> = Arc::new(MockLlm::with_responses(vec![
        serde_json::json!({"p_s": 6, "reasoning": "oops"}).to_string(),
    ]));
    let proposition_repo = Arc::new(SqlitePropositionRepo::new(pool.clone()));
    let observation_repo = Arc::new(SqliteObservationRepo::new(pool.clone()));
    let notification_log = Arc::new(SqliteNotificationLogRepo::new(pool.clone()));
    let score_repo: Arc<dyn ProactivityScoreRepo> =
        Arc::new(SqliteProactivityScoreRepo::new(pool.clone()));

    let scorer = ProactivityScorer::new(
        proposition_repo,
        observation_repo,
        notification_log,
        score_repo.clone(),
        llm,
        default_retrieval_cfg(),
        4,
        None,
    );
    assert!(scorer.score(&prop).await.is_err());
    assert!(score_repo.by_proposition_id(prop.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn llm_exhausted_returns_err() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop = seed_proposition(&pool).await;

    let llm: Arc<dyn LlmProvider> = Arc::new(MockLlm::with_responses(vec![]));
    let proposition_repo = Arc::new(SqlitePropositionRepo::new(pool.clone()));
    let observation_repo = Arc::new(SqliteObservationRepo::new(pool.clone()));
    let notification_log = Arc::new(SqliteNotificationLogRepo::new(pool.clone()));
    let score_repo: Arc<dyn ProactivityScoreRepo> =
        Arc::new(SqliteProactivityScoreRepo::new(pool.clone()));

    let scorer = ProactivityScorer::new(
        proposition_repo,
        observation_repo,
        notification_log,
        score_repo.clone(),
        llm,
        default_retrieval_cfg(),
        4,
        None,
    );
    assert!(scorer.score(&prop).await.is_err());
    assert!(score_repo.by_proposition_id(prop.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn empty_context_still_works() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let prop = seed_proposition(&pool).await;

    let llm_resp = serde_json::json!({"p_s": 3, "reasoning": "中性"}).to_string();
    let mock = Arc::new(MockLlm::with_responses(vec![llm_resp]));
    let mock_clone = mock.clone();
    let llm: Arc<dyn LlmProvider> = mock;

    let proposition_repo = Arc::new(SqlitePropositionRepo::new(pool.clone()));
    let observation_repo = Arc::new(SqliteObservationRepo::new(pool.clone()));
    let notification_log = Arc::new(SqliteNotificationLogRepo::new(pool.clone()));
    let score_repo: Arc<dyn ProactivityScoreRepo> =
        Arc::new(SqliteProactivityScoreRepo::new(pool.clone()));

    let scorer = ProactivityScorer::new(
        proposition_repo,
        observation_repo,
        notification_log,
        score_repo,
        llm,
        default_retrieval_cfg(),
        4,
        None,
    );
    let score = scorer.score(&prop).await.unwrap();
    assert_eq!(score.p_s, 3);
    let prompt = mock_clone.last_prompt().unwrap();
    assert!(prompt.contains("（暂无相关命题）"));
    assert!(prompt.contains("（暂无最近活动）"));
    assert!(prompt.contains("（这是该主题首次推送）"));
    assert!(prompt.contains("过去 1 小时无推送"));
}
```

- [ ] **Step 2: Run all scorer tests**

Run: `cd src-tauri && cargo test --test proactivity_scorer`
Expected: all 4 error + 1 happy = 5 PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/tests/proactivity_scorer.rs
git commit -m "test(proactivity_scorer): cover invalid JSON, range violation, exhausted LLM, empty context"
```

---

## Task 8: `PromptDebugConfig.score_override` (backend)

**Files:**
- Modify: `src-tauri/src/domain/config.rs`
- Modify: `src-tauri/tests/config_validate.rs`

- [ ] **Step 1: Write the failing tests**

Append to `src-tauri/tests/config_validate.rs`:

```rust
#[test]
fn prompt_debug_config_default_has_score_override_none() {
    use corivo_app_lib::domain::config::PromptDebugConfig;
    let cfg = PromptDebugConfig::default();
    assert!(cfg.score_override.is_none());
}

#[test]
fn prompt_debug_config_normalize_blank_score_override_to_none() {
    let mut cfg = Config::default();
    cfg.prompt_debug.score_override = Some("   \n  ".to_string());
    let normalized = cfg.normalize();
    assert!(normalized.prompt_debug.score_override.is_none());
}
```

- [ ] **Step 2: Run test to verify fail**

Run: `cd src-tauri && cargo test prompt_debug_config_default_has_score_override_none`
Expected: FAIL — field doesn't exist.

- [ ] **Step 3: Add field + normalize**

Edit `src-tauri/src/domain/config.rs`.

Update `PromptDebugConfig`:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PromptDebugConfig {
    pub summary_override: Option<String>,
    pub push_judgment_override: Option<String>,
    pub suggest_override: Option<String>,
    pub score_override: Option<String>,
}
```

Extend the `normalize` block (find the existing sequence):

```rust
        self.prompt_debug.summary_override =
            normalize_optional_prompt_override(self.prompt_debug.summary_override);
        self.prompt_debug.push_judgment_override =
            normalize_optional_prompt_override(self.prompt_debug.push_judgment_override);
        self.prompt_debug.suggest_override =
            normalize_optional_prompt_override(self.prompt_debug.suggest_override);
        self.prompt_debug.score_override =
            normalize_optional_prompt_override(self.prompt_debug.score_override);
```

- [ ] **Step 4: Run test to verify pass**

Run: `cd src-tauri && cargo test prompt_debug_config`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/domain/config.rs src-tauri/tests/config_validate.rs
git commit -m "feat(config): add PromptDebugConfig.score_override"
```

---

## Task 9: Cutover — delete push_decider, rewrite push_pipeline, simplify PushConfig

This is the largest task. It **breaks the build at each intermediate step** until complete; run `cargo build --tests` only at the end of Step 9.

**Files:**
- Create: `src-tauri/src/services/clock.rs`
- Delete: `src-tauri/src/services/push_decider.rs`
- Delete: `src-tauri/tests/push_decider.rs`
- Delete: `src-tauri/tests/push_pipeline_dismiss_penalty.rs`
- Modify: `src-tauri/src/services/push_pipeline.rs` (rewrite)
- Modify: `src-tauri/src/services/mod.rs`
- Modify: `src-tauri/src/domain/config.rs` (PushConfig simplify + validate)
- Modify: `src-tauri/src/lib.rs` (wire scorer)
- Modify: `src-tauri/tests/config_validate.rs` (threshold range checks)
- Modify: `src-tauri/tests/push_pipeline_end_to_end.rs` (drop cooldown test + add scorer mock)
- Modify: `src-tauri/tests/suggestion_generator.rs` / `suggestion_generator_end_to_end.rs` (if they reference the old PushConfig; grep to verify — expected zero references)

- [ ] **Step 1: Create `services/clock.rs`**

Create `src-tauri/src/services/clock.rs`:

```rust
//! Time provider abstraction used by push_pipeline and scorer tests.
//! Extracted from the now-deleted push_decider module.

use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Test helper: advanceable mock clock.
#[cfg(any(test, feature = "test-support"))]
pub struct MockClock {
    inner: Mutex<DateTime<Utc>>,
}

#[cfg(any(test, feature = "test-support"))]
impl MockClock {
    pub fn at(t: DateTime<Utc>) -> Self {
        Self {
            inner: Mutex::new(t),
        }
    }

    pub fn advance(&self, by: Duration) {
        let mut inner = self.inner.lock().unwrap();
        *inner += by;
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Clock for MockClock {
    fn now(&self) -> DateTime<Utc> {
        *self.inner.lock().unwrap()
    }
}
```

- [ ] **Step 2: Delete `push_decider.rs` and its tests**

```bash
rm src-tauri/src/services/push_decider.rs
rm src-tauri/tests/push_decider.rs
rm src-tauri/tests/push_pipeline_dismiss_penalty.rs
```

- [ ] **Step 3: Update `services/mod.rs`**

Edit `src-tauri/src/services/mod.rs` — remove `push_decider`, add `clock` and `proactivity_scorer`:

```rust
pub mod capture_loop;
pub mod capture_store;
pub mod clock;
pub mod config_service;
pub mod keychain_service;
pub mod llm_service;
pub mod macos_system_surface;
pub mod notification_service;
pub mod observation_ingest;
pub mod proactivity_scorer;
pub mod push_pipeline;
pub mod storage_cleanup;
pub mod suggestion_generator;
pub mod user_model;
```

- [ ] **Step 4: Simplify `PushConfig`**

Edit `src-tauri/src/domain/config.rs`.

Remove old constants:
```rust
const DEFAULT_PUSH_BASE_COOLDOWN_SECS: u32 = 24 * 3600;
const DEFAULT_PUSH_DISMISS_PENALTY_SECS: u32 = 7 * 24 * 3600;
const DEFAULT_PUSH_MIN_CONFIDENCE: u32 = 6;
const DEFAULT_PUSH_PUSH_ON_CONTRADICT: bool = true;
```

Add:
```rust
const DEFAULT_PUSH_PROACTIVITY_THRESHOLD: u8 = 4;
```

Replace `PushConfig`:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PushConfig {
    /// 1 = 随时打扰；5 = 只在关键时刻打扰。LLM 给的 P_S ≥ θ 才推。
    pub proactivity_threshold: u8,
}

impl Default for PushConfig {
    fn default() -> Self {
        Self {
            proactivity_threshold: DEFAULT_PUSH_PROACTIVITY_THRESHOLD,
        }
    }
}
```

Update `Config::validate()` — remove old push validations, add:
```rust
if !(1..=5).contains(&self.user_model.push.proactivity_threshold) {
    return Err(ConfigError::OutOfRange {
        field: "user_model.push.proactivity_threshold",
        value: self.user_model.push.proactivity_threshold as i64,
    });
}
```

If `ConfigError` doesn't have an `OutOfRange` variant, extend it. Check with:

```bash
grep -n "enum ConfigError\|OutOfRange\|NonPositive" src-tauri/src/domain/config.rs
```

If `OutOfRange` exists, use it; else add:

```rust
#[derive(Debug, Error)]
pub enum ConfigError {
    // ... existing variants ...
    #[error("配置字段 {field} 超出允许范围：{value}")]
    OutOfRange { field: &'static str, value: i64 },
}
```

- [ ] **Step 5: Rewrite `push_pipeline.rs`**

Full replacement for `src-tauri/src/services/push_pipeline.rs`:

```rust
//! Wires `PropositionRevised` events to the user-facing push path.
//!
//! C-stage flow:
//! 1. Subscribe to the `PropositionRevised` broadcast.
//! 2. For each event:
//!    - Resolve the proposition row.
//!    - Hard rule: skip if the revision_group has ever been dismissed by the user.
//!    - Call proactivity_scorer → if fail, skip; persist score row on success.
//!    - If p_s ≥ θ (threshold at score time), call suggestion_generator; on
//!      failure degrade to raw proposition.text (preserved B-stage behavior).
//!    - Insert `notification_log` linking to both suggestion and score.
//!    - Send the overlay payload.
//!
//! Failures at individual stages are logged at `warn`/`error` but never take
//! the task down; it exits cleanly when the broadcast sender is dropped.

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::{
    db::repos::{
        notification_log::{NewNotificationLog, NotificationLogRepo},
        propositions::PropositionRepo,
    },
    error::Result,
    events::PropositionRevised,
    providers::notification::{NotificationLevel, NotificationPayload},
    services::{
        clock::Clock,
        notification_service::NotificationService,
        proactivity_scorer::ProactivityScorer,
        suggestion_generator::{SuggestionBundle, SuggestionGenerator},
    },
};

pub struct PushPipelineDeps {
    pub revised_rx: broadcast::Receiver<PropositionRevised>,
    pub proposition_repo: Arc<dyn PropositionRepo>,
    pub notification_log: Arc<dyn NotificationLogRepo>,
    pub notification_service: Arc<NotificationService>,
    pub scorer: Arc<ProactivityScorer>,
    pub suggestion_generator: Arc<SuggestionGenerator>,
    pub clock: Arc<dyn Clock>,
}

pub fn spawn(deps: PushPipelineDeps) {
    tokio::spawn(run(deps));
}

async fn run(mut deps: PushPipelineDeps) {
    loop {
        match deps.revised_rx.recv().await {
            Ok(event) => {
                if let Err(error) = handle_event(&deps, event).await {
                    tracing::warn!(%error, "push_pipeline: handle_event failed");
                }
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, "push_pipeline: lagged on revised stream");
            }
            Err(broadcast::error::RecvError::Closed) => {
                tracing::info!("push_pipeline: revised stream closed; exiting");
                break;
            }
        }
    }
}

#[tracing::instrument(
    skip(deps),
    fields(
        proposition_id = event.proposition_id,
        revision_group = %event.revision_group
    )
)]
async fn handle_event(deps: &PushPipelineDeps, event: PropositionRevised) -> Result<()> {
    let Some(proposition) = deps
        .proposition_repo
        .by_id(event.proposition_id)
        .await?
    else {
        tracing::warn!(
            proposition_id = event.proposition_id,
            "push_pipeline: revised proposition disappeared before push"
        );
        return Ok(());
    };

    // Hard rule: user's explicit dismiss is permanent for this revision_group.
    if deps
        .notification_log
        .group_ever_dismissed(&proposition.revision_group)
        .await?
    {
        tracing::info!(
            outcome = "skip",
            reason = "group_dismissed_ever",
            proposition_id = proposition.id,
            revision_group = %proposition.revision_group,
            "push.decided"
        );
        return Ok(());
    }

    // LLM-scored proactivity. Failure → skip, no score row.
    let score = match deps.scorer.score(&proposition).await {
        Ok(s) => s,
        Err(error) => {
            tracing::warn!(
                scorer_failed = true,
                %error,
                proposition_id = proposition.id,
                revision_group = %proposition.revision_group,
                "push_pipeline: scorer failed; skipping push"
            );
            return Ok(());
        }
    };

    if score.p_s < deps.scorer.threshold() {
        tracing::info!(
            outcome = "skip",
            reason = "below_threshold",
            proposition_id = proposition.id,
            revision_group = %proposition.revision_group,
            p_s = score.p_s,
            threshold = score.threshold_at_time,
            "push.decided"
        );
        return Ok(());
    }

    // B-stage behavior preserved: suggestion_generator failure → degrade to proposition text.
    let bundle: Option<SuggestionBundle> =
        match deps.suggestion_generator.generate(&proposition).await {
            Ok(b) => Some(b),
            Err(error) => {
                tracing::warn!(
                    suggestion_failed = true,
                    %error,
                    proposition_id = proposition.id,
                    "push_pipeline: suggestion_generator failed; degrading"
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
            proactivity_score_id: Some(score.id),
        })
        .await?;

    let confidence = proposition
        .confidence
        .map(|c| c.clamp(0, u8::MAX as i32) as u8);

    let (title, body) = match bundle.as_ref() {
        Some(b) => ("Corivo 想跟你说一句".to_string(), b.surfaced.text.clone()),
        None => (
            "Corivo 学到了一条新判断".to_string(),
            proposition.text.clone(),
        ),
    };

    tracing::info!(
        outcome = "push",
        proposition_id = proposition.id,
        revision_group = %proposition.revision_group,
        p_s = score.p_s,
        proactivity_score_id = score.id,
        notification_log_id = entry.id,
        suggestion_id = ?suggestion_id,
        "push.decided"
    );

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

Note: `spawn_default` helper is **deleted** — lib.rs constructs `PushPipelineDeps` directly now.

- [ ] **Step 6: Rewrite `lib.rs` wiring**

Edit `src-tauri/src/lib.rs`.

Find the imports block and replace `push_decider::SystemClock` with `clock::SystemClock`, and `push_pipeline` with its direct usage. Specifically, replace the `services::{ ... }` import block:

```rust
use services::{
    capture_loop::{CaptureLoop, CaptureRuntimeConfig},
    capture_store::CaptureStore,
    clock::SystemClock,
    config_service::ConfigService,
    keychain_service::KeychainService,
    llm_service::LlmService,
    macos_system_surface::{
        apply_macos_system_surface_mode, sync_macos_system_surface_from_runtime,
        MacOSSystemSurfaceMode, DEFAULT_MAIN_WINDOW_LABEL,
    },
    notification_service::NotificationService,
    observation_ingest::ObservationIngestService,
    proactivity_scorer::ProactivityScorer,
    push_pipeline::{self, PushPipelineDeps},
    storage_cleanup,
    user_model::{UserModel, UserModelDeps},
};
```

In the setup closure (around where `push_pipeline::spawn_default` is called), replace that call with:

```rust
            // C-stage wiring: construct scorer + generator, pass both into push_pipeline.
            let observation_repo_for_scorer: std::sync::Arc<dyn crate::db::repos::observations::ObservationRepo> =
                Arc::new(SqliteObservationRepo::new(db.pool()));
            let proactivity_score_repo: std::sync::Arc<dyn crate::db::repos::proactivity_scores::ProactivityScoreRepo> =
                Arc::new(crate::db::repos::proactivity_scores::SqliteProactivityScoreRepo::new(db.pool()));

            let scorer = std::sync::Arc::new(ProactivityScorer::new(
                proposition_repo.clone(),
                observation_repo_for_scorer,
                notification_log.clone(),
                proactivity_score_repo,
                llm_provider.clone(),
                config.user_model.retrieval.clone(),
                config.user_model.push.proactivity_threshold,
                config.prompt_debug.score_override.clone(),
            ));

            let suggestion_generator = std::sync::Arc::new(
                crate::services::suggestion_generator::SuggestionGenerator::new(
                    proposition_repo.clone(),
                    suggestion_repo.clone(),
                    llm_provider.clone(),
                    config.user_model.retrieval.clone(),
                    config.prompt_debug.suggest_override.clone(),
                ),
            );

            tauri::async_runtime::block_on(async {
                let clock: std::sync::Arc<dyn crate::services::clock::Clock> =
                    Arc::new(SystemClock);
                push_pipeline::spawn(PushPipelineDeps {
                    revised_rx: user_model.revised_receiver(),
                    proposition_repo: proposition_repo.clone(),
                    notification_log: notification_log.clone(),
                    notification_service: notification_service.clone(),
                    scorer,
                    suggestion_generator,
                    clock,
                });
            });
```

Remove any leftover reference to `PushConfig::from_domain` or `DefaultPushDecider`.

- [ ] **Step 7: Update `push_pipeline_end_to_end.rs`**

Edit `src-tauri/tests/push_pipeline_end_to_end.rs`:

1. Remove the `second_push_in_same_group_is_skipped_within_cooldown` test (the entire `#[tokio::test]` function body).
2. Replace all `push_decider::{...}` imports with `clock::{Clock, MockClock}`.
3. Remove `DefaultPushDecider`, `PushDecider`, `PushConfig` imports.
4. Update `PushPipelineDeps { ... }` literals: remove `decider`, add `scorer` field. Construct a `ProactivityScorer` using the new constructor shape (see Task 6). For the happy/fallback tests, have MockLlm **return two responses in order**: first the score JSON (`{"p_s": 5, "reasoning": "test"}`), then the suggestion JSON (3-element array). Happy test score = 5 to pass threshold=4; fallback test drains the MockLlm queue so scorer fails → SKIP (the test then expects **zero** payloads, not a degraded push — adjust the existing assertion).

Example rewrite of the happy case (`first_push_surfaces_suggestion_text_with_new_title`):

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_push_surfaces_suggestion_text_with_new_title() {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let propositions = Arc::new(SqlitePropositionRepo::new(pool.clone()));
    let logs = Arc::new(SqliteNotificationLogRepo::new(pool.clone()));
    let suggestions: Arc<dyn SuggestionRepo> = Arc::new(SqliteSuggestionRepo::new(pool.clone()));
    let observations_repo = Arc::new(SqliteObservationRepo::new(pool.clone()));
    let score_repo: Arc<dyn ProactivityScoreRepo> =
        Arc::new(SqliteProactivityScoreRepo::new(pool.clone()));

    let provider = Arc::new(RecordingProvider::default());
    let factory = Arc::new(RecordingFactory { provider: provider.clone() });
    let config_service = Arc::new(
        ConfigService::with_backend(Arc::new(InMemoryConfigStore::default())).unwrap(),
    );
    let notification_service =
        Arc::new(NotificationService::with_factory(config_service, factory));

    let clock = Arc::new(MockClock::at(ts("2026-04-19T10:00:00Z")));
    let clock_for_pipeline: Arc<dyn Clock> = Arc::new(MockClockArc(clock.clone()));

    // Scorer queue[0], suggestion queue[1].
    let mock_llm = Arc::new(MockLlm::with_responses(vec![
        serde_json::json!({"p_s": 5, "reasoning": "t"}).to_string(),
        serde_json::json!([
            {"text": "看起来你在研究推送机制", "reasoning": "情境回声"},
            {"text": "要不要我整理最新的引用论文清单？", "reasoning": "可行动提议"},
            {"text": "需要我把 2 个 tab 里的 TODO 拎出来吗？", "reasoning": "数据回看"},
        ]).to_string(),
    ]));
    let llm_provider: Arc<dyn LlmProvider> = mock_llm;

    let scorer = Arc::new(ProactivityScorer::new(
        propositions.clone(),
        observations_repo,
        logs.clone(),
        score_repo.clone(),
        llm_provider.clone(),
        RetrievalConfig {
            w_confidence: 1.0, w_decay: 1.0, k_decay_days: 14.0, limit_multiplier: 3,
        },
        4,
        None,
    ));
    let suggestion_generator = Arc::new(SuggestionGenerator::new(
        propositions.clone(),
        suggestions.clone(),
        llm_provider,
        RetrievalConfig {
            w_confidence: 1.0, w_decay: 1.0, k_decay_days: 14.0, limit_multiplier: 3,
        },
        None,
    ));

    let (revised_tx, revised_rx) = broadcast::channel::<PropositionRevised>(4);
    push_pipeline::spawn(PushPipelineDeps {
        revised_rx,
        proposition_repo: propositions.clone(),
        notification_log: logs.clone(),
        notification_service,
        scorer,
        suggestion_generator,
        clock: clock_for_pipeline,
    });

    let prop_id = seed_proposition(&propositions, "happy", 1, 8).await;
    revised_tx.send(PropositionRevised {
        proposition_id: prop_id,
        revision_group: "happy".into(),
        op: RevisionOpTag::Update,
    }).unwrap();

    wait_for(|| provider.sent.lock().unwrap().len() == 1).await;
    let payload = provider.sent.lock().unwrap()[0].clone();
    assert_eq!(payload.title, "Corivo 想跟你说一句");
    assert_eq!(payload.body, "看起来你在研究推送机制");

    let log = logs.recent(1).await.unwrap();
    assert!(log[0].proactivity_score_id.is_some());
    assert!(log[0].suggestion_id.is_some());

    let scores = score_repo.by_proposition_id(prop_id).await.unwrap();
    assert_eq!(scores.len(), 1);
    assert_eq!(scores[0].p_s, 5);
    assert_eq!(scores[0].threshold_at_time, 4);
}
```

The `fallback_path_uses_proposition_text_and_null_suggestion_id` test keeps its logic but **now requires scorer to pass**. Two strategies:

**Strategy A** (preferred — keeps the test's spirit): scorer returns a passing P_S, suggestion LLM fails. Mock exactly one entry (the score), so the suggestion_generator's pull fails.

```rust
let mock_llm = Arc::new(MockLlm::with_responses(vec![
    serde_json::json!({"p_s": 5, "reasoning": "pass"}).to_string(),
    // No suggestion response → suggestion_generator fails → degrade
]));
```

Assertions update:
```rust
assert_eq!(payload.title, "Corivo 学到了一条新判断");
assert_eq!(payload.body, "prop fallback v1");
assert_eq!(log[0].suggestion_id, None);
assert!(log[0].proactivity_score_id.is_some());
```

Imports at top of file — replace push_decider with clock + scorer + score_repo:

```rust
use corivo_app_lib::{
    db::{
        migrations::apply_migrations,
        pool::test_in_memory_pool,
        repos::{
            notification_log::{NotificationLogRepo, SqliteNotificationLogRepo},
            observations::SqliteObservationRepo,
            proactivity_scores::{ProactivityScoreRepo, SqliteProactivityScoreRepo},
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
        clock::{Clock, MockClock},
        config_service::{ConfigService, InMemoryConfigStore},
        notification_service::{NotificationProviderFactory, NotificationService},
        proactivity_scorer::ProactivityScorer,
        push_pipeline::{self, PushPipelineDeps},
        suggestion_generator::SuggestionGenerator,
    },
};
```

- [ ] **Step 8: Update `config_validate.rs` threshold tests**

Append to `src-tauri/tests/config_validate.rs`:

```rust
#[test]
fn proactivity_threshold_zero_rejected() {
    let mut cfg = Config::default();
    cfg.user_model.push.proactivity_threshold = 0;
    assert!(cfg.validate().is_err());
}

#[test]
fn proactivity_threshold_six_rejected() {
    let mut cfg = Config::default();
    cfg.user_model.push.proactivity_threshold = 6;
    assert!(cfg.validate().is_err());
}
```

Also **remove** stale tests that referenced the old fields — search and delete any test with names like `base_cooldown_zero_is_rejected`, `dismiss_penalty_zero_is_rejected`, `min_confidence_above_ten_is_rejected`, `min_confidence_below_one_is_rejected` (grep first; delete each).

```bash
grep -n "base_cooldown\|dismiss_penalty\|min_confidence\|push_on_contradict" src-tauri/tests/config_validate.rs
```

For each hit, delete the entire `#[test] fn ... { ... }` block.

- [ ] **Step 9: Run the full build + test suite**

Run: `cd src-tauri && cargo build --tests`
Expected: compile success.

Run: `cd src-tauri && cargo test`
Expected: all tests PASS (37+ suites depending on existing count; zero failures).

If any `PushConfig { base_cooldown: ..., ... }` literal remains (e.g. in other test files), the compiler will tell you; patch each to `PushConfig { proactivity_threshold: 4 }`. Run:

```bash
grep -rn "base_cooldown\|dismiss_penalty\|push_on_contradict" src-tauri --include="*.rs"
```

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "feat(push_pipeline): cutover to LLM-scored proactivity (C stage)

- Delete push_decider service and its tests (cooldown/dismiss_penalty/min_confidence/push_on_contradict all removed).
- Move Clock/SystemClock/MockClock to services/clock.rs.
- Rewrite push_pipeline::handle_event: hard dismiss-block → scorer → θ gate → suggestion_generator (degrade preserved).
- Simplify PushConfig to a single proactivity_threshold:u8 (default 4).
- Wire ProactivityScorer into lib.rs."
```

---

## Task 10: Integration test `push_pipeline_proactivity.rs`

**Files:**
- Create: `src-tauri/tests/push_pipeline_proactivity.rs`

- [ ] **Step 1: Write the integration test file**

Create `src-tauri/tests/push_pipeline_proactivity.rs`:

```rust
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Utc};
use corivo_app_lib::{
    db::{
        migrations::apply_migrations,
        pool::test_in_memory_pool,
        repos::{
            notification_log::{NotificationLogRepo, SqliteNotificationLogRepo},
            observations::SqliteObservationRepo,
            proactivity_scores::{ProactivityScoreRepo, SqliteProactivityScoreRepo},
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
        clock::{Clock, MockClock},
        config_service::{ConfigService, InMemoryConfigStore},
        notification_service::{NotificationProviderFactory, NotificationService},
        proactivity_scorer::ProactivityScorer,
        push_pipeline::{self, PushPipelineDeps},
        suggestion_generator::SuggestionGenerator,
    },
};
use tokio::sync::broadcast;

fn ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

#[derive(Default)]
struct RecordingProvider {
    sent: Mutex<Vec<NotificationPayload>>,
}
impl NotificationProvider for RecordingProvider {
    fn send(&self, p: NotificationPayload) -> corivo_app_lib::providers::notification::Result<()> {
        self.sent.lock().unwrap().push(p);
        Ok(())
    }
}
struct RecordingFactory {
    provider: Arc<RecordingProvider>,
}
impl NotificationProviderFactory for RecordingFactory {
    fn create(
        &self,
        _k: &NotificationProviderKind,
        _c: &NotificationConfig,
    ) -> corivo_app_lib::error::Result<Arc<dyn NotificationProvider>> {
        Ok(self.provider.clone())
    }
}
struct MockClockArc(Arc<MockClock>);
impl Clock for MockClockArc {
    fn now(&self) -> DateTime<Utc> {
        self.0.now()
    }
}

async fn seed(
    repo: &SqlitePropositionRepo,
    group: &str,
    ver: i32,
    conf: i32,
) -> i64 {
    repo.insert(NewProposition {
        text: format!("prop {group} v{ver}"),
        reasoning: "because".into(),
        confidence: Some(conf),
        decay: Some(8),
        revision_group: group.into(),
        version: ver,
        contradicts_proposition_id: None,
    })
    .await
    .unwrap()
    .id
}

async fn wait_for<F: FnMut() -> bool>(mut p: F) {
    for _ in 0..60 {
        if p() {
            return;
        }
        tokio::time::sleep(StdDuration::from_millis(50)).await;
    }
    panic!("timed out");
}

fn cfg() -> RetrievalConfig {
    RetrievalConfig {
        w_confidence: 1.0,
        w_decay: 1.0,
        k_decay_days: 14.0,
        limit_multiplier: 3,
    }
}

struct Harness {
    pool: r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    propositions: Arc<SqlitePropositionRepo>,
    logs: Arc<SqliteNotificationLogRepo>,
    score_repo: Arc<dyn ProactivityScoreRepo>,
    revised_tx: broadcast::Sender<PropositionRevised>,
    provider: Arc<RecordingProvider>,
    mock_llm: Arc<MockLlm>,
}

fn build(threshold: u8, llm_responses: Vec<String>) -> Harness {
    let pool = test_in_memory_pool().unwrap();
    apply_migrations(&pool.get().unwrap()).unwrap();
    let propositions = Arc::new(SqlitePropositionRepo::new(pool.clone()));
    let logs = Arc::new(SqliteNotificationLogRepo::new(pool.clone()));
    let suggestions: Arc<dyn SuggestionRepo> = Arc::new(SqliteSuggestionRepo::new(pool.clone()));
    let observations = Arc::new(SqliteObservationRepo::new(pool.clone()));
    let score_repo: Arc<dyn ProactivityScoreRepo> =
        Arc::new(SqliteProactivityScoreRepo::new(pool.clone()));

    let provider = Arc::new(RecordingProvider::default());
    let factory = Arc::new(RecordingFactory { provider: provider.clone() });
    let config_service = Arc::new(
        ConfigService::with_backend(Arc::new(InMemoryConfigStore::default())).unwrap(),
    );
    let notification_service =
        Arc::new(NotificationService::with_factory(config_service, factory));

    let mock_llm = Arc::new(MockLlm::with_responses(llm_responses));
    let mock_clone = mock_llm.clone();
    let llm: Arc<dyn LlmProvider> = mock_llm;

    let scorer = Arc::new(ProactivityScorer::new(
        propositions.clone(),
        observations,
        logs.clone(),
        score_repo.clone(),
        llm.clone(),
        cfg(),
        threshold,
        None,
    ));
    let suggestion_generator = Arc::new(SuggestionGenerator::new(
        propositions.clone(),
        suggestions,
        llm,
        cfg(),
        None,
    ));
    let clock_backing = Arc::new(MockClock::at(ts("2026-04-19T10:00:00Z")));
    let clock_for_pipeline: Arc<dyn Clock> = Arc::new(MockClockArc(clock_backing));

    let (revised_tx, revised_rx) = broadcast::channel::<PropositionRevised>(4);
    push_pipeline::spawn(PushPipelineDeps {
        revised_rx,
        proposition_repo: propositions.clone(),
        notification_log: logs.clone(),
        notification_service,
        scorer,
        suggestion_generator,
        clock: clock_for_pipeline,
    });

    Harness {
        pool,
        propositions,
        logs,
        score_repo,
        revised_tx,
        provider,
        mock_llm: mock_clone,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dismissed_ever_group_skipped_without_llm_call() {
    let h = build(4, vec![]); // empty LLM queue — any scorer call would fail

    // Seed and dismiss a prior push.
    let dismissed_log = h
        .logs
        .insert(corivo_app_lib::db::repos::notification_log::NewNotificationLog {
            revision_group: "g-dismissed".into(),
            proposition_id: None,
            pushed_at: ts("2026-04-18T12:00:00Z"),
            suggestion_id: None,
            proactivity_score_id: None,
        })
        .await
        .unwrap();
    h.logs
        .set_outcome_at(dismissed_log.id, "dismissed", ts("2026-04-18T12:10:00Z"))
        .await
        .unwrap();

    // New proposition in that group.
    let prop_id = seed(&h.propositions, "g-dismissed", 2, 9).await;
    h.revised_tx
        .send(PropositionRevised {
            proposition_id: prop_id,
            revision_group: "g-dismissed".into(),
            op: RevisionOpTag::Update,
        })
        .unwrap();

    // Give the pipeline a beat to process.
    tokio::time::sleep(StdDuration::from_millis(200)).await;

    // No push, no score, no LLM consumed.
    assert_eq!(h.provider.sent.lock().unwrap().len(), 0);
    assert!(h
        .score_repo
        .by_proposition_id(prop_id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(h.mock_llm.remaining(), 0); // queue was empty; must not have tried
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn score_below_threshold_writes_score_but_no_push() {
    let h = build(
        4,
        vec![serde_json::json!({"p_s": 2, "reasoning": "redundant"}).to_string()],
    );

    let prop_id = seed(&h.propositions, "g-low", 1, 7).await;
    h.revised_tx
        .send(PropositionRevised {
            proposition_id: prop_id,
            revision_group: "g-low".into(),
            op: RevisionOpTag::Update,
        })
        .unwrap();

    tokio::time::sleep(StdDuration::from_millis(200)).await;

    assert_eq!(h.provider.sent.lock().unwrap().len(), 0);
    let scores = h.score_repo.by_proposition_id(prop_id).await.unwrap();
    assert_eq!(scores.len(), 1);
    assert_eq!(scores[0].p_s, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn score_above_threshold_pushes_with_link() {
    let h = build(
        4,
        vec![
            serde_json::json!({"p_s": 5, "reasoning": "urgent"}).to_string(),
            serde_json::json!([
                {"text": "t1", "reasoning": "r1"},
                {"text": "t2", "reasoning": "r2"},
                {"text": "t3", "reasoning": "r3"},
            ])
            .to_string(),
        ],
    );

    let prop_id = seed(&h.propositions, "g-high", 1, 8).await;
    h.revised_tx
        .send(PropositionRevised {
            proposition_id: prop_id,
            revision_group: "g-high".into(),
            op: RevisionOpTag::Update,
        })
        .unwrap();

    wait_for(|| h.provider.sent.lock().unwrap().len() == 1).await;

    let payload = h.provider.sent.lock().unwrap()[0].clone();
    assert_eq!(payload.title, "Corivo 想跟你说一句");
    assert_eq!(payload.body, "t1");

    let logs = h.logs.recent(1).await.unwrap();
    assert!(logs[0].proactivity_score_id.is_some());
    assert!(logs[0].suggestion_id.is_some());

    let scores = h.score_repo.by_proposition_id(prop_id).await.unwrap();
    assert_eq!(scores.len(), 1);
    assert_eq!(scores[0].p_s, 5);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scorer_failure_skips_no_score_no_push() {
    let h = build(4, vec![]); // scorer call will fail (queue empty)

    let prop_id = seed(&h.propositions, "g-fail", 1, 8).await;
    h.revised_tx
        .send(PropositionRevised {
            proposition_id: prop_id,
            revision_group: "g-fail".into(),
            op: RevisionOpTag::Update,
        })
        .unwrap();

    tokio::time::sleep(StdDuration::from_millis(200)).await;

    assert_eq!(h.provider.sent.lock().unwrap().len(), 0);
    assert!(h
        .score_repo
        .by_proposition_id(prop_id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scorer_pass_but_suggestion_fails_pushes_degraded() {
    let h = build(
        4,
        vec![
            // Scorer succeeds.
            serde_json::json!({"p_s": 5, "reasoning": "go"}).to_string(),
            // No suggestion response → suggestion_generator fails → degrade.
        ],
    );

    let prop_id = seed(&h.propositions, "g-degrade", 1, 8).await;
    h.revised_tx
        .send(PropositionRevised {
            proposition_id: prop_id,
            revision_group: "g-degrade".into(),
            op: RevisionOpTag::Update,
        })
        .unwrap();

    wait_for(|| h.provider.sent.lock().unwrap().len() == 1).await;

    let payload = h.provider.sent.lock().unwrap()[0].clone();
    assert_eq!(payload.title, "Corivo 学到了一条新判断");
    assert_eq!(payload.body, "prop g-degrade v1");

    let logs = h.logs.recent(1).await.unwrap();
    assert!(logs[0].proactivity_score_id.is_some());
    assert_eq!(logs[0].suggestion_id, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn threshold_change_uses_current_value_at_score_time() {
    // Harness built with threshold=4, score returned 4 ⇒ passes.
    // After score row persisted, we verify threshold_at_time == 4
    // (even if a later push might run with a different θ).
    let h = build(
        4,
        vec![
            serde_json::json!({"p_s": 4, "reasoning": "boundary"}).to_string(),
            serde_json::json!([
                {"text": "t1", "reasoning": "r1"},
                {"text": "t2", "reasoning": "r2"},
                {"text": "t3", "reasoning": "r3"},
            ])
            .to_string(),
        ],
    );

    let prop_id = seed(&h.propositions, "g-bound", 1, 7).await;
    h.revised_tx
        .send(PropositionRevised {
            proposition_id: prop_id,
            revision_group: "g-bound".into(),
            op: RevisionOpTag::Update,
        })
        .unwrap();

    wait_for(|| h.provider.sent.lock().unwrap().len() == 1).await;

    let scores = h.score_repo.by_proposition_id(prop_id).await.unwrap();
    assert_eq!(scores.len(), 1);
    assert_eq!(scores[0].threshold_at_time, 4);
    assert_eq!(scores[0].p_s, 4);
}
```

- [ ] **Step 2: Run the tests**

Run: `cd src-tauri && cargo test --test push_pipeline_proactivity`
Expected: all 6 PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/tests/push_pipeline_proactivity.rs
git commit -m "test(push_pipeline): end-to-end proactivity scorer scenarios"
```

---

## Task 11: Frontend — types.ts + config-tauri.test.ts

**Files:**
- Modify: `src/lib/types.ts`
- Modify: `src/lib/config-tauri.test.ts`

- [ ] **Step 1: Update `types.ts`**

Edit `src/lib/types.ts`. Replace `PushConfig` with:

```typescript
export interface PushConfig {
  proactivity_threshold: number; // 1-5
}
```

Add `score_override` to `PromptDebugConfig`:

```typescript
export interface PromptDebugConfig {
  summary_override: string | null;
  push_judgment_override: string | null;
  suggest_override: string | null;
  score_override: string | null;
}
```

- [ ] **Step 2: Update test fixture**

Edit `src/lib/config-tauri.test.ts`:

Change the `push:` block:
```typescript
push: {
  proactivity_threshold: 4,
},
```

Change the `prompt_debug:` block to add `score_override: null`:
```typescript
prompt_debug: {
  summary_override: "custom summary prompt",
  push_judgment_override: null,
  suggest_override: null,
  score_override: null,
},
```

- [ ] **Step 3: Verify tsc**

Run: `pnpm exec tsc --noEmit`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/lib/types.ts src/lib/config-tauri.test.ts
git commit -m "feat(types): mirror PushConfig simplification + score_override"
```

---

## Task 12: Frontend — Settings UI (slider + delete toggle)

**Files:**
- Modify: `src/pages/settings/sections/general-section.tsx`

- [ ] **Step 1: Remove the `push_on_contradict` ToggleRow**

Edit `src/pages/settings/sections/general-section.tsx`. Find and **delete** the entire `<ToggleRow label="冲突时仍推送" ...>` block (added in a prior session; grep with `冲突时仍推送`).

- [ ] **Step 2: Add the proactivity slider**

In the same "窗口与通知" FieldGroup where the toggle was removed, insert the slider UI. Find the FieldGroup closing `</FieldGroup>` and add **before** it:

```tsx
<div className="space-y-2">
  <div className="flex items-center justify-between">
    <Label htmlFor="proactivity-threshold" className="text-sm">
      Corivo 主动性
    </Label>
    <span className="font-mono text-sm text-muted-foreground">
      {config.user_model.push.proactivity_threshold}
    </span>
  </div>
  <input
    id="proactivity-threshold"
    type="range"
    min={1}
    max={5}
    step={1}
    value={config.user_model.push.proactivity_threshold}
    onChange={(event) =>
      update((prev) => ({
        ...prev,
        user_model: {
          ...prev.user_model,
          push: {
            ...prev.user_model.push,
            proactivity_threshold: Number(event.target.value),
          },
        },
      }))
    }
    className="w-full accent-foreground"
  />
  <div className="flex justify-between text-[11px] text-muted-foreground">
    <span>1 · 随时主动</span>
    <span>3 · 中性</span>
    <span>5 · 极度克制</span>
  </div>
  <p className="text-xs text-muted-foreground">
    数字越高，Corivo 越只在重要时刻打扰你。改了立即生效。
  </p>
</div>
```

- [ ] **Step 3: Verify**

Run: `pnpm exec tsc --noEmit`
Expected: clean.

(Optional smoke: `pnpm tauri dev`, open Settings → General → drag slider → value shown updates → save persists on restart.)

- [ ] **Step 4: Commit**

```bash
git add src/pages/settings/sections/general-section.tsx
git commit -m "feat(settings): replace push_on_contradict toggle with proactivity slider"
```

---

## Task 13: Frontend — Prompt-debug page adds score_override

**Files:**
- Modify: `src/pages/settings/sections/prompt-debug-section.tsx`

- [ ] **Step 1: Add `DEFAULT_SCORE_PROMPT` constant**

Edit `src/pages/settings/sections/prompt-debug-section.tsx`. Near the other `DEFAULT_*` constants (after `DEFAULT_SUGGEST_PROMPT`), add:

```typescript
const DEFAULT_SCORE_PROMPT = `<!-- version: 1 | C 阶段 Proactivity Score (P_S) — replaces hard-gate min_confidence -->

你是 Corivo 的"打扰守门人"。Corivo 长期观察用户行为，刚刚得出了一条新的判断（命题）。
你要给这条命题打一个**主动性分数 P_S（1-5 整数）**——决定它现在值不值得用刘海通知打扰用户。

打分语义（**保守倾向**——拿不准就给低）：
- 5 = 必须现在告诉用户（紧急 / 用户极可能错过 / 关键决策依据）
- 4 = 应该告诉（明显有价值，且用户当下能接收）
- 3 = 可推可不推（一般性观察、价值中性）
- 2 = 不推为宜（轻微相关、用户大概率不感兴趣）
- 1 = 绝不要推（与用户无关 / 已知重复 / 冗余）

判分时综合考虑：
- **新意**：相比"已有相关命题"，这条带来了新信息吗？还是冗余？
- **当下相关性**：从"用户最近做的事"看，现在打扰合不合适？
- **频率**：最近 1 小时已经推了几条？同主题之前推过吗？反应如何？
- **置信度**：低 confidence 命题（≤4）一般给低分；除非内容本身高价值
- **冲突**：是 contradict 类时用户可能需要裁决，倾向给高分

---

【新命题】
- 内容：{{proposition_text}}
- 依据：{{proposition_reasoning}}
- 置信度：{{proposition_confidence}}/10
- 衰减权重：{{proposition_decay}}/10
- 是否冲突命题：{{is_contradict}}{{contradicts_clause}}

【相关已有命题（按相关性排序）】
{{related_propositions}}

【用户最近的屏幕活动（最新 3 条 observations，由早到晚）】
{{recent_observations}}

【该主题（同 revision_group）最近的推送】
{{group_push_history}}

【全局最近 1 小时的推送概况】
{{global_recent_summary}}

---

仅输出 JSON，恰好一个对象：

\`\`\`json
{"p_s": 4, "reasoning": "为什么打这个分（中文，30-50 字）"}
\`\`\`

不要 Markdown 包裹，不要任何额外文字。p_s 必须是 1-5 之间的整数。`;
```

- [ ] **Step 2: Extend `PromptKey` union**

Find the `PromptKey` type (at top of file) and extend:

```typescript
type PromptKey =
  | "summary_override"
  | "push_judgment_override"
  | "suggest_override"
  | "score_override";
```

- [ ] **Step 3: Add `scoreDraft` state + useEffect sync + resetOverride branch**

Next to the existing `useState` / `useEffect` / `resetOverride` for `suggestDraft`:

```typescript
const [scoreDraft, setScoreDraft] = useState("");
```

In the `useEffect` that syncs from `config`:

```typescript
setScoreDraft(config.prompt_debug.score_override ?? "");
```

In `resetOverride`, add a case (extend the existing if/else-if/else chain; the current last `else` handles `suggest_override`):

```typescript
} else if (key === "suggest_override") {
  setSuggestDraft("");
} else {
  setScoreDraft("");
}
```

Add a `scoreSource`:
```typescript
const scoreSource = sourceLabel(config.prompt_debug.score_override);
```

- [ ] **Step 4: Add the FieldGroup UI**

After the existing `<FieldGroup title="建议生成 Prompt">...</FieldGroup>` (B-stage's suggest_override group), add another FieldGroup right before the component's final `</div>`:

```tsx
<FieldGroup title="评分 Prompt">
  <div className="flex flex-wrap items-center gap-2">
    <Badge variant="outline">{scoreSource}</Badge>
    <p className="text-xs text-muted-foreground">
      影响 push 之前的主动性评分（P_S 1-5）。
    </p>
  </div>

  <div className="space-y-2">
    <Label htmlFor="score-override">覆盖值</Label>
    <Textarea
      id="score-override"
      name="score-override"
      rows={14}
      value={scoreDraft}
      placeholder="留空表示继续使用内置默认 prompt。"
      onChange={(event) => setScoreDraft(event.target.value)}
    />
  </div>

  <div className="flex flex-wrap gap-2">
    <Button
      onClick={() => saveOverride("score_override", scoreDraft)}
      disabled={isSaving}
    >
      保存
    </Button>
    <Button
      variant="outline"
      onClick={() => resetOverride("score_override")}
      disabled={isSaving}
    >
      恢复默认
    </Button>
    <Button
      variant="outline"
      onClick={() =>
        void copyEffectivePrompt(
          config.prompt_debug.score_override,
          DEFAULT_SCORE_PROMPT,
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
      {config.prompt_debug.score_override ?? DEFAULT_SCORE_PROMPT}
    </pre>
  </div>
</FieldGroup>
```

- [ ] **Step 5: Verify**

Run: `pnpm exec tsc --noEmit`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/pages/settings/sections/prompt-debug-section.tsx
git commit -m "feat(settings): add score_override textarea to prompt-debug page"
```

---

## Task 14: Final regression sweep

- [ ] **Step 1: Full Rust test suite**

Run: `cd src-tauri && cargo test`
Expected: all PASS; zero failures. Note: occasional `pool_isolation` flake is pre-existing — re-run once to confirm stability.

- [ ] **Step 2: Frontend**

Run: `pnpm exec tsc --noEmit`
Expected: clean.

Run: `pnpm test`
Expected: any pre-existing flakes acknowledged (not caused by this change).

- [ ] **Step 3: Manual smoke (optional, if you can launch)**

`pnpm tauri dev`:
- Open Settings → General → observe new "Corivo 主动性" slider; default position = 4.
- Drag slider to 2, confirm displayed number updates. Quit + relaunch — value persists.
- Settings → Prompt 调试 → new "评分 Prompt" FieldGroup appears with DEFAULT_SCORE_PROMPT contents.
- Trigger a fake push via Settings → 测试 tab (if it exists) or wait for real capture pipeline.
- Inspect `~/Library/Application Support/<app>/corivo.sqlite`: `proactivity_scores` table should be present; `notification_log.proactivity_score_id` column populated on new pushes.

- [ ] **Step 4: Final commit (if cleanup needed)**

If trivial fixes came out:

```bash
git add -A
git commit -m "chore: post-C-stage regression adjustments"
```

---

## Done Criteria

- All 14 tasks committed.
- `cd src-tauri && cargo test` green.
- `pnpm exec tsc --noEmit` green.
- `push_decider.rs`, `push_decider` test, and `push_pipeline_dismiss_penalty` test are deleted.
- `proactivity_scores` table holds every LLM scoring attempt; `notification_log.proactivity_score_id` links pushed entries.
- Settings page shows a 1-5 slider; moving it takes effect immediately on the next push decision.
- Hard rule: a dismissed revision_group is never pushed to again, regardless of P_S.

---

## Self-Review

Spec coverage:
- §1 Background & goals — covered in plan header.
- §2 Architecture overview — realized across Tasks 1-10; flow matches spec §3.
- §3 Data flow (hard rule / scorer / failure / handle_event rewrite) — Tasks 3, 9.
- §4 Data model (schema, repo methods, PushConfig change, push_decider delete) — Tasks 1, 2, 3, 9.
- §5 Prompt design — Task 5.
- §6 Configuration + UI — Tasks 8, 11, 12, 13.
- §7 Testing strategy — Tasks 1-7, 10, 14.
- §8 File listing — reflected in the file map at top.
- §9 Risks — mitigations wired into tests (threshold bounds, CHECK constraint, scorer failure path, snapshot stability).

No placeholders remain. Type names are consistent: `ProactivityScore` / `NewProactivityScore` / `ProactivityScoreRepo` / `SqliteProactivityScoreRepo` / `ProactivityScorer` / `ScoreInput` / `RawScore` are the same in every task that references them.
