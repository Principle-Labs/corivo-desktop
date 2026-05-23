# Corivo × GUM 重构实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Spec source of truth:** `docs/gum-refactor-spec.md`（commit 3eadac9）。所有章节引用（§x.y）都指向该 spec。

**Goal:** 把 Corivo 的核心数据/服务层从"Supermemory + attention_items + mvp_pipeline 一把梭"迁到 GUM 五层架构（Observer → Observation → Propose → Retrieve → Revise）。一次破坏性升级，旧数据全部丢弃、Supermemory 整体退役、attention item 概念移除。

**Architecture:**
- Rust 后端拆成 `services/capture/`（保留）+ `services/observation_ingest/`（新）+ `services/user_model/`（新：batcher、proposition_pipeline、retrieval、Observer trait）+ `services/push_decider.rs`（新）+ `services/notification_service`（保留，入参改）。
- 数据库在 v5 migration 里同事务完成：建 `observations` / `propositions` / `observation_proposition` / `notification_log` 四表 + DROP 旧 attention / facts / conflict / decision_log 表 + `ALTER TABLE segments DROP COLUMN memory_id` / `sessions` 的两个 supermemory_* 列。
- 前端 IPC 同次重写，`lib/tauri.ts` 只留 `user_model_*` 与 `notification_log_*`。
- 第一版 retrieval 走 `LIKE` + `confidence × decay × recency` 加权，不建 FTS5（v6 预留）。

**Tech Stack:** Tauri 2 + Rust（rusqlite + r2d2 + tokio + tracing + thiserror + async-trait + insta）、React 19 + TanStack Router + TanStack Query + Zustand + Vitest、pnpm workspace。

---

## 0. PR 地图

本计划拆成 7 个 self-contained、可独立 merge 的 PR。每个 PR 在自己 §末尾列出本次必过的 §13.8 DoD 子集；第 7 个 PR 是最终门（全部 10 项齐）。

| # | 标题 | 核心产出 | 依赖 |
|---|------|---------|------|
| 1 | Foundation: v5 schema + Supermemory/attention 整体拆除 | v5 migration SQL、删 `mvp_pipeline` / `memory_service` / `attention_items` 全家、清空相关 IPC，`capture_loop` 被"哑化"为仅写截图 | 无（起点） |
| 2 | `observation_ingest`：Vision LLM 转录 + 时间线摘要 → `observations` 表 | `services/observation_ingest/{mod,transcriber,summarizer,writer}.rs`、`db/repos/observations.rs`、`ObservationIngestError` | PR1 |
| 3 | `user_model` 数据层 + Observer + Batcher + Retrieval v1 | `services/user_model/{mod,observer,screen_observer,batcher,retrieval}.rs`、`db/repos/{propositions,observation_proposition}.rs`、`UserModelError` | PR2 |
| 4 | `proposition_pipeline`：PROPOSE / SIMILAR / REVISE 四种 RevisionOp | `services/user_model/proposition_pipeline.rs`、`src-tauri/prompts/*.md`、`PipelineError`、insta snapshots | PR3 |
| 5 | 读路径 Tauri 命令 + 前端 Propositions/Knowledge 页 + 版本链 UI | `commands/user_model.rs`、`lib/tauri.ts` user_model wrapper、`hooks/use-propositions*`、`pages/knowledge/*` | PR4 |
| 6 | `push_decider` + `notification_log` + Overlay 切换到命题 | `services/push_decider.rs`、`db/repos/notification_log.rs`、`commands/notification_log.rs`、overlay payload 换 | PR5 |
| 7 | 工程化收尾 + §13.8 DoD 终验 | tracing 审查、`Config::validate`、final cargo machete / knip / clippy -D warnings、`rg` 零命中复核 | PR6 |

**硬约束（每个 PR 都要遵守）**：
- 魔法数字一律进 `domain::config::Config`，模块内禁 `const MAGIC: u32 = X;`。
- 新代码禁 `println!` / `eprintln!`，走 `tracing::{info,warn,error,debug}`。
- 不写兼容层 / shim / deprecation；被删对象同 PR 内连带删除其使用者。
- §13 的工程化改动只动"本次 PR 反正要打开的文件"，不顺手改无关模块。

---

## PR 1 — Foundation: v5 Schema + Supermemory / Attention 全量拆除

### 1.1 Scope

- **Spec 引用**：§3.5（退役 Supermemory + attention 家族）、§4（IPC 全量重构，不留兼容）、§6（v5 schema 新表 + drop 清单）、§7.0（`mvp_pipeline.rs` 整体删除）、§10 阶段 1、§13.6 死代码清零、§13.7 migration 落到 `.sql` 文件（v5 起）。
- **目标**：PR 结束时应用能冷启动到 `schema_version = 5`、`capture_loop` 仍按原逻辑采集截图进入 `screenshots` 表、**没有下游** pipeline（屏幕采集到数据库为止）；前端能进入主窗口但 `/memory` 路由被整页替换为"Knowledge placeholder"空态；所有 `rg` 命中检查（§13.8）过。

### 1.2 文件一览（Create / Modify / Delete）

**Create**
- `src-tauri/src/db/migrations/` 目录 + `005_gum_user_model.sql`（本次所有 DDL）。
- `src-tauri/src/db/migrations/mod.rs`（若 migrations.rs 需要拆分；否则在 `migrations.rs` 里 `include_str!` 引用）。

**Modify**
- `src-tauri/src/db/migrations.rs`：`LATEST_VERSION = 5`；新增 `apply_v5(conn)` 读取 `005_gum_user_model.sql` 并按单事务执行。
- `src-tauri/src/db/schema.sql`：补一行 `INSERT OR IGNORE INTO schema_version (version) VALUES (5);` 仅用于全新安装同步记录；主 DDL 仍走 v5 .sql 文件（避免重复）。
- `src-tauri/src/services/mod.rs`：删除 `mvp_pipeline` / `memory_service` / `notification_decision_log` 模块声明。
- `src-tauri/src/services/capture_loop.rs`：把对 `mvp_pipeline` 的调用点改成空实现（只记录 tracing + 存截图），保留 shape 以便 PR2 切入。
- `src-tauri/src/services/idle_shortcut.rs`：**删除** `assess_idle_batch` 及所有 attention judgment 逻辑；shortcut 仅保留"主动触发一次 capture"的接口（§10 阶段 1）。
- `src-tauri/src/commands/mod.rs`：删 `memory` 模块导出；`notification` 模块里仅保留 overlay 通道收发所需命令（不涉及 attention）。
- `src-tauri/src/commands/notification.rs`：删 attention item 相关命令；保留 overlay 生命周期命令（`open_overlay` / `close_overlay` 之类），入参改成占位（payload 在 PR6 重新定义）。
- `src-tauri/src/commands/config.rs`：删掉对 supermemory API key 读写的所有入口。
- `src-tauri/src/lib.rs`：`generate_handler!` 宏重写——删掉全部旧命令；本 PR 只保留 `capture` / `config` / `llm` / `onboarding` / `settings` / `db_debug` 的**非 Supermemory** 子集。
- `src-tauri/src/domain/config.rs`：
  - 删 `supermemory` 相关字段（`SupermemoryConfig` struct / `supermemory_api_key` slot）。
  - 新增空 `UserModelConfig { batcher: BatcherConfig, retrieval: RetrievalConfig, pipeline: PipelineConfig, push: PushConfig }` struct，字段全部 `#[derive(Default)]`，数值默认符合 spec §7.5 / §13.4（见 1.6）。本 PR 只挂空骨架，后续 PR 填实。
- `src-tauri/src/error.rs`：保留 `CorivoError` 主型；新增 `ErrorKind` enum 的占位 variant（`Pipeline` / `UserModel` / `Push`）供后续 PR 填充——本 PR 不写实现，仅预留 variant 名。
- `src-tauri/src/db/repos/mod.rs`：删 `attention_items` 导出。
- `src-tauri/Cargo.toml`：删 `supermemory-rs`（若直接依赖）/ 删 `reqwest` 的 Supermemory 专用 feature（如有）；新增 `thiserror`（若未在 deps）、`async-trait`、`insta`（dev-dependencies）。确认 `tracing` + `tracing-subscriber` 已在。
- `src/lib/tauri.ts`：**整体重写**——删除所有 supermemory / attention / memory / fact / conflict_alerts 相关 wrapper，只保留 `capture*` / `config*` / `llm_probe*` / `onboarding*` / `settings*` 的非 Supermemory 子集 + 本 PR 新加的空 `user_model`（如果本 PR 尚未引入则不加）。
- `src/lib/types.ts`：删 `Memory` / `AttentionItem` / `ExtractedFact` / `ConflictAlert` / 任何含 `canonical_key` 的 interface；删对应 enum / union。
- `src/app/router.tsx`（或路由文件）：把 `/memory` 路由组件替换成占位空态 `<Knowledge placeholder />`，`/connections/screenshot` 路由保留但 hook 换掉（或占位）。
- `src/routes/memory.tsx`：替换为占位组件，显示"Knowledge page coming in PR5"（仅开发期，临时文案，禁 emoji）。
- `src/routes/connections.index.tsx`：删除 Supermemory 连接卡片；保留 API keys 卡片但移除 Supermemory slot（UI 只显示 Gemini/Codex，§已完成）。

**Delete（整文件）**
- `src-tauri/src/services/mvp_pipeline.rs`
- `src-tauri/src/services/memory_service.rs`
- `src-tauri/src/services/notification_decision_log.rs`
- `src-tauri/src/providers/memory/`（整个目录）
- `src-tauri/src/commands/memory.rs`
- `src-tauri/src/db/repos/attention_items.rs`
- `src/hooks/use-memories.ts` + `src/hooks/use-memories.test.ts`
- `src/hooks/use-memory-list.ts`
- `src/pages/memory/`（整个目录）
- 任何 `src/pages/connections/` 下的 Supermemory 连接视图。

### 1.3 v5 Migration SQL（`005_gum_user_model.sql`）

把 spec §6 的 v5 DDL **原封不动**写入 `src-tauri/src/db/migrations/005_gum_user_model.sql`。该文件内容如下（完整、可粘贴）：

