# Event / Task / Project 架构 Spec

> ⚠️ **本 spec 的核心架构（P/T/E 三层）已被 [corivo-architecture-v2-spec.md](corivo-architecture-v2-spec.md) 取代**
> 新方案（task-centric draft-2）砍掉 P/T/E 三层,把 project 降级为 namespace='project' 的可选 tag，task 升级为骨架。
> 本文件保留作为历史决策档案，以及 fact / anchor / reassignment_log / DbInstant 等延续概念的来源。
> 落地节奏受 [design-decisions.md](design-decisions.md) Part 3 的 alpha 用户反馈控制 —— 架构决策成立不等于现在就动手。

> 版本：draft-2（改起点为 V0 + mockup 复核修正）
> 取代：`work-memory-brain-spec.md`（brain 重构的决策主干）
> 基线：`334a5e8`（`feat(replay): implement session-aware attribution replay for existing WorkContexts`）+ cherry-pick `f3bd8a0` 的时间层
> 状态：设计，尚未落代码 —— **在你点头之前，不改一行代码**
> 核心判断：
> - V0 已经有了 Project / project_stream / work_context / focus_session / signals / reconcile 这套结构，实盘之所以失败不是结构问题，是 **prompt 产出画像标签 + project_resolver 用自由命名而非身份锚点** 的问题
> - brain 重构为了换"线程工作台"范式，把 V0 这套整块删掉，换成了 threads/facts/narrator —— 解决了 fact 结构化抽取和 narrator 产出，但**删掉了 Project 层 + signals + 多截图 session**，而这些正是新需求要的
> - 最小代价：**回到 V0，拆掉画像层，把 V0 已有的三层按 Project/Task/Event 重命名和改造，补 identity anchor + 结构化 fact + narrator**，比从 brain 往前加两层更省
> - 从 brain 里 **cherry-pick** 三样东西：`DbInstant` 时间层（纯基础设施）、`fact_extractor` 结构化抽取、`narrator` → 改作 task narrator

---

## 1. 问题陈述

### 1.1 历史路径

- **V0（`334a5e8` 及以前）** 建立了 `propositions → work_contexts → project_streams → projects` + `focus_sessions` 聚合 + `reconcile_log` 审计 + `work_context_signals` 的信号锚点
- **brain 重构（`7e45a66`）** 把画像管道连带 Project/Stream/FocusSession/Signals 一起拆掉，换成 `facts / threads / thread_facts / links / push_events` + narrator
- **现状**：brain 范式下 `/facts` 页面只能看到 `[intent] recall detector query conversion error = 修复 ...` 这种平铺的原子碎片，看不到"在哪个项目推进了什么"

### 1.2 V0 的真实根因（不是结构）

V0 实盘表现差的原因：

1. `propose/revise/similar/score/suggest/work_propose` 产出的是人物画像标签（"用户倾向于 X"），不是事件
2. `project_resolver` 用 **LLM 语义相似度 + 自由命名**，没有身份锚点 → 25 个 wc 全塞进名字相近的 `tarot` project
3. `focus_session.title/summary` 字段是空字符串（聚合层的产出 bug 没修）
4. `work_contexts` 是短 TTL 的中间层，在 Project/Stream 和 observation 之间，其实多余

**结构本身是对的**，改 prompt + 修锚点 + 砍掉画像管道就够。

### 1.3 brain 重构的得失

**得**：
- `facts (kind/subject/attribute/value/unit/confidence)` 的干净结构化抽取
- `narrator` 服务产出 summary / resolution / open_questions_advice
- `DbInstant` + 全局 `Clock` 时间组件

**失**：
- Project 层删光了
- `work_context_signals` 这类信号锚点删光了
- `focus_sessions` 连续 session 聚合删光了
- `project_resolver / stream_resolver / replay / reconcile` 整套归属机制删光了

**失去的正是新需求要的**。brain 的得项（fact / narrator / time）用 cherry-pick 就能保留。

### 1.4 目标形态（以 AI 生图项目为例）

```
Project: AI 生图项目
  identity_anchors: ["生图", "image", "T2I", launcher: "ComfyUI.app", docs: ["生图模型对比.md"]]
  Task: 修复生图模型                               (= V0 project_stream)
    Event 1: 更新中转站版本 (3 张截图, 09:14–09:17)  (= V0 focus_session + event_screenshots)
    Event 2: 切换生图模型到 FLUX (5 张截图)
    Event 3: 测试生成一张样图 (2 张截图)
  Task: 整理 prompt 模板
    Event 4: ...
```

---

## 2. 目标 / 非目标

### 2.1 目标

1. 信息架构为三层：`projects → tasks → events`，每层有清晰职责
2. 保留并重定位 `facts`（从 brain 回收）—— 从 Event **内部**提取的结构化锚点
3. Event 由 **Observation Session** 驱动：连续若干张截图由 LLM 判定边界
4. Project 由 AI **被动建立**，基于 **identity anchors** 绑定（继承 V0 signals 思路，上移一层）
5. 支持**被动重分派**：系统自动发现"最近 3 个 event 该独立成新 task"或"这个 event 该换 project"，写 `reassignment_log`
6. 显式区分 **"待分类箱"**（pending）与 **"杂项"**（background，可隐藏）
7. **Push / overlay 暂停** —— 首要目标是信息收集准，推送是下一步
8. **破坏性迁移**：从 V0 拉分支，清空所有画像层 + 中间层，新库 schema 直接到位

### 2.2 非目标

- 不重写截图管道、LLM provider、tauri-plugin-store、overlay 渲染
- 不做画像 / soul / 用户倾向
- 不做跨设备同步
- 不做 overlay 推送决策（暂停，不删渲染壳）
- 不做 RAG / 向量检索（FTS + anchor 索引够）
- 不保留任何历史 DB 数据（discardable per D8）

---

## 3. 新原语

### 3.1 Project

