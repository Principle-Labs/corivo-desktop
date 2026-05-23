# Corivo × GUM 改造 Spec

> 参考：`generalusermodels/gum`（MIT）+ 论文 *Creating General User Models from Computer Use* (arXiv:2505.10831)。
> 范围：仅讨论架构与数据层的改造方向，不包含实施代码。

---

## 1. GUM 架构要点（我们要吸收的部分）

GUM 只有四个核心概念：

| 概念 | 含义 | 仓库对应 |
|------|------|----------|
| **Observer** | 从系统里源源不断采集 *unstructured observation* 的模块。屏幕观察器是默认实现：鼠标事件触发 before/after 截图 → Vision LLM 转录 + 摘要 → 入队。 | `gum/observers/screen.py` |
| **Observation** | 原始的文本化观察。只有 `observer_name / content / content_type / created_at`。 | `gum.models.Observation` |
| **Proposition** | 置信度加权的用户画像命题。字段：`text, reasoning, confidence(1-10), decay(1-10), revision_group, version`。多对多关联到 observations。 | `gum.models.Proposition` |
| **Revision pipeline** | `PROPOSE → SIMILAR → REVISE` 三段 LLM 调用：先从最新批次观察里产出命题草稿 → 用检索（v1 LIKE / v2 BM25）召回已有命题 → 让 LLM 判断 IDENTICAL / SIMILAR / UNRELATED → 对 SIMILAR 聚类做 **merge / update / contradict / rewrite** 四种修订之一（展开见 §1.1）。 | `gum/gum.py::_process_batch` |

其他值得抄的设计决策：

- **批处理**：观察先进 `ObservationBatcher` 排队，达到 `min_batch_size` 才触发一次 LLM，减少 API 抖动。
- **检索**：SQLite + FTS5 + 自写 BM25 + 可选 MMR + 按 `decay × 天数` 指数衰减的 score。*没有* embedding。（我们的第一版暂不抄 FTS5，见下文第 4、6、7.3 节。）
- **命题版本化**：同一簇的修订**沿用原 `revision_group`**、`version` 递增；旧 row 保留为历史 version，当前 version 就是 `MAX(version)`。这样 UI 能直接渲染"同一件事被校准的历次过程"（§9）。
- **隐私 Audit**：可选的前置 LLM 门，决定当前观察是否允许进入推理链。
- **Proactive 不在核心包里**：GUM 本体只暴露 `query(user_query) → [(Proposition, score)]`，主动建议是上游应用行为（论文里是 chat/2）。
- **`revision_group` 是语义 ID**：SIMILAR/REVISE 把"说的是同一件事"的命题合并到同一 `revision_group`（多个 `version`）。**这比字符串级的 `canonical_key` 稳得多，是 Corivo 做推送去重的天然基准**，见 7.5。

### 1.1 ⭐ 第 5 层：Revise（修订）——为什么 GUM 不是 User Log

如果把 GUM 展开成 5 层 pipeline（**Observer → Observation → Propose → Retrieve → Revise**），**Revise 是唯一让 GUM 配得上 "General User Model" 而不是 "User Log" 的一层**。

新命题生成后，**不是直接 append**，而是：

1. 用 Retrieve 拿到相关的旧命题；
2. 把旧命题 + 新命题 + 双方的 `reasoning` + `grounding`（关联的 observations）+ `decay` 全部打包喂给 LLM；
3. LLM 对每个簇做下列操作之一：

| 操作 | 含义 |
|---|---|
| **合并 (merge)** | 把多条表达相似意思的命题合并成一条更精确的 |
| **更新 (update)** | 保留命题文本，但调整 `confidence` / `reasoning` / `decay` |
| **标记矛盾 (contradict)** | 新证据与旧命题冲突时两者都保留，但记录对立关系 |
| **重写 (rewrite)** | 把旧命题改写成更准确的版本 |

修订过程还会**重新生成 `confidence` 和 `decay`**——也就是说命题的元数据会随着证据累积不断被重新校准，而不是一次写死。

**论文评测里的关键发现**：消融实验显示，**去掉 Retrieve 或 Revise，GUM 的质量会显著下降**。这说明"持续修订"不是锦上添花，而是核心——没有修订的话，GUM 就退化成一个带置信度的流水账 log。

**对 Corivo 的直接含义**：

- REVISE 不是可选优化，是本次改造的价值支点。任何"跳过 REVISE 先上线看看"的想法都等于把 GUM 降级回 user log。
- `push_decider` 基于 `revision_group` 做去重的前提，就是 REVISE 真的把"同一件事"合并进了同一 group；REVISE 质量差 → §11 风险表里那条"推送去重被穿透"就会发生。
- `confidence / decay` 随 `version` 演进是**预期行为**，不是 bug。前端 knowledge 页要能展示同一 `revision_group` 的历史版本链，让用户直观看到画像是怎么被校准的（§9 新增要求）。
- 评测时的守门线：`cargo test` 至少覆盖 merge / update / contradict / rewrite 四种 REVISE 分支各一条用例（§12 补充里程碑）。

---

## 2. Corivo 现状与 GUM 的映射