```sql
-- v5: GUM user model
-- Creates observations / propositions / observation_proposition / notification_log.
-- Drops all Supermemory + attention-item legacy tables and columns.
-- Runs in one transaction in apply_v5().

BEGIN;

CREATE TABLE observations (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  observer_name TEXT NOT NULL,
  content TEXT NOT NULL,
  content_type TEXT NOT NULL,
  source_session_id TEXT,
  source_screenshot_id INTEGER,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_observations_created_at ON observations(created_at DESC);
CREATE INDEX idx_observations_source_screenshot ON observations(source_screenshot_id);

CREATE TABLE propositions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  text TEXT NOT NULL,
  reasoning TEXT NOT NULL,
  confidence INTEGER,
  decay INTEGER,
  revision_group TEXT NOT NULL,
  version INTEGER NOT NULL DEFAULT 1,
  contradicts_proposition_id INTEGER,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  FOREIGN KEY (contradicts_proposition_id) REFERENCES propositions(id) ON DELETE SET NULL
);
CREATE INDEX idx_propositions_revision_group ON propositions(revision_group);
CREATE INDEX idx_propositions_revision_version ON propositions(revision_group, version DESC);
CREATE INDEX idx_propositions_created_at ON propositions(created_at DESC);
CREATE INDEX idx_propositions_confidence ON propositions(confidence DESC);

CREATE TABLE observation_proposition (
  observation_id INTEGER NOT NULL,
  proposition_id INTEGER NOT NULL,
  PRIMARY KEY (observation_id, proposition_id),
  FOREIGN KEY (observation_id) REFERENCES observations(id) ON DELETE CASCADE,
  FOREIGN KEY (proposition_id) REFERENCES propositions(id) ON DELETE CASCADE
);
CREATE INDEX idx_obs_prop_proposition ON observation_proposition(proposition_id);

CREATE TABLE notification_log (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  revision_group  TEXT    NOT NULL,
  proposition_id  INTEGER,
  pushed_at       TEXT    NOT NULL DEFAULT (datetime('now')),
  outcome         TEXT,
  outcome_at      TEXT,
  FOREIGN KEY (proposition_id) REFERENCES propositions(id) ON DELETE SET NULL
);
CREATE INDEX idx_notif_group_time ON notification_log(revision_group, pushed_at DESC);

-- Drop legacy tables (旧数据直接丢弃)
DROP TABLE IF EXISTS attention_item_events;
DROP TABLE IF EXISTS attention_item_evidence;
DROP TABLE IF EXISTS attention_items;
DROP TABLE IF EXISTS notification_decision_log;
DROP TABLE IF EXISTS extracted_facts;
DROP TABLE IF EXISTS conflict_alerts;

-- Drop legacy columns (SQLite ≥ 3.35)
ALTER TABLE segments DROP COLUMN memory_id;
ALTER TABLE sessions DROP COLUMN supermemory_document_id;
ALTER TABLE sessions DROP COLUMN supermemory_custom_id;

INSERT OR IGNORE INTO schema_version (version) VALUES (5);

COMMIT;
```

说明：
- `BEGIN; ... COMMIT;` 外包一层事务 → `conn.execute_batch(sql)` 会把整段当一个事务执行；如果任何一句失败，全部回滚（SQLite 默认行为）。也可以在 Rust 侧显式 `conn.transaction()`，二选一（见 1.4 step 4）。
- `attention_item_events` 在 spec drop 清单里没列，但它是现存表（见代码勘察），必须一并 drop 才能消灭残留。
- `sessions.supermemory_document_id` / `supermemory_custom_id` 两列同样要 drop（现 v2 migration 加的），否则会残留字段并留下类型 dead code。
- 不建 FTS5（§4 / §6 / §10 阶段 1 明确）。

### 1.4 Task 分解

**Task 1.1 — 枚举旧代码接触面**
- [ ] **Step 1**：在 repo 根跑 `rg -n "supermemory|attention_items|attention_item_evidence|attention_item_events|extracted_facts|conflict_alerts|canonical_key|memory_service|mvp_pipeline|MvpPipeline|notification_decision_log|SupermemoryConfig|supermemory_api_key"` src/ src-tauri/src/ src-tauri/tests/ src-tauri/Cargo.toml package.json pnpm-lock.yaml`。把结果保存到 `/tmp/gum-demolition-hits.txt`（plan 本身不持久化）。这是 PR 最终要让"零命中"的基线。
- [ ] **Step 2**：对照结果列出每个文件要做的动作（删 / 改）。不改代码，仅核对。

**Task 1.2 — 新增 v5 migration 文件**
- [ ] **Step 3**：`mkdir -p src-tauri/src/db/migrations` 并创建 `005_gum_user_model.sql`，内容完全采用 §1.3 的 SQL 块。
- [ ] **Step 4**：`src-tauri/src/db/migrations.rs` 增加 `apply_v5`：
  ```rust
  const SCHEMA_V5: &str = include_str!("migrations/005_gum_user_model.sql");

  fn apply_v5(conn: &DbConnection) -> Result<()> {
      conn.execute_batch(SCHEMA_V5).map_err(|e| {
          CorivoError::Internal(format!("V5 migration failed: {e}"))
      })?;
      tracing::info!(schema_version = 5, "applied v5 migration");
      Ok(())
  }
  ```
- [ ] **Step 5**：把 `LATEST_VERSION` 改成 `5`，并在 `apply_migrations` 里追加：
  ```rust
  let current_version = get_current_version(conn)?;
  if current_version < 5 {
      apply_v5(conn)?;
  }
  ```
- [ ] **Step 6**：写 migration smoke test `src-tauri/tests/migrations_v5.rs`：
  ```rust
  use corivo::db::{migrations::apply_migrations, pool::test_in_memory_pool};

  #[test]
  fn v5_creates_new_tables_and_drops_legacy() {
      let pool = test_in_memory_pool().expect("pool");
      let conn = pool.get().unwrap();
      apply_migrations(&conn).expect("migrations apply");

      // schema_version
      let v: i64 = conn
          .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
          .unwrap();
      assert_eq!(v, 5);

      // new tables exist
      for t in ["observations", "propositions", "observation_proposition", "notification_log"] {
          let count: i64 = conn
              .query_row(
                  "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                  [t],
                  |r| r.get(0),
              )
              .unwrap();
          assert_eq!(count, 1, "table {t} should exist");
      }

      // legacy tables gone
      for t in [
          "attention_items",
          "attention_item_evidence",
          "attention_item_events",
          "notification_decision_log",
          "extracted_facts",
          "conflict_alerts",
      ] {
          let count: i64 = conn
              .query_row(
                  "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                  [t],
                  |r| r.get(0),
              )
              .unwrap();
          assert_eq!(count, 0, "table {t} must be dropped");
      }

      // legacy columns gone
      let cols = |t: &str| -> Vec<String> {
          let mut stmt = conn
              .prepare(&format!("PRAGMA table_info({t})"))
              .unwrap();
          stmt.query_map([], |r| r.get::<_, String>(1))
              .unwrap()
              .map(|x| x.unwrap())
              .collect()
      };
      let seg_cols = cols("segments");
      assert!(!seg_cols.iter().any(|c| c == "memory_id"), "segments.memory_id must be dropped");
      let sess_cols = cols("sessions");
      assert!(!sess_cols.iter().any(|c| c == "supermemory_document_id"));
      assert!(!sess_cols.iter().any(|c| c == "supermemory_custom_id"));
  }
  ```
  注意：`test_in_memory_pool` 若不存在，先在 `db/pool.rs` 里 `#[cfg(test)]` 加一个工厂函数（共享内存 DB，打开一次连接，跟单元测试复用）。
- [ ] **Step 7**：`cd src-tauri && cargo test --test migrations_v5 -- --exact v5_creates_new_tables_and_drops_legacy`。预期：红（现有旧代码里 `attention_items` 模块还在 import，编译先失败）。这是期望的，因为我们要先扫干净旧代码，下面逐步 unblock。

**Task 1.3 — 拆除 Rust 侧旧模块**（每个子步都以"先删 + 让编译器替我们找剩下的使用点"为策略）
- [ ] **Step 8**：`git rm src-tauri/src/services/mvp_pipeline.rs src-tauri/src/services/memory_service.rs src-tauri/src/services/notification_decision_log.rs`；`git rm -r src-tauri/src/providers/memory`；`git rm src-tauri/src/commands/memory.rs src-tauri/src/db/repos/attention_items.rs`。
- [ ] **Step 9**：打开 `src-tauri/src/services/mod.rs`、`src-tauri/src/commands/mod.rs`、`src-tauri/src/providers/mod.rs`、`src-tauri/src/db/repos/mod.rs`，把删掉的 `pub mod xxx;` 行清掉。
- [ ] **Step 10**：`cargo check`，按报错顺序修 `lib.rs` 的 `generate_handler!` 宏（删调用 `memory::*` / `notification::assess_attention` 等旧命令），再修 `AppState` / `setup(app)` 里构建被删服务的片段。
- [ ] **Step 11**：`services/capture_loop.rs` 原本 `drop_batch → mvp_pipeline::run` 的调用处改成：
  ```rust
  tracing::info!(
      batch_size = batch.len(),
      "capture batch completed; no downstream consumer until PR2"
  );
  ```
  保留 `capture_loop` 对 `screenshots` 表的写入；截图文件仍落盘。
- [ ] **Step 12**：`services/idle_shortcut.rs` 整个文件删除（实施偏差）。原计划只删 `assess_idle_batch` + `attention_*` 辅助、保留 "idle 触发一次 capture" trigger 闭包；但该文件实际全部是 attention judgment 逻辑，idle 状态机已经完整地落在 `capture_loop::CaptureRuntimeState` 里（`capture_loop.rs` 的 `observe_frame_idle` / `spawn_idle_watcher`，PR1 不碰）。删除整个 `idle_shortcut.rs` 后重建"shortcut 触发一次 capture"不是 PR1 要解决的事；若未来真需要，在对应 PR 新开一个干净的 `shortcut_capture` 模块即可。`capture_loop` 仍保留 idle 观察/暂停/恢复能力。
- [ ] **Step 13**：`domain/config.rs`：
  - 删 `pub supermemory: SupermemoryConfig`、`SupermemoryConfig` struct、`supermemory_api_key` 相关 keychain slot 引用；
  - 新增（仅骨架、字段全默认）：
    ```rust
    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    #[serde(default)]
    pub struct UserModelConfig {
        pub batcher: BatcherConfig,
        pub retrieval: RetrievalConfig,
        pub pipeline: PipelineConfig,
        pub push: PushConfig,
    }
    // 下面四个 struct 各自 #[derive(Default)]，字段 spec §13.4 的 key 全部放进来，
    // 默认值按 §7.5 / §7.3：
    //   BatcherConfig { min_batch_size: 5, flush_interval_ms: 30_000 }
    //   RetrievalConfig { w_confidence: 1.0, w_decay: 1.0, k_decay_days: 14.0, limit_multiplier: 3 }
    //   PipelineConfig { similar_pool_size: 20 }
    //   PushConfig { base_cooldown_secs: 24 * 3600, dismiss_penalty_secs: 7 * 24 * 3600, min_confidence: 6 }
    ```
    每个 struct 加 `Serialize + Deserialize + Default`；字段取 `u32` / `f32` 等具体数值类型，不用 `Duration`（前端 JSON 交互方便），Rust 侧需要时再包一层。
  - 在 `Config` struct 里加 `#[serde(default)] pub user_model: UserModelConfig`。
- [ ] **Step 14**：`cargo check` 直到绿；`cargo fmt`。
- [ ] **Step 15**：再跑 `cargo test --test migrations_v5`；现在应 PASS。

**Task 1.4 — 拆除前端侧旧模块**
- [ ] **Step 16**：`git rm src/hooks/use-memories.ts src/hooks/use-memories.test.ts src/hooks/use-memory-list.ts`；`git rm -r src/pages/memory`。
- [ ] **Step 17**：在 `src/routes/memory.tsx` 里写一个占位组件：
  ```tsx
  import { createFileRoute } from "@tanstack/react-router";

  export const Route = createFileRoute("/memory")({
    component: KnowledgePlaceholder,
  });

  function KnowledgePlaceholder() {
    return (
      <div className="p-8 text-muted-foreground">
        知识库重构中（见 docs/gum-refactor-spec.md），下一个 PR 开始接入命题视图。
      </div>
    );
  }
  ```