继承 V0 `projects` 表，**加** identity anchors、user_renamed；**去掉** category。

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | PK | |
| `title` | TEXT | AI 命名（"AI 生图项目"），用户可改名 |
| `user_renamed` | BOOL | 区分 AI 默认名 vs 用户改过的名 |
| `summary` | TEXT | LLM 基于旗下 tasks 产出 |
| `identity_anchors_json` | JSON | 见 §3.5 |
| `status` | ENUM | `active` / `dormant` / `archived` |
| `started_at` / `last_active_at` / `created_at` / `updated_at` | TIMESTAMP | |

V0 已有的 `category`（project/research/ops/personal）**去掉** —— 用 identity anchor 替代分类。

### 3.2 Task（= V0 `project_streams` 重命名）

| 字段 | 说明 |
|---|---|
| `id` | PK |
| `project_id` | FK nullable（NULL = 游离任务） |
| `title` | "修复生图模型" |
| `goal` | 可选，更长的目标陈述（**V0 没有**，新增） |
| `summary` | LLM 产出（**V0 有但空**，修复） |
| `state` | `active` / `blocked` / `done` / `dropped` |
| `started_at` / `last_touched_at` / `resolved_at` | TIMESTAMP |

**State 枚举只有 4 个**，不包含 `dormant`。"长时间没动的 task"是 **UI 层概念**，基于 `last_touched_at + state` 计算（见 §6.1），不影响数据语义。

Task 不直接挂截图 —— 截图 → event → task 是唯一路径。

### 3.3 Event（= V0 `focus_sessions` 重命名 + 加多截图映射）

| 字段 | 说明 |
|---|---|
| `id` | PK |
| `task_id` | FK nullable（NULL = 待分类） |
| `is_background` | BOOL（true = 杂项，不进推进视图） |
| `title` | LLM 起的一句话（**V0 focus_session.title 字段已有，要修空 bug**） |
| `summary` | LLM 2–3 句（**同上**） |
| `started_at` / `ended_at` | 覆盖的截图时间跨度 |
| `screenshot_count` | 冗余字段 |
| `confidence` | LLM 对 event 边界的置信度 |

**新增关联表** `event_screenshots(event_id, screenshot_id, ordinal)` —— V0 的 `work_context_observation` 拆得更远（wc → observation → segment → screenshot），新 spec 直接做 event → screenshot。

**边界规则**（D1）：LLM 驱动，不用时长/相似度阈值。见 §4.2。

### 3.4 Fact（从 brain cherry-pick）

V0 **没有** 结构化 fact 抽取。直接 cherry-pick brain 的 `facts` 表 schema 和 `fact_extractor` 服务。

| 字段 | 说明 |
|---|---|
| `id` | PK |
| `event_id` | FK → events ON DELETE CASCADE（绑到 event，而不是 brain 的 thread） |
| `kind` | `amount` / `entity` / `error` / `path` / `url` / `version` / ... |
| `subject` / `attribute` / `value` / `unit` | 结构化键值 |
| `confidence` | LLM 抽取置信度 |
| `observed_at` | 用第一次出现的截图时间 |

**用途**：
- 支撑 `/facts` 页面（以 event 分组展示）
- 给 project 的 identity anchor 提供候选（反复出现的 entity/path 自动追加）
- **不驱动 push**（push 整体 postpone）

### 3.5 Identity Anchor（V0 `work_context_signals` 上移到 project）

V0 的 `work_context_signals(work_context_id, kind, value, last_seen_at)` 挂在 wc 上；新 spec **上移**到 project 层，并扩展 kind 集合。

**存储选项**：
- (a) 独立表 `project_identity_anchors(project_id, kind, value, hit_count, last_hit_at)`
- (b) `projects.identity_anchors_json` 存整包 JSON

**选 (a)**，因为：
- 需要按 `(kind, value)` 索引做命中查询
- anchors 是事实增量，多行操作比 JSON rewrite 干净
- 统计 hit_count 方便

**kind 集合**（继承 V0 + 扩展）：

```
name          -- 项目别名、关键词
launcher      -- 应用名（"ComfyUI.app"）
document      -- 文档名（"生图模型对比.md"）
entity        -- 概念实体（"FLUX", "SDXL"）
url           -- URL 或域名
path          -- 文件路径
command       -- shell 命令（保留 V0 的 kind）
```

**谁写入**：
- AI 建项目时，从当时的 event/fact 数据抽出初始 anchor
- 后续每次 event 归入该 project 时，如果 fact 里出现新高频 anchor（阈值 TBD），自动追加
- 用户可手动编辑（v1 开放 JSON 文本框，v2 做 UI）

**怎么用**：
- 新 event 决定 project 时，算 **anchor 命中数** 比每个现存 project 的 anchor 集
- 命中数 ≥ 阈值（§9 TBD）→ 归入该 project
- 零命中 → 新建 project（初始 anchor 从当前 event 的 fact 抽）

### 3.6 Reassignment Log（改造 V0 `reconcile_log`）

V0 `reconcile_log(ran_at, duration_ms, summary_json, trigger)` 记录**运行**级信息。改造成 **subject 级**：

| 字段 | 说明 |
|---|---|
| `id` | PK |
| `subject_kind` | `event` / `task` |
| `subject_id` | |
| `from_parent_kind` / `from_parent_id` | 原归属，NULL = 原先在待分类箱 |
| `to_parent_kind` / `to_parent_id` | 新归属 |
| `trigger` | `user_drag` / `retrospective_auto` / `anchor_match_rerun` |
| `reason` | 人话解释（自动重分派必填；user_drag 可空） |
| `occurred_at` | TIMESTAMP |

用途：undo、审计、训练信号。

---

## 4. Pipeline