| GUM | Corivo 现状 | 差距 |
|-----|------------|------|
| Observer | `services/capture_loop.rs` + `capture_store.rs`：定时截图 + idle 快捷键，产物是成批的 PNG/JPG。 | 观察产物**不是文本化的 observation**，而是原图；没有 "observation 入队" 的抽象。 |
| Observation store | 无。截图元数据记录在 `screenshots` 表，**转录结果没有单独落库**，直接被 `mvp_pipeline` 喂给 summary prompt。 | 无法被其他推理重用，只能一次性消耗。 |
| PROPOSE | `services/mvp_pipeline.rs` 里的 `SUMMARY_PROMPT`（时间线）+ `idle_shortcut::assess_idle_batch`（attention judgment）。 | 产出的是 "detailed summary" 和 attention item，不是抽象的用户命题。 |
| SIMILAR / REVISE | `db/repos/attention_items.rs` 用 `canonical_key` 做去重，`notification_decision_log` 记录判断。 | 近似 GUM 的簇合并，但只对 attention item 一种类型生效；用户长期偏好/知识没有被建模。 |
| 检索 | Supermemory（外部向量库）+ 本地 `attention_items` 查询。 | 所有长期知识都在外部服务；离线、审计、替换 provider 都受制。论文本质是 "local-first 用户模型"。 |
| 主动通知 | `notification_service` + `notification_decision_log` + Dynamic Island overlay。 | 这是 Corivo 的差异化，可保留；但目前触发源只有 attention item。接入 proposition 后可以扩展。 |

一句话：**Corivo 已经有 Observer 和 Proactive 两端，中间缺的是 GUM 的 "proposition 层"**。

---

## 3. 改造目标

1. 在本地 SQLite 内建立 **observation / proposition 双表 + 多对多关联**，作为用户画像的权威存储。**第一版不建 FTS5**（中文默认分词效果差，tokenizer 选型未定），预留后续迁移位。
2. 把 `capture_loop` 的输出规范化为 *文本化 observation*（转录 + 摘要），写入 observation 表后再触发批处理，而不是每批都立即发给 summary prompt。
3. 引入 PROPOSE / SIMILAR / REVISE 三段式 pipeline，以**命题**作为用户画像和推送判断的唯一一等公民；**独立的 "attention item" 概念整体移除**——以前由 attention item 承担的"值得推送"职责，拆成两部分：推什么由命题的 `confidence` / `decay` + `user_model_query` 决定，什么时候不推由 `push_decider` + `notification_log` 决定（见 §7.5）。
4. `query(text) → [(Proposition, score)]` 作为统一召回接口；`notification_service`、前端记忆/知识页都从这个接口拿数据。
5. **彻底退役 Supermemory 与旧记忆/注意力结构**：`attention_items` / `attention_item_evidence` / `notification_decision_log` / `extracted_facts` / `conflict_alerts` / `segments.memory_id` / `services/memory_service.rs` 全部删除，不保留只读回退，不做数据迁移（旧数据**直接丢弃**）。
6. 新增 `notification_log` 表承担"推送过不再重复推"的职责，基于 `revision_group` 做语义层去重 + 冷却窗口做时间层去重（详见 7.5）。

---

## 4. 不做的事（Out of scope）

- 不替换通知 overlay、idle shortcut、config/keychain、updater 等基础设施。
- 不引入 embedding / 向量库。
- **第一版不建 FTS5**。`porter ascii` / `unicode61` 对中文都不理想，tokenizer 选型（`unicode61 tokenchars` vs jieba 预分词）单独决定；在此之前用 `LIKE` + `confidence × decay × recency` 组合排序已经够用。FTS5 作为独立 virtual table 以后随时能补，回填一条 `INSERT INTO xxx_fts(xxx_fts) VALUES('rebuild')` 就行，不影响主表数据。
- **IPC 全面重构，不保留旧命令**：与 Supermemory / `attention_items` / `extracted_facts` / `conflict_alerts` / `notification_decision_log` / `segments.memory_id` 相关的所有 `#[tauri::command]` 和 `lib/tauri.ts` wrapper **一律删除**，不做 deprecated 过渡期、不做兼容 shim。新 IPC 只围绕 `user_model_*` 和 `notification_log_*` 展开。前端调用点（hooks / pages / stores）一次性迁到新命令，旧代码路径同次 PR 清理干净。
- Audit prompt 先跳过，等主链路稳定后再加。

---

## 5. 目标架构

```
┌─────────────┐      ┌────────────────────┐      ┌──────────────────┐
│ capture_loop├─────▶│ observation_ingest │─────▶│ observations (DB)│
└─────────────┘      │  (Vision LLM:      │      └────────┬─────────┘
                     │   transcribe +     │               │ batch ready
                     │   summary)         │               ▼
                     └────────────────────┘      ┌──────────────────┐
                                                  │ proposition_pipe │
                                                  │ PROPOSE → SIMILAR│
                                                  │        → REVISE  │
                                                  └────────┬─────────┘
                                                           ▼
        ┌─────────────┐       query()              ┌──────────────────┐
        │ frontend UI │◀──────────────────────────▶│ propositions (DB)│
        │             │ LIKE + confidence×decay    │ (FTS5 v6 再加)   │
        │             │        × recency            │                  │
        └─────────────┘                             └────────┬─────────┘
                                                             │
                                                             ▼
                                                    ┌──────────────────┐
                                                    │  push_decider    │
                                                    │ (基于 revision_  │
                                                    │  group + 冷却)   │
                                                    └────────┬─────────┘
                                                             │
                                  ┌──────────────────────────┼─────────────┐
                                  ▼                          ▼             ▼
                         ┌──────────────────┐       ┌──────────────────┐   │
                         │ notification_log │◀──────│ notification_svc │──▶│ overlay
                         │ (去重+冷却证据)  │       │ (推送下发)       │   │
                         └──────────────────┘       └──────────────────┘   │
```

---

## 6. 数据库改造（`src-tauri/src/db/schema.sql`）

