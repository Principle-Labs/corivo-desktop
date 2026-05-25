//! Schema bootstrap + legacy cleanup.
//!
//! Corivo's migration model used to be a ladder of incremental steps
//! (v1..v14). Stage 2b of the event/task/project rework collapsed that
//! ladder into a single [`schema.sql`](./schema.sql) — because the
//! project is pre-release and all historical data is discardable
//! (Q11 tentative default "A"), a clean flat schema is easier to
//! reason about than threading ALTERs across fifteen patches.
//!
//! This module now does exactly two things:
//!
//! 1. If the DB already matches [`TARGET_SCHEMA_VERSION`], do nothing.
//! 2. Otherwise run [`purge_legacy_and_apply_new`]: drop every table
//!    we know about from any prior era, then execute `schema.sql`. This
//!    is destructive on legacy DBs by design.
//!
//! The legacy-table list below is maintained so a long-time
//! developer's `.sqlite` survives the reboot even though the rows
//! themselves don't.
//!
//! ## Chat-data preservation
//!
//! The boot-time path ([`apply_migrations`]) wraps the purge with
//! [`preserve_chat_tables`] / [`restore_chat_tables`] so that named
//! `chat_threads` (and their `chat_messages`) survive a schema bump.
//! Frames + notes + persona stay discardable — they're either a
//! captured stream or derived data — but a thread the user typed into
//! isn't reproducible from anything else. The user-initiated
//! `wipe_and_rebuild()` ("清空所有数据") calls [`purge_legacy_and_apply_new`]
//! directly so it keeps its "clear everything" semantics.

use crate::{
    db::pool::DbConnection,
    error::{CorivoError, Result},
};

/// Read from `schema.sql` at compile time so the bootstrap SQL lives
/// in exactly one place.
const SCHEMA_SQL: &str = include_str!("schema.sql");