```
capture_loop ──────────┐
                       ▼
┌──────────────────────────────┐
│  observation_session         │  维护一个"活动 LLM 会话"：
│  (重写 V0 work_context/pipeline)│  - 持续吃新截图
│                              │  - 每 N 张或 Y 分钟 flush 一次
│                              │  - LLM 决定: 继续 / 结束+开新 / 背景
└────────┬─────────────────────┘
         │ event_emitted
         ▼
┌──────────────────────────────┐
│  fact_extractor (cherry-pick │
│   brain 的同名服务)          │
└────────┬─────────────────────┘
         │ facts attached
         ▼
┌──────────────────────────────┐
│  event_attributor            │  替代 V0 project_resolver：
│  (改写 V0 project_resolver)  │   1. identity anchor 命中匹配 project
│                              │   2. 同 project 的现存 task 里挑或新建
│                              │   3. 零命中 → task_id=NULL（待分类）
│                              │      或 is_background=true（杂项）
└────────┬─────────────────────┘
         │
         ▼
┌──────────────────────────────┐
│  task_narrator (cherry-pick  │  被动周期跑，给 task 写 summary
│   brain 的 narrator)         │  和 project 写 rollup summary
└──────────────────────────────┘

┌──────────────────────────────┐
│  retrospective_reviewer      │  改写 V0 replay：
│  (改写 V0 work_context/replay│   - 不再"wipe sessions 全重算"
│   的思路)                    │   - 被动触发扫描最近 event 序列
│                              │   - 低置信度边界 / anchor 强证据 触发重分派
│                              │   - 每次动作写 reassignment_log
└──────────────────────────────┘
```

### 4.1 Observation Session

**服务**（新）：`src-tauri/src/services/observation_session/`

**职责**：
- 维护一个长活 LLM 会话（或按 project 一个）
- 订阅 `capture_loop` 的新截图事件
- 每 N 张截图或 Y 分钟做一次 flush：问 LLM"这批是继续 / 结束+新开 / 背景"
- flush 时一并请求 event 的 title/summary（减一次调用）

**和 V0 关系**：概念上对应 V0 `focus_session.upsert` + `work_context/pipeline::process_batch`，但**改 hot-path 为 LLM 驱动**（V0 是 signals 匹配驱动）。

### 4.2 Event Boundary（D1）

LLM 决策，不用启发式。每次 flush 的 prompt 输入：
- 当前 event 的 title/summary（如果有）
- 最近若干张截图的 transcription
- 当前活跃 project 列表 + identity_anchors
- 上一个 event 结束时间

LLM 输出：
```json
{
  "decision": "continue" | "end_and_start" | "background",
  "current_event_title_update": "...",
  "new_event_title": "...",           // 仅 end_and_start
  "new_event_summary": "...",
  "confidence": 0.0 ~ 1.0,
  "reasoning": "..."
}
```

低 `confidence` 的边界进入 `retrospective_reviewer` 的候选队列。

### 4.3 Event Attributor

**改写** V0 `project_resolver`：

**输入**：
- 新 event 的 title/summary + 抽出的 facts
- 所有 active project 的 identity_anchors
- 同 project 的现存 task 列表（活跃 N 条）

**决策**（快速路径，只在 event flush 时跑一次）：

1. **Project**：
   - 对每个现存 project 算 anchor 命中分（按 kind 加权：launcher/document > entity > name）
   - 命中分 ≥ 阈值 → 归入该 project
   - 零命中或低分 → **不做 LLM 兜底**，直接 `project_id=NULL` 进入待分类队列（交给 retrospective_reviewer 异步处理）
   - **特例**：observation_session 的 event boundary 判定里已经标了 `background=true` 的，直接 `is_background=true` 落库，不走归属
2. **Task**（只有 project 成功命中时才跑）：
   - 对该 project 的活跃 task 列表做 LLM 判定：挑一个 / 新建 / 留 NULL
   - project 没命中时 task 也留 NULL

**为什么 anchor-贫瘠的 event 不在这里 LLM 兜底**：
- 浏览器搜索、通用工具查询这类 event 常常 0–1 个 anchor 命中（如 mockup 里 Event 8 搜 "sqlite rename column"）
- 热路径 LLM 兜底意味着**每次 anchor 没命中都多一次调用**，成本会炸
- 这种 event 的正确归属强依赖**时间临近 + 语义延续**（它前后的 event 在干啥），这种信号只有 retrospective_reviewer 在有多个相邻 event 的上下文下才能用
- 所以快速路径选择"放弃归属 → 待分类 → 5–30 分钟后由 retrospective 冷静处理"

**和 V0 区别**：
- V0 每次都 LLM 二选一 "existing project 或 new project"，没 anchor → 失败时"就近塞一个名字相近的 project"，根因 1.2.2
- 现在：anchor 先硬匹配 → 命中即归 / 不中即 NULL，LLM 只在 retrospective 阶段出手

### 4.4 Retrospective Reviewer

**改写** V0 `work_context/replay.rs`：V0 的 replay 是"wipe 所有 session 全重算"，粒度粗；新版本粒度精细且**同时承担 3 种判断**。

#### 4.4.1 触发时机

- **C. 每次 event flush 时顺带回看最近 K 个 event**（K = 10；轻量、即时、复用 flush 的 LLM 上下文）
- **D. idle 窗口触发**（用户停手 ≥ 5 分钟 → 跑一次，之后每 15 分钟继续跑直到用户回来）

C + D 组合：C 负责局部即时、D 负责大图复盘（可看最近 1–4 小时）。

#### 4.4.2 三类重分派信号

**信号 A — anchor 后发命中**
- 一个 event 原先归属 X task 或 `task_id=NULL`
- 用户新 event 产出后，某个 project 的 anchor 集增长
- 重跑该 event 的 anchor 匹配，如果现在能命中更强的 project → 触发重分派

**信号 B — 相邻 event 归属一致性 + 主题延续**（关键补充，mockup Event 8 的场景）
- 扫描窗口内所有 `task_id=NULL` 或低 confidence 归属的 event
- 看该 event 前后 K 个 event 的归属：如果前后 event 都属于同一个 task / project，且 LLM 判定当前 event 的主题跟它们连续（浏览器搜索的关键词呼应前后代码 / 查的文档是前后讨论的对象 / 等），就归入该 task
- 此信号专门兜底 **anchor-贫瘠但语义连续** 的 event（浏览器搜索、通用工具查询、跳到文档查定义）