**v5（本次要做的）**：新增两张表 + 关联，仅此而已。**不建 FTS5 virtual table，不建 triggers**。

```sql
-- Observation: 一条文本化观察
CREATE TABLE observations (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  observer_name TEXT NOT NULL,         -- e.g. "screen"
  content TEXT NOT NULL,               -- transcription + summary 的结果
  content_type TEXT NOT NULL,          -- "input_text" | "input_image"
  source_session_id TEXT,              -- 关联到既有 sessions
  source_screenshot_id INTEGER,        -- 关联到既有 screenshots
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Proposition: 置信度加权的命题
CREATE TABLE propositions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  text TEXT NOT NULL,
  reasoning TEXT NOT NULL,
  confidence INTEGER,                      -- 1..10
  decay INTEGER,                           -- 1..10，越大越持久
  revision_group TEXT NOT NULL,            -- UUID，用于追溯合并链
  version INTEGER NOT NULL DEFAULT 1,      -- 每次 REVISE +1，同一 revision_group 内递增
  contradicts_proposition_id INTEGER,      -- REVISE 的 contradict 分支使用，互指另一条命题
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  FOREIGN KEY (contradicts_proposition_id) REFERENCES propositions(id) ON DELETE SET NULL
);
CREATE INDEX idx_propositions_revision_group ON propositions(revision_group);
CREATE INDEX idx_propositions_revision_version ON propositions(revision_group, version DESC);
CREATE INDEX idx_propositions_created_at ON propositions(created_at DESC);
CREATE INDEX idx_propositions_confidence ON propositions(confidence DESC);

-- 多对多
CREATE TABLE observation_proposition (
  observation_id INTEGER NOT NULL,
  proposition_id INTEGER NOT NULL,
  PRIMARY KEY (observation_id, proposition_id),
  FOREIGN KEY (observation_id) REFERENCES observations(id) ON DELETE CASCADE,
  FOREIGN KEY (proposition_id) REFERENCES propositions(id) ON DELETE CASCADE
);
CREATE INDEX idx_obs_prop_proposition ON observation_proposition(proposition_id);

-- 通知推送日志（承担"推过不再重复推"的职责；见 7.5）
CREATE TABLE notification_log (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  revision_group  TEXT    NOT NULL,        -- 跨 REVISE 稳定的语义 ID
  proposition_id  INTEGER,                  -- 推送时命题的具体 version（审计；命题删除后置空）
  pushed_at       TEXT    NOT NULL DEFAULT (datetime('now')),
  outcome         TEXT,                     -- NULL / 'acknowledged' / 'dismissed' / 'ignored'
  outcome_at      TEXT,
  FOREIGN KEY (proposition_id) REFERENCES propositions(id) ON DELETE SET NULL
);
CREATE INDEX idx_notif_group_time ON notification_log(revision_group, pushed_at DESC);
```

**同一迁移里 drop 旧表**（旧数据直接丢弃）：

```sql
DROP TABLE IF EXISTS attention_items;
DROP TABLE IF EXISTS attention_item_evidence;
DROP TABLE IF EXISTS notification_decision_log;
DROP TABLE IF EXISTS extracted_facts;
DROP TABLE IF EXISTS conflict_alerts;
-- segments.memory_id 列通过 ALTER TABLE 去掉（SQLite 3.35+ 支持 DROP COLUMN）
ALTER TABLE segments DROP COLUMN memory_id;
```

迁移：作为 `migrations.rs` 的 version 5，`schema_version = 5`；一次事务里完成建表 + drop；不保留任何回退路径。

**v6（预留，不在本次范围）**：等 tokenizer 选型定了再加，届时一条迁移脚本搞定：

```sql
CREATE VIRTUAL TABLE propositions_fts USING fts5(
  text, reasoning, content='propositions', content_rowid='id',
  tokenize='unicode61'          -- 或 'unicode61 tokenchars ""' / jieba 预分词
);
CREATE VIRTUAL TABLE observations_fts USING fts5(
  content, content='observations', content_rowid='id',
  tokenize='unicode61'
);
-- 一次性回填（对 external content 表是官方支持的操作）
INSERT INTO propositions_fts(propositions_fts) VALUES('rebuild');
INSERT INTO observations_fts(observations_fts) VALUES('rebuild');
-- + ai/ad/au triggers（参考 gum models.py::create_fts_table）
```

回填是秒级的本地 IO，不涉及 LLM 重算，风险极低。

**为什么现在不加**：中文 FTS5 的痛点不在索引本身，而在分词器。`porter ascii` 把中文当单字符串；`unicode61` 默认按 codepoint 切，对中日韩会退化成近似单字索引；真要好用得接 jieba 做 pre-tokenize（把 `content` 提前切好空格分隔存进去）。这些决定都需要真实数据观察，提早定反而容易返工。

---

## 7. Rust 模块改造

### 7.0 `services/mvp_pipeline.rs` 彻底拆分并改名

`mvp_pipeline.rs` 是 MVP 阶段的产物，它把"截图批次 → LLM 摘要 → push judgment → attention 解析 → memory 存储 → notification"一把梭在一个文件里。**改造后这个文件整体删除**，"MVP" 命名一并废弃（项目已经过了 MVP 阶段）。原来那条流水线按职责拆成若干**互不耦合**的模块，通过 channel / event 串起来：