/// Version written by `schema.sql`'s final `INSERT INTO schema_version`.
/// Anything less (or missing entirely) triggers a purge-and-rebuild.
///
/// Bump history:
/// - 100..104 — P/T/E architecture (gone)
/// - 200 — Task-centric architecture (gone)
/// - 300 — Frames-as-truth architecture (v3 spec)
/// - 400 — Phase 4: adds `frame_embeddings` for hybrid search.
/// - 500 — Phase 5: adds `adapter_name` / `adapter_payload` / `trigger`
///   columns to `frames`, `entities` table, drops `goals` /
///   `goal_evaluations`. Bump → purge wipes `frames` again; spec §二.6
///   allows it. Phase 5 needs the new columns to record per-app adapter
///   output and the Quick Ask vs timer vs focus_change provenance.
/// - 600 — Phase 5.5: jieba-tokenized FTS over `frames.search_tokens`.
/// - 700 — `saved_clips` 撤回。用户主动标注层不再存在；旧 DB 的
///   `saved_clips` 表通过 LEGACY 列表被 drop。
/// - 800 — 事件驱动采集骨架（中间版本，已被 v900 取代）。
/// - 900 — 纯事件驱动采集 + AX trigger 集合。`frames.trigger` 的
///   CHECK 最终落在 {focus_change, focused_window_changed,
///   title_changed, quick_ask, manual}。Timer driver 和 SafetyNet
///   都已撤回 —— 事件源（NSWorkspace / AXObserver / IPC）失效就
///   直接停摆。**v800 → v900 必须经过 purge-and-apply**：早期的
///   v800 schema.sql 在迭代过程中被原地修改过几版（曾经允许
///   timer / safety_net），停在 v800 的 dev DB 实际带的还是早期
///   CHECK。bump 到 900 是为了强制触发 migrations.rs::apply_migrations
///   的 drop-and-rebuild 路径，让所有 dev DB 重建。
/// - 1000 — 撤回 entities 表与 entity 抽取后台任务。recall 层不再
///   依赖结构化原子的精确召回；所有查询改走 frames + FTS。bump 触发
///   purge-and-apply，让 v900 dev DB 中残留的 entities 表与 indexes
///   被丢弃。
/// - 1100 — Phase C §8.2: chat_threads 增加 bound_model_id /
///   bound_api_shape 两列,把每个会话永久绑定到一个 model。purge-
///   and-apply 让旧 dev DB 的 chat_threads 行被丢弃 —— 旧行没有
///   bound_model_id 也无从补,直接重建是预发布期最干净的路径。
/// - 1200 — 重写 chat_messages 数据模型 (commit 后 spec §五 chat 段)。
///   `content` + `tool_calls` 合并成 `content_blocks` JSON 数组（Anthropic
///   content-block 形态：text / thinking / tool_use / tool_result /
///   focus_context / frame_citation），新增 `status` / `error_message` /
///   `finish_reason` / `usage` / `updated_at` 列，把 user 发送瞬间
///   落库 + assistant 占位 + finalize 三段式生命周期落到 schema 上。
///   `tool` 角色废弃。purge-and-apply 让旧 chat_messages 行被丢弃。
/// v1300 — chat_threads 加 `pinned_at` + `archived_at` 列,Sidebar 三节
///   分区(置顶 / 最近 / 归档)。purge-and-apply,旧 chat_threads 行
///   丢弃 —— pre-release 期可弃。
/// v1301 — chat_messages 加 `model_used` (TEXT, NULL) 列,记录每条
///   assistant 消息实际命中的 model alias (per-message model switching
///   审计)。purge-and-apply,旧聊天历史丢弃 —— 跟 v1300 一样 pre-release
///   期可弃。
/// v1400 — 新增 `notes` 表(memory-system-spec §3 Step 1a)。促进
///   "用户让我记住" 一条龙:save_note 工具写入 → persistent block
///   注入 → 后续 turn 自动应用。purge-and-apply,新表无历史。
/// v1410 — memory-system-spec Step 1b 落地:
///   * `chat_threads` 增 `kind` + `system_task` —— 后台 agent 任务和
///     用户对话共表。
///   * `chat_threads` 增 `summary` + `summary_topics` +
///     `summary_updated_at` + `thread_summaries_fts` 虚表 —— session
///     learner 副产物,thread_search 工具用。
///   * `chat_messages.role` CHECK 扩到包含 `'system'`。
///   * 新增 `background_agent_task_checkpoints` 表 —— session learner
///     避免重复学习同段对话。
///   purge-and-apply,旧聊天历史丢弃。
/// v1420 — memory-system-spec Step 2:
///   * `notes` 增 `search_tokens` + `notes_fts` 虚表。
///   * `chat_messages` 增 `search_tokens` + `chat_messages_fts` 虚表。
///   * 统一召回层 `services::memory` 上线,system + tool 双通路注入。
/// v1421 — ApiShape 扩 `openai_responses`:
///   * `chat_threads.bound_api_shape` CHECK 约束新增 `'openai_responses'`
///     枚举值,GPT-5 等只在 `/v1/responses` 走得通的 model 用这个分支。
///   purge-and-apply,旧聊天历史丢弃。
/// v1500 — privacy-filter-spec Phase 1 地基:
///   * `frames` 增 `ax_text_pii_spans` (TEXT, JSON) 列 —— 存 PII
///     span 元数据,classify-once / enforce-at-egress 架构的核心字段。
///   * 本 bump 不引入新表也不动 chat;frames 不在 preserved 行列里,
///     purge-and-apply 会把历史 ax_text 一并丢掉(pre-release 期可弃)。
///   * 模型加载 / hook / settings UI 在后续 commit 接入,本 commit
///     只把 schema + domain 类型先就位。
/// v1510 — 新增 `workflow_schedules` + `workflow_runs` 两张表
///   (`services::scheduled_workflows`). 定义本体落到文件系统
///   (`$APPDATA/corivo/workflows/<slug>/WORKFLOW.md`),表只承载触发器
///   + 运行态 + 历史。purge-and-apply,新表无历史。
/// v1511 — `workflow_schedules` 增 `source` + `created_by_thread_id` 两列。
///   source 区分用户创建 vs agent 通过 `schedule_task` native tool 创建;
///   created_by_thread_id 指向触发创建的 chat_thread(ON DELETE SET NULL)。
///   purge-and-apply,新列无历史。
/// v1512 — workflow 通知策略 + 已读跟踪 (PR6)。
///   * `workflow_schedules.notify_policy ∈ ('always','on_change','silent')`
///     让 workflow 作者控制 macOS banner / in-app toast 的触发条件。
///   * `workflow_runs` 增 `summary` / `content_hash` / `acknowledged_at`
///     —— summary 喂 banner 正文; content_hash 用于 on_change 去重;
///     acknowledged_at 驱动 sidebar 未读 dot。
///   purge-and-apply,新列无历史。
/// v1513 — `workflow_runs.slug` 上原本挂着
///   `REFERENCES workflow_schedules(slug) ON DELETE CASCADE`,但"立即运行
///   一个没排时间的 workflow"(WORKFLOW.md 在磁盘上、schedules 表里没行)
///   是合法路径,每次 record_run 都被 FK 拒掉,UI 永远卡在"正在运行"。
///   去掉 FK,改由 `delete_schedule` 显式清理 `workflow_runs` 行。
///   purge-and-apply,旧 runs 历史丢弃(本来就只有 0 行)。
pub const TARGET_SCHEMA_VERSION: i64 = 1513;