**信号 C — 主题抽象级别漂移**（mockup Event 15-16 的场景）
- 扫描当前 task 下最近若干 event，LLM 判定"这几个 event 其实在设计一个**通用机制**，跟 task 原本的具体目标（例：修 supermemory config 报错）是两件事"
- 触发：**自动 CREATE 新 task**（例："错误分类机制"），把相关 event 迁入
- 判断标准是**主题抽象级别**而非时间或 anchor：原 task 目标具体、新 event 主题抽象 / 通用 → 应分家

#### 4.4.3 动作类型

1. **Event 换 task**（信号 A / B 都可能触发，最常见）
2. **Event 序列独立成新 task**（信号 C 触发）
3. **Task 换 project**（信号 A 触发，少见）
4. **Event 从 待分类 归入现存 task**（信号 B 触发，是 mockup Event 8 的场景）

#### 4.4.4 实施保障

- 每次动作写 `reassignment_log(trigger=retrospective_auto, reason=<LLM 解释>)`
- 用户手动拖动的 event/task 进入"锁定"：后续 retrospective 只读取不改写，除非 anchor 证据极强（TBD 阈值）
- 低 confidence 动作（LLM 自报 confidence < 阈值）：v1 直接执行 + 标 confidence；v2 做建议队列 + 用户确认

---

## 5. 数据模型（新 schema 草稿）

从 V0 拉起后，新 migration `015_event_task_project.sql`（或把所有老 migration 合并成一个 `schema.sql` 版的干净起点 —— 这是推荐方案）。

```sql
-- Projects (改造 V0 projects)
CREATE TABLE projects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    user_renamed INTEGER NOT NULL DEFAULT 0,
    summary TEXT,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active','dormant','archived')),
    started_at TEXT NOT NULL CHECK (...),
    last_active_at TEXT NOT NULL CHECK (...),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Identity anchors (V0 work_context_signals 上移到 project)
CREATE TABLE project_identity_anchors (
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind TEXT NOT NULL
        CHECK (kind IN ('name','launcher','document','entity','url','path','command')),
    value TEXT NOT NULL,
    hit_count INTEGER NOT NULL DEFAULT 1,
    last_hit_at TEXT NOT NULL,
    PRIMARY KEY (project_id, kind, value)
);
CREATE INDEX idx_anchors_lookup ON project_identity_anchors(kind, value);

-- Tasks (= V0 project_streams 重命名 + 加 goal)
CREATE TABLE tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    goal TEXT,
    summary TEXT,
    state TEXT NOT NULL DEFAULT 'active'
        CHECK (state IN ('active','blocked','done','dropped')),
    started_at TEXT NOT NULL,
    last_touched_at TEXT NOT NULL,
    resolved_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_tasks_project ON tasks(project_id);
CREATE INDEX idx_tasks_state ON tasks(state);

-- Events (= V0 focus_sessions 重命名 + 字段扩展)
CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER REFERENCES tasks(id) ON DELETE SET NULL,
    is_background INTEGER NOT NULL DEFAULT 0,
    title TEXT,
    summary TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT NOT NULL,
    screenshot_count INTEGER NOT NULL DEFAULT 0,
    confidence REAL NOT NULL DEFAULT 0.5,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active','closed')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_events_task ON events(task_id);
CREATE INDEX idx_events_started ON events(started_at);
CREATE INDEX idx_events_background ON events(is_background);

-- Event ↔ Screenshot 映射 (新)
CREATE TABLE event_screenshots (
    event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    screenshot_id INTEGER NOT NULL REFERENCES screenshots(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    PRIMARY KEY (event_id, screenshot_id)
);

-- Facts (cherry-pick brain 的 facts schema, event 级而非 thread 级)
CREATE TABLE facts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    subject TEXT,
    attribute TEXT,
    value TEXT NOT NULL,
    unit TEXT,
    confidence REAL NOT NULL DEFAULT 0.5,
    observed_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_facts_event ON facts(event_id);
CREATE INDEX idx_facts_kind ON facts(kind);

-- Reassignment log (改造 V0 reconcile_log，subject 级)
CREATE TABLE reassignment_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_kind TEXT NOT NULL CHECK (subject_kind IN ('event','task')),
    subject_id INTEGER NOT NULL,
    from_parent_kind TEXT,
    from_parent_id INTEGER,
    to_parent_kind TEXT,
    to_parent_id INTEGER,
    trigger TEXT NOT NULL CHECK (trigger IN ('user_drag','retrospective_auto','anchor_match_rerun')),
    reason TEXT,
    occurred_at TEXT NOT NULL
);
CREATE INDEX idx_reassign_subject ON reassignment_log(subject_kind, subject_id);

-- FTS (参考 V0 work_contexts_fts 思路)
CREATE VIRTUAL TABLE events_fts USING fts5(
    title, summary,
    content='events', content_rowid='id',
    tokenize='unicode61'
);
CREATE VIRTUAL TABLE tasks_fts USING fts5(
    title, goal, summary,
    content='tasks', content_rowid='id',
    tokenize='unicode61'
);
```

**所有 CHECK 内容按 f3bd8a0 的 GLOB 约束风格补**（`????-??-??T??:??:??*Z` 或 `????-??-?? ??:??:??*`）。

---

## 6. UI 语义

### 6.1 `/now` 页面

```
┌─────────────┬─────────────────────┬──────────────────────────┐
│ Projects    │ Tasks               │ Event timeline           │
├─────────────┼─────────────────────┼──────────────────────────┤
│ ▸ AI 生图   │ ▸ 完成 spec 设计     │ 23:00  提交 spec 草稿    │
│   Corivo    │   修复 supermemory  │ 22:40  ...               │
│   ─────     │   错误分类机制      │ 15:33  补 identity章节   │
│   待分类 ⓘ  │   ─────             │ 15:00  ...               │
│   ─────     │   done / dormant ▾  │ ...                      │
│             │                     │ ─── 杂项 3 条 · 47min ▾ │
└─────────────┴─────────────────────┴──────────────────────────┘
```