| 旧 `mvp_pipeline` 里的步骤 | 拆分后落点 | 说明 |
|---|---|---|
| 截图批次拼装 | `services/capture/` | `capture_loop.rs` 原地保留，只负责采集 |
| 转录 + 时间线摘要 | `services/observation_ingest/` **（新）** | 把截图批次转成文本化 observation 写入 DB |
| PROPOSE / SIMILAR / REVISE | `services/user_model/proposition_pipeline.rs` | 见 §7.2 |
| push judgment | `services/push_decider.rs` | 见 §7.5 |
| attention 解析 | 删除 | 概念本身取消，见 §3 目标 3 |
| memory 存储 | 删除 | Supermemory 整体退役 |
| notification 下发 | `services/notification_service` | 原文件保留，入参改自 proposition |

拆完之后 `services/` 目录大致是：

```
src-tauri/src/services/
├── capture/                  # 原 capture_loop (不动，只改对外事件 shape)
├── observation_ingest/       # 新：vision LLM 转录 + 摘要 → observations 表
│   ├── mod.rs               # 协调器：订阅 capture 事件，产出 observation
│   ├── transcriber.rs       # vision LLM transcription（对应 GUM TRANSCRIPTION_PROMPT）
│   ├── summarizer.rs        # 时间线摘要（沿用原 SUMMARY_PROMPT 的中文提示）
│   └── writer.rs            # 写 observations 表，关联 sessions/screenshots
├── user_model/               # 见 §7.1–§7.4
├── push_decider.rs           # 见 §7.5
├── notification_service/     # 保留，入参从 AttentionItem 改成 Proposition
├── notification_overlay_geometry.rs  # 不动
└── ... (config_service / keychain_service / updater 等基础设施不动)
```

**边界约定**：
- `observation_ingest` **不认识 proposition**；它只往下发 `ObservationReady { id }` 事件。
- `user_model` **不认识 screenshot/session**；它只消费 `observation_id`。
- `push_decider` **不认识 observation**；它只拿 `Proposition` 和 `notification_log` 做决策。
- `notification_service` **不认识 LLM**；它只接收已经决策过的"该推什么、payload 是什么"。

这样每一层都能单独单测（mock 上游 channel + 下游 repo 即可），LLM 调用集中在 `observation_ingest` + `user_model`，其他模块完全无 LLM 依赖。

`user_model/` 的内部结构：

```
services/user_model/
├── mod.rs                   # UserModel: 对外唯一入口 (query / ingest / recent)
├── observer.rs              # Observer trait + Update struct
├── screen_observer.rs       # 把 observation_ingest 的事件封装成 Observer 输入
├── batcher.rs               # 等价于 gum.batcher.ObservationBatcher
├── proposition_pipeline.rs  # PROPOSE / SIMILAR / REVISE 三段
├── prompts.rs               # 中文化 PROPOSE / SIMILAR / REVISE
└── retrieval.rs             # v1: LIKE + confidence×decay×recency；v2 再换 BM25
```

新增 repos：`db/repos/observations.rs`、`db/repos/propositions.rs`、`db/repos/notification_log.rs`。

### 7.1 `Observer` trait

```rust
#[async_trait]
pub trait Observer: Send + Sync {
    fn name(&self) -> &'static str;
    fn subscribe(&self) -> mpsc::Receiver<Update>;
}

pub struct Update {
    pub content: String,
    pub content_type: ContentType, // Text / ImageRef
}
```

`screen_observer.rs` 不自己采屏，也不再调用 LLM——它**只消费 `observation_ingest` 已经写好的 observation**：
- 订阅 `ObservationReady { id }` 事件 → 从 `observations` 表取对应记录 → 封装成 `Update { content: observation.content, content_type: Text }` → 送入 `batcher`。
- LLM 调用（转录 + 摘要）在上游 `observation_ingest` 里完成；`user_model` 这侧不关心 observation 的来源，只要是合法的文本化 observation 都能喂进 pipeline（未来加 audio / clipboard observer 也走同一条路）。

### 7.2 `proposition_pipeline.rs`

对应 `gum.py::_process_batch`：

1. `generate_and_search(batch_observations)`：拼 PROPOSE prompt，得到 draft `PropositionItem[]`；对每个 draft 用 `retrieval.rs::query()` 做召回（v1 走 LIKE + 加权打分，v2 才换 BM25，见 §7.3），组成 `pool`。
2. `filter(pool)`：拼 SIMILAR prompt，解析为 `RelationSchema`，切成 `identical / similar / unrelated`。
3. `handle_identical`：把当前 batch observation 挂到已有 proposition（`INSERT OR IGNORE`）。
4. `handle_similar`：收集簇内所有关联 observation（grounding）+ 当前 batch + 旧命题的 `text / reasoning / confidence / decay` → REVISE prompt → LLM 输出 `RevisionOp` 之一：
   - `merge`：把多条旧命题 + 新草稿合并成**一条新 row** 作为 current version（`version = MAX(簇内旧 version) + 1`）；**旧 row 全部保留**为历史 version，不做物理删、不需要软删列。若簇内命题原本跨多个 `revision_group`，统一收敛到簇内最老的那个 `revision_group`（被并入的 row 更新 `revision_group`，但 `version` 保持原值），新 row 的 `confidence / decay` 由 LLM 重算后写入。
   - `update`：命题文本保持不变，写一条新 row，只覆盖 `confidence / reasoning / decay`；`version += 1`，`revision_group` 不变；旧 row 保留。
   - `contradict`：新旧命题**分属两条独立 row**、各自持有自己的 `revision_group`，通过 `contradicts_proposition_id` 互指；两者都会是各自 `revision_group` 的 current version，前端并排展示供用户仲裁。
   - `rewrite`：写一条新 row 携带改写后的文本，沿用 `revision_group`，`version += 1`；旧 row 保留作为历史版本，不挂新 observation。

   "current version" 全局用 `MAX(version) per revision_group` 一把判断，不需要额外的 `is_active` / `deleted_at` 列。四种分支在 §12 里程碑 1 里都有独立测试用例。