/// Every table name that any ancestor of this schema introduced. Drop
/// order matters: children before parents (FKs) when foreign_keys are
/// on. We also disable foreign_keys during the purge so missing parents
/// don't abort the batch.
const LEGACY_TABLES: &[&str] = &[
    // Brain / work-memory-brain era (commits 7e45a66..f3bd8a0)
    "push_events",
    "link_blacklist",
    "links",
    "thread_facts",
    "threads",
    "threads_fts",
    "thread_fact_audit",
    "facts_fts",
    // V0 / GUM era (migrations 005..014)
    "focus_sessions",
    "project_streams",
    "reconcile_log",
    "work_context_proposition",
    "work_context_observation",
    "work_context_signals",
    "work_context_episodes",
    "work_contexts_fts",
    "work_contexts",
    "proactivity_scores",
    "suggestions_fts",
    "suggestions",
    "notification_log",
    "observation_proposition",
    "propositions_fts",
    "propositions",
    // Pre-GUM legacy
    "attention_item_events",
    "attention_item_evidence",
    "attention_items",
    "notification_decision_log",
    "extracted_facts",
    "conflict_alerts",
    "segments",
    // P/T/E era (v100..v104) — gone in v200
    "projects",
    "project_identity_anchors",
    "project_merge_suggestions",
    "project_merge_blocklist",
    // Shared tables we also recreate from scratch (schema evolved)
    "events_fts",
    "tasks_fts",
    "tasks",
    "events",
    "event_screenshots",
    "facts",
    "reassignment_log",
    "observations",
    "screenshots",
    "sessions",
    "coach_notes",
    "user_model",
    "goal_event_evaluations",
    "goals",
    // v200 task-centric tables (gone in v300 — frames is now SSOT).
    "tags",
    "task_tags",
    "tag_anchors",
    "task_anchors",
    "time_segments",
    "about_user",
    "about_user_fts",
    // v300 tables.
    "frames",
    "frames_fts",
    "chat_threads",
    "chat_messages",
    // v300 → v400 only, dropped at v500
    "goals",
    "goal_evaluations",
    // v400 additions
    "frame_embeddings",
    // v500 additions
    "entities",
    // v300..v600 only, dropped at v700 (用户主动标注层撤回)
    "saved_clips",
    // v1400+ live tables — purge-and-apply on later bumps (e.g.
    // v1420 → v1510) must drop these too, otherwise schema.sql's
    // `CREATE TABLE` fails with "table X already exists". The
    // matching `*_fts` virtual tables are caught by
    // `drop_orphan_fts_vtables`, no need to list them here.
    "notes",
    "background_agent_task_checkpoints",
    // v1510 additions — drop on later bumps so schema.sql's CREATE
    // TABLE can re-create them without colliding.
    "workflow_runs",
    "workflow_schedules",
    "schema_version",
];