- [ ] **Step 18**：`src/lib/tauri.ts`：删掉所有 `invoke("supermemory_*", ...)` / `invoke("memory_*", ...)` / `invoke("attention_*", ...)` / `invoke("extract_facts", ...)` / 所有 fact、conflict_alert、canonical_key 相关 wrapper。本 PR 结束后 `tauri.ts` 里只剩 capture / config / llm_probe / onboarding / settings 相关 wrapper。
- [ ] **Step 19**：`src/lib/types.ts`：删掉 `Memory` / `AttentionItem` / `AttentionImportance` / `ExtractedFact` / `ConflictAlert` / `CanonicalKey` 等 interface/enum；以及 `Config` 里对 `SupermemoryConfig` 的 mirror type。保留 `Session` / `Segment` / `Screenshot` / `CaptureConfig` / `LlmConfig`（其中 Segment 要去掉 `memory_id` 字段）。
- [ ] **Step 20**：`src/routes/connections.index.tsx` 与 `src/pages/connections/*`：把 Supermemory 连接卡片/流程整体删掉；API key 设置页仅保留 Gemini / Codex slot（§与之前 PR 一致）。
- [ ] **Step 21**：`src/routes/connections.screenshot.tsx`：若该页里有依赖已删 hook/type 的地方，改成从 `Session`/`Screenshot` 单独拉；保持页面能渲染。
- [ ] **Step 22**：`pnpm tsc --noEmit`。按报错顺序清理每一处 import。期待一次跑完零 error。
- [ ] **Step 23**：`pnpm test`。失败/不再适用的测试直接删文件；保留覆盖率不依赖被删对象的测试。

**Task 1.5 — 依赖清理 + 验证**
- [ ] **Step 24**：`cd src-tauri && cargo machete`。若报 `supermemory-rs` / `reqwest` 的 Supermemory 专用 feature / 任何只被 `memory_service.rs` / `providers/memory` 用的包，按提示删 `Cargo.toml` 里对应条目，`cargo update -p <crate> --precise ...` 不需要——直接 `cargo build` 重跑。`cargo machete` 最终输出 0 unused。
- [ ] **Step 25**：`pnpm knip`（若未配置，先在 `package.json` 加 `"knip": "^5"` devDep 并加一个 `knip.json` 最小配置：`{"entry": ["src/main.tsx", "src/overlay/main.tsx", "vite.config.ts"], "project": ["src/**/*.{ts,tsx}"]}`）。期望：0 unused files / exports。
- [ ] **Step 26**：repo 根跑
  ```bash
  rg -n "supermemory|attention_items|attention_item_evidence|attention_item_events|extracted_facts|conflict_alerts|canonical_key|memory_service|mvp_pipeline|MvpPipeline|SupermemoryConfig|notification_decision_log" src/ src-tauri/src/
  ```
  期待 **0 命中**（spec 文档、plan 文档不在路径内）。若有命中，逐一清理。
- [ ] **Step 27**：`cd src-tauri && cargo clippy --all-targets -- -D warnings`。零 warning。
- [ ] **Step 28**：`pnpm tauri dev` 冷启动一次，在 app 里触发一次 capture → 只需确认：没有崩溃、`screenshots` 表有新 row、没有任何 IPC 调用旧命令报错。关掉。

**Task 1.6 — Commit 边界**
- [ ] **Step 29**：单个 commit（便于日后 revert 整个 demolition）：
  ```
  feat!(db,services,ui): v5 schema + Supermemory/attention demolition

  - add src-tauri/src/db/migrations/005_gum_user_model.sql (GUM tables + drops)
  - delete services/mvp_pipeline + memory_service + notification_decision_log
  - delete providers/memory + commands/memory + db/repos/attention_items
  - delete frontend memory/attention hooks and pages; /memory route becomes placeholder
  - ALTER segments DROP memory_id; sessions DROP supermemory_document_id/custom_id
  - Config.supermemory removed; empty UserModelConfig skeleton added

  Refs docs/gum-refactor-spec.md §3, §4, §6, §7.0, §10, §13.6.
  ```
  `git add -A` 前先逐一 `git status` 确认没有误删。

### 1.5 本 PR DoD 子集（§13.8）

- [x] `cargo test`（含 `migrations_v5` 新测试）全绿 — T3 sweep 跑出 12 个 binary / 约 89 passed, 2 ignored, 0 failed；`migrations_v5::v5_creates_new_tables_and_drops_legacy` PASS。
- [x] `cargo clippy --all-targets -- -D warnings` 零 warning — T3 sweep 确认 touch 后重编译仍无 warning。
- [x] `cargo machete` 零未用依赖 — T3 sweep 输出 "didn't find any unused dependencies"。
- [x] `pnpm build`（tsc）零报错 — T3 sweep 3322 modules transformed, 无 error。
- [x] `pnpm test` 全绿 — T3 sweep 17 files / 25 tests passed。
- [ ] `pnpm knip` 零 unused — **本 PR 不达标**：knip 报告 4 unused files、7 unused deps、23 unused exports、13 unused types（均为预先就存在的 dead code，不是本 PR 引入）。本 PR 已（a）补齐 `knip` devDep + `knip.json` 基线配置，（b）在 T3 sweep 把 `knip.json` 的 redundant entry hint 清零。真正的 dead-code 修整延到 PR7 engineering hygiene。
- [x] `rg` 关键字扫描零命中（见 Step 26）。**范围例外**：`src-tauri/src/db/schema.sql`、`src-tauri/src/db/migrations.rs` 以及 `src-tauri/src/db/migrations/*.sql`（legacy v1–v4 migration 字符串按 §13.7 要求保持原样不回改；新的 v5 migration 必须**提到**那些旧表才能 DROP 它们）。rg 扫描范围实际是 `src/ src-tauri/src/ src-tauri/tests/` 但**排除** `src-tauri/src/db/schema.sql`、`src-tauri/src/db/migrations.rs`、`src-tauri/src/db/migrations/`。T3 sweep 确认所有命中都落在这三个例外路径里。
- [x] 冷启动后 `schema_version = 5` — 以 `tests/migrations_v5.rs` 作为自动化替身（断言 `MAX(version)=5` + legacy 表/列全部消失）。**Step 28（`pnpm tauri dev` 真·冷启动 + 手动触发 capture 验证）属于人工手动验收项**，headless agent 环境无法启动 GUI，请在 merge 前由人手完成一次。
- [ ] **本 PR 不 tick**：IPC 表面与 §7.4 一致（PR5 / PR6 做）
- [x] 新写的模块无 `println!` / `eprintln!`（本 PR 主要是删除 + 小改，新代码就是 migration 单测）— T3 sweep `git diff eb612bd..HEAD` 在 `src/` / `src-tauri/src/` 下新增行 0 `println!` / `eprintln!`。

---

## PR 2 — `services/observation_ingest`（Vision LLM → observations 表）

### 2.1 Scope

- **Spec 引用**：§5（观察层产物入 `observations`）、§7.0 `observation_ingest` 子目录、§8（TRANSCRIPTION / SUMMARY prompt 中文化）、§13.1（thiserror 有名错误 `ObservationIngestError`）、§13.5（prompt 独立 `.md` 文件 + insta snapshot）。
- **目标**：`capture_loop` 的每一次"批完成"事件都会被 `observation_ingest` 订阅 → LLM 转录 + 摘要 → 写入 `observations`（关联 `source_session_id` / `source_screenshot_id`）→ emit `ObservationReady { id }` tracing 事件（PR3 才会真订阅）。下游仍无 consumer，`observations` 表只进不出。

### 2.2 文件一览

**Create**
- `src-tauri/src/services/observation_ingest/mod.rs`：对外的 `ObservationIngestService`，订阅 capture 事件，内部调度 transcriber + summarizer + writer，emit tracing 事件 `observation.ingested`。
- `src-tauri/src/services/observation_ingest/transcriber.rs`：Vision LLM 转录（单张截图 → 文本）。
- `src-tauri/src/services/observation_ingest/summarizer.rs`：批级时间线摘要（多张截图 + 转录 → 摘要文本）。
- `src-tauri/src/services/observation_ingest/writer.rs`：把最终 `content` 写入 `observations` 表。
- `src-tauri/src/services/observation_ingest/error.rs`：
  ```rust
  #[derive(Debug, thiserror::Error)]
  pub enum ObservationIngestError {
      #[error("vision transcription failed: {0}")]
      Transcription(#[source] anyhow::Error),
      #[error("summary generation failed: {0}")]
      Summary(#[source] anyhow::Error),
      #[error("observation write failed: {0}")]
      Write(#[from] rusqlite::Error),
      #[error("input missing: {0}")]
      MissingInput(&'static str),
  }
  ```
- `src-tauri/src/db/repos/observations.rs`：
  ```rust
  #[async_trait]
  pub trait ObservationRepo: Send + Sync {
      async fn insert(&self, new: NewObservation) -> Result<Observation>;
      async fn by_id(&self, id: i64) -> Result<Option<Observation>>;
      async fn recent(&self, limit: u32) -> Result<Vec<Observation>>;
  }
  pub struct SqliteObservationRepo { pool: DbPool }
  ```
- `src-tauri/prompts/transcription.md`、`src-tauri/prompts/summary.md`：§8 要求的中文 prompt 文件；首行 `<!-- version: 1 -->`，正文中文化。
- `src-tauri/src/services/observation_ingest/prompts.rs`：`include_str!("../../../prompts/transcription.md")` + `include_str!("../../../prompts/summary.md")` + 强类型 format 函数 `render_transcription(input: &TranscriptionInput) -> String`。
- `src-tauri/tests/observation_ingest.rs`：集成测试（mock LLM、内存 SQLite）。
- `src-tauri/tests/prompt_snapshots.rs`：`insta` snapshot 测试（按 §13.5）。

**Modify**
- `src-tauri/src/services/mod.rs`：`pub mod observation_ingest;`
- `src-tauri/src/services/capture_loop.rs`：原本 Step 11 的 "no downstream" tracing 改成发 `CaptureBatchCompleted { session_id, screenshot_ids, batch_started_at }` 到一个 `tokio::sync::broadcast::Sender`（存 `AppState`）。
- `src-tauri/src/lib.rs`：`setup` 里构建 `broadcast::channel(64)`，把 sender 挂 `AppState`，`ObservationIngestService::spawn(receiver, llm_provider.clone(), repo.clone(), config_watcher.clone())` 起协程。
- `src-tauri/src/providers/llm/mod.rs`：抽 `trait LlmProvider { async fn complete_json<T: DeserializeOwned>(&self, req: LlmRequest) -> Result<T>; async fn complete_text(&self, req: LlmRequest) -> Result<String>; async fn complete_vision(&self, imgs: &[VisionInput], prompt: &str) -> Result<String>; }`（§13.2）。现有 Gemini / Codex provider 实现此 trait。
- `src-tauri/src/events/mod.rs`（或等价位置）：定义 `CaptureBatchCompleted` / `ObservationReady` struct，派生 `Clone + Debug + Serialize`。

### 2.3 关键接口签名

```rust
// mod.rs
pub struct ObservationIngestService;

impl ObservationIngestService {
    pub fn spawn(
        rx: broadcast::Receiver<CaptureBatchCompleted>,
        llm: Arc<dyn LlmProvider>,
        repo: Arc<dyn ObservationRepo>,
        config: ConfigWatcher,
        observation_tx: broadcast::Sender<ObservationReady>,
    ) -> tokio::task::JoinHandle<()>;
}

// transcriber.rs
pub async fn transcribe(
    llm: &dyn LlmProvider,
    screenshot: &ScreenshotRef,
) -> Result<String, ObservationIngestError>;

// summarizer.rs
pub async fn summarize_batch(
    llm: &dyn LlmProvider,
    items: &[TranscribedItem],
) -> Result<String, ObservationIngestError>;

// writer.rs
pub async fn write_observation(
    repo: &dyn ObservationRepo,
    content: &str,
    session_id: &str,
    screenshot_id: Option<i64>,
) -> Result<Observation, ObservationIngestError>;
```

### 2.4 Task 分解（TDD 顺序：红 → 绿 → 重构 → 提交）