5. `handle_different`：作为全新命题入库，挂当前 batch observation。

LLM 调用走 `providers/llm/`（当前是 Gemini）。JSON schema 用 `serde_json` 的 struct 约束，prompt 末尾声明 `response_format`（Gemini structured output）。

### 7.3 `retrieval.rs`

对外接口与 GUM 对齐：

```rust
pub async fn query(
    text: Option<&str>,
    limit: u32,
    start_time: Option<DateTime<Utc>>,
    end_time: Option<DateTime<Utc>>,
    include_observations: bool,
) -> Result<Vec<ScoredProposition>>;
```

**v1 实现（不依赖 FTS5）**：

1. **关键词过滤**（`text` 非空时）：把查询按空白切成若干 token（中文场景基本就是整串），对每个 token 在 `text` / `reasoning` 上做 `LIKE '%token%'` AND 组合。中文 substring match 天然不需要分词，命中率对 Corivo 的量级够用。
2. **时间窗过滤**：用 `created_at` 配合可选的 `start_time` / `end_time`。
3. **打分排序**：在 SQL 里一把排完，避免回拉全表：
   ```
   score = confidence * w_c
         + decay      * w_d * exp(-age_days / K_DECAY)
         + match_bonus                     -- token 命中数，LIKE 时加
   ```
   `w_c / w_d / K_DECAY` 放 config，先写死再调。
4. 取前 `limit * 3` 回到 Rust 侧做去重（同 `revision_group` 只保留最新 `version`），截断到 `limit`。
5. `include_observations=true` 时 join `observation_proposition` 拼关联 observation。

**v2 实现（加 FTS5 之后）**：把第 1 步换成 `MATCH` 查询 + `bm25(propositions_fts)` 排序，再把归一化后的 BM25 score 替换上式里的 `match_bonus`。**接口签名不变，调用方零改动**。

MMR 在 v1 先不上——命题数量到几千条之前冗余可控；真正需要多样性时再加。

### 7.4 Tauri 命令

**删除全部旧命令**（与 Supermemory / attention / facts / conflict_alerts 相关的 `#[tauri::command]` 一并清掉，不留别名、不加 deprecated 注解）。

**新命令（这就是全部 IPC 表面）**：

```rust
// user model 读路径（对齐 GUM README chat/2 demo 的 API）
#[tauri::command] async fn user_model_query(text: String, limit: u32) -> Result<Vec<ScoredProposition>>;
#[tauri::command] async fn user_model_recent_propositions(limit: u32) -> Result<Vec<Proposition>>;
#[tauri::command] async fn user_model_recent_observations(limit: u32) -> Result<Vec<Observation>>;
#[tauri::command] async fn user_model_proposition_observations(id: i64) -> Result<Vec<Observation>>;
#[tauri::command] async fn user_model_versions_in_group(group: String) -> Result<Vec<Proposition>>; // 为 Knowledge 页版本链 UI 补充

// 推送决策与日志
#[tauri::command] async fn notification_log_recent(limit: u32) -> Result<Vec<NotificationLogEntry>>;
#[tauri::command] async fn notification_log_set_outcome(id: i64, outcome: String) -> Result<()>;
```

前端侧 `lib/tauri.ts` 同次 PR 里把对应的旧 wrapper 整体删除，hooks / pages / stores 里调用旧命令的地方一次性迁到上面这份列表，不允许"先留着旧的、等后面再删"。

### 7.5 `push_decider` & 推送去重

目标：替代原 `attention_items` + `notification_decision_log` 那套"canonical_key 去重"的脆弱方案，改成基于 `revision_group` + `notification_log` 的两层去重。

新增模块 `services/push_decider.rs`：

```rust
pub struct PushConfig {
    pub base_cooldown:    Duration,   // 默认 24h：同一 revision_group 推过后的静默期
    pub dismiss_penalty:  Duration,   // 默认 7 天：用户 dismiss 后的加长静默期
    pub min_confidence:   u8,         // 低于此置信度的命题不参与推送
}

pub enum PushDecision { Push, Skip(SkipReason) }

pub enum SkipReason {
    WithinCooldown { last_pushed_at: DateTime<Utc> },
    DismissPenalty { dismissed_at: DateTime<Utc> },
    BelowConfidence,
}

pub fn decide(prop: &Proposition, log: &NotificationLogRepo, cfg: &PushConfig) -> PushDecision;
```

**`notification_log` 的双锚点设计**：`revision_group` 是 **NOT NULL** 的主锚点（语义簇 ID，跨 REVISE 稳定），`push_decider` 的冷却/惩罚查询全部走这个字段；`proposition_id` 是 **nullable** 的副锚点（指向当时推送的那个具体 `version`），只用于审计/调试/历史页。之所以不能只留 `proposition_id`，是因为 REVISE 会不断产出新 `version`——如果用 `proposition_id` 做去重 key，每次 revision 都会查不到旧记录、重复推送；`revision_group` 保持稳定才能命中冷却。`ON DELETE SET NULL` 是防御性的，给未来"清理老 propositions"命令留出余地，不让 log 跟着断裂。

**去重逻辑**（一句话版）：

> 候选命题若其 `revision_group` 在 `base_cooldown` 内已被推送过，或最近一次 `outcome='dismissed'` 距今不足 `dismiss_penalty`，则跳过；否则推送并写一条 `notification_log`。