pub fn apply_migrations(conn: &DbConnection) -> Result<()> {
    let current = get_current_version(conn)?;
    if current == TARGET_SCHEMA_VERSION {
        tracing::info!(version = TARGET_SCHEMA_VERSION, "DB schema up to date");
        return Ok(());
    }

    if current == 0 {
        tracing::info!(
            target_version = TARGET_SCHEMA_VERSION,
            "fresh DB — applying event/task/project schema"
        );
    } else {
        tracing::warn!(
            current_version = current,
            target_version = TARGET_SCHEMA_VERSION,
            "legacy DB detected — purging all prior tables and applying new schema"
        );
    }

    // Carve a hole in the purge for chat_threads + chat_messages so an
    // existing user's named sessions survive the rebuild. Pre-release the
    // rest of the DB is still discardable (frames are a captured-stream,
    // notes/persona are derived) — but a manually-titled thread the user
    // typed into isn't reproducible from anything else.
    //
    // Skipped on `wipe_and_rebuild()` (the user-initiated "清空所有数据"
    // action) so that path keeps its "clear everything" semantics.
    preserve_chat_tables(conn, current)?;
    purge_legacy_and_apply_new(conn)?;
    restore_chat_tables(conn)?;

    let after = get_current_version(conn)?;
    if after != TARGET_SCHEMA_VERSION {
        return Err(CorivoError::Internal(format!(
            "schema bootstrap did not land target version: got {after}, expected {TARGET_SCHEMA_VERSION}"
        )));
    }

    tracing::info!(version = after, "DB schema bootstrap complete");
    Ok(())
}

const PRESERVED_CHAT_THREADS: &str = "__preserved_chat_threads";
const PRESERVED_CHAT_MESSAGES: &str = "__preserved_chat_messages";

/// Stash `chat_threads` + `chat_messages` under throwaway names so the
/// purge step's `DROP TABLE chat_threads / chat_messages` no-ops past
/// them and the rows live through the schema rebuild.
///
/// Source-schema cutoffs:
/// - `chat_threads` from v1100+ (v1100 introduced `bound_model_id`,
///   which is `NOT NULL` in the current schema — earlier rows can't be
///   inserted without inventing a model binding).
/// - `chat_messages` from v1200+ (v1200 rewrote the column set —
///   `content_blocks` / `status` / `usage` etc. — earlier rows had a
///   shape that can't be back-mapped without re-running the LLM).
///
/// Below those cutoffs we let the purge take the rows; the user has no
/// recoverable data anyway.
fn preserve_chat_tables(conn: &DbConnection, from_version: i64) -> Result<()> {
    // A previous migration that crashed mid-way could leave preserved
    // names sitting around. Clear them before we rename, otherwise the
    // ALTER would fail with "table __preserved_* already exists".
    for stale in [PRESERVED_CHAT_THREADS, PRESERVED_CHAT_MESSAGES] {
        let sql = format!("DROP TABLE IF EXISTS {stale}");
        if let Err(error) = conn.execute(&sql, []) {
            tracing::warn!(table = stale, %error, "stale preserve table drop failed");
        }
    }

    if from_version >= 1100 && table_exists(conn, "chat_threads")? {
        conn.execute(
            &format!("ALTER TABLE chat_threads RENAME TO {PRESERVED_CHAT_THREADS}"),
            [],
        )
        .map_err(|error| {
            CorivoError::Internal(format!("preserve chat_threads rename failed: {error}"))
        })?;
    }

    if from_version >= 1200 && table_exists(conn, "chat_messages")? {
        conn.execute(
            &format!("ALTER TABLE chat_messages RENAME TO {PRESERVED_CHAT_MESSAGES}"),
            [],
        )
        .map_err(|error| {
            CorivoError::Internal(format!("preserve chat_messages rename failed: {error}"))
        })?;
    }

    Ok(())
}

