-- Corivo SQLite schema — frames-as-truth architecture (v1000, v3 spec).
--
-- Per docs/corivo-architecture-v3-spec.md §五. Single source of truth.
-- Migration model is purge-and-apply: db/migrations.rs drops every
-- legacy table it knows about then applies this file. Pre-release
-- project; historical rows are intentionally discardable.
--
-- Tables in v1000:
--   frames              → SSOT (one row per capture; ax_text + ocr_text +
--                          adapter_name + adapter_payload + trigger)
--   frames_fts          → FTS5 mirror over (app_name, window_title, url, ax_text, ocr_text)
--   frame_embeddings    → Phase 4 vector index (one row per embedded frame)
--   chat_threads        → /ask + Quick Ask conversation containers
--   chat_messages       → /ask + Quick Ask messages with tool_calls + cited_frame_ids
--
-- Removed in v500 (per spec §五 「不存在的表」):
--   goals / goal_evaluations  → "被动 drift 评估已被 Quick Ask 取代"
-- Removed in v700:
--   saved_clips               → 用户主动标注层下线，clip 概念整体撤回
-- Removed in v1000:
--   entities                  → entity 抽取整层撤回；recall 层只走 frames + FTS
--
-- Time discipline (db/time.rs):
--   * Every *_at column is TEXT in canonical RFC3339
--     `YYYY-MM-DDTHH:MM:SS.sssZ`.
--   * DEFAULT clauses use strftime('%Y-%m-%dT%H:%M:%fZ','now').
--   * Raw datetime-now calls banned (tests/time_discipline.rs grep-scans).

------------------------------------------------------------------------
-- frames — 唯一的事实源
------------------------------------------------------------------------

CREATE TABLE frames (
    id                       TEXT PRIMARY KEY,            -- ULID
    captured_at              TEXT NOT NULL
        CHECK (captured_at GLOB '????-??-??T??:??:??.???Z'),
    device_id                TEXT NOT NULL,
    capture_session_id       TEXT NOT NULL,

    app_bundle_id            TEXT,
    app_name                 TEXT,
    window_title             TEXT,
    url                      TEXT,

    screenshot_path          TEXT,
    screenshot_hash          TEXT,
    screenshot_size_bytes    INTEGER,

    ax_text                  TEXT,
    ocr_text                 TEXT,
    -- v1600: `ax_text_pii_spans` 列已撤销。隐私过滤从 classify-at-capture
    -- 改成 classify-at-egress —— PII spans 不再落盘,exec_agent 出口处
    -- 现算现用(blake3(text) → spans 走内存 LRU)。原始 ax_text 不做
    -- 物理替换,redact 仅作用在送进 cloud LLM 的副本上。
    -- 历史:v1500 曾在此处加 `ax_text_pii_spans TEXT` 存模型识别出的
    -- PII span JSON,classify-once / enforce-at-egress 架构的中间产物;
    -- 实际从未在 capture 路径接通过(Hook A 一直是 NULL),撤掉省一刀
    -- IO/CPU + 避免 PII span 元数据二次落盘。
    -- Phase 5 per-app adapter output. `adapter_name` is an open enum
    -- ('chrome' / 'vscode' / 'lark' / ... / 'generic_ax'); `adapter_payload`
    -- is JSON with adapter-specific structured fields (URL / file path /
    -- selection / cwd / channel / etc.). NULL when extraction didn't go
    -- through an adapter (legacy 'ax' / 'ocr' / 'skipped' rows).
    adapter_name             TEXT,
    adapter_payload          TEXT,
    extraction_strategy      TEXT NOT NULL
        CHECK (extraction_strategy IN ('ax', 'ocr', 'ax+ocr', 'adapter', 'skipped')),
    extraction_duration_ms   INTEGER,
    fallback_reason          TEXT,

    -- What woke capture_pipeline up for this frame. Pure event-driven —
    -- no fixed-cadence variant.
    --   focus_change            — NSWorkspace front-app shift.
    --   focused_window_changed  — AXObserver: focused window switched
    --                             inside the active app.
    --   title_changed           — AXObserver: window title changed
    --                             (URL / page change main signal).
    --   quick_ask               — global hotkey invoke.
    --   manual                  — Tauri IPC requested a snapshot.
    trigger                  TEXT NOT NULL
        DEFAULT 'manual'
        CHECK (trigger IN ('focus_change', 'focused_window_changed',
                           'title_changed', 'quick_ask', 'manual')),

    content_hash             TEXT,
    derived_from_frame_id    TEXT REFERENCES frames(id) ON DELETE SET NULL,
    still_present_until      TEXT
        CHECK (still_present_until IS NULL
               OR still_present_until GLOB '????-??-??T??:??:??.???Z'),

    exclusion_match          TEXT,

    -- v600: jieba-tokenized search index (spec §五). Populated by the
    -- ingest pipeline as space-separated tokens (jieba.cut精确模式 over
    -- ax_text + ocr_text + window_title + url + app_name). frames_fts
    -- indexes ONLY this column with the unicode61 tokenizer, so CJK
    -- queries that hit no whitespace in the original text still match.
    -- NULL is fine — older rows just won't be FTS-searchable until they
    -- get re-ingested or back-filled.
    search_tokens            TEXT,

    created_at               TEXT NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        CHECK (created_at GLOB '????-??-??T??:??:??.???Z')
);