**触发源**：`proposition_pipeline` 每次产出新/修订命题 → 发 channel 事件 → `push_decider.decide()` → 通过就调 `notification_service`。不再由 idle shortcut 直接生成 attention item。

**outcome 回填**：overlay 的 "acknowledge" / "dismiss" 交互通过既有 Tauri event 回流，`notification_service` 写 `outcome / outcome_at`。

**配置暴露**：`PushConfig` 三个参数进 `domain::config::Config`，前端设置页可调。

**为什么不再需要 `canonical_key`**：REVISE 已经保证了"同一件事 → 同一 `revision_group`"；而字符串级 key 面对"用户换个说法、LLM 措辞微调"就会穿透。`revision_group` 是 GUM pipeline 自带的、经过 LLM 语义判断的稳定 ID，天然更稳。

---

## 8. Prompt 对应

保留 Corivo 的时间线摘要（用户体验友好）作为 **observation 内容**，再叠加 GUM 的命题 prompt。

| 用途 | 来源 | 备注 |
|------|------|------|
| 观察转录 | `gum/prompts/screen.py::TRANSCRIPTION_PROMPT` | 中文化，强调包含应用名、文件名、URL |
| 观察摘要 | Corivo 现有的 `SUMMARY_PROMPT`（时间线格式） | 作为 observation 的 `content`；也用于前端时间线 |
| PROPOSE | `gum/prompts/gum.py::PROPOSE_PROMPT` | 中文化；至少 5 条命题；`confidence/decay` 评分维持 1-10 |
| SIMILAR | 同上 `SIMILAR_PROMPT` | 输出严格 JSON；用 Gemini structured output 强制 |
| REVISE | 同上 `REVISE_PROMPT` | 输出严格 JSON 的 `RevisionOp`：**merge / update / contradict / rewrite** 四选一；每种 op 的行为与落库方式见 §7.2 step 4 / §1.1 |

---

## 9. 前端改造点

**一次性重写，不保留兼容层**：

- `lib/tauri.ts`：删光与 Supermemory / attention / facts / conflict_alerts 相关的 wrapper；按 §7.4 新命令列表重新生成一套对外 API。
- `hooks/`、`stores/`、`pages/` 里所有调用旧 wrapper 的地方全部迁到新 API，**同一 PR 内完成**，禁止留"暂时双跑"的分支。
- **重写的页面**：
  - 记忆/知识页（原来可能基于 Supermemory 渲染）改成 Propositions 浏览页，按 confidence × recency 排序，展示 `reasoning`、关联 observation 列表，以及**同一 `revision_group` 的版本链**——每次 REVISE 产生的新 `version` 都可点开对比，让用户看到命题的 `text` / `confidence` / `decay` 是如何被校准的（对应 §1.1 的要求）。
  - notification 相关的调试/历史页改读 `notification_log`。
- `overlay` 窗口壳（位置/动画/尺寸）保留，但内部渲染和事件 payload 按 §7.5 重写：数据源是 `proposition_id`，acknowledge / dismiss 写 `notification_log.outcome`。
- 所有遗留的"memory_id / attention_id / canonical_key"类型定义、枚举、props 全部删除，`lib/types.ts` 里的对应 interface 连同 import 一起清理。

---

## 10. 迁移路径（分阶段，每阶段可单独发布）

因为**旧数据全部丢弃**，不再需要"并行运行 + 读路径先切 + 写路径后切"的逐步迁移。整个改造可以作为一次破坏性升级一次性完成。

**阶段 1 — 破坏性升级（一次性发布，dev feature flag 保护）**
- `migrations.rs` v5 一次事务内完成：建 `observations` / `propositions` / `observation_proposition` / `notification_log` + drop `attention_items` / `attention_item_evidence` / `notification_decision_log` / `extracted_facts` / `conflict_alerts` + `ALTER TABLE segments DROP COLUMN memory_id`。
- 删除 `services/memory_service.rs` 对 Supermemory 的引用（文件可整体删）；前端 `lib/tauri.ts` 里对应 invoke 清掉。
- 上线 `user_model/` 模块：`screen_observer` → `batcher` → `proposition_pipeline` → `propositions` 表。
- 上线 `push_decider` + `notification_log`：通知触发源完全切到命题事件，不再走 `attention_items` / `idle_shortcut::assess_idle_batch`。
- `idle_shortcut` 只保留"主动采集触发器"的职责（驱动 `capture_loop` 生成一批截图），不再自己做 attention 判断。

**阶段 2 — 质量调优（上线后 1~2 周）**
- 采集 `notification_log.outcome` 的真实分布（acknowledged / dismissed / ignored 比例），反向调 PROPOSE prompt、`confidence/decay` 阈值、冷却窗口默认值。
- 人工抽检 REVISE 的 4 种 outcome 在真实命题上的判断质量（尤其是 merge vs contradict 的边界），必要时修 `REVISE_PROMPT` 的 few-shot。
- 观察 `LIKE` 召回延迟和命中质量，决定是否触发下一阶段（阶段 3 补 FTS5）。

**阶段 3（可选）— 补 FTS5（对应 schema v6）**
- 前置条件：v1 的 `LIKE` 召回在真实数据上出现瓶颈（慢查询 / 召回漏）**或者**需要更丰富的排序（同义词、词组权重）。
- tokenizer 先选型：优先尝试 `unicode61 tokenchars`；若仍不理想，引入 jieba 在写入侧预分词（把空格分隔的 tokens 存到 `content_tokens` 辅助列里，FTS5 索引辅助列）。
- 迁移：新建两个 `_fts` virtual table + triggers，执行 `INSERT INTO xxx_fts(xxx_fts) VALUES('rebuild')` 回填。
- `retrieval.rs` 切到 v2 路径；因为接口签名未变，调用方（`user_model_query` / 前端）不需要改动。