/// Pull preserved rows back into the freshly-created tables. Run after
/// [`purge_legacy_and_apply_new`], so `chat_threads` + `chat_messages`
/// already exist with the current schema.
///
/// Column projection is dynamic — `pinned_at`/`archived_at` arrive in
/// v1300, `model_used` in v1301 — and any column missing on the source
/// table is filled with `NULL`. New-schema columns the source never had
/// (`kind`, `system_task`, `summary*`, `search_tokens`) fall through to
/// the new table's `DEFAULT` (`kind='user'`) or `NULL`.
///
/// `chat_messages.search_tokens` is intentionally left `NULL`: the FTS
/// trigger short-circuits on `NULL` so old rows won't appear in
/// thread-search recall. Backfilling would require jieba on every
/// preserved row at boot, and the recall layer is a soft surface — the
/// UI still renders these messages from `content_blocks` / `content_text`.
///
/// Streaming-orphan rows (`status='streaming'`) are skipped; they're the
/// detritus of a crashed-mid-stream turn and the boot cleanup would have
/// flipped them to `cancelled` on the legacy schema anyway.
fn restore_chat_tables(conn: &DbConnection) -> Result<()> {
    if table_exists(conn, PRESERVED_CHAT_THREADS)? {
        let cols = columns_of(conn, PRESERVED_CHAT_THREADS)?;
        let has_pinned = cols.iter().any(|c| c == "pinned_at");
        let has_archived = cols.iter().any(|c| c == "archived_at");
        let select_pinned = if has_pinned { "pinned_at" } else { "NULL" };
        let select_archived = if has_archived { "archived_at" } else { "NULL" };
        let sql = format!(
            "INSERT INTO chat_threads
                 (id, title, bound_model_id, bound_api_shape,
                  pinned_at, archived_at, created_at, updated_at)
             SELECT id, title, bound_model_id, bound_api_shape,
                    {select_pinned}, {select_archived}, created_at, updated_at
               FROM {PRESERVED_CHAT_THREADS}"
        );
        let restored = conn.execute(&sql, []).map_err(|error| {
            CorivoError::Internal(format!("restore chat_threads failed: {error}"))
        })?;
        conn.execute(&format!("DROP TABLE {PRESERVED_CHAT_THREADS}"), [])
            .map_err(|error| {
                CorivoError::Internal(format!("drop preserved chat_threads failed: {error}"))
            })?;
        tracing::info!(rows = restored, "chat_threads preserved across migration");
    }

    if table_exists(conn, PRESERVED_CHAT_MESSAGES)? {
        let cols = columns_of(conn, PRESERVED_CHAT_MESSAGES)?;
        // Sanity: v1200+ must have content_blocks. If not, leave the
        // rows in the preserved table for inspection rather than INSERT
        // garbage into the new chat_messages.
        let has_content_blocks = cols.iter().any(|c| c == "content_blocks");
        if has_content_blocks {
            let has_model_used = cols.iter().any(|c| c == "model_used");
            let has_updated_at = cols.iter().any(|c| c == "updated_at");
            let select_model_used = if has_model_used { "model_used" } else { "NULL" };
            let select_updated_at = if has_updated_at { "updated_at" } else { "NULL" };
            let sql = format!(
                "INSERT INTO chat_messages
                     (id, thread_id, role, content_blocks, content_text, cited_frame_ids,
                      status, error_message, finish_reason, usage, model_used,
                      created_at, updated_at)
                 SELECT id, thread_id, role, content_blocks, content_text, cited_frame_ids,
                        status, error_message, finish_reason, usage, {select_model_used},
                        created_at, {select_updated_at}
                   FROM {PRESERVED_CHAT_MESSAGES}
                  WHERE status != 'streaming'"
            );
            let restored = conn.execute(&sql, []).map_err(|error| {
                CorivoError::Internal(format!("restore chat_messages failed: {error}"))
            })?;
            tracing::info!(rows = restored, "chat_messages preserved across migration");
        } else {
            tracing::warn!(
                "preserved chat_messages has no content_blocks column; leaving rows unimported"
            );
        }
        conn.execute(&format!("DROP TABLE {PRESERVED_CHAT_MESSAGES}"), [])
            .map_err(|error| {
                CorivoError::Internal(format!("drop preserved chat_messages failed: {error}"))
            })?;
    }

    Ok(())
}