**Task 2.1 — repo + 新模块骨架**
- [ ] **Step 1**：`cargo test observations_repo::inserts_and_reads` 先失败。新建 `src-tauri/tests/observations_repo.rs`：
  ```rust
  use corivo::db::{migrations::apply_migrations, pool::test_in_memory_pool};
  use corivo::db::repos::observations::{NewObservation, ObservationRepo, SqliteObservationRepo};

  #[tokio::test]
  async fn inserts_and_reads() {
      let pool = test_in_memory_pool().unwrap();
      apply_migrations(&pool.get().unwrap()).unwrap();
      let repo = SqliteObservationRepo::new(pool);
      let row = repo.insert(NewObservation {
          observer_name: "screen".into(),
          content: "用户在 Notion 里写 meeting 纪要".into(),
          content_type: "input_text".into(),
          source_session_id: Some("sess-1".into()),
          source_screenshot_id: None,
      }).await.unwrap();
      let got = repo.by_id(row.id).await.unwrap().unwrap();
      assert_eq!(got.content, "用户在 Notion 里写 meeting 纪要");
      assert_eq!(got.observer_name, "screen");
  }
  ```
- [ ] **Step 2**：实现 `observations.rs` 让测试通过（SQL `INSERT ... RETURNING *`）。派生 `Observation` struct（`id / observer_name / content / content_type / source_session_id / source_screenshot_id / created_at / updated_at`）。
- [ ] **Step 3**：在 `db/repos/mod.rs` 导出；`cargo test observations_repo`。
- [ ] **Step 4**：commit `feat(db): add observations repo with SqliteObservationRepo`.

**Task 2.2 — prompt 文件 + insta snapshot**
- [ ] **Step 5**：写 `src-tauri/prompts/transcription.md`（中文、至少涵盖 "应用名 / 文件名 / URL / 输入框文本" 等信息点）；文件首行 `<!-- version: 1 | GUM §8 mapped from TRANSCRIPTION_PROMPT -->`。
- [ ] **Step 6**：写 `src-tauri/prompts/summary.md`（沿用现有中文 SUMMARY_PROMPT 的时间线风格；首行 `<!-- version: 1 -->`）。
- [ ] **Step 7**：新建 `src-tauri/src/services/observation_ingest/prompts.rs`：`include_str!`、带 `TranscriptionInput { app_hint: Option<String>, prior_content_hint: Option<String> }` 与 `SummaryInput { session_id: String, transcripts: Vec<String>, window_start: DateTime<Utc>, window_end: DateTime<Utc> }` 两个 struct + `render_*` 函数。
- [ ] **Step 8**：新建 `src-tauri/tests/prompt_snapshots.rs` 写一个 transcription + 一个 summary 的 insta snapshot。对 `SummaryInput` 给定确定时间戳 + 3 条固定 transcript，断言生成的 prompt 字符串与 `src-tauri/tests/snapshots/prompt_snapshots__summary_basic.snap` 一致。
- [ ] **Step 9**：`cargo insta review`（首次接受 baseline）→ commit `feat(prompts): add transcription/summary markdown prompts with insta snapshots`.

**Task 2.3 — LlmProvider trait 化**
- [ ] **Step 10**：`providers/llm/mod.rs` 定义 `trait LlmProvider`（签名见 2.2）。现有 Gemini / Codex 实现改成 `impl LlmProvider for GeminiProvider { ... }`；构造返回 `Arc<dyn LlmProvider>`。
- [ ] **Step 11**：新建 `providers/llm/mock.rs`（`#[cfg(test)]` 或 `#[cfg(feature = "test-support")]`），`MockLlm { responses: Mutex<Vec<String>> }`，`complete_vision` / `complete_text` / `complete_json` 按顺序 pop 返回。
- [ ] **Step 12**：`cargo check`（要动旧调用点把 `Arc<GeminiProvider>` 改 `Arc<dyn LlmProvider>`）。commit `refactor(llm): abstract LlmProvider trait; keep gemini/codex as impls`.

**Task 2.4 — transcriber / summarizer / writer（TDD）**
- [ ] **Step 13**：`src-tauri/tests/observation_ingest.rs` 写第一个失败测试：
  ```rust
  #[tokio::test]
  async fn transcriber_invokes_vision_with_prompt() {
      let mock = Arc::new(MockLlm::with_responses(vec!["Chrome > arXiv 2505.10831 页面".into()]));
      let screenshot = ScreenshotRef::fixture();
      let out = transcribe(&*mock, &screenshot).await.unwrap();
      assert!(out.contains("Chrome"));
      assert_eq!(mock.last_prompt_version().unwrap(), 1);
  }
  ```
  预期 FAIL（函数尚未实现）。
- [ ] **Step 14**：实现 `transcriber.rs`（读取 PNG/JPG bytes、调用 `llm.complete_vision`、错误映射到 `ObservationIngestError::Transcription`）。tracing: `observation.transcribed { screenshot_id, bytes_in, text_len }`。绿。
- [ ] **Step 15**：写 summarizer 的失败测试：给 3 条 transcribed 文本 → 返回拼好的摘要字符串。实现后绿。
- [ ] **Step 16**：写 writer 的失败测试：传入 content → `observations` 表多一行 + 返回值 id 非零。
- [ ] **Step 17**：commit `feat(observation_ingest): add transcriber/summarizer/writer with thiserror`.

**Task 2.5 — `ObservationIngestService::spawn` 集成测试**
- [ ] **Step 18**：写集成测试：
  - 起一个 broadcast channel；push 一个 `CaptureBatchCompleted { session_id: "s1", screenshot_ids: vec![1,2,3], ... }`
  - mock LLM 预置 3 条转录 + 1 条摘要
  - 等 `observation.ingested` tracing event（用 `tracing-subscriber::fmt::testing` 捕获）或等 `observation_tx.subscribe().recv()` 收到 `ObservationReady { id }`
  - 断言 `observations` 表里有一行 `content` 包含摘要、`observer_name = "screen"`、`source_session_id = "s1"`、`source_screenshot_id = Some(3)`（取 batch 最后一张作为锚点，保留 spec §6 的"关联最后一张截图"约束；若 spec 未明确，采用此约定并在 service 注释里记录）。
- [ ] **Step 19**：实现 `mod.rs::spawn`（`while let Ok(batch) = rx.recv().await { handle(...).await }`）；`handle` 里按 batch 跑 transcribe → summarize → write → emit。
- [ ] **Step 20**：`cargo test observation_ingest`。绿后 commit `feat(observation_ingest): ObservationIngestService consuming capture broadcasts`.

**Task 2.6 — wire into capture_loop**
- [ ] **Step 21**：`capture_loop.rs` 原本 Step 11 的 tracing 改成 `tx.send(CaptureBatchCompleted { ... })`；`tx` 从 `AppState` 注入。
- [ ] **Step 22**：`lib.rs::setup` 构建 `broadcast::channel(64)`（capacity 进 `Config::capture.broadcast_capacity`，见 §13.4），把 sender 传给 `capture_loop`，把 receiver 传给 `ObservationIngestService::spawn`；同时构建 `observation_tx`（PR3 用）放 `AppState` 供 user_model 订阅。
- [ ] **Step 23**：`pnpm tauri dev` 冷启动，手动触发一次 capture → 观察 stderr tracing：应该依次看到 `observation.transcribed` / `observation.ingested` 事件，并在 `observations` 表里看到新行（通过 `db_debug` 命令或 SQLite CLI）。
- [ ] **Step 24**：commit `feat(capture_loop): broadcast CaptureBatchCompleted; wire observation_ingest`.

**Task 2.7 — 收尾**
- [ ] **Step 25**：`cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo machete`、`pnpm build`、`pnpm test`、`pnpm knip` 全过。
- [ ] **Step 26**：`rg` 关键字扫描仍为 0（未引入旧名字）。
- [ ] **Step 27**：`rg -n "println!\|eprintln!" src-tauri/src/services/observation_ingest/`：0 命中。

### 2.5 本 PR DoD 子集

- [x] `cargo test` 全绿（新增 observations_repo / prompt_snapshots / observation_ingest）
- [x] `cargo clippy -D warnings`
- [x] `cargo machete` 零
- [x] `pnpm build` / `pnpm test` / `pnpm knip` 零
- [x] `rg` 扫描零命中
- [x] schema_version = 5
- [ ] IPC 一致（PR5）
- [x] 新目录无 `println!` / `eprintln!`

---

## PR 3 — `services/user_model` 数据层 + Observer + Batcher + Retrieval v1

### 3.1 Scope

- **Spec 引用**：§1（Proposition / Observation 模型）、§7.0 / §7.1（user_model 内部结构、Observer trait）、§7.3（retrieval v1 LIKE + 加权打分）、§13.1–13.4。
- **目标**：`observations` → `ScreenObserver` → `Batcher` → 发 `BatchReady { observation_ids }` 事件；`db/repos/propositions.rs` + `db/repos/observation_proposition.rs` 完成 CRUD；`retrieval::query` 能在有测试 fixture 的情况下按 LIKE + 加权排序返回 proposition。**本 PR 尚无 proposition 写入路径**——`BatchReady` 事件只落 tracing，PR4 才真正处理。

### 3.2 文件一览

**Create**
- `src-tauri/src/services/user_model/mod.rs`：`UserModel` facade（spawn batcher + observer + 预留 pipeline 占位）。
- `src-tauri/src/services/user_model/observer.rs`：`trait Observer` + `Update` struct。
- `src-tauri/src/services/user_model/screen_observer.rs`：订阅 `ObservationReady` → 从 repo 读取 observation → 输出 `Update`。
- `src-tauri/src/services/user_model/batcher.rs`：`ObservationBatcher`，按 `batcher.min_batch_size` + `batcher.flush_interval_ms` 攒批；输出 `BatchReady { observation_ids: Vec<i64> }`。
- `src-tauri/src/services/user_model/retrieval.rs`：`query(...)`，v1 实现。
- `src-tauri/src/services/user_model/error.rs`：`UserModelError`（`Retrieval` / `Batcher` / `Repo(#[from] rusqlite::Error)` 三个 variant）。
- `src-tauri/src/db/repos/propositions.rs`：
  ```rust
  #[async_trait]
  pub trait PropositionRepo: Send + Sync {
      async fn insert(&self, new: NewProposition) -> Result<Proposition>;
      async fn by_id(&self, id: i64) -> Result<Option<Proposition>>;
      async fn current_by_group(&self, group: &str) -> Result<Option<Proposition>>; // MAX(version)
      async fn versions_in_group(&self, group: &str) -> Result<Vec<Proposition>>;   // 升序
      async fn recent(&self, limit: u32) -> Result<Vec<Proposition>>;              // current version per group
      async fn query_like(&self, args: RetrievalArgs) -> Result<Vec<ScoredProposition>>;
      async fn link_observation(&self, observation_id: i64, proposition_id: i64) -> Result<()>;
      async fn observations_for(&self, proposition_id: i64) -> Result<Vec<Observation>>;
      async fn set_contradicts(&self, a: i64, b: i64) -> Result<()>;
      async fn update_revision_group(&self, id: i64, new_group: &str) -> Result<()>;
  }
  ```
- `src-tauri/src/db/repos/observation_proposition.rs`：薄薄一层 `INSERT OR IGNORE`（大多数操作其实走 `PropositionRepo::link_observation`，这里保留一个独立文件方便 PR4 做聚合查询 `observations_for_group`）。
- `src-tauri/tests/propositions_repo.rs` + `src-tauri/tests/retrieval.rs` + `src-tauri/tests/batcher.rs`。