---

## 11. 风险与取舍

| 风险 | 说明 | 缓解 |
|------|------|------|
| **LLM 成本翻倍** | 每批观察现在要多调 PROPOSE / SIMILAR / REVISE。 | 上 `ObservationBatcher`（min batch ≥ 5）；SIMILAR 输入只放 BM25 top-k；开启 Gemini structured output 减少重试。 |
| **中文全文检索质量** | 不引入 FTS5 的代价是 v1 用 `LIKE` 做召回，长查询/长命题性能有上限。 | Corivo 量级（单机 <10 万条命题）`LIKE` + 索引过滤完全够用；观察到召回瓶颈再走 v6 迁移补 FTS5，届时再决定 tokenizer（`unicode61 tokenchars` / jieba 预分词）。 |
| **命题漂移** | REVISE 可能过度重写。 | 保留 `revision_group` + `version`，UI 支持回溯；加 nightly 评测脚本跑 few-shot 回归。 |
| **隐私** | 本地库变大、信息更集中。 | 预留 Audit prompt 位；UI 里允许按 observer/时间段批量删除；SQLite 走 `$APPDATA`，跟随系统磁盘加密。 |
| **破坏性升级，旧数据全丢** | v5 迁移直接 drop 旧表、清 Supermemory 依赖；升级后用户的历史 attention / memory 归零。 | 当前仍在自用/dev 阶段，接受代价；发布前在 changelog 明确告知；提供"一键 reset"作为兜底（反正旧数据也要扔）。 |
| **推送去重依赖 REVISE 合并质量** | 如果 SIMILAR 判得不准，同一件事被写成两个 `revision_group`，会绕过 `notification_log` 冷却。 | 命题数量小时人工抽检 SIMILAR 输出；nightly 跑 few-shot 评测集；必要时加 "`notification_log` 按最近 N 条命题内容做相似度兜底" 的二级去重，但先不上。 |

---

## 12. 可验证的里程碑

1. `cargo test` 新增 `user_model::proposition_pipeline` 的集成测试（用 mock LLM 返回固定 JSON），**分别覆盖 REVISE 的 merge / update / contradict / rewrite 四种分支各至少一条用例**（对应 §1.1 守门线），并断言产生新 `version` 时 `revision_group` 保持一致、`confidence` / `decay` 会被重新写入。
2. Dev 环境运行 30 分钟后，`propositions` 表非空、`observation_proposition` 多对多关系覆盖 ≥ 80% observation。
3. `user_model_query("我在 Notion 里写什么")` 返回的命题包含正确的 app 名称，`confidence × decay × recency` 综合打分可解释（在日志里能看到每项的贡献）。
4. 推送链路端到端：新命题触发 `push_decider.decide()` → 通过则落 overlay 并写 `notification_log`；**对同一 `revision_group` 连续两次触发，第二次返回 `Skip::WithinCooldown`**（集成测试用 mock clock 验证）。
5. overlay 上点 dismiss 后，同一 `revision_group` 在 `dismiss_penalty`（默认 7 天）内不会再被 push，集成测试可覆盖。
6. `schema_version = 5` 生效后，`SELECT name FROM sqlite_master WHERE type='table'` 里不再出现 `attention_items` / `extracted_facts` / `conflict_alerts` / `notification_decision_log`。
7. CI 在 merge 前强制：`cargo clippy --all-targets -- -D warnings` 零 warning、`cargo machete` 输出为空、前端 `pnpm knip`（或 `tsc --noUnusedLocals`）零遗漏。
8. 旧代码痕迹验证：在项目根跑 `rg -n "supermemory|attention_items|extracted_facts|conflict_alerts|canonical_key|memory_service" src/ src-tauri/src/` 返回 0 命中（spec 文档本身的历史引用除外）；`lib/tauri.ts` 只包含 §7.4 列出的 wrapper，手动 diff 无意外。

---

## 13. 工程化与技术债清理

**范围原则**：只重构本次改造**实际触碰到的**文件/模块。判断标准——这个文件因为 GUM 改造反正要打开吗？是 → 顺手优化；否 → 不碰。项目里还有大量未被本次 PR 经过的历史包袱，不在本次范围内，避免战线失控。

### 13.1 类型化错误

- 新增模块一律用 `thiserror` 定义有名错误：`ObservationIngestError` / `PipelineError` / `PushError` / `UserModelError`；不再把一切错误都 `.to_string()` 后塞到 Tauri 返回。
- Tauri 命令的 error 返回统一走 `{ kind: String, message: String, detail?: Value }` 的 tagged union；Rust 端提供 `impl From<XxxError> for TauriError` 映射。
- 前端 `lib/tauri.ts` 对应声明 discriminated union：

  ```ts
  type TauriError =
    | { kind: 'network'; message: string }
    | { kind: 'llm_rate_limited'; message: string; retry_after_ms?: number }
    | { kind: 'pipeline'; message: string; detail?: unknown }
    | { kind: 'unknown'; message: string };
  ```

  hooks 调用点按 `kind` 分支处理，不再到处 `catch (e: any)`。
- 旧模块（`capture_loop` / `keychain_service` / `updater` 等）的 error 类型**不动**，保持原样。

### 13.2 Trait 化外部依赖