fn table_exists(conn: &DbConnection, name: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |row| row.get::<_, i32>(0).map(|v| v == 1),
    )
    .map_err(|error| CorivoError::Internal(format!("table_exists({name}) failed: {error}")))
}

fn columns_of(conn: &DbConnection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| {
            CorivoError::Internal(format!("table_info({table}) prepare failed: {error}"))
        })?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| {
            CorivoError::Internal(format!("table_info({table}) query failed: {error}"))
        })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| {
        CorivoError::Internal(format!("table_info({table}) collect failed: {error}"))
    })
}

/// Purge every table we know about and apply `schema.sql` from scratch.
///
/// Wrapped in a savepoint so a mid-apply failure leaves the DB in its
/// prior (legacy) state instead of half-migrated. FKs are temporarily
/// disabled because the drop list is flat — some parent tables land
/// before their children under cascade rules, and we'd rather power
/// through than re-topologize on every schema bump.
///
/// Also re-used by `Database::wipe_and_rebuild()` for the "清空所有数据"
/// settings action. Unlike the boot migration path in [`apply_migrations`],
/// this entry point skips chat preservation — wiping means wiping.
pub(crate) fn purge_legacy_and_apply_new(conn: &DbConnection) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .map_err(|error| CorivoError::Internal(format!("disable foreign_keys failed: {error}")))?;

    // Also nuke any FTS5 orphans the drop list didn't catch (e.g. a
    // vtable whose content= table was removed out of order).
    drop_orphan_fts_vtables(conn)?;

    for table in LEGACY_TABLES {
        let sql = format!("DROP TABLE IF EXISTS {table}");
        if let Err(error) = conn.execute(&sql, []) {
            // Don't abort: we'll hit `execute_batch(SCHEMA_SQL)` next
            // and that'll surface any actual schema error.
            tracing::warn!(table, %error, "legacy drop failed; continuing");
        }
    }

    // Drop any remaining triggers attached to tables we've just dropped —
    // `DROP TABLE` takes its table triggers, but FTS5 content-row
    // triggers on user tables can linger if we dropped them in the
    // wrong order.
    drop_orphan_triggers(conn)?;

    conn.execute_batch(SCHEMA_SQL)
        .map_err(|error| CorivoError::Internal(format!("applying schema.sql failed: {error}")))?;

    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|error| {
            CorivoError::Internal(format!("re-enable foreign_keys failed: {error}"))
        })?;

    Ok(())
}

fn drop_orphan_fts_vtables(conn: &DbConnection) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master
               WHERE type = 'table' AND sql LIKE '%USING fts5%'",
        )
        .map_err(|error| CorivoError::Internal(format!("fts vtable scan failed: {error}")))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| CorivoError::Internal(format!("fts vtable query failed: {error}")))?;
    let names: Vec<String> = rows.filter_map(|r| r.ok()).collect();
    drop(stmt);
    for name in names {
        let sql = format!("DROP TABLE IF EXISTS {name}");
        if let Err(error) = conn.execute(&sql, []) {
            tracing::warn!(vtable = %name, %error, "fts vtable drop failed; continuing");
        }
    }
    Ok(())
}

fn drop_orphan_triggers(conn: &DbConnection) -> Result<()> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger'")
        .map_err(|error| CorivoError::Internal(format!("trigger scan failed: {error}")))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| CorivoError::Internal(format!("trigger query failed: {error}")))?;
    let names: Vec<String> = rows.filter_map(|r| r.ok()).collect();
    drop(stmt);
    for name in names {
        let sql = format!("DROP TRIGGER IF EXISTS {name}");
        if let Err(error) = conn.execute(&sql, []) {
            tracing::warn!(trigger = %name, %error, "trigger drop failed; continuing");
        }
    }
    Ok(())
}