**左栏（Projects）**：
- 活跃 project（按 `last_active_at` 排序，靠前）
- `待分类`：常驻伪条目，`task_id=NULL && is_background=false` 的 event 数
- `archived` 的 project 不显示（通过 Project 右键"归档"把玩完的大事收起来）

**中栏（Tasks）** —— 选中 project 后展示：
- 活跃 task（`state ∈ {active, blocked}`，按 `last_touched_at` 排序）
- 折叠分区 `done / dormant ▾`，默认折叠，展开后显示：
  - `state ∈ {done, dropped}` 的 task —— 显示 ✓ 或 × 前缀
  - `state ∈ {active, blocked}` 且 `last_touched_at < now - 48h` 的 task —— 标 "dormant"，灰色
  - **"dormant" 只是 UI 层 label，数据层 state 不变**
- 今日有活动的 done task 优先排在展开区顶部（方便回看）

**右栏（Events timeline）** —— 选中 task 后展示：
- 该 task 下的 event 列表，倒序（最新在顶）
- 底部分隔线 + `杂项 N 条 · 累计时长 ▾` 折叠分区
  - 默认折叠
  - 展开后显示该时间范围内 `is_background=true` 的 event（不限 task 归属）
  - 设置项里有"永久隐藏杂项"开关（对重度洁癖用户）

**交互**：
- Event 卡片拖到别的 task / project → 写 `reassignment_log(trigger=user_drag)`，event 进入"锁定"状态
- Project 右键：重命名 / 归档 / 编辑 anchors
- Task 右键：重命名 / 改 project / 标记 done/dropped

### 6.2 `/facts` 页面

按 event 分组：

```
Event: 切换生图模型到 FLUX (09:20–09:28)
  [entity] model = FLUX
  [path]   config = ~/ComfyUI/models/flux.safetensors
  [version] version = 2.0.1
```

---

## 7. 从 brain cherry-pick

以下代码/文件从 brain commit（`7e45a66`）cherry-pick 到 V0：

**数据层 / 基础设施**（纯价值增量，无范式绑定）：
- `src-tauri/src/db/time.rs` 全部（`DbInstant` + `now_utc` + `Clock`）
- `src-tauri/src/db/schema.sql` 的 CHECK 约束风格（f3bd8a0）
- `time_discipline` CI 测试（grep 防回归）
- 删除 `src-tauri/src/services/clock.rs`（V0 里存在，brain 里也删了，我们保持删）

**Fact 抽取**：
- `src-tauri/prompts/fact_extract.md`
- `src-tauri/src/services/user_model/fact_extractor.rs`（brain 里叫这个名字，迁到 `services/fact_extractor/` 独立模块）
- brain 的 `facts` repo 结构 —— 但 `thread_id` FK 改为 `event_id`

**Narrator**：
- `src-tauri/prompts/thread_narrate.md` → 改造成 `task_narrate.md`
- `src-tauri/src/services/narrator.rs` → 改 `task_narrator`，输入从 `thread_facts` 换成 `event_screenshots + facts`

**所有 push / detector 层不 cherry-pick**：
- `push_events / push_gate / completion_detector / connection_detector / recall_detector / thread_pipeline` 整组跳过（push 暂停）

---

## 8. 破坏性迁移

按 D8：historical data discardable。不做数据迁移。

**Migration 策略**（推荐方案 A）：

**方案 A — 塌陷成单一 schema.sql（推荐）**
- 从 V0 拉分支后，把 `migrations/001..014` 合并成一个干净的 `schema.sql` + bootstrap
- 新库直接建目标 schema，旧库启动时一次性 drop 全部旧表、建新表
- 理由：CLAUDE.md "Corivo 尚未发布，更新与维护时可尽量少考虑向后兼容性"，以及 brain commit 已经做过一次 single-file `schema.sql` 的塌陷（可以参考它的 `purge_legacy_schema` 做法）

**方案 B — 新增 015_event_task_project.sql，delta**
- 保留 V0 ladder，新增一条增量 migration
- 手写 `DROP TABLE IF EXISTS propositions, suggestions, proactivity_scores, work_contexts, work_context_signals, work_context_episodes, work_context_observation, work_context_proposition, notification_log, ...`
- 然后 `ALTER TABLE projects` 加字段、新建 `project_identity_anchors / events / event_screenshots / facts / reassignment_log`
- 重命名 `project_streams → tasks`、`focus_sessions → events`（SQLite 重命名复杂，不如新建 + drop）

**选 A**，因为新库 + 破坏性更干净。

**要删的表**：
```
propositions
proposition_supporting_observations
proposition_fts
suggestions
suggestions_fts
proactivity_scores
work_contexts
work_context_signals
work_context_observation
work_context_proposition
work_context_episodes
work_contexts_fts
notification_log              (push 层停摆，暂无 log)
reconcile_log                  (改造成 reassignment_log)
segments                       (检查是否 event_screenshots 能直接绑到 screenshots，若是则 segments 可删)
```

**要删的代码**：
```
src-tauri/prompts/
  propose.md, revise.md, similar.md, score.md, suggest.md,
  work_propose.md, project_merge_decide.md, project_resolve.md (重写),
  project_stream_merge.md, project_stream_resolve.md (重写), summary.md,
  souls/

src-tauri/src/services/
  user_model/proposition_pipeline.rs
  user_model/screen_observer.rs
  user_model/observer.rs
  user_model/retrieval.rs
  proactivity_scorer.rs
  suggestion_generator.rs
  push_pipeline.rs

src-tauri/src/commands/
  notification_log.rs (push log 停摆)
  user_model.rs
  tune.rs (prompt playground，依赖画像 prompt，暂停)

src-tauri/src/db/repos/
  propositions.rs
  suggestions.rs
  proactivity_scores.rs
  notification_log.rs
  reconcile_log.rs (改 reassignment_log)

前端相关:
  src/pages/overview (依赖画像数据)
  src/pages/tune (prompt playground)
```