- 新代码跨边界的协作者都过 trait：
  - `trait LlmProvider`：`async fn complete_json<T: DeserializeOwned>(prompt, schema) -> Result<T>`
  - `trait ObservationRepo` / `trait PropositionRepo` / `trait NotificationLogRepo`
- 注入方式统一 `Arc<dyn Trait + Send + Sync>`，在 `AppState` 里集中装配。
- 测试用 mock 实现；集成测试不依赖真实 Gemini / 真实 SQLite 文件（用 in-memory SQLite）。
- **不**给简单工具函数硬抽 trait——只对"可能被 mock / 替换 provider"的协作者做。

### 13.3 Tracing 统一

- 引入 / 正确启用 `tracing` + `tracing-subscriber`；本次新写的每个 public 方法加 `#[tracing::instrument(skip(...))]`。
- 事件命名约定（snake_case，`.` 分层）：
  - `observation.ingested { observation_id, source }`
  - `proposition.proposed { count }`
  - `proposition.revised { op: "merge" | "update" | "contradict" | "rewrite", revision_group }`
  - `push.decided { outcome: "push" | "skip", reason?, revision_group }`
- 新代码里**禁止** `println!` / `eprintln!`；debug 信息走 `tracing::debug!`。
- 本次**不**改造老模块的 log 风格，作为独立任务留到后续。

### 13.4 配置集中化

- 所有魔法数字集中到 `domain::config::Config`：
  - `batcher.min_batch_size`、`batcher.flush_interval_ms`
  - `push.base_cooldown`、`push.dismiss_penalty`、`push.min_confidence`
  - `retrieval.w_confidence`、`retrieval.w_decay`、`retrieval.k_decay`、`retrieval.limit_multiplier`
  - `pipeline.similar_pool_size`
- `Config` 派生 `Default`，加载时用 `serde` + 自定义 `validate()` 检查范围（cooldown > 0、权重非负、`min_confidence ∈ [1,10]`）。
- 模块内不再出现 `const MAGIC: u32 = 24;`——所有数字从 `Config` 读。

### 13.5 Prompt 版本化 + 快照测试

- 五个 prompt 落到独立文件，脱离 Rust 字符串：

  ```
  src-tauri/prompts/
  ├── transcription.md
  ├── summary.md
  ├── propose.md
  ├── similar.md
  └── revise.md
  ```

- 每个文件开头以 Markdown 注释形式写 `version: N` + ChangeLog；改 prompt **必须**同时改版本号（code review 明确看得出）。
- Rust 侧用 `include_str!` 读取，提供强类型 format 函数（入参是 struct，不是 ad-hoc `format!`）。
- 用 `insta` crate 为"给定输入 → 拼好的最终 prompt"写 snapshot 测试；误改 prompt 会 CI 红。
- 不做完整 eval / few-shot benchmark——留到阶段 2 质量调优。

### 13.6 死代码清零

同次 PR 内完成：

- **Rust 侧**：`cargo machete` 无输出（`Cargo.toml` 里删 Supermemory 专用依赖等）；整体删除 `services/memory_service.rs`、对应的 `providers/memory/` 实现。
- **前端侧**：`lib/types.ts` 里 `Memory` / `AttentionItem` / `ExtractedFact` / `ConflictAlert` / `canonical_key` 相关 interface 全清；`hooks/use-memories.*`、`hooks/use-memory-list.*` 整文件删；相关 page 文件替换或删除。
- **验证**：项目根跑
  ```bash
  rg -n "supermemory|attention_items|extracted_facts|conflict_alerts|canonical_key|memory_service" src/ src-tauri/src/
  ```
  命中数应为 0。
- 前端跑 `knip`（或 `tsc --noUnusedLocals`）找 import 遗漏。

### 13.7 DB 层规范

- Repos 统一接口形状：`async fn by_id(&self, id) -> Result<Option<T>>` / `async fn recent(&self, limit) -> Result<Vec<T>>` / `async fn upsert(&self, input) -> Result<T>`；一个文件一个 repo。
- **SQL migration 从 Rust 字符串里剥出来**，落到 `.sql` 文件：

  ```
  src-tauri/src/db/migrations/
  ├── 001_initial.sql
  ├── 002_*.sql
  ├── ...
  └── 005_gum_user_model.sql   ← 本次
  ```

  `migrations.rs` 只负责按版本顺序读取 + 记录 `schema_version`。**旧版本的 inline 字符串保持原样不回改**，只把 v5 这样做，作为后续样板。
- 每个新 repo 附带 `#[tokio::test]`-level smoke test，用内存 SQLite 跑增删改查。

### 13.8 Definition of Done（pre-merge 必过）

```
[ ] cargo test                         # 全通过（含 REVISE 4 分支 + 推送去重 + prompt 快照）
[ ] cargo clippy --all-targets -- -D warnings
[ ] cargo machete                      # 输出为空
[ ] pnpm build                         # tsc 零报错
[ ] pnpm test                          # Vitest 全通过
[ ] pnpm knip                          # 或 tsc --noUnusedLocals，零遗漏
[ ] rg -n "supermemory|attention_items|extracted_facts|conflict_alerts|canonical_key|memory_service" src/ src-tauri/src/   # 零命中
[ ] 应用冷启动后 schema_version = 5
[ ] Tauri IPC 表面与 §7.4 完全一致（手动 diff `lib.rs` 的 generate_handler! 宏）
[ ] 新代码目录（observation_ingest / user_model / push_decider）内无 println! / eprintln!
```

任意一项 failed → 不 merge。