**Modify**
- `src-tauri/src/services/mod.rs`：`pub mod user_model;`
- `src-tauri/src/lib.rs`：构建 `UserModel::spawn(...)`，订阅 `observation_tx` 下发的 `ObservationReady`。
- `src-tauri/src/domain/config.rs`：把 `BatcherConfig` / `RetrievalConfig` 填实（PR1 已预留字段）。`#[cfg(test)] Config::test_default()` 暴露给测试。

### 3.3 Observer / Batcher / Retrieval 关键签名

```rust
// observer.rs
pub enum ContentType { Text, ImageRef(String) }

pub struct Update { pub content: String, pub content_type: ContentType, pub observation_id: i64 }

#[async_trait]
pub trait Observer: Send + Sync {
    fn name(&self) -> &'static str;
    async fn subscribe(&self) -> mpsc::Receiver<Update>;
}

// batcher.rs
pub struct BatchReady { pub observation_ids: Vec<i64> }

pub struct ObservationBatcher { /* ... */ }

impl ObservationBatcher {
    pub fn new(cfg: BatcherConfig) -> Self;
    pub async fn run(
        mut self,
        mut rx: mpsc::Receiver<Update>,
        tx: broadcast::Sender<BatchReady>,
    );
}

// retrieval.rs
pub struct RetrievalArgs {
    pub text: Option<String>,
    pub limit: u32,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub include_observations: bool,
}

pub struct ScoredProposition {
    pub proposition: Proposition,
    pub score: f32,
    pub breakdown: ScoreBreakdown, // (confidence_term, decay_term, match_bonus, recency_age_days)
    pub observations: Option<Vec<Observation>>,
}

pub async fn query(
    repo: &dyn PropositionRepo,
    cfg: &RetrievalConfig,
    args: RetrievalArgs,
) -> Result<Vec<ScoredProposition>, UserModelError>;
```

### 3.4 Task 分解

**Task 3.1 — propositions repo**
- [ ] **Step 1**：`tests/propositions_repo.rs` 先写四个 must-fail：
  1. `insert_then_fetch_by_id`
  2. `versions_within_group_return_in_order`
  3. `current_by_group_returns_max_version`
  4. `link_and_read_observations`

  ```rust
  #[tokio::test]
  async fn current_by_group_returns_max_version() {
      let pool = test_in_memory_pool().unwrap();
      apply_migrations(&pool.get().unwrap()).unwrap();
      let repo = SqlitePropositionRepo::new(pool);
      let group = "g-1".to_string();
      let a = repo.insert(NewProposition::fixture(group.clone(), 1, "v1 text")).await.unwrap();
      let b = repo.insert(NewProposition::fixture(group.clone(), 2, "v2 text")).await.unwrap();
      let cur = repo.current_by_group(&group).await.unwrap().unwrap();
      assert_eq!(cur.id, b.id);
      assert_eq!(cur.version, 2);
      assert_ne!(cur.id, a.id);
  }
  ```
- [ ] **Step 2**：实现 `SqlitePropositionRepo`，SQL：
  - `current_by_group`：`SELECT * FROM propositions WHERE revision_group=?1 ORDER BY version DESC LIMIT 1`
  - `versions_in_group`：`ORDER BY version ASC`
  - `recent(limit)`：用窗口函数或子查询取每 group 的 MAX(version)：
    ```sql
    SELECT p.* FROM propositions p
    JOIN (
      SELECT revision_group, MAX(version) AS v
      FROM propositions GROUP BY revision_group
    ) m ON p.revision_group = m.revision_group AND p.version = m.v
    ORDER BY p.created_at DESC LIMIT ?1
    ```
- [ ] **Step 3**：四个测试全绿，commit `feat(db): add PropositionRepo with sqlite impl`.

**Task 3.2 — retrieval v1 打分**
- [ ] **Step 4**：`tests/retrieval.rs` 写失败测试：准备 6 条命题（不同 confidence / decay / created_at / 含或不含关键词 "Notion"）、给定 `RetrievalArgs { text: Some("Notion"), limit: 3 }`，断言返回的 3 条的 id 顺序与预期加权打分一致。用一个 `fake_clock` / `chrono::Utc::now()` 做参照（测试里构造记录时显式写 `created_at`）。
- [ ] **Step 5**：实现 `query_like` 的 SQL（§7.3 §1–§4）：
  ```sql
  SELECT p.*,
    (CAST(p.confidence AS REAL) * ?w_c)
    + (CAST(p.decay AS REAL) * ?w_d * exp(-((julianday(?now) - julianday(p.created_at)) / ?k_decay)))
    + ?match_bonus AS score
  FROM propositions p
  WHERE (?text IS NULL OR (p.text LIKE ?like OR p.reasoning LIKE ?like))
    AND (?start IS NULL OR p.created_at >= ?start)
    AND (?end   IS NULL OR p.created_at <= ?end)
  ORDER BY score DESC LIMIT ?lim
  ```
  SQLite 没有内置 `exp()`，用 `rusqlite::functions::create_scalar_function` 在 pool 初始化时注册 `exp(x)`；或在 Rust 侧算完 decay 项后回传。本 PR 走后者更简单：SQL 侧只做 `LIKE` 过滤 + 粗排 `p.confidence DESC`，取 `limit * limit_multiplier` 回到 Rust 按 `confidence × w_c + decay × w_d × e^(-age/k) + match_bonus` 算最终 score、排序、去重（同 `revision_group` 仅留最新 `version`）、截断 `limit`。
- [ ] **Step 6**：实现 `retrieval::query`，注入 `cfg: &RetrievalConfig` 从 Config 拿权重与 `k_decay`。
- [ ] **Step 7**：测试绿；commit `feat(user_model): retrieval v1 LIKE + confidence×decay×recency scoring`.

**Task 3.3 — Observer + ScreenObserver**
- [ ] **Step 8**：实现 `observer.rs` trait + `Update` struct。
- [ ] **Step 9**：`screen_observer.rs`：`ScreenObserver::spawn(rx: broadcast::Receiver<ObservationReady>, repo: Arc<dyn ObservationRepo>) -> mpsc::Receiver<Update>`。对每个 event → `repo.by_id(id).await?` → 包成 `Update { content, content_type: Text, observation_id: id }` → 送入 mpsc（容量从 Config 拿）。
- [ ] **Step 10**：测试 `tests/screen_observer.rs`：把一个 `ObservationReady { id }` 广播进去，断言 mpsc 端收到对应 content 的 `Update`。commit `feat(user_model): ScreenObserver reading observations on ObservationReady`.

**Task 3.4 — Batcher（rigidly test `min_batch_size` & `flush_interval_ms`）**
- [ ] **Step 11**：`tests/batcher.rs`：
  - `fills_when_min_batch_reached`：min_batch=3，推 3 个 update，立刻收到 BatchReady。
  - `flushes_on_timeout`：min_batch=5，推 2 个 update，等 flush_interval 后收到 BatchReady（用 `tokio::time::pause()` + `advance`）。
  - `empty_window_does_not_emit`。
- [ ] **Step 12**：实现 `ObservationBatcher::run`（`tokio::select!` 一边 `rx.recv()` 一边 `tokio::time::sleep_until(deadline)`）。tracing: `batcher.flushed { size, reason: "full"|"timeout" }`。
- [ ] **Step 13**：commit `feat(user_model): ObservationBatcher with size+timeout flushing`.

**Task 3.5 — UserModel facade + wire**
- [ ] **Step 14**：`mod.rs::UserModel::spawn(...)` 串联 ScreenObserver → Batcher → broadcast::Sender<BatchReady>；暴露 `fn batch_receiver(&self) -> broadcast::Receiver<BatchReady>` 供 PR4。
- [ ] **Step 15**：`lib.rs::setup` 把 `observation_tx` 的 `subscribe()` 喂给 `UserModel::spawn`。
- [ ] **Step 16**：手动冷启动，等一段 batcher timeout 后日志里应见 `batcher.flushed { size, reason: "timeout" }`。commit `feat(user_model): spawn UserModel wiring observer -> batcher -> broadcast`.

### 3.5 本 PR DoD 子集

- [x] `cargo test` 全绿（含 propositions_repo / retrieval / screen_observer / batcher 四个测试文件）
- [x] `cargo clippy -D warnings` / `cargo machete`
- [x] `pnpm build` / `pnpm test` / `pnpm knip`
- [x] `rg` 关键字扫描零命中
- [x] schema_version = 5
- [ ] IPC 一致（PR5）
- [x] 新目录无 `println!` / `eprintln!`

---

## PR 4 — `proposition_pipeline`：PROPOSE / SIMILAR / REVISE（4 种 RevisionOp）

### 4.1 Scope

- **Spec 引用**：§1.1（Revise 是价值支点）、§7.2（pipeline 步骤 + 4 种 op 的落库规则）、§8（PROPOSE / SIMILAR / REVISE prompt 中文化）、§12 里程碑 1（merge/update/contradict/rewrite 各至少 1 条用例）、§13.5（prompt .md + insta snapshot）。
- **目标**：订阅 `BatchReady` → 跑 `generate_and_search → filter → handle_*`；每次 REVISE 正确落库（见 §7.2 step 4 四种分支）；`cargo test user_model::proposition_pipeline` 覆盖 4 种 op 各一条用例 + `revision_group` 内 version 递增 + confidence/decay 被重写。

### 4.2 文件一览

**Create**
- `src-tauri/src/services/user_model/proposition_pipeline.rs`
- `src-tauri/src/services/user_model/prompts.rs`（PR2 已经有 observation_ingest 的 prompts.rs；此处是 user_model 的）
- `src-tauri/prompts/propose.md` / `similar.md` / `revise.md`（各自 `<!-- version: 1 -->`）
- `src-tauri/src/services/user_model/pipeline_types.rs`（`PropositionDraft` / `RelationKind` / `RevisionOp`（enum with 4 variants） / `RevisionOutcome` struct，派生 `Serialize + Deserialize` 用于 LLM structured output）
- `src-tauri/src/services/user_model/error.rs`（扩展）：新增 `PipelineError::Propose / Similar / Revise / InvalidOutput`。
- `src-tauri/tests/proposition_pipeline.rs`：4 个 happy-path + 若干边界。
- `src-tauri/tests/snapshots/`：3 份 prompt snapshot。

**Modify**
- `src-tauri/src/services/user_model/mod.rs`：`UserModel::spawn` 里订阅 `BatchReady` → `proposition_pipeline::process_batch(...)`。
- `src-tauri/src/domain/config.rs`：`PipelineConfig` 已有 `similar_pool_size`；加 `min_draft_count: u32 = 1`（宽松下限，避免 LLM 空返回卡死）。

### 4.3 关键签名与 4 种 RevisionOp 落库规则（照搬 spec §7.2 step 4）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum RevisionOp {
    Merge { text: String, reasoning: String, confidence: u8, decay: u8,
            absorbed_proposition_ids: Vec<i64> },
    Update { target_proposition_id: i64, reasoning: String, confidence: u8, decay: u8 },
    Contradict { existing_proposition_id: i64, new_text: String, new_reasoning: String,
                 new_confidence: u8, new_decay: u8 },
    Rewrite { target_proposition_id: i64, text: String, reasoning: String,
              confidence: u8, decay: u8 },
}