**要改造的代码**：
```
src-tauri/src/db/repos/
  projects.rs              → 加 identity_anchors 相关方法
  project_streams.rs       → 改名 tasks.rs + 加 goal/summary
  focus_sessions.rs        → 改名 events.rs + 加 event_screenshots 表
  work_contexts.rs         → 删除

src-tauri/src/services/
  work_context/pipeline.rs → 改写为 observation_session/pipeline.rs
  work_context/project_resolver.rs → 改写为 event_attributor/
  work_context/stream_resolver.rs  → 合并进 event_attributor
  work_context/focus_session.rs    → 改写为 observation_session/session.rs
  work_context/replay.rs           → 改写为 retrospective_reviewer/
  project_reconcile/               → 删除（整个 reconcile 思路被 retrospective_reviewer 取代）

src-tauri/prompts/
  project_resolve.md       → event_attribute.md（加 identity anchor 逻辑）
  (new) event_boundary.md
  (new) retrospective_review.md
  (new) task_narrate.md   (cherry-pick brain thread_narrate 改造)
  (new) fact_extract.md   (cherry-pick brain)
```

**要保留的代码**：
```
src-tauri/src/services/
  capture_loop.rs / capture_store.rs
  config_service / keychain_service / llm_service / macos_system_surface
  notification_service / storage_cleanup
  observation_ingest/ (观察 ingest 基础设施)

src-tauri/src/commands/
  capture / config / demo / llm / notification / onboarding / settings

src-tauri/src/providers/
  llm/ / notification/ (全保留)

前端:
  app-boot / layout / sidebar / routes 基础
  /settings (部分，去掉 tune page)
  /connections onboarding
  overlay 渲染壳 (push 停摆，但壳保留)
```

---

## 9. Open Questions

每条给出：**问题 / 候选方案 / 我的倾向 + 理由**。你只需要在每条后写 "A" / "B" / "按倾向走" 就行。

> **draft-2 修订**：基于 mockup 复核，Q8 倾向从 C 改到 B（默认折叠在时间线底部，而非完全隐藏）。其余 11 条倾向不变。

---

### Q1. 被动重分派的触发时机（§4.4）

**问题**：什么时候让 retrospective_reviewer 跑一遍？

- **A. 每产生 M 个新 event 触发一次**（例：每 5 个 event）
  - 优点：动作频率跟活跃度绑定
  - 缺点：用户一直在干活时反而 reviewer 频繁打断 LLM 调用预算
- **B. 每 T 分钟定时触发**（例：每 15 分钟）
  - 优点：可预测
  - 缺点：没活动的 30 分钟里也在空跑
- **C. 当前 event flush 时顺带回看最近 K 个 event**（例：每次 flush 回看最近 5 个）
  - 优点：复用 flush 的 LLM 会话上下文，成本低；修正有即时性
  - 缺点：只看局部，看不到"回看全天"这种大图
- **D. idle 窗口触发**（用户停手 ≥ 5 分钟）
  - 优点：用户不在的时候做"冷静复盘"，不打断工作
  - 缺点：短时高强度使用时整天跑不起来

**倾向：C + D 组合** —— C 负责即时局部修正、D 负责全天大图复盘，彼此互补不重复。

---

### Q2. Observation session 的上下文窗口

**问题**：LLM 会话持续吃新截图，上下文会膨胀，怎么收尾？

- **A. 硬切 30 min 自动开新会话**
  - 优点：简单
  - 缺点：30 min 边界处可能切断一个连贯事件
- **B. 滚动保留最近 N 张截图的 transcription**（例：N=20）
  - 优点：上下文稳定、成本可控
  - 缺点：跨 event 的长时记忆丢
- **C. 按 token 预算切**（超过 80% context window 就压缩前半）
  - 优点：榨干模型能力
  - 缺点：实现复杂、要做摘要压缩
- **D. 每产生一个 event 就 reset 会话**
  - 优点：最干净
  - 缺点：下一个 event 起手无上下文，边界判定会偏

**倾向：B（滚动保留最近 N 张）+ 落库后用 event.summary 做轻量长记忆**。N 取 **15–20**，对应 5–15 张正在推进的 event 内截图 + 5 张跨 event 的上下文缓冲。

---

### Q3. Event flush 的触发粒度

**问题**：observation_session 多久问 LLM 一次"继续 / 结束+新开 / 背景"？

- **A. 每 N 张新截图触发**（候选：3 / 5 / 10）
  - 太频繁浪费 LLM；太稀疏边界延迟
- **B. 每 T 秒/分钟触发**（候选：60s / 90s / 180s）
  - 用户密集操作时反应慢；空闲时会频繁空转
- **C. 先到者触发（A OR B）**（例：3 张 OR 90 秒，先到哪个触发哪个）

**倾向：C，3–5 张 OR 90 秒取先到**。截图本身是 X 秒一张（见 `capture_loop`），所以 3–5 张通常跨 30–150 秒，刚好和 90 秒兜底错峰。

---

### Q4. Fact 抽取发生在哪一步

**问题**：结构化 fact（金额、entity、path、error 码）从什么时候抽？

- **a. 在 observation_session flush 的那次 LLM 调用里一并产出** —— 即同一个 prompt 同时输出：边界决策 + event title/summary + fact 列表
  - 优点：只多一次 LLM 调用（和 event 产出合并）
  - 缺点：prompt 变长，单次失败影响面大
- **b. event 落库后再跑一次专门 prompt** —— 独立 `fact_extractor` 服务订阅 event_emitted 事件
  - 优点：职责清晰、单次 prompt 短、失败独立重试
  - 缺点：每个 event 多一次 LLM 调用（成本 × 2）

**倾向：a**。成本敏感是首要因素，prompt 长一点可以接受；而且 event 的 title/summary 本身就要看截图内容，fact 抽取跟它是同一个认知动作。

---

### Q5. Identity anchor 命中阈值

**问题**：一个新 event 要匹配到现存 project，需要命中多少个 anchor？

- **打分规则**：每个 anchor kind 有权重，命中即加分

| kind | 权重 | 理由 |
|---|---|---|
| launcher | 3 | 应用级强信号（ComfyUI.app ≈ 生图） |
| document | 3 | 在某文档里干活 = 正在做这件事 |
| path | 2 | 文件路径有稳定语义 |
| url | 2 | |
| entity | 1 | 单个 entity 可能巧合 |
| command | 1 | |
| name | 1 | 名字最容易撞 |