CREATE INDEX idx_frames_captured_at ON frames(captured_at DESC);
CREATE INDEX idx_frames_app         ON frames(app_bundle_id, captured_at DESC);
CREATE INDEX idx_frames_url         ON frames(url) WHERE url IS NOT NULL;
CREATE INDEX idx_frames_content     ON frames(content_hash);
CREATE INDEX idx_frames_session
    ON frames(capture_session_id, captured_at DESC);
-- Quick Ask history can be queried independently of timer / focus_change
-- so the /ask UI can surface prior Quick Ask threads. spec §五.
CREATE INDEX idx_frames_trigger     ON frames(trigger, captured_at DESC);

------------------------------------------------------------------------
-- frames_fts — FTS5 external content over the jieba-tokenized search
-- column (spec §五).
--
-- v600 swap: previously a 5-column index over the raw app_name /
-- window_title / url / ax_text / ocr_text using `unicode61`. CJK
-- queries failed because unicode61 sees Chinese text as one giant
-- token. The ingest pipeline now writes a `search_tokens` column with
-- jieba-segmented, space-separated tokens; FTS5 indexes that single
-- column and unicode61 happily splits on whitespace.
------------------------------------------------------------------------

CREATE VIRTUAL TABLE frames_fts USING fts5(
    search_tokens,
    content='frames',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER frames_fts_insert AFTER INSERT ON frames BEGIN
    INSERT INTO frames_fts(rowid, search_tokens)
    VALUES (new.rowid, COALESCE(new.search_tokens, ''));
END;

CREATE TRIGGER frames_fts_delete AFTER DELETE ON frames BEGIN
    INSERT INTO frames_fts(frames_fts, rowid, search_tokens)
    VALUES ('delete', old.rowid, COALESCE(old.search_tokens, ''));
END;

CREATE TRIGGER frames_fts_update AFTER UPDATE ON frames BEGIN
    INSERT INTO frames_fts(frames_fts, rowid, search_tokens)
    VALUES ('delete', old.rowid, COALESCE(old.search_tokens, ''));
    INSERT INTO frames_fts(rowid, search_tokens)
    VALUES (new.rowid, COALESCE(new.search_tokens, ''));
END;

------------------------------------------------------------------------
-- frame_embeddings — Phase 4 vector index (spec §五).
--
-- One row per embedded frame. Vector stored as raw little-endian f32
-- bytes (4 bytes × dimensions); cosine similarity is computed in Rust
-- against this blob (full-table scan — fine at <100k frames; Phase 5+
-- can swap in sqlite-vec or hnswlib if scan time becomes a problem).
------------------------------------------------------------------------

CREATE TABLE frame_embeddings (
    frame_id     TEXT PRIMARY KEY REFERENCES frames(id) ON DELETE CASCADE,
    model        TEXT NOT NULL,
    dimensions   INTEGER NOT NULL,
    vector       BLOB NOT NULL,
    created_at   TEXT NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        CHECK (created_at GLOB '????-??-??T??:??:??.???Z')
);

CREATE INDEX idx_frame_embeddings_model ON frame_embeddings(model);

------------------------------------------------------------------------
-- chat_threads + chat_messages — /ask conversation history (Phase 3).
--
-- Stored ahead of Phase 3 so schema_version doesn't need a second bump
-- when the recall layer lands. cited_frame_ids and tool_calls are JSON
-- text columns; the Vercel AI SDK Data Stream Protocol fields for tool
-- use are reconstructable from these two strings + role.
------------------------------------------------------------------------

CREATE TABLE chat_threads (
    id              TEXT PRIMARY KEY,
    title           TEXT,
    -- v1400 (memory-system-spec §11.2): thread classification.
    --   'user'   — opened by the user via /ask or Quick Ask. The only
    --              kind that appears in the sidebar.
    --   'system' — opened by a background agent task (session learner,
    --              persona distill, future). Never surfaces in the
    --              `/ask` sidebar; only the Settings "记忆 → 诊断"
    --              panel sees these.
    -- `system_task` carries the task discriminator when kind='system'
    -- ('persona_distill' | 'session_memory_learning' | future kinds);
    -- NULL when kind='user'.
    kind            TEXT NOT NULL DEFAULT 'user'
                        CHECK (kind IN ('user', 'system')),
    system_task     TEXT,
    -- v1100: model binding (spec §8.2). A thread is permanently
    -- bound to the model it was created against — the Settings
    -- model picker only affects new threads. `bound_api_shape`
    -- belongs in the row so the sidecar can build SidecarInput
    -- without re-reading the Settings cache (`/v1/models` may be
    -- stale or offline).
    bound_model_id  TEXT NOT NULL,
    bound_api_shape TEXT NOT NULL CHECK (bound_api_shape IN ('anthropic', 'openai', 'openai_responses')),
    -- v1300: sidebar lifecycle.
    --   pinned_at   — when set, the thread renders under the
    --                 "置顶" section and ignores the recency sort.
    --   archived_at — when set, the thread is hidden from the
    --                 "最近" section and only shown after the
    --                 sidebar's "归档(N)" disclosure expands.
    -- Both NULL means the thread is a normal "recent" thread.
    -- They're independent — pinning then archiving keeps both
    -- timestamps; the UI treats archived as the dominant state.
    pinned_at       TEXT
        CHECK (pinned_at IS NULL OR pinned_at GLOB '????-??-??T??:??:??.???Z'),
    archived_at     TEXT
        CHECK (archived_at IS NULL OR archived_at GLOB '????-??-??T??:??:??.???Z'),
    -- v1400 (memory-system-spec §12.2): per-thread working memory.
    --   summary             — 150–300 字自然语言概要,session learner 输出
    --   summary_topics      — 空格分隔关键词(jieba 预分词,FTS5 用)
    --   summary_updated_at  — NULL = 还没生成
    -- `summary_topics` 走 FTS,thread_summaries_fts 虚表索引 summary +
    -- summary_topics,仅 kind='user' 入索引(见下方触发器)。
    summary               TEXT,
    summary_topics        TEXT,
    summary_updated_at    TEXT
        CHECK (summary_updated_at IS NULL OR summary_updated_at GLOB '????-??-??T??:??:??.???Z'),
    created_at      TEXT NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        CHECK (created_at GLOB '????-??-??T??:??:??.???Z'),
    updated_at      TEXT NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        CHECK (updated_at GLOB '????-??-??T??:??:??.???Z')
);

CREATE INDEX idx_chat_threads_updated ON chat_threads(updated_at DESC);
-- Sidebar list queries hit this — partial indexes keep them small.
CREATE INDEX idx_chat_threads_pinned ON chat_threads(pinned_at DESC) WHERE pinned_at IS NOT NULL;
CREATE INDEX idx_chat_threads_archived ON chat_threads(archived_at DESC) WHERE archived_at IS NOT NULL;
-- v1400 (memory-system-spec §11.2): the sidebar / diagnostic queries
-- both branch on `kind` first, then sort by created_at desc.
CREATE INDEX idx_chat_threads_kind_created ON chat_threads(kind, created_at DESC);

------------------------------------------------------------------------
-- thread_summaries_fts — FTS5 over chat_threads.summary +
-- summary_topics (memory-system-spec §12.2).
--
-- External-content over chat_threads; rowid = chat_threads.rowid.
-- Triggers only sync rows where kind='user' so the thread_search
-- native tool never surfaces background-task threads.
------------------------------------------------------------------------

CREATE VIRTUAL TABLE thread_summaries_fts USING fts5(
    summary,
    summary_topics,
    content='chat_threads',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER thread_summaries_fts_ai AFTER INSERT ON chat_threads
WHEN new.kind = 'user' AND new.summary IS NOT NULL BEGIN
    INSERT INTO thread_summaries_fts(rowid, summary, summary_topics)
    VALUES (new.rowid, new.summary, COALESCE(new.summary_topics, ''));
END;

-- Only delete from the index when the dropped row was actually
-- indexed (`old.summary IS NOT NULL`). Calling 'delete' on a rowid
-- that was never inserted corrupts the FTS5 index.
CREATE TRIGGER thread_summaries_fts_ad AFTER DELETE ON chat_threads
WHEN old.kind = 'user' AND old.summary IS NOT NULL BEGIN
    INSERT INTO thread_summaries_fts(thread_summaries_fts, rowid, summary, summary_topics)
    VALUES ('delete', old.rowid, old.summary, COALESCE(old.summary_topics, ''));
END;

-- The kind toggle (user ↔ system) is never expected in practice but
-- the trigger covers every kind transition to keep the index honest:
--   * old indexed, new not → delete
--   * old indexed, new indexed → delete + insert
--   * old not, new indexed → insert
--   * neither indexed → no-op
CREATE TRIGGER thread_summaries_fts_au_delete AFTER UPDATE ON chat_threads
WHEN old.kind = 'user' AND old.summary IS NOT NULL BEGIN
    INSERT INTO thread_summaries_fts(thread_summaries_fts, rowid, summary, summary_topics)
    VALUES ('delete', old.rowid, old.summary, COALESCE(old.summary_topics, ''));
END;

CREATE TRIGGER thread_summaries_fts_au_insert AFTER UPDATE ON chat_threads
WHEN new.kind = 'user' AND new.summary IS NOT NULL BEGIN
    INSERT INTO thread_summaries_fts(rowid, summary, summary_topics)
    VALUES (new.rowid, new.summary, COALESCE(new.summary_topics, ''));
END;

-- v1200: rich content-block model. One row per role per turn (user OR
-- assistant). `content_blocks` is a JSON array of typed blocks (text,
-- thinking, tool_use, tool_result, focus_context, frame_citation) — the
-- shape mirrors Anthropic's Messages API content blocks so re-feeding
-- history to the model is a verbatim pass.
--
-- Lifecycle (commands/chat.rs):
--   user send       → INSERT (role='user', status='complete')
--   assistant start → INSERT (role='assistant', status='streaming',
--                              content_blocks='[]')
--   stream finish   → UPDATE (status='complete'|'error'|'cancelled',
--                              content_blocks=<accumulated>, usage=...,
--                              finish_reason=..., updated_at=now)
--   boot cleanup    → UPDATE rows still 'streaming' to 'cancelled' so
--                     a crashed-mid-stream turn doesn't render as live.
--
-- `content_text` is the plain-text rollup (concat of every text +
-- thinking block) — used for FTS / sidebar preview / quick reads
-- without parsing the JSON. NOT NULL but may be empty string.
--
-- `cited_frame_ids` is duplicated from any FrameCitation block (and
-- the user-side frame anchor for Quick Ask) so "which threads cite
-- frame X" is a cheap index hit.
CREATE TABLE chat_messages (
    id              TEXT PRIMARY KEY,
    thread_id       TEXT NOT NULL REFERENCES chat_threads(id) ON DELETE CASCADE,
    -- v1400 (memory-system-spec §11.5): 'system' role added so a
    -- background agent task failure can drop an error record into the
    -- task's thread (without leaking into the user-visible chat).
    role            TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system')),
    content_blocks  TEXT NOT NULL,                        -- JSON array of ContentBlock
    content_text    TEXT NOT NULL DEFAULT '',             -- plain-text rollup for FTS / preview
    cited_frame_ids TEXT,                                 -- JSON array of frame ULIDs
    status          TEXT NOT NULL DEFAULT 'complete'
        CHECK (status IN ('complete', 'streaming', 'error', 'cancelled')),
    error_message   TEXT,                                 -- non-NULL when status='error'
    finish_reason   TEXT,                                 -- 'EndTurn' | 'ToolUse' | 'Truncated' | 'Cancelled' | 'Error'
    usage           TEXT,                                 -- JSON: { input_tokens, output_tokens, cache_read?, cache_write? }
    -- Audit: which model directory alias this turn actually used.
    -- User rows leave it NULL (the user didn't speak any model);
    -- assistant rows get written by exec_agent_send right after
    -- runtime resolution, before the stream starts. NULL on legacy
    -- rows from before v1301, and on assistant rows whose runtime
    -- resolution failed before write (rare; surfaces as plain
    -- "unknown model" in the UI).
    model_used      TEXT,
    -- v1410 Step 2 (memory-system-spec §7): jieba-tokenized mirror of
    -- content_text. Populated by the chat repo on insert + finalize so
    -- the recall layer can FTS-search across past messages.
    search_tokens   TEXT,
    created_at      TEXT NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        CHECK (created_at GLOB '????-??-??T??:??:??.???Z'),
    updated_at      TEXT
        CHECK (updated_at IS NULL OR updated_at GLOB '????-??-??T??:??:??.???Z')
);

CREATE INDEX idx_chat_messages_thread ON chat_messages(thread_id, created_at);
-- Boot-time orphan cleanup queries on this. Tiny table → tiny index.
CREATE INDEX idx_chat_messages_status ON chat_messages(status);

------------------------------------------------------------------------
-- chat_messages_fts — Step 2 jieba-tokenized FTS over the assistant /
-- user message text. Only `complete` rows are indexed by the repo;
-- streaming placeholders and error rows are filtered application-side.
------------------------------------------------------------------------

CREATE VIRTUAL TABLE chat_messages_fts USING fts5(
    search_tokens,
    content='chat_messages',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER chat_messages_fts_ai AFTER INSERT ON chat_messages
WHEN new.search_tokens IS NOT NULL BEGIN
    INSERT INTO chat_messages_fts(rowid, search_tokens)
    VALUES (new.rowid, new.search_tokens);
END;

CREATE TRIGGER chat_messages_fts_ad AFTER DELETE ON chat_messages
WHEN old.search_tokens IS NOT NULL BEGIN
    INSERT INTO chat_messages_fts(chat_messages_fts, rowid, search_tokens)
    VALUES ('delete', old.rowid, old.search_tokens);
END;

CREATE TRIGGER chat_messages_fts_au_delete AFTER UPDATE ON chat_messages
WHEN old.search_tokens IS NOT NULL BEGIN
    INSERT INTO chat_messages_fts(chat_messages_fts, rowid, search_tokens)
    VALUES ('delete', old.rowid, old.search_tokens);
END;

CREATE TRIGGER chat_messages_fts_au_insert AFTER UPDATE ON chat_messages
WHEN new.search_tokens IS NOT NULL BEGIN
    INSERT INTO chat_messages_fts(rowid, search_tokens)
    VALUES (new.rowid, new.search_tokens);
END;

------------------------------------------------------------------------
-- notes — promoted declarative memory (memory-system-spec §3).
--
-- The "记得说过的话" layer: things the user (or the session learner)
-- has explicitly asked Corivo to remember long-term. `scope='global'` +
-- `source_type='user_explicit'` + `status='active'` rows feed the
-- persistent prompt block every turn (see local_context.rs).
--
-- `scope_ref` is `project_id` for `scope='project'` and `thread_id`
-- for `scope='session'`. NULL for `scope='global'`.
--
-- Notes is the only memory layer with a status flag — agent_inferred
-- writes default to `status='suggested'` and don't reach the prompt
-- until the user confirms (Step 1a sets them as suggested; promotion UI
-- arrives with Step 3).
------------------------------------------------------------------------

CREATE TABLE notes (
    id                  TEXT    PRIMARY KEY,                -- ULID
    content             TEXT    NOT NULL,
    -- Step 2: jieba-tokenized mirror of `content` written by the Rust
    -- repo (NotesRepo) at INSERT / UPDATE time. Indexed by notes_fts
    -- using unicode61 so CJK queries find the right rows.
    search_tokens       TEXT,
    scope               TEXT    NOT NULL
                                CHECK (scope IN ('global','project','session')),
    scope_ref           TEXT,
    source_type         TEXT    NOT NULL
                                CHECK (source_type IN ('user_explicit','agent_inferred')),
    source_message_id   TEXT,
    source_thread_id    TEXT,
    confidence          REAL    NOT NULL DEFAULT 1.0,
    status              TEXT    NOT NULL DEFAULT 'active'
                                CHECK (status IN ('active','suggested','superseded','contradicted','archived')),
    superseded_by       TEXT REFERENCES notes(id) ON DELETE SET NULL,
    created_at          TEXT    NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        CHECK (created_at GLOB '????-??-??T??:??:??.???Z'),
    updated_at          TEXT    NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        CHECK (updated_at GLOB '????-??-??T??:??:??.???Z'),
    last_referenced_at  TEXT
        CHECK (last_referenced_at IS NULL OR last_referenced_at GLOB '????-??-??T??:??:??.???Z'),
    expires_at          TEXT
        CHECK (expires_at IS NULL OR expires_at GLOB '????-??-??T??:??:??.???Z')
);

CREATE INDEX idx_notes_scope_status ON notes(scope, status);
CREATE INDEX idx_notes_scope_ref ON notes(scope, scope_ref) WHERE scope_ref IS NOT NULL;
CREATE INDEX idx_notes_source_message ON notes(source_message_id) WHERE source_message_id IS NOT NULL;

------------------------------------------------------------------------
-- notes_fts — Step 2 jieba-tokenized FTS over notes.search_tokens
-- (memory-system-spec §7 / §3.1). Mirrors frames_fts: the Rust repo
-- writes `search_tokens` at INSERT / UPDATE time.
------------------------------------------------------------------------

CREATE VIRTUAL TABLE notes_fts USING fts5(
    search_tokens,
    content='notes',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER notes_fts_ai AFTER INSERT ON notes BEGIN
    INSERT INTO notes_fts(rowid, search_tokens)
    VALUES (new.rowid, COALESCE(new.search_tokens, ''));
END;

CREATE TRIGGER notes_fts_ad AFTER DELETE ON notes BEGIN
    INSERT INTO notes_fts(notes_fts, rowid, search_tokens)
    VALUES ('delete', old.rowid, COALESCE(old.search_tokens, ''));
END;

CREATE TRIGGER notes_fts_au AFTER UPDATE ON notes BEGIN
    INSERT INTO notes_fts(notes_fts, rowid, search_tokens)
    VALUES ('delete', old.rowid, COALESCE(old.search_tokens, ''));
    INSERT INTO notes_fts(rowid, search_tokens)
    VALUES (new.rowid, COALESCE(new.search_tokens, ''));
END;

------------------------------------------------------------------------
-- background_agent_task_checkpoints — per-task progress markers
-- (memory-system-spec §3.4.2 / §11.6).
--
-- Currently only the session learner writes here: (task='session_memory_learning',
-- target_id=<chat_threads.id>, last_processed_id=<chat_messages.id>) lets a
-- subsequent run pick up only the messages added since the last successful
-- consume_output. Persona distill uses a different cadence (daily, not
-- per-thread) and stores `last_run_at` separately on Config.
--
-- Designed loose enough for future tasks to share; (task, target_id) is
-- the natural key. When task is global (no target), target_id stays
-- empty string for the PK.
------------------------------------------------------------------------

CREATE TABLE background_agent_task_checkpoints (
    task                TEXT NOT NULL,
    target_id           TEXT NOT NULL DEFAULT '',
    last_processed_id   TEXT,
    last_run_at         TEXT NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        CHECK (last_run_at GLOB '????-??-??T??:??:??.???Z'),
    PRIMARY KEY (task, target_id)
);

------------------------------------------------------------------------
-- workflow_schedules — scheduled workflow runtime state (v1510).
--
-- Definitions live on the filesystem under
-- `$APPDATA/corivo/workflows/<slug>/WORKFLOW.md` (system prompt
-- template + tool whitelist in frontmatter). This table only tracks
-- the *when* + *what happened*:
--   trigger_kind / trigger_expr — see services::scheduled_workflows::trigger
--   next_run_at  — Ticker fires when this is <= now()
--   last_run_at  — most recent firing (NULL = never run)
--   last_status  — 'success' | 'failure' | NULL
--
-- One schedule per slug. v1 deliberately doesn't support "many
-- triggers per workflow" — keeps the Ticker scan trivially indexable
-- and matches the structured-picker UI.
------------------------------------------------------------------------

CREATE TABLE workflow_schedules (
    slug          TEXT PRIMARY KEY,
    trigger_kind  TEXT NOT NULL
        CHECK (trigger_kind IN ('interval', 'daily', 'weekly', 'once', 'cron')),
    trigger_expr  TEXT NOT NULL,                            -- JSON-tagged Trigger payload
    enabled       INTEGER NOT NULL DEFAULT 0
        CHECK (enabled IN (0, 1)),
    last_run_at   TEXT
        CHECK (last_run_at IS NULL OR last_run_at GLOB '????-??-??T??:??:??.???Z'),
    next_run_at   TEXT
        CHECK (next_run_at IS NULL OR next_run_at GLOB '????-??-??T??:??:??.???Z'),
    last_status   TEXT
        CHECK (last_status IS NULL OR last_status IN ('success', 'failure')),
    -- v1511: who originally created this schedule.
    --   'user'  — authored from the /workflows UI.
    --   'agent' — the corivo-agent called the `schedule_task` native
    --             tool mid-conversation. The UI surfaces an "由 Corivo
    --             自动创建" badge for these so the user can audit + prune.
    -- `source` is preserved across user edits (UPDATE never overwrites
    -- it) so the audit trail survives the user fine-tuning an agent-
    -- proposed schedule.
    source        TEXT NOT NULL DEFAULT 'user'
        CHECK (source IN ('user', 'agent')),
    -- v1511: when source='agent', the chat_thread the agent was driving
    -- when it issued `schedule_task`. ON DELETE SET NULL preserves the
    -- workflow row when the originating thread is deleted (the schedule
    -- itself is still meaningful).
    created_by_thread_id TEXT
        REFERENCES chat_threads(id) ON DELETE SET NULL,
    -- v1512: notification policy chosen by the workflow author (UI
    -- drawer / schedule_task tool parameter / WORKFLOW.md frontmatter).
    --   'always'    — fire macOS banner + in-app toast every successful run
    --   'on_change' — only when this run's content_hash differs from the
    --                 previous run for this slug (suppresses "same weekly
    --                 summary again" spam)
    --   'silent'    — never push; still write to workflow_runs and the
    --                 sidebar Corivo 提议 section
    notify_policy TEXT NOT NULL DEFAULT 'always'
        CHECK (notify_policy IN ('always', 'on_change', 'silent')),
    created_at    TEXT NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        CHECK (created_at GLOB '????-??-??T??:??:??.???Z'),
    updated_at    TEXT NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        CHECK (updated_at GLOB '????-??-??T??:??:??.???Z')
);

-- Ticker scans `WHERE enabled = 1 AND next_run_at <= now()` — partial
-- index keeps the hot path tiny when most schedules are paused or
-- already-fired one-shots (`next_run_at IS NULL`).
CREATE INDEX idx_workflow_schedules_due
    ON workflow_schedules(next_run_at)
    WHERE enabled = 1 AND next_run_at IS NOT NULL;

------------------------------------------------------------------------
-- workflow_runs — one row per dispatched run (v1510).
--
-- Written by `ScheduledWorkflowTask::consume_output`. `thread_id`
-- points at the chat_threads row the BackgroundAgentScheduler created
-- with kind='system' + system_task='scheduled_workflow' — the
-- workflow history sub-route renders that thread inline.
------------------------------------------------------------------------

CREATE TABLE workflow_runs (
    id              TEXT PRIMARY KEY,                       -- ULID
    -- No FK to workflow_schedules: a run-now click on a definition
    -- that has a WORKFLOW.md but no schedule row is a valid flow,
    -- and the previous FK ON DELETE CASCADE rejected every such
    -- insert as a FOREIGN KEY constraint failure. Definitions live
    -- on the filesystem (`$APPDATA/corivo/workflows/<slug>/`);
    -- workflow_runs is a flat audit log keyed by slug, and
    -- workflows_delete is responsible for clearing its own rows.
    slug            TEXT NOT NULL,
    thread_id       TEXT
        REFERENCES chat_threads(id) ON DELETE SET NULL,
    status          TEXT NOT NULL CHECK (status IN ('success', 'failure')),
    started_at      TEXT NOT NULL
        CHECK (started_at GLOB '????-??-??T??:??:??.???Z'),
    finished_at     TEXT NOT NULL
        CHECK (finished_at GLOB '????-??-??T??:??:??.???Z'),
    error_message   TEXT,
    -- v1512: notification payload + read tracking.
    --   summary       — short body text (~140 chars) used for macOS
    --                   banner / in-app toast / sidebar preview.
    --                   Truncated assistant final text on success;
    --                   error_message on failure.
    --   content_hash  — sha256 of the raw assistant output. Used by
    --                   notify_policy='on_change' to skip duplicate
    --                   pushes when this run produced the same content
    --                   as the previous run for this slug.
    --   acknowledged_at — when the user opened/read this run via the
    --                   sidebar "Corivo 提议" section or the history
    --                   dialog. NULL = unread (feeds the sidebar dot).
    summary         TEXT,
    content_hash    TEXT,
    acknowledged_at TEXT
        CHECK (acknowledged_at IS NULL OR acknowledged_at GLOB '????-??-??T??:??:??.???Z'),
    created_at      TEXT NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        CHECK (created_at GLOB '????-??-??T??:??:??.???Z')
);

-- Sidebar 's unread badge query: COUNT(*) WHERE acknowledged_at IS NULL.
-- Partial index keeps the lookup cheap once most rows have been read.
CREATE INDEX idx_workflow_runs_unread
    ON workflow_runs(slug, started_at DESC)
    WHERE acknowledged_at IS NULL;

CREATE INDEX idx_workflow_runs_slug
    ON workflow_runs(slug, started_at DESC);

------------------------------------------------------------------------
-- Schema-version sentinel.
--
-- Bump history (see migrations.rs):
--   100..104  → P/T/E architecture (gone)
--   200       → Task-centric architecture (gone)
--   300       → Frames-as-truth architecture (v3 spec)
--   400       → Phase 4 — adds frame_embeddings; jieba FTS deferred
--               to Phase 4.5
--   500       → Phase 5 — adds adapter_name / adapter_payload / trigger
--               columns to frames; extraction_strategy gains 'adapter';
--               adds entities table; drops goals + goal_evaluations
--               (被动 drift 评估 → Quick Ask).
--   600       → Phase 5.5 — adds frames.search_tokens (jieba-tokenized
--               text for CJK-friendly FTS5); frames_fts collapses to a
--               single-column index over search_tokens.
--   700       → 撤回 saved_clips —— 用户主动标注层不再保留。spec §五 同步删除。
--   800       → 事件驱动采集骨架（中间版本，已被 v900 取代）。
--   900       → 纯事件驱动采集 + AX trigger 集合最终态。
--               frames.trigger CHECK 收敛到 {focus_change,
--               focused_window_changed, title_changed, quick_ask,
--               manual}。Timer + SafetyNet 都撤了。v800 → v900
--               必须经过 purge-and-apply，因为 v800 的 schema.sql
--               在迭代过程中被多次原地修改，停在 v800 的 dev DB
--               实际带的还是早期 CHECK（含 timer / safety_net）。
--   1000      → 撤回 entities 表与 entity 抽取后台任务。recall 层
--               不再依赖 entity 精确召回，所有查询改走 frames + FTS。
--   1100      → Phase C §8.2: chat_threads 增加 bound_model_id /
--               bound_api_shape 两列。purge-and-apply 让旧 dev DB
--               里没有这两列的 chat_threads 行被丢弃。
--   1300      → chat_threads 增加 pinned_at / archived_at 两列(spec
--               §sidebar)。purge-and-apply 让旧 dev DB 的 chat_threads
--               行被丢弃。Sidebar 列表查询走分区索引,小尺寸 + 高
--               cardinality 的常态下走默认 updated_at 索引。
--   1200      → 重写 chat_messages 数据模型：
--               * content / tool_calls 两列 → 单一 content_blocks
--                 JSON（Anthropic content-block 形态：text / thinking
--                 / tool_use / tool_result / focus_context /
--                 frame_citation）。
--               * 新增 status / error_message / finish_reason / usage
--                 / updated_at —— 让 user 消息能在发送瞬间落库、
--                 assistant 行先以 'streaming' 占位再 finalize；
--                 流到一半崩 / 取消时数据不丢。
--               * role CHECK 收敛到 ('user', 'assistant')，工具调
--                 用现在以 ToolUse + ToolResult content block 出现
--                 在 assistant 行上，'tool' 角色废弃。
--               旧 chat_messages 行整体丢弃（pre-release 期数据可弃）。
--   1301      → chat_messages 增加 model_used 列(per-message model
--               switching 审计 —— 记录每条 assistant 消息实际命中的
--               directory alias)。purge-and-apply,旧聊天历史丢弃。
--   1400      → 新增 notes 基表(memory-system-spec §3,Step 1a)。
--               用户/agent 显式声明要长期记住的偏好,
--               scope='global' + source_type='user_explicit' +
--               status='active' 走 persistent prompt block。
--               purge-and-apply,新表无历史包袱。
--   1420      → memory-system-spec Step 2:
--               * notes 加 search_tokens 列 + notes_fts 虚表。
--               * chat_messages 加 search_tokens 列 + chat_messages_fts
--                 虚表(仅 status='complete' 入索引)。
--               * 统一召回层 services::memory 上线。
--   1513      → workflow_runs.slug 不再 FK 到 workflow_schedules。
--               run-now 一个没排时间的 workflow(WORKFLOW.md 在磁盘上、
--               schedules 表里没行)是合法路径,旧的 FK ON DELETE CASCADE
--               把每次 record_run 都拒成 FOREIGN KEY constraint failed,
--               UI 永远卡在「正在运行...」。改由 delete_schedule 显式
--               清理 workflow_runs 行(参见 store.rs::delete_schedule)。
--   1512      → workflow_schedules 增 notify_policy; workflow_runs
--               增 summary / content_hash / acknowledged_at 三列。
--               支撑 PR6：macOS banner + in-app toast + sidebar 「Corivo 提议」
--               未读分区。notify_policy ∈ ('always','on_change','silent')
--               让 workflow 作者控制噪声;on_change 用 content_hash 比上
--               一次同 slug 的输出来去重。
--   1511      → workflow_schedules 增 source + created_by_thread_id 两列。
--               source ∈ ('user','agent') 区分定时任务是用户在 /workflows
--               UI 手写的,还是 agent 通过 `schedule_task` native tool
--               自创建的(后者 UI 上贴 "由 Corivo 自动创建" badge)。
--   1510      → 新增 workflow_schedules + workflow_runs 两张表
--               (services::scheduled_workflows). 定义文件仍在
--               $APPDATA/corivo/workflows/<slug>/WORKFLOW.md;表只承
--               载触发器、enabled、next_run_at、last_run_at、运行历史。
--               chat_threads.system_task CHECK 不再硬约束枚举值,
--               所以新增 'scheduled_workflow' kind 无需再 bump
--               schema(SystemTaskKind 是开放枚举,见 domain::chat)。
--   1500      → privacy-filter-spec Phase 1 地基：frames 增加
--               `ax_text_pii_spans` (TEXT, JSON) 列，记录 OpenAI
--               privacy-filter 模型识别出的 PII span。purge-and-apply
--               把旧 frames 表丢弃；frames 不在 preserved 行列里，
--               历史 ax_text 也跟着丢，但 pre-release 期可弃。
--               docs/privacy-filter-spec.md §5.1。
--   1410      → memory-system-spec Step 1b:
--               * chat_threads 增 kind/system_task —— 后台 agent
--                 任务和用户对话共表,kind='system' 永不入侧栏。
--               * chat_threads 增 summary/summary_topics/
--                 summary_updated_at —— session learner 副产物,
--                 thread_summaries_fts 给 thread_search 工具用。
--               * chat_messages.role CHECK 扩到 ('user',
--                 'assistant','system') —— 后台任务失败落
--                 role='system' 错误记录。
--               * 新增 background_agent_task_checkpoints 表 —— session
--                 learner 记录已学到哪一条 message,避免重复学习。
--               purge-and-apply,旧聊天历史丢弃。
--   1600      → privacy-filter 架构调整:撤掉 v1500 加的
--               `frames.ax_text_pii_spans` 列。隐私过滤从
--               classify-at-capture 改成 classify-at-egress ——
--               PII spans 不落盘,每次出口现算现用,exec_agent.rs
--               的 Hook B 自己负责 classify + redact。purge-and-apply
--               让历史 frames 丢弃,新表无 ax_text_pii_spans 列。
------------------------------------------------------------------------

CREATE TABLE schema_version (
    version     INTEGER PRIMARY KEY,
    applied_at  TEXT NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        CHECK (applied_at GLOB '????-??-??T??:??:??.???Z')
);

INSERT INTO schema_version (version) VALUES (1600);