pub async fn process_batch(
    ctx: PipelineCtx<'_>,
    observation_ids: &[i64],
) -> Result<ProcessSummary, PipelineError>;
```

**落库规则**（`proposition_pipeline.rs` 必须严格照办；四个 op 每条一句话测试断言）：

| op | 新行？ | `revision_group` | `version` | 旧行处理 | contradicts_id |
|----|-------|------------------|-----------|-----------|----------------|
| Merge | ✅ 1 条新行 | 簇内最老 `revision_group`（其它 group 的旧行通过 `update_revision_group` 统一过来，`version` 保留原值） | MAX(簇内旧 version) + 1 | 保留；group 统一到簇内最老 | NULL |
| Update | ✅ 1 条新行（text 沿用） | 不变 | +1 | 保留 | NULL |
| Contradict | ✅ 新建一条独立命题（自己的 new `revision_group`） | 新 UUID | 1 | 旧行保留；两条行通过 `contradicts_proposition_id` 互指 | 互指 |
| Rewrite | ✅ 1 条新行（text 替换） | 不变 | +1 | 保留 | NULL |

### 4.4 Task 分解

**Task 4.1 — pipeline types**
- [ ] **Step 1**：建 `pipeline_types.rs`。严格用 `#[serde(tag = "op")]` 区分 4 个 variant。编译通过。
- [ ] **Step 2**：写 `tests/pipeline_types.rs` 一个反序列化 test：给定 LLM 可能输出的 JSON（merge / update / contradict / rewrite 各一条），能正确 parse。commit `feat(user_model): RevisionOp tagged union`.

**Task 4.2 — prompt .md + snapshot**
- [ ] **Step 3**：写 `propose.md` / `similar.md` / `revise.md`。内容参考 `generalusermodels/gum` 仓库的 `gum/prompts/gum.py`，中文化；强制 JSON 输出 schema（用 `{{ }}` 占位描述 schema，end-of-prompt 提示 "仅输出 JSON"）。`revise.md` 的 few-shot 必须涵盖 4 个 op 各一例。
- [ ] **Step 4**：`src-tauri/src/services/user_model/prompts.rs`：`include_str!` + 三个 `render_*`（入参 struct；不要 `format!` 自由字符串）。
- [ ] **Step 5**：`tests/prompt_snapshots.rs` 追加 3 份 snapshot（propose / similar / revise）。`cargo insta review` 接受基线。commit `feat(prompts): propose/similar/revise with insta snapshots`.

**Task 4.3 — PROPOSE + SIMILAR**
- [ ] **Step 6**：写测试 `pipeline_propose_creates_drafts`：mock LLM 预置一段含 3 条 draft 的 JSON → `process_batch` 应调用 retrieval 3 次（被 LLM draft 数触发）→ 合并成 pool。
- [ ] **Step 7**：写测试 `pipeline_similar_partitions_pool`：mock LLM 预置 RelationSchema JSON（identical / similar / unrelated 各占一条）→ 分三类正确路由。
- [ ] **Step 8**：实现 `generate_and_search` + `filter`。tracing: `proposition.proposed { count }`, `proposition.similar_partitioned { identical, similar, unrelated }`.
- [ ] **Step 9**：commit `feat(proposition_pipeline): PROPOSE + SIMILAR stages`.

**Task 4.4 — REVISE 的 4 分支（§12 里程碑 1）**

每个分支独立 TDD，按顺序：

- [ ] **Step 10**：`test_revise_merge_merges_into_oldest_group`
  - 准备：两条旧命题 `p1 (group=g1, version=1)`、`p2 (group=g2, version=1)`。mock LLM 返回 merge op（absorb [p1.id, p2.id]）。
  - 断言：新行 (group=g1, version=2, text=merge.text)；查 `versions_in_group("g1")` 返回 2 条；查 `versions_in_group("g2")` 返回 p2（它的 group 被 update 到 g1 还是保留？按 spec "被并入的 row 更新 revision_group"）→ 改：`versions_in_group("g2")` 应 empty，`versions_in_group("g1")` 包含 p1 / p2 / 新行。
- [ ] **Step 11**：实现 merge handler；绿后 commit。
- [ ] **Step 12**：`test_revise_update_rewrites_metadata_only`：mock LLM 返回 update op（target=p1）。新行 version=p1.version+1，text 与 p1 相同，confidence/decay/reasoning 被覆盖。
- [ ] **Step 13**：实现 update handler；commit。
- [ ] **Step 14**：`test_revise_contradict_pairs_two_rows`：mock LLM 返回 contradict（existing=p1）。新行在自己新 `revision_group`、version=1；两行 `contradicts_proposition_id` 互指。
- [ ] **Step 15**：实现 contradict handler（`set_contradicts(a, b)` 双向写）；commit。
- [ ] **Step 16**：`test_revise_rewrite_keeps_group_bumps_version`：target=p1；新行 group=p1.group、version=p1.version+1、text=REVISE.text。
- [ ] **Step 17**：实现 rewrite handler；commit。

**Task 4.5 — handle_identical + handle_different**
- [ ] **Step 18**：`test_identical_just_links_existing_proposition`（`INSERT OR IGNORE` 到 `observation_proposition`，不产生新 row）。
- [ ] **Step 19**：`test_unrelated_creates_brand_new_proposition`（新 group、version=1、绑当前 batch 的 observations）。
- [ ] **Step 20**：实现；commit。

**Task 4.6 — 集成 + wire**
- [ ] **Step 21**：在 `UserModel::spawn` 里消费 `BatchReady`，调用 `process_batch`。emit `proposition.revised { op, revision_group }` tracing。
- [ ] **Step 22**：端到端 smoke test `tests/pipeline_end_to_end.rs`：直接推 observation → batcher timeout → pipeline 处理 → propositions 表有行。
- [ ] **Step 23**：`cargo test` 全绿；commit `feat(user_model): wire proposition_pipeline into UserModel`.

### 4.5 本 PR DoD 子集

- [x] `cargo test`（含 REVISE 4 分支 + 新 prompt 快照）
- [x] `cargo clippy -D warnings`、`cargo machete`
- [x] `pnpm build` / `pnpm test` / `pnpm knip`
- [x] `rg` 零命中
- [x] schema_version = 5
- [ ] IPC 一致（PR5）
- [x] 新目录无 `println!` / `eprintln!`
- [x] §12 里程碑 1 达成

---

## PR 5 — 读路径 Tauri 命令 + 前端 Propositions/Knowledge 页 + 版本链 UI

### 5.1 Scope

- **Spec 引用**：§7.4（user_model_* 读命令）、§9（前端知识页要展示 `revision_group` 版本链）、§13.1（Tauri tagged error union）。
- **目标**：5 个 read-only 命令可从前端直接 `invoke()`；React Query hooks 驱动 `/knowledge` 页渲染命题列表（按 confidence × recency 排序）+ 详情抽屉展示 reasoning + 关联 observations + 同 `revision_group` 版本链（diff：text / confidence / decay 的跨版本对比）。
- **Spec drift（同 PR 修订）**：§7.4 原本列 4 个读命令。本 PR 为版本链 UI 真实可用额外加一个 `user_model_versions_in_group(group)`；`§7.4` 段落的命令清单在本 PR 同次 commit 更新为 5 条，避免后续 `rg` / 手动 diff 失准。

### 5.2 文件一览

**Create**
- `src-tauri/src/commands/user_model.rs`：5 个 `#[tauri::command]`（§7.4 原 4 个 + 新增 `user_model_versions_in_group`）。
- `src-tauri/src/domain/ipc_error.rs`：`TauriError` tagged union（§13.1），`impl From<PipelineError|UserModelError|ObservationIngestError|...>`。
- `src/hooks/use-propositions.ts` / `use-proposition-detail.ts` / `use-proposition-versions.ts`。
- `src/routes/knowledge.tsx`（替换 PR1 的占位）与路由声明更新（把 `/memory` → `/knowledge`，并在 sidebar 里改文案）。
- `src/pages/knowledge/propositions-list.tsx`、`proposition-detail-panel.tsx`、`version-chain-timeline.tsx`、`observation-pill.tsx`。
- `src/pages/knowledge/propositions-list.test.tsx`、`version-chain-timeline.test.tsx`（Vitest + Testing Library，仅 DOM-less 单元；用 `@testing-library/react` + jsdom 需要在 vitest config 里为这两个文件临时切 `environment: "jsdom"`，写在文件顶部 `/** @vitest-environment jsdom */`）。

**Modify**
- `src-tauri/src/lib.rs`：`generate_handler!` 追加 5 个 user_model 命令；错误类型映射成 `TauriError`。
- `src/lib/tauri.ts`：新增 `userModelQuery(text, limit)` / `userModelRecentPropositions(limit)` / `userModelRecentObservations(limit)` / `userModelPropositionObservations(id)` / `userModelVersionsInGroup(group)` 五个 wrapper；返回类型严格 `ScoredProposition[]` / `Proposition[]` / `Observation[]`。并新增 `TauriError` discriminated union（§13.1）。
- `docs/gum-refactor-spec.md` §7.4：把读路径命令清单从 4 条更新为 5 条，加 `user_model_versions_in_group(group) -> Vec<Proposition>` 并在注脚注明"为 Knowledge 页版本链 UI 补充"（同 PR commit）。
- `src/lib/types.ts`：新增 `Proposition` / `Observation` / `ScoredProposition` / `TauriError` 类型，字段完全对齐 Rust struct（`revision_group` / `version` / `confidence` / `decay` / `reasoning` / `contradicts_proposition_id` / `created_at`）。

### 5.3 关键签名

```rust
#[tauri::command]
pub async fn user_model_query(
    state: State<'_, AppState>, text: String, limit: u32,
) -> Result<Vec<ScoredProposition>, TauriError>;

#[tauri::command]
pub async fn user_model_recent_propositions(
    state: State<'_, AppState>, limit: u32,
) -> Result<Vec<Proposition>, TauriError>;

#[tauri::command]
pub async fn user_model_recent_observations(
    state: State<'_, AppState>, limit: u32,
) -> Result<Vec<Observation>, TauriError>;

#[tauri::command]
pub async fn user_model_proposition_observations(
    state: State<'_, AppState>, id: i64,
) -> Result<Vec<Observation>, TauriError>;

#[tauri::command]
pub async fn user_model_versions_in_group(
    state: State<'_, AppState>, group: String,
) -> Result<Vec<Proposition>, TauriError>;
```

```ts
export type TauriError =
  | { kind: "network"; message: string }
  | { kind: "llm_rate_limited"; message: string; retry_after_ms?: number }
  | { kind: "pipeline"; message: string; detail?: unknown }
  | { kind: "user_model"; message: string; detail?: unknown }
  | { kind: "unknown"; message: string };

export async function userModelQuery(text: string, limit: number): Promise<ScoredProposition[]>;
```

### 5.4 Task 分解

**Task 5.1 — Tauri error tagged union**
- [ ] **Step 1**：`domain/ipc_error.rs` 定义：
  ```rust
  #[derive(Debug, Serialize)]
  #[serde(tag = "kind", rename_all = "snake_case")]
  pub enum TauriError {
      Network { message: String },
      LlmRateLimited { message: String, retry_after_ms: Option<u64> },
      Pipeline { message: String, detail: Option<serde_json::Value> },
      UserModel { message: String, detail: Option<serde_json::Value> },
      Unknown { message: String },
  }
  ```
  实现 `From<UserModelError>` / `From<PipelineError>` / `From<ObservationIngestError>` / fallback `From<anyhow::Error>` -> `Unknown`。
- [ ] **Step 2**：写 `tests/ipc_error.rs`：给每个 From 一个样例，断言 JSON 序列化结果的 `kind` / `message` 字段。commit `feat(domain): TauriError tagged union with From impls`.