- **A. 阈值 2 分**（倾向）：至少 1 个高权重命中，或 2 个弱命中
- **B. 阈值 3 分**：需要明确强信号
- **C. 阈值 1 分**：宽松，什么都算命中（V0 失败的那种）
- **D. 用比例**：命中分 / project 总 anchor 分 ≥ X%

**倾向：A（2 分起），初期保守，v1 跑一天看数据再调**。不选 B 是因为冷启动阶段 project 的 anchor 数量不够多，3 分命中太严；不选 C 是因为重蹈 V0 覆辙。

---

### Q6. Event confidence 过低时的处理

**问题**：LLM 给 event 边界的 confidence < X（例如 0.5）时怎么办？

- **A. 先全落库 + 标 low-confidence flag + 事后 reviewer 回扫**
  - 优点：不丢数据、事后可修
  - 缺点：UI 上可能暂时看到质量差的 event
- **B. 低于阈值暂不落库 + 入一个"待判定队列" + 等下次 flush 重新决策**
  - 优点：只把高质量 event 推到 UI
  - 缺点：实现复杂、低置信度素材可能积压

**倾向：A**。破坏性可修改 > 暂存队列。UI 可以用淡色/虚线标示 low-confidence，不影响结构。

---

### Q7. `/now` 页面 UI 形态

**问题**：三层结构怎么在一屏里表达？

- **A. 三栏：Project | Task | Event**（见 §6.1）
  - 优点：一屏看齐三层；选中 project 筛选 task、选中 task 筛选 event
  - 缺点：屏幕窄时拥挤
- **B. Breadcrumb + 单栏时间线**（"AI 生图 / 修复生图 / "下面平铺 event）
  - 优点：简洁
  - 缺点：层级导航要点击，看不到兄弟项目/任务
- **C. 左栏 project 可折叠树（project → task）+ 右栏 event 详情**（类似邮件客户端）
  - 优点：任意层级都能折叠/展开
  - 缺点：树状交互复杂

**倾向：A**。Corivo 的窗口不算窄，三栏信息密度高；兄弟 project / task 可见对"切换注意力"有直接帮助。

---

### Q8. "杂项"（background）的默认展示

**问题**：刷 Twitter、摸鱼这类 background event，默认怎么展示？

- **A. 默认显示在时间线里，只是淡色标注**
  - 优点：透明；不藏东西
  - 缺点：污染"推进"视图
- **B. 默认折叠到 event 时间线底部的分隔分区**（标签 "杂项 N 条 · 累计时长 ▾"）
  - 优点：透明 + 一键翻出来 + 不污染推进视图
  - 缺点：常驻条目略占空间（但折叠状态只一行）
- **C. 默认完全隐藏，设置里开关打开**
  - 优点：最干净
  - 缺点：新用户可能以为系统没记录

**倾向：B（默认折叠在时间线底部）+ 设置项有 C 的开关**。

为什么从 C 改到 B：mockup 里杂项是跟推进视图同页分区展示的（"—— 杂项（is_background=true, 不进推进视图）" 然后列出 B1–B3）。这表明默认状态下杂项可见但不打扰，而不是要去设置页翻。强洁癖的用户通过设置里的"永久隐藏"开关走 C 的路径。

---

### Q9. 用户重命名 project 后 anchor 是否跟着改

**问题**：user 把 "AI 生图项目" 改名为 "画图"，anchors 里之前 AI 写进去的 `name="AI 生图"` 要处理吗？

- **A. 不改**（倾向）
  - anchors 是"这个 project 历史上被什么证据关联过"的事实记录，不是标签
  - user 改 title 只是改显示名，不否认历史证据
- **B. 把旧 title 从 anchors.name 里移除，新 title 加进去**
  - 跟用户心智"我不认 AI 生图这个名字"一致
  - 但会丢历史 anchor 信息
- **C. 只追加新 title，不删旧的**
  - 妥协方案

**倾向：A**。名字是标签，锚点是证据 —— 两者解耦对重分派逻辑更健壮。

---

### Q10. 跨 project 的 task

**问题**：例如"整理笔记"既服务 AI 生图又服务 Corivo 自研，怎么办？

- **A. v1 不支持，强制单归属**（倾向）
- **B. 多对多链接表 `task_projects(task_id, project_id)`**
- **C. 单归属 + 打标签**（task 挂一个主 project，另外用 `task_tags` 表打跨项目标签）

**倾向：A**。v1 先不开这个口子；如果用户真遇到了，在 reassignment_log 里能看到高频"同一 task 在两 project 间来回搬"，再升级 schema 到 B 或 C。

---

### Q11. Migration 方案

**问题**：schema 改这么大，怎么落 migration？

- **A. 塌陷成单一 `schema.sql`**（参考 brain commit 的 `purge_legacy_schema` 思路）
  - 新库直接建目标 schema；旧库启动时 DROP 所有老表，重建新表
  - 优点：干净；以后改 schema 也只看一个文件
  - 缺点：任何已有数据都会被清掉（D8 已确认可接受）
- **B. 新增 delta migration `015_event_task_project.sql`**
  - 手写一大堆 DROP TABLE + ALTER TABLE + CREATE TABLE
  - 优点：理论上保留 ladder 连续性
  - 缺点：ladder 已经 14 条，加一条 delete-everything 很丑；V0 的 ladder 本身也不是生产环境必须保留的

**倾向：A**。CLAUDE.md 明确"更新与维护时可尽量少考虑向后兼容性"；brain commit 已经做过一次塌陷，这次再塌一次反而回到单一 schema.sql 的健康状态。

---

### Q12. `segments` 表去留

**问题**：V0 有 `screenshots → segments → observations` 三层，event 能直接绑 screenshot 跳过 segments 吗？

- **现状**：`segments` 把相邻相似截图合并成一帧，减少 LLM 输入量；observation 绑的是 segment 不是 screenshot
- **A. 删 segments，event 直接绑 screenshots**
  - 优点：链路短、schema 干净
  - 缺点：丢了"相邻帧去重"的优化，每张截图都进 LLM