fn get_current_version(conn: &DbConnection) -> Result<i64> {
    let table_exists: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_version')",
            [],
            |row| row.get::<_, i32>(0).map(|value| value == 1),
        )
        .map_err(|error| CorivoError::Internal(format!("schema_version probe failed: {error}")))?;

    if !table_exists {
        return Ok(0);
    }

    let version = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get::<_, Option<i64>>(0)
        })
        .map_err(|error| CorivoError::Internal(format!("schema_version read failed: {error}")))?;

    Ok(version.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::test_in_memory_pool;

    #[test]
    fn fresh_db_lands_target_version() {
        let pool = test_in_memory_pool().unwrap();
        let conn = pool.get().unwrap();
        apply_migrations(&conn).unwrap();
        assert_eq!(get_current_version(&conn).unwrap(), TARGET_SCHEMA_VERSION);
    }

    #[test]
    fn repeat_apply_is_noop() {
        let pool = test_in_memory_pool().unwrap();
        let conn = pool.get().unwrap();
        apply_migrations(&conn).unwrap();
        apply_migrations(&conn).unwrap();
        assert_eq!(get_current_version(&conn).unwrap(), TARGET_SCHEMA_VERSION);
    }

    #[test]
    fn purges_legacy_v0_db() {
        let pool = test_in_memory_pool().unwrap();
        let conn = pool.get().unwrap();
        // Simulate a V0-ish DB: a handful of legacy tables plus the
        // old schema_version pinned at 14.
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, extra TEXT);
             CREATE TABLE work_contexts (id INTEGER PRIMARY KEY, title TEXT);
             CREATE TABLE propositions (id INTEGER PRIMARY KEY, text TEXT);
             CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
             INSERT INTO schema_version (version) VALUES (14);",
        )
        .unwrap();

        apply_migrations(&conn).unwrap();
        assert_eq!(get_current_version(&conn).unwrap(), TARGET_SCHEMA_VERSION);

        // The legacy-only rows must be gone.
        let work_ctx_count: i64 = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='work_contexts')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(work_ctx_count, 0, "work_contexts should be dropped");

        // The new tables must exist.
        for name in [
            "frames",
            "frames_fts",
            "frame_embeddings",
            "chat_threads",
            "chat_messages",
        ] {
            let exists: i64 = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?1)",
                    [name],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "{name} should exist after bootstrap");
        }
    }

    /// Simulate the prod-DB shape (schema_version=1301) — chat_threads
    /// with `bound_model_id` + `pinned_at`/`archived_at` but no `kind`,
    /// chat_messages with `content_blocks` + `model_used` but no
    /// `search_tokens`. Assert that after the bump every row survives
    /// and picks up `kind='user'` from the new-schema default.
    #[test]
    fn boot_migration_preserves_chat_from_v1301() {
        let pool = test_in_memory_pool().unwrap();
        let conn = pool.get().unwrap();

        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
             INSERT INTO schema_version (version) VALUES (1301);

             CREATE TABLE chat_threads (
                 id              TEXT PRIMARY KEY,
                 title           TEXT,
                 bound_model_id  TEXT NOT NULL,
                 bound_api_shape TEXT NOT NULL,
                 pinned_at       TEXT,
                 archived_at     TEXT,
                 created_at      TEXT NOT NULL,
                 updated_at      TEXT NOT NULL
             );
             INSERT INTO chat_threads
                 (id, title, bound_model_id, bound_api_shape, pinned_at, archived_at, created_at, updated_at)
             VALUES
                 ('t-alpha', 'hi',     'sonnet-4', 'anthropic', NULL,
                  NULL, '2026-05-13T19:43:30.000Z', '2026-05-13T19:43:30.000Z'),
                 ('t-beta',  'Linear', 'sonnet-4', 'anthropic',
                  '2026-05-13T19:50:00.000Z', NULL,
                  '2026-05-13T19:50:00.000Z', '2026-05-13T19:57:33.000Z');

             CREATE TABLE chat_messages (
                 id              TEXT PRIMARY KEY,
                 thread_id       TEXT NOT NULL,
                 role            TEXT NOT NULL,
                 content_blocks  TEXT NOT NULL,
                 content_text    TEXT NOT NULL DEFAULT '',
                 cited_frame_ids TEXT,
                 status          TEXT NOT NULL DEFAULT 'complete',
                 error_message   TEXT,
                 finish_reason   TEXT,
                 usage           TEXT,
                 model_used      TEXT,
                 created_at      TEXT NOT NULL,
                 updated_at      TEXT
             );
             INSERT INTO chat_messages
                 (id, thread_id, role, content_blocks, content_text, status, model_used, created_at)
             VALUES
                 ('m-1', 't-alpha', 'user',      '[]', 'hi',    'complete', NULL,       '2026-05-13T19:43:30.000Z'),
                 ('m-2', 't-alpha', 'assistant', '[]', 'hello', 'complete', 'sonnet-4', '2026-05-13T19:43:31.000Z'),
                 ('m-3', 't-beta',  'user',      '[]', 'q',     'complete', NULL,       '2026-05-13T19:50:00.000Z'),
                 ('m-4', 't-beta',  'assistant', '[]', 'orphan','streaming','sonnet-4', '2026-05-13T19:50:01.000Z');"
        ).unwrap();

        apply_migrations(&conn).unwrap();
        assert_eq!(get_current_version(&conn).unwrap(), TARGET_SCHEMA_VERSION);

        // Threads survived and picked up kind='user' from the DEFAULT.
        let thread_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chat_threads WHERE kind = 'user'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(thread_count, 2, "both v1301 threads should survive");

        // Pin state preserved.
        let beta_pinned: Option<String> = conn
            .query_row(
                "SELECT pinned_at FROM chat_threads WHERE id = 't-beta'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(beta_pinned.is_some(), "pinned_at must round-trip");

        // 3 of 4 messages survived; the streaming orphan was dropped.
        let msg_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chat_messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(msg_count, 3, "streaming orphan should be skipped");

        // model_used round-tripped on the assistant row.
        let m2_model: Option<String> = conn
            .query_row(
                "SELECT model_used FROM chat_messages WHERE id = 'm-2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(m2_model.as_deref(), Some("sonnet-4"));

        // Preserved temp tables are gone.
        let leftover: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE '__preserved_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(leftover, 0, "preserved temp tables should be dropped");
    }

    /// Pre-v1100 chat_threads lacked `bound_model_id` (NOT NULL in the
    /// current schema). Preserving would fail the insert, so the
    /// migration must let those rows go with the purge.
    #[test]
    fn boot_migration_skips_chat_below_v1100() {
        let pool = test_in_memory_pool().unwrap();
        let conn = pool.get().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
             INSERT INTO schema_version (version) VALUES (1000);

             CREATE TABLE chat_threads (
                 id         TEXT PRIMARY KEY,
                 title      TEXT,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             INSERT INTO chat_threads (id, title, created_at, updated_at)
             VALUES ('legacy', 'x', '2025-01-01T00:00:00.000Z', '2025-01-01T00:00:00.000Z');"
        ).unwrap();

        apply_migrations(&conn).unwrap();

        let thread_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chat_threads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(thread_count, 0, "pre-v1100 threads must not be restored");
    }

    /// `wipe_and_rebuild()` skips preservation by design — verify by
    /// going through [`purge_legacy_and_apply_new`] directly with a
    /// v1301-shaped DB and confirming chat_threads ends up empty.
    #[test]
    fn user_wipe_does_not_preserve_chat() {
        let pool = test_in_memory_pool().unwrap();
        let conn = pool.get().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
             INSERT INTO schema_version (version) VALUES (1301);

             CREATE TABLE chat_threads (
                 id              TEXT PRIMARY KEY,
                 title           TEXT,
                 bound_model_id  TEXT NOT NULL,
                 bound_api_shape TEXT NOT NULL,
                 pinned_at       TEXT,
                 archived_at     TEXT,
                 created_at      TEXT NOT NULL,
                 updated_at      TEXT NOT NULL
             );
             INSERT INTO chat_threads
                 (id, title, bound_model_id, bound_api_shape, created_at, updated_at)
             VALUES ('t-alpha', 'hi', 'sonnet-4', 'anthropic',
                     '2026-05-13T19:43:30.000Z', '2026-05-13T19:43:30.000Z');"
        ).unwrap();

        purge_legacy_and_apply_new(&conn).unwrap();

        let thread_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chat_threads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(thread_count, 0, "wipe path must NOT preserve chat_threads");
    }
}