**Task 5.2 — 4 个命令 + 单元测试**
- [ ] **Step 3**：`commands/user_model.rs` 实现 4 个命令，内部调用 `AppState.user_model.query(...)` / `PropositionRepo::recent(...)` 等。
- [ ] **Step 4**：`tests/commands_user_model.rs`（用 `tauri::test` 的 mock invoke）：5 个命令各一个 happy-path + 一个 error-path（mock LLM / repo 返回错误 → 得到 `TauriError::Pipeline` / `TauriError::UserModel`）。`user_model_versions_in_group` 的 happy-path 插入 3 条同 group 的 v1/v2/v3 命题，断言返回按 `version ASC` 排。
- [ ] **Step 5**：`lib.rs` 把 5 个命令挂进 `generate_handler!`；PR1 以来第一次更新 handler 宏。commit `feat(commands): user_model read commands with TauriError`.

**Task 5.3 — 前端 wrapper + hooks**
- [ ] **Step 6**：`src/lib/tauri.ts` 新增 5 个 wrapper（含 `userModelVersionsInGroup`）+ `TauriError` type + `fromInvokeError` 小工具（把 Tauri 抛出的 string/object 统一归一成 `TauriError`）。
- [ ] **Step 7**：`src/lib/types.ts` 补 `Proposition` / `Observation` / `ScoredProposition`（含 `score` + `breakdown`）。
- [ ] **Step 8**：`src/hooks/use-propositions.ts`：
  ```ts
  export function useRecentPropositions(limit = 30) {
    return useQuery({
      queryKey: ["propositions", "recent", limit],
      queryFn: () => userModelRecentPropositions(limit),
    });
  }
  export function useProposition(id: number | null) {
    return useQuery({
      queryKey: ["proposition", id],
      queryFn: () => userModelPropositionObservations(id!),
      enabled: id != null,
    });
  }
  export function usePropositionQuery(text: string, limit = 20) { /* ... */ }
  ```
- [ ] **Step 9**：`src/hooks/use-proposition-versions.ts`：
  ```ts
  export function usePropositionVersions(group: string | null) {
    return useQuery({
      queryKey: ["proposition-versions", group],
      queryFn: () => userModelVersionsInGroup(group!),
      enabled: !!group,
    });
  }
  ```
  后端 `user_model_versions_in_group(group)` 直接走 `PropositionRepo::versions_in_group`（PR3 已实现，返 `ORDER BY version ASC`），确保拿到该 group 的完整历史链，不依赖前端分组——避免 `recent(200)` 窗口外漏。
- [ ] **Step 10**：commit `feat(frontend): useRecentPropositions + useProposition + useQuery + useVersions`.

**Task 5.4 — Knowledge 页 UI**
- [ ] **Step 11**：`src/routes/knowledge.tsx` 替换 PR1 的 `/memory` 占位：router 里把 path 改成 `/knowledge`，sidebar 项目把"记忆"改成"知识"（中文）。
- [ ] **Step 12**：`propositions-list.tsx`：按 `confidence × recency` 排序的卡片列表，每张卡片显示 `text` / `confidence / decay` Chip / `version` badge / `revision_group` 尾号截取（hex ≥ 6 位）。点击打开 `proposition-detail-panel`。
- [ ] **Step 13**：`proposition-detail-panel.tsx`：顶部显示当前 version、reasoning；中间 `version-chain-timeline` 展示同 group 的所有 version（升序）+ 相邻 version 的 diff（至少展示 text 改变、confidence/decay 数值）；底部 `observation-pill` 列出关联 observations（可点跳到原截图 session）。
- [ ] **Step 14**：`propositions-list.test.tsx`：渲染 3 条 fixtures，断言 version badge、confidence 徽标、点击回调。
- [ ] **Step 15**：`version-chain-timeline.test.tsx`：渲染 3 个 version（v1/v2/v3），断言顺序 + diff 标注。
- [ ] **Step 16**：commit `feat(knowledge): propositions list + detail panel + version chain timeline`.

**Task 5.5 — E2E 冷跑**
- [ ] **Step 17**：`pnpm tauri dev` 启动，手动触发若干次 capture → 等 pipeline 落第一批命题 → 进 `/knowledge` 能看到卡片。点进去验证 detail panel 数据齐。
- [ ] **Step 18**：`pnpm build` / `pnpm test` / `pnpm knip` / `cargo test` / `cargo clippy -D warnings` / `cargo machete` 全绿。commit `chore: PR5 green bar`.

### 5.5 本 PR DoD 子集

- [x] `cargo test` / `cargo clippy -D warnings` / `cargo machete`
- [x] `pnpm build` / `pnpm test` / `pnpm knip`
- [x] `rg` 零命中
- [x] schema_version = 5
- [x] **IPC 表面一致性（partial）**：`lib.rs::generate_handler!` 现在应含 5 个 user_model_* 命令；手动 diff 过 `lib/tauri.ts` 与 §7.4（本 PR 已更新到 5 条）的 **user_model 部分**完全一致（notification_log_* 留到 PR6）
- [x] 新目录无 `println!` / `eprintln!`

---

## PR 6 — `push_decider` + `notification_log` + Overlay 切到命题

### 6.1 Scope

- **Spec 引用**：§7.5（`push_decider` + 双锚点 `notification_log`）、§7.4 中 `notification_log_*` 两条命令、§9（overlay 数据源切到 proposition_id）、§12 里程碑 4 / 5（冷却 + dismiss 惩罚集成测试用 mock clock）。
- **目标**：`proposition.revised`（merge/update/rewrite 三类——contradict 是否触发推送先按"也发、让用户仲裁"处理，在 `push_decider::decide` 里可配置）→ `push_decider::decide` → 通过则 `notification_service::push(PushPayload { proposition_id, revision_group, text, ... })` + 写 `notification_log`。overlay 的 "ack" / "dismiss" 回写 `notification_log.outcome / outcome_at`。`PushConfig` 三参数走 `Config::user_model.push`。

### 6.2 文件一览

**Create**
- `src-tauri/src/services/push_decider.rs`
- `src-tauri/src/db/repos/notification_log.rs`：
  ```rust
  #[async_trait]
  pub trait NotificationLogRepo: Send + Sync {
      async fn insert(&self, row: NewNotificationLog) -> Result<NotificationLogEntry>;
      async fn last_in_group(&self, group: &str) -> Result<Option<NotificationLogEntry>>;
      async fn last_dismissed_in_group(&self, group: &str) -> Result<Option<DateTime<Utc>>>;
      async fn set_outcome(&self, id: i64, outcome: &str) -> Result<()>;
      async fn recent(&self, limit: u32) -> Result<Vec<NotificationLogEntry>>;
  }
  ```
- `src-tauri/src/commands/notification_log.rs`：2 个命令。
- `src-tauri/tests/push_decider.rs`：冷却 / dismiss 惩罚 / below-confidence / happy path。
- `src-tauri/tests/notification_log_repo.rs`。
- `src/hooks/use-notification-log.ts`。
- `src/pages/knowledge/notification-history-panel.tsx`（嵌入 `/knowledge` 或单开 `/notifications`；spec 未指定路径，决策：放 `/knowledge` 页签以复用路由，本 PR 加 Tab "推送历史"）。

**Modify**
- `src-tauri/src/services/user_model/proposition_pipeline.rs`：每次落库 revise 后 emit `PropositionRevised { proposition_id, revision_group, op }` 到一个 broadcast；`push_decider` 订阅。
- `src-tauri/src/services/notification_service/mod.rs`：`PushPayload` struct 重写（`proposition_id / revision_group / text / reasoning / confidence`）；overlay 下发的 JSON 换字段；保留 geometry / panel 逻辑不动（§4 明确 overlay 壳保留）。
- `src-tauri/src/lib.rs`：`generate_handler!` 追加 `notification_log_recent` / `notification_log_set_outcome`；事件流接线。
- `src/overlay/overlay-app.tsx` + `overlay/overlay-model.ts`：`OverlayPhase` 接收 `proposition_id`；ack / dismiss 通过 `invoke("notification_log_set_outcome", { id, outcome })`（`id = notification_log.id`，在 PushPayload 里带）。
- `src/lib/tauri.ts`：`notificationLogRecent(limit)` / `notificationLogSetOutcome(id, outcome)`。
- `src/lib/types.ts`：`NotificationLogEntry` / `NotificationOutcome`。
- `src-tauri/src/domain/config.rs`：`PushConfig { base_cooldown_secs, dismiss_penalty_secs, min_confidence, push_on_contradict: bool }` 已有字段 + 加 `push_on_contradict: bool = true`；`Config::validate()` 检查 `base_cooldown_secs > 0` / `dismiss_penalty_secs > 0` / `min_confidence in 1..=10`（§13.4 要求的 validate）。

### 6.3 关键签名（照 spec §7.5）

```rust
pub struct PushConfig {
    pub base_cooldown: Duration,   // from base_cooldown_secs
    pub dismiss_penalty: Duration, // from dismiss_penalty_secs
    pub min_confidence: u8,
    pub push_on_contradict: bool,
}

#[derive(Debug, Clone)]
pub enum PushDecision { Push, Skip(SkipReason) }

#[derive(Debug, Clone)]
pub enum SkipReason {
    WithinCooldown { last_pushed_at: DateTime<Utc> },
    DismissPenalty { dismissed_at: DateTime<Utc> },
    BelowConfidence,
    ContradictDisabled,
}

pub async fn decide(
    prop: &Proposition,
    op: RevisionOp,
    log: &dyn NotificationLogRepo,
    cfg: &PushConfig,
    clock: &dyn Clock,
) -> Result<PushDecision, PushError>;

pub trait Clock: Send + Sync { fn now(&self) -> DateTime<Utc>; }
```

### 6.4 Task 分解

**Task 6.1 — notification_log repo**
- [ ] **Step 1**：`tests/notification_log_repo.rs` 写失败测试：insert / last_in_group / last_dismissed_in_group / set_outcome / recent 各一条。
- [ ] **Step 2**：实现 `SqliteNotificationLogRepo`。SQL `last_dismissed_in_group`：
  ```sql
  SELECT outcome_at FROM notification_log
  WHERE revision_group = ?1 AND outcome = 'dismissed'
  ORDER BY outcome_at DESC LIMIT 1
  ```
- [ ] **Step 3**：commit `feat(db): NotificationLogRepo sqlite impl`.

**Task 6.2 — push_decider（TDD 四分支）**
- [ ] **Step 4**：引入 `Clock` trait + `SystemClock` 实际实现 + `MockClock` 测试实现（带 `advance(dur)`）。
- [ ] **Step 5**：`tests/push_decider.rs` 写 4 个失败测试：
  1. `push_when_never_pushed_and_confident_enough`
  2. `skip_when_within_base_cooldown`
  3. `skip_when_dismissed_in_penalty_window`
  4. `skip_when_below_min_confidence`

  `skip_when_within_base_cooldown` 例：
  ```rust
  let clock = MockClock::at("2026-04-17T10:00:00Z");
  let log = InMemoryNotificationLogRepo::new();
  log.insert(NewNotificationLog {
      revision_group: "g-1".into(),
      proposition_id: Some(42),
      pushed_at: parse("2026-04-17T03:00:00Z"), // 7h ago
      outcome: None,
  }).await.unwrap();
  let cfg = PushConfig { base_cooldown: hours(24), dismiss_penalty: days(7), min_confidence: 6, push_on_contradict: true };
  let prop = Proposition::fixture("g-1", 2, /*confidence*/ 8);
  let decision = decide(&prop, RevisionOp::Update { .. }, &log, &cfg, &clock).await.unwrap();
  assert!(matches!(decision, PushDecision::Skip(SkipReason::WithinCooldown { .. })));
  ```
- [ ] **Step 6**：实现 `decide`。commit `feat(push_decider): cooldown + dismiss-penalty + confidence gate`.