- **B. 保留 segments，event 绑 segments**（`event_segments` 代替 `event_screenshots`）
  - 优点：保留去重优化
  - 缺点：多一层间接、多一个 join
- **C. 保留 segments 但 event 直接绑 screenshots**，去重在 observation_session 内部做（不落表）
  - 优点：表干净 + 去重保留
  - 缺点：去重结果无法跨 session 复用

**倾向：Stage 2 设计时定**，暂不预设。初步倾向 **A**（screenshot 一张一张喂 LLM，用 JPEG 压缩 + 小分辨率就够便宜），但这要看 Stage 2 真实数据量评估再拍板。

---

## 10. 执行计划

从 `334a5e8` 拉分支（V0 最后一个 commit）；不碰当前 main（brain 仍在 main 上）。

### Stage 0 — 起点准备
- `git checkout -b event-task-project 334a5e8`
- Cherry-pick `f3bd8a0`（时间组件重构）—— 注意 f3bd8a0 的 schema CHECK 是针对 brain 的表；冲突解决时保留 DbInstant/Clock/time_discipline test，丢弃 schema.sql 的 brain-specific CHECK
- `cargo build` 验证时间层 cherry-pick 成功
- **一次原子提交**："cherry-pick DbInstant/Clock time layer onto V0"

### Stage 1 — 清画像层
- 删 propositions / suggestions / proactivity / screen_observer / proposition_pipeline / soul / push_pipeline / suggestion_generator / score / propose / revise / similar / work_propose / soul prompts
- 删对应 repos / commands / 前端 overview / tune 页面
- `cargo build` 绿、剩余测试绿
- **一次原子提交**："strip proposition/画像 layer from V0"

### Stage 2 — 新 schema + migration 塌陷
- 方案 A：合并所有老 migration 到单一 `schema.sql`，新库直接建目标 schema
- 新建 `projects / project_identity_anchors / tasks / events / event_screenshots / facts / reassignment_log / events_fts / tasks_fts`
- 删 `work_contexts / work_context_signals / work_context_episodes / work_context_observation / work_contexts_fts / reconcile_log / focus_sessions（已改名）/ project_streams（已改名）`
- 重写 repos: `projects.rs`（加 anchor 方法）、`tasks.rs`（原 project_streams）、`events.rs`（原 focus_sessions + event_screenshots）、`facts.rs`（新）、`reassignment_log.rs`（原 reconcile_log 改）、`project_identity_anchors.rs`（新）
- 单元测试 CRUD + FTS 查询
- **一次原子提交**："schema rename and new tables for event/task/project"

### Stage 3 — observation_session + fact_extractor
- 新 service `observation_session`（改写 V0 work_context/pipeline + focus_session）
- 新 prompt `event_boundary.md`
- Cherry-pick brain 的 `fact_extractor` + `fact_extract.md`，改 FK 为 event_id
- 集成测试：塞若干假截图 → 检查产生的 event + fact 结构
- **一次原子提交**："observation session + structured fact extraction"

### Stage 4 — event_attributor + identity anchors
- 新 service `event_attributor`（改写 V0 project_resolver + stream_resolver）
- 新 prompt `event_attribute.md`
- 实现 anchor 命中打分 + LLM 兜底
- anchor auto-update：event 归入 project 时，从 facts 里反哺高频 anchor
- 集成测试：AI 生图项目的模拟场景（3 个 event，不同 launcher/path/entity），确认不塞同一 project
- **一次原子提交**："event attributor with identity anchor matching"

### Stage 5 — task_narrator
- Cherry-pick brain `narrator` → 改 `task_narrator`
- 输入改 event_screenshots + facts 聚合（不再是 thread_facts）
- 产出 task.summary + project.summary rollup
- **一次原子提交**："task narrator backed by event + fact aggregation"

### Stage 6 — retrospective_reviewer
- 新 service，改写 V0 replay 思路
- 触发条件 §4.4 的 (C) + (D)
- 每次动作写 reassignment_log
- 集成测试：人为制造"应该分裂成两 task"的 event 序列 → 检查是否自动分裂
- **一次原子提交**："retrospective reassignment with audit log"

### Stage 7 — UI
- 改 `/now` 三栏
- 改 `/facts` 按 event 分组
- 拖拽重分派的前端 + 后端 command（`move_event_to_task` / `move_task_to_project`）
- 归档 / 重命名 project 的 UI
- **一次原子提交**："three-pane /now and event-grouped /facts"

### Stage 8 — 合进 main
- 开 PR，一起 review
- `git merge` 时保留 brain 的 commit 记录（不强制 rebase）—— V0 分支是**重做**，不是 revert

---

## 11. 不做的事（明确记下来）

- ❌ Push / overlay 推送逻辑（暂停，不删渲染壳）
- ❌ Thread / Proposition 范式（退役）
- ❌ 跨设备同步
- ❌ Soul / 用户画像
- ❌ Tune page / prompt playground（依赖画像 prompt，一并下线）
- ❌ 历史数据迁移
- ❌ 多归属（一 event 一 task，一 task 一 project）
- ❌ 向量检索
- ❌ 用户手动建 project / task（v1 只能 AI 建 + 用户改名/归档）

---

## 12. 评估标准

用户在真实使用 1 天后：

1. 左栏出现 2–5 个 project，名字可读、能区分（不是 V0 实盘那种"tarot 下塞一切"）
2. 每个 project 下 1–3 条 task，状态分布合理
3. 每个 task 下若干 event，时间线可读，鼠标悬停能看截图缩略图
4. "待分类" event 数 < 新产生 event 的 20%（归类成功率 ≥ 80%）
5. "杂项" 占总 event 数 < 40%（否则说明 background 判定过宽）
6. 用户手动拖动重分派次数 < 自动重分派次数 × 2
7. 不再出现"48 条 prop 全是人物画像标签" 或 "/facts 页只能看到原子碎片" 这种错配
