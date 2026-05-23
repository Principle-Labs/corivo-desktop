# Work Context — `/now` 页面 Spec（前端先行 · 数据 mock）

> 版本：draft-0 · 分支：`feat/work-context-now-page` · 状态：仅前端，数据全部 mock

---

## 1. 问题陈述

当前 GUM 管线产出的 `propositions` 都在回答**"这个用户是谁"**——他的习惯、偏好、知识背景、技术风格。这部分知识质量很好，但它只解决了 story 的一半。

Story 里另一半是**"这个用户现在在做什么"**——他当前的项目、正在卡住的任务、最近一直在看的代码/文档主题。没有这一半，Corivo 很难做到"帮用户做工作"：

- push_decider 只能基于人物画像触发，无法组合"你在做 X + 你通常 Y"
- `/knowledge` 里全是人物特质，用户看不到"此刻"这一层
- 推送内容也只能是"关于你的观察"，不能是"关于你手头这件事的建议"

**本 spec 不重建 GUM 管线**。它只定义 work context 这一层独立的数据模型与展示，并清楚地描述后端需要补什么才能让这个页面跑在真数据上。

---

## 2. 为什么是独立模块，不是给 proposition 加字段

| 维度 | Proposition（人物） | Work Context（工作） |
|---|---|---|
| 变化速度 | 几天～几周 | 几分钟～几小时 |
| 语义 | "用户倾向于 X" | "用户正在做 X" |
| 生命周期 | 版本链 + REVISE 合并 | 滑动窗口 + TTL + stale |
| 关键字段 | confidence / decay / version | started_at / last_seen_at / ttl / status |
| 合并逻辑 | SIMILAR → MERGE/UPDATE/CONTRADICT | "匹配现有 active context → 续命 or 新建" |
| 查询问法 | "他是谁" | "他现在在忙什么" |

如果共用 `propositions` 表和 PROPOSE→SIMILAR→REVISE 管线，会出现两种失败模式：

1. 工作上下文的频繁变化会压住长期画像（每小时都 REVISE 一遍）
2. 或者为了不压住画像而放宽合并，导致旧工作上下文永不过期

所以两条独立的采集/存储/展示链路。push 侧在**决策层**汇合。

---

## 3. 数据模型（TypeScript，前端视角）

```ts
export type WorkContextCategory = "project" | "task" | "topic";
export type WorkContextStatus = "active" | "stale" | "archived";

export interface WorkContext {
  id: number;
  title: string;              // "调试 supermemory config 的三种错误类型"
  summary: string;            // 一句话，描述用户在这件事上的进展
  category: WorkContextCategory;
  status: WorkContextStatus;
  started_at: string;         // ISO
  last_seen_at: string;       // ISO — 最后一次被 observation 命中
  ttl_seconds: number;        // 过了 last_seen_at + ttl 之后转 stale
  signals: WorkContextSignal[];        // 观察里提取出的实体（文件、URL、命令、app）
  related_proposition_ids: number[];   // 人物 × 当下 的交叉引用
  created_at: string;
  updated_at: string;
}

export interface WorkContextSignal {
  kind: "file" | "url" | "app" | "command" | "entity";
  value: string;         // 原文
  last_seen_at: string;  // 用于热度排序
}
```

### 后端需要提供的表（草案，等前端定型后再落 migration）

```sql
CREATE TABLE work_contexts (
  id                      INTEGER PRIMARY KEY AUTOINCREMENT,
  title                   TEXT NOT NULL,
  summary                 TEXT NOT NULL,
  category                TEXT NOT NULL CHECK (category IN ('project','task','topic')),
  status                  TEXT NOT NULL CHECK (status IN ('active','stale','archived')),
  started_at              TEXT NOT NULL,
  last_seen_at            TEXT NOT NULL,
  ttl_seconds             INTEGER NOT NULL,
  created_at              TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at              TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE work_context_signals (
  work_context_id INTEGER NOT NULL REFERENCES work_contexts(id) ON DELETE CASCADE,
  kind            TEXT NOT NULL,
  value           TEXT NOT NULL,
  last_seen_at    TEXT NOT NULL,
  PRIMARY KEY (work_context_id, kind, value)
);

CREATE TABLE work_context_observation (
  work_context_id INTEGER NOT NULL REFERENCES work_contexts(id) ON DELETE CASCADE,
  observation_id  INTEGER NOT NULL REFERENCES observations(id) ON DELETE CASCADE,
  PRIMARY KEY (work_context_id, observation_id)
);

CREATE TABLE work_context_proposition (
  work_context_id INTEGER NOT NULL REFERENCES work_contexts(id) ON DELETE CASCADE,
  proposition_id  INTEGER NOT NULL REFERENCES propositions(id) ON DELETE CASCADE,
  PRIMARY KEY (work_context_id, proposition_id)
);
```

---

## 4. 前端 — `/now` 页面

### 4.1 路由与导航

- 新增路由 `/now` → `src/routes/now.tsx`
- Sidebar 在"概览"下方新增一项 **此刻**（lucide `Sparkles` 或 `Activity`），路由到 `/now`
- 这条线不改 `/knowledge`

### 4.2 布局（高度对齐 `/knowledge`）