**Task 6.3 — 接线：proposition_pipeline → push_decider → notification_service → notification_log**
- [ ] **Step 7**：`proposition_pipeline.rs` 里每次 commit revise 后 `tx.send(PropositionRevised { ... })`。
- [ ] **Step 8**：新 service task `push_pipeline::spawn(rx, decider, notifier, log_repo, clock)`：对每个事件 → 读 prop 当前 version → `decide` → push 的话 `notification_log.insert` 再 `notifier.push(PushPayload { notification_log_id, proposition_id, revision_group, ... })`。
- [ ] **Step 9**：集成测试 `tests/push_pipeline_end_to_end.rs`：对同一 revision_group 连续触发两次 → 第二次 `Skip::WithinCooldown`（§12 里程碑 4）。
- [ ] **Step 10**：commit `feat(services): push_pipeline wiring proposition events to overlay`.

**Task 6.4 — overlay outcome 回写（§12 里程碑 5）**
- [ ] **Step 11**：`commands/notification_log.rs`：`notification_log_set_outcome(id, outcome: String)`。合法 outcome：`acknowledged | dismissed | ignored`。
- [ ] **Step 12**：overlay 前端：`overlay-model.ts` 持有 `notification_log_id`；ack/dismiss 按钮 → `invoke("notification_log_set_outcome", { id, outcome: "acknowledged"|"dismissed" })`。
- [ ] **Step 13**：集成测试 `tests/push_pipeline_dismiss_penalty.rs`：dismiss → 7 天内同 group 再触发返回 `Skip::DismissPenalty`。commit `feat(overlay): roundtrip outcome to notification_log`.

**Task 6.5 — notification history panel**
- [ ] **Step 14**：`notification_log_recent(limit)` 命令 + wrapper。
- [ ] **Step 15**：`/knowledge` 页加 Tab "推送历史"，列出最近 50 条 notification_log（时间 / revision_group 尾号 / outcome）。
- [ ] **Step 16**：commit `feat(knowledge): notification history tab`.

**Task 6.6 — Config validate**
- [ ] **Step 17**：实现 `Config::validate() -> Result<(), ConfigError>`；在 `config_service::load_and_validate()` 调用；无效时 log error 并 fallback 到 `Config::default()`（不 panic，onboarding 还要能进）。`tests/config_validate.rs` 覆盖：cooldown <= 0 / min_confidence = 0 / min_confidence > 10 / 权重负数。
- [ ] **Step 18**：commit `feat(config): validate ranges for push + retrieval`.

### 6.5 本 PR DoD 子集

- [x] `cargo test`（§12 里程碑 4 + 5 覆盖）
- [x] `cargo clippy -D warnings` / `cargo machete`
- [x] `pnpm build` / `pnpm test` / `pnpm knip`
- [x] `rg` 零命中
- [x] schema_version = 5
- [x] **IPC 一致**：`generate_handler!` 现在应完全对应 §7.4（5 user_model_* + 2 notification_log_* + capture/config/llm_probe/onboarding/settings 剩余）
- [x] 新目录无 `println!` / `eprintln!`

---

## PR 7 — 工程化收尾 + §13.8 DoD 终验

### 7.1 Scope

- **Spec 引用**：§13 全部（尤其 13.3 tracing 审查、13.6 死代码清零、13.8 DoD）。
- **目标**：在本次重构已经打开过的文件范围内，把工程化残留一次收干净：tracing 补齐 `#[tracing::instrument(skip(...))]`、所有 println! 清零、`cargo machete` / `pnpm knip` 继续零、Config 魔法数字复核、prompt 版本号复核、最终 `rg` 关键字扫描零命中。**不碰本次未打开过的历史文件**（§13 范围原则）。

### 7.2 文件一览（全部 Modify，可能少量 Delete）

- `src-tauri/src/services/observation_ingest/*.rs`、`services/user_model/*.rs`、`services/push_decider.rs`、`services/notification_service/mod.rs`、`commands/user_model.rs`、`commands/notification_log.rs`、`domain/ipc_error.rs`、`domain/config.rs`、`db/repos/{observations,propositions,observation_proposition,notification_log}.rs`、`providers/llm/*.rs`：统一审查 `tracing::instrument` + 事件命名 + 错误 variant。
- `src-tauri/prompts/*.md`：确认每个文件 `version: N` 元信息与本次写入一致；REVISE 文件若在 PR4 之后有 few-shot 修订，要 bump version。
- `src-tauri/Cargo.toml`：`cargo machete` 最终 pass；若仍有残留依赖（比如 `serde_urlencoded` 只被被删模块用过）清掉。
- `package.json` / `knip.json`：最终确认 `pnpm knip` 0 unused。
- 顶层 README / CLAUDE.md：若 CLAUDE.md 架构节还提 "mvp_pipeline" / "Supermemory" / "attention items"，改成 "observation_ingest / user_model / push_decider"。（CLAUDE.md 是用户指令来源，**改动前必须本 PR 的 commit 中明确标注**——不偷改）

### 7.3 Task 分解

**Task 7.1 — tracing 审计**
- [ ] **Step 1**：`rg -n "fn " src-tauri/src/services/observation_ingest src-tauri/src/services/user_model src-tauri/src/services/push_decider.rs src-tauri/src/commands/user_model.rs src-tauri/src/commands/notification_log.rs`，挑所有 `pub (async )?fn` 看是否挂 `#[tracing::instrument(skip(...))]`。没挂的补；`skip(self, pool, conn)` 这类非 `Debug` 或含敏感数据的参数。
- [ ] **Step 2**：标准事件名检查（§13.3）：
  - `observation.ingested` ✅ PR2
  - `proposition.proposed` ✅ PR4
  - `proposition.revised { op, revision_group }` ✅ PR4
  - `push.decided { outcome, reason?, revision_group }`：本 PR 如 PR6 中是别的名字，统一成此。
  - 补充：`batcher.flushed` / `retrieval.queried { match_bonus, scored_count }` 推荐加；但只有相关文件打开了才动（range control）。
- [ ] **Step 3**：commit `chore(tracing): normalize event names + instrument gaps`.

**Task 7.2 — println! 扫荡**
- [ ] **Step 4**：
  ```bash
  rg -n "println!|eprintln!" src-tauri/src/services/observation_ingest src-tauri/src/services/user_model src-tauri/src/services/push_decider.rs src-tauri/src/commands/user_model.rs src-tauri/src/commands/notification_log.rs src-tauri/src/providers/llm
  ```
  应当 0。若非 0，每处替换 `tracing::debug!` / `info!` / `error!`。commit `chore: replace println with tracing in new modules`.

**Task 7.3 — 魔法数字复核**
- [ ] **Step 5**：
  ```bash
  rg -n "\\bconst\\s+\\w+\\s*:\\s*(u8|u16|u32|u64|usize|f32|f64|Duration)" src-tauri/src/services/observation_ingest src-tauri/src/services/user_model src-tauri/src/services/push_decider.rs
  ```
  除 `tracing::instrument` 默认参数外不应有任何硬编码业务数值。若有，迁到 `Config`。commit `chore(config): move magic numbers into Config`.

**Task 7.4 — dead code 最终扫荡**
- [ ] **Step 6**：`cargo machete` → 0 unused。
- [ ] **Step 7**：`pnpm knip` → 0 unused。
- [ ] **Step 8**：`rg -n "supermemory|attention_items|attention_item_evidence|attention_item_events|extracted_facts|conflict_alerts|canonical_key|memory_service|mvp_pipeline|MvpPipeline|SupermemoryConfig|notification_decision_log" src/ src-tauri/src/` → 0 命中。
- [ ] **Step 9**：commit `chore: final dead-code sweep (§13.6)`.

**Task 7.5 — IPC 表面 diff**
- [ ] **Step 10**：手动读 `src-tauri/src/lib.rs::generate_handler!` 块 vs `src/lib/tauri.ts` vs spec §7.4 的命令清单。三边一致。若有漂移，收敛到 §7.4。commit `chore(ipc): verify Tauri surface matches §7.4`.

**Task 7.6 — 文档同步**
- [ ] **Step 11**：`CLAUDE.md` 架构一节改写：services 列表 / IPC 入口 / 数据库章节全部同步到新世界。此改动应**只触碰 CLAUDE.md 里本次反正要改的段落**（架构 / 数据库 / 关键服务）——onboarding / tauri 基础设施说明保留原状。commit `docs(claude-md): sync architecture to GUM refactor`.

**Task 7.7 — 全量绿棒**
- [ ] **Step 12**：
  ```bash
  cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings && cargo machete
  cd .. && pnpm build && pnpm test && pnpm knip
  ```
  全 0。
- [ ] **Step 13**：冷启动 `pnpm tauri dev`，人工冒烟：capture → observation → batcher flush → propositions 表新行 → overlay push → dismiss 回写。
- [ ] **Step 14**：打 release tag / changelog。commit `chore: PR7 final green bar`.

### 7.4 §13.8 完整 DoD（本 PR 必须 10/10）

```
[x] cargo test                         # 全通过（含 REVISE 4 分支 + 推送去重 + prompt 快照）
[x] cargo clippy --all-targets -- -D warnings
[x] cargo machete                      # 输出为空
[x] pnpm build                         # tsc 零报错
[x] pnpm test                          # Vitest 全通过
[x] pnpm knip                          # 或 tsc --noUnusedLocals，零遗漏
[x] rg -n "supermemory|attention_items|extracted_facts|conflict_alerts|canonical_key|memory_service" src/ src-tauri/src/   # 零命中
[x] 应用冷启动后 schema_version = 5
[x] Tauri IPC 表面与 §7.4 完全一致（手动 diff lib.rs 的 generate_handler! 宏）
[x] 新代码目录（observation_ingest / user_model / push_decider）内无 println! / eprintln!
```

任意一项 failed → 不 merge。

---

## 附录 A：§12 可验证里程碑到 PR 的映射

| 里程碑 | 覆盖 PR |
|-------|---------|
| 1. REVISE merge/update/contradict/rewrite 各 1 条测试 | PR4 |
| 2. Dev 30min 后 `propositions` 非空、obs 覆盖 ≥ 80% | PR5 冒烟 |
| 3. `user_model_query("我在 Notion 里写什么")` 返回命题 + 打分可解释 | PR5 冒烟（tracing: `retrieval.queried`） |
| 4. 连续两次触发第二次 `Skip::WithinCooldown` | PR6 集成测试 |
| 5. Dismiss 后 7 天内同 group 不再 push | PR6 集成测试 |
| 6. `schema_version = 5` 且旧表消失 | PR1 migration smoke test |
| 7. CI 红线（clippy / machete / knip） | 每个 PR + PR7 终验 |
| 8. rg 扫描 0 命中 | PR1 起每 PR 复查 |

## 附录 B：Self-Review checklist（写完计划后我自己跑过）

1. **Spec 覆盖**：§1 / §1.1 / §3 / §4 / §5 / §6 / §7（全节）/ §8 / §9 / §10 阶段 1 / §11（作为风险背景消化在 PR 边界里）/ §12 / §13 — 每条都有落地 PR。§10 阶段 2 / 3（调优、FTS5）**不在本计划**，这是 spec 明示的本次范围外。
2. **Placeholder 扫描**：无 "TBD" / "implement later"；每个 step 给了测试骨架或具体 SQL / 代码块。
3. **类型一致性**：`RevisionOp` 的 4 variant、`ScoredProposition` / `Proposition` / `Observation` / `TauriError` / `PushDecision` / `SkipReason` 跨 PR 同名同字段。
4. **硬约束**：魔法数字→Config / 禁 println! / 无兼容层 / 无 FTS5 / §13 范围受控——每个 PR 都在 DoD 里点到。