```
┌──────────────────────────────────────────────────────────────┐
│ SheetPageHeading                                             │
│   kicker  NOW · 3 件正在做                                    │
│   title   它看到你在 <em>忙</em>的事。                          │
├──────────────────────────────────────────────────────────────┤
│ ┌────────────── FocusCard（当前焦点，大卡） ───────────────┐ │
│ │ 🟢 active  ·  持续 42 分钟  ·  45 秒前刚看到              │ │
│ │ 调试 supermemory 的配置错误类型                          │ │
│ │ 你在确认 ~/.corivo/config.json 损坏时应该报…            │ │
│ │ [signals] config.json  main.rs  "Corivo is not init…"   │ │
│ └─────────────────────────────────────────────────────────┘ │
├──────────────────────────────────────────────────────────────┤
│ Tabs: 进行中 | 今天 | 归档                                    │
├──────────────────────────────────────────────────────────────┤
│ ┌── 列表 ─────────────┐ ┌── 详情面板 ──────────────────────┐ │
│ │ · work card         │ │ header(title, status, kicker)    │ │
│ │ · work card         │ │                                   │ │
│ │ · work card         │ │ Signals                           │ │
│ │ · work card         │ │ 关联观察 (N)                      │ │
│ │                     │ │ 相关画像 (M)  ← proposition 横向  │ │
│ └─────────────────────┘ └──────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
```

### 4.3 组件清单

```
src/pages/now/
  now-page.tsx                # 外壳，镜像 knowledge-page.tsx
  focus-card.tsx              # 顶部大卡：最热 active 的那一条
  work-context-list.tsx       # 左列卡片列表
  work-context-detail-panel.tsx   # 右侧详情
  signal-pill.tsx             # 单条 signal 的小 chip
  related-proposition-ref.tsx # "相关画像"里的一条 — 点击跳 /knowledge#<id>
  mock-data.ts                # 所有 mock — 单一入口，方便后面替换成真 hooks
```

### 4.4 状态与交互

- 默认选中列表第一条（跟 `/knowledge` 对齐）
- 卡片视觉：`active` 绿点，`stale` 灰点 + 灰文，`archived` 隐藏在"归档" tab
- 相对时间：`last_seen_at` 显示"45 秒前 / 2 分钟前 / 刚才"；`started_at` 显示"持续 42 分钟"
- 详情面板里"相关画像"点击会先原地弹提示（真跳转等 `/knowledge` 支持 deep link 再接）

### 4.5 不做的事（本迭代）

- 不实装 `invoke()` / 不加 Tauri 命令、不接事件
- 不做编辑/归档/合并这类写操作
- 不做检索/搜索
- 不影响 overlay 推送
- 不加 i18n 分支（页面文案写中文，照 `/knowledge` 的惯例）

---

## 5. 后端需要解决的问题（给真数据用）

按优先级列出。前端 mock 到位后，后端按这个顺序补就能点亮。

### P0 · 采集与存储

1. **新建表**：上面第 3 节的四张表 + 一次 migration（schema_version 6）
2. **新建 distiller**：`services/work_context/`
   - 订阅 `ObservationBatcher::BatchReady`，跟 `proposition_pipeline` **平级**
   - 一个 LLM 步：`work_propose` prompt，输入批次内观察，输出 `{title, summary, category, signals[]}[]`
   - 一个轻量合并步：对每个候选，在现有 `status='active'` 的 work_contexts 里找最接近的（按 title/signals 命中），命中则续命 `last_seen_at`，未命中则新建
   - **不要** REVISE 那一整套，合并规则极简
3. **TTL 衰减任务**：后台 tick（每分钟一次即可），把 `last_seen_at + ttl_seconds < now()` 的 active 行置为 stale

### P1 · 读路径与事件

4. **Repo**：`db/repos/work_contexts.rs`，至少 `list_by_status`, `by_id`, `signals_for`, `observations_for`, `propositions_for`
5. **Tauri 命令**（放 `commands/work_context.rs`）：
   - `list_work_contexts(status?) -> WorkContext[]`
   - `work_context_observations(id) -> Observation[]`
   - `work_context_propositions(id) -> Proposition[]`
6. **广播事件** `WorkContextChanged { id, kind: "created" | "touched" | "staled" | "archived" }` — 前端用它做 React Query invalidate，跟现有 `proposition-revised` 的模式一致

### P2 · push 侧汇合（story 的落点）

7. `push_pipeline` 在决策时同时读 proposition + 当前 active work_contexts，组合进 suggestion prompt
8. overlay payload 带上 `work_context_id`，点击通知后 `/now` 或 `/knowledge` 能定位回源

### 开放问题

- **work_propose 该不该传 propositions 进去**？  
  给足人物画像 → suggestion 更贴，但也更慢更贵。初版建议**不传**，只传这一批观察。push 侧再交叉。
- **signals 怎么抽**？两条路：让 LLM 直接返回 `signals[]`；或者用规则从观察文本里 extract（文件路径、URL、代码块里的 identifier）。倾向 LLM，一致性更好。
- **title 稳定性**？同一件事 LLM 每次措辞可能不同。合并步要以 signals 交集为主、title 相似度为辅。
- **TTL 取值**：`task` 30 分钟、`project` 4 小时、`topic` 1 小时是起点，之后 tune。

---

## 6. 验收（本迭代）

- `pnpm tauri dev` 起得来
- `/now` 可导航、可渲染 mock，focus card + 列表 + 详情三段都显示正确
- `pnpm build`（vite+tsc）通过
- 不破坏现有 `/knowledge`、`/overview`、`/settings`、`/connections`
- 视觉与 `/knowledge` 对齐（SheetPageHeading / Tabs / grid 布局 / 卡片样式）

下一步：demo 给自己用一晚，确认页面结构能承载真数据，再开 P0 的后端分支。
