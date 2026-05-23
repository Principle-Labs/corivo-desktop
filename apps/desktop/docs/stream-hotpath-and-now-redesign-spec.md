# Stream hot-path 改造 + `/now` 重构 Spec

> 版本：draft-0
> 依赖：`project-layer-spec.md`、`project-reconcile-spec.md`（后端阶段 A/B/C 骨架已落；本 spec 改 C 的角色 + 上新前端）
> 取代：`project-reconcile-spec.md` §6 的 Stream 时间窗、§7 Stream 聚类（hot path 版代替 batch 聚类；reconcile 只做 周级收敛）

---

## 1. 问题陈述

当前状态：
- **后端**：migration 013 + `ProjectStreamRepo` + `ReconcileService`（A/B/C 三阶段）+ `reconcile_now` command + `list_project_streams` command + `useReconcileNow` / `useProjectStreams` hooks 已上线
- **但** Stream 按原 spec 定位为**周级**"推进阶段"（"代码开发 / 用户调研 / 产品打磨"），仅在 daily reconcile 阶段 C 批量产出
- **前端**：`/now` 仍是 `Project > WorkContext` 两级平铺，看不到 Stream；Settings 没有"记忆维护"区；FocusCard 还在
- 用户反馈：`/now` 让他看不清"今天推进了哪几件事"——WorkContext 粒度太碎（一天几十条），Project 粒度太粗（一个月同一个 Project）

核心诉求（用户原话）：
> 左侧看到我主要在做的几件事。这些事可以是分类（即 Project），我可以从 Project 底下看到正在推进的一些事情。这些事情可以叫 Stream / Thread / Focus，每天大概会有三五个。每一个推进的事情都是由许多个 WorkContent 组成的。

所以 Stream 语义要**从周级"阶段"下移到日/多日级"推进项"**——一天 3–5 个、每个由若干 WorkContext 组成、跨 1–7 天的事为主。

---

## 2. 目标

1. **Stream 重定位**：周级"阶段"→ 日/多日级"推进项"
2. **Stream 生成上 hot path**：新 WC 落库同步走 `stream_resolver`，不再等 daily reconcile
3. **Reconcile 阶段 C 改角色**：由"每日批量聚类"改为"周级收敛"（合并误分、归档久不动、补跑 NULL 兜底、刷新 title/summary）
4. **`/now` 重构**：`Project（分组 header）> Stream（主列表，每日 3–5 条）> WC（点开展开）`，WC 详情走右侧 Sheet 浮层
5. **Settings 新增"记忆维护"区**：展示上次 reconcile 摘要 + "立即整理"按钮

## 3. 非目标

- 不改 WorkContext 产出逻辑（distiller、episode、signals 不动）
- 不做 Stream 层级嵌套（不搞 sub-stream）
- 不做跨 Project 的 Stream
- 不做用户手动创建 / 拖拽 Stream（v2）
- 不做 Stream 完成度百分比、甘特图等进阶可视化

---

## 4. Stream 语义（新定义）

| 维度 | 旧（周级阶段） | 新（日/多日级推进项） |
|---|---|---|
| 时间跨度 | 周 | 1–7 天为主，偶尔更长 |
| 典型标题 | "代码开发" / "用户调研" | "改 /now 页面成 Stream 视图" / "写 stream hot-path spec" |
| 数量/ Project | 1–4 条常驻 | 3–8 条滚动，完成就 done |
| 生成时机 | daily batch 聚类 | 新 WC 落库时 hot path 同步决 |
| 门槛 | 项目需 ≥5 WC 才聚类 | 无门槛，第一条 WC 就会有 Stream |
| 状态机 | active / paused / done | active / paused / done（复用 migration 013 schema） |

状态转换时间窗（替换 `project-reconcile-spec.md` §6 Stream 部分）：
- `active → paused`：**2 天**无 touch
- `paused → done`：**7 天**无 touch
- `done` 复活：被新 WC 归回 → 回 `active`（touch 更新 `last_active_at`）

Project 时间窗保持原 spec（7d active→dormant、30d dormant→archived），不改。

---

## 5. Stream Resolver（hot path）

新文件：`src-tauri/src/services/work_context/stream_resolver.rs`（与 `project_resolver.rs` 同目录）

### 5.1 触发点

[`pipeline.rs::process_batch`](../src-tauri/src/services/work_context/pipeline.rs) 里 `project_resolver::resolve` 之后调用：

```rust
let project_id = project_resolver::resolve(&resolver_ctx, context_id, &draft).await;
if let Some(pid) = project_id {
    let _ = stream_resolver::resolve(&stream_ctx, context_id, pid, &draft).await;
}
// emit WorkContextChanged
```

约束：
- `project_id` 为 `None` → 跳过 Stream resolve（没 project 就没候选集）
- 失败 / 超时 → 吞掉错误，`stream_id=NULL`，reconcile 阶段 C 会兜底补跑
- merge 路径（幸存者已有 stream_id）→ 跳过 resolve，调 `inherit_stream_from(survivor_id)` 维持继承

### 5.2 候选集

```
SELECT * FROM project_streams
 WHERE project_id = ?
   AND status IN ('active', 'paused')
 ORDER BY last_active_at DESC
 LIMIT 8
```

每条附带 `recent_titles` (top 5) — 复用 `WorkContextRepo::recent_titles_for_stream(stream_id, 5)`（新增）。

### 5.3 LLM 决策

三选一（JSON 输出，与 `project_resolver` 格式镜像）：

```json
{"choice": "existing", "stream_id": 42}
```
```json
{"choice": "new", "new_stream": {"title": "...", "summary": "..."}}
```
```json
{"choice": "none"}
```

`none` 用于极端情况（LLM 判定该 WC 是一次性浏览、不值得挂 Stream）。正常 95%+ 应落到 existing 或 new。

### 5.4 快路径（不调 LLM）

三种情况直接落库，**跳过 LLM 调用**：

1. **候选 0 条**（Project 的第一条 Stream）→ 用 `draft.title` + `draft.summary` 直接 new
2. **候选 1 条、且该条 `last_active_at` 在 24h 内**（单活跃 Stream）→ inherit existing
3. **merge 幸存者已有 stream_id** → inherit survivor.stream_id

覆盖估计：冷启动阶段 80%+ 走快路径，稳态后 30–40%。

### 5.5 超时 + 失败

- LLM 超时 8s（与 `project_resolver` 对齐）
- 任何失败（LLM 错 / 解析错 / DB 错）都吞，`stream_id` 留 NULL
- 日志 `tracing::warn!` 记 context_id + project_id + 失败原因

### 5.6 Prompt

新文件：`src-tauri/prompts/project_stream_resolve.md`（替换现有 `project_stream_cluster.md` — 后者是 batch 聚类的，本 spec 后用不到）。

结构模仿 `project_resolve.md`：

```markdown
<!-- version: 1 | docs/stream-hotpath-and-now-redesign-spec.md §5 -->

你是 Corivo 的 Stream 归属员。用户刚被识别到一段新的工作上下文（WorkContext），
它已经归到 Project "{{project_title}}" 下。你的任务是判断它属于 Project 下
**正在推进的某条 Stream**，还是**应该新开一条 Stream**，或者**都不是**（一次性浏览）。

## Stream 是什么
Stream 是"最近几天在推进的一件具体的事"。粒度在 WorkContext（分钟级）和 Project（月级）
之间——一天 3–5 条为常见，每条通常由 3–10 个 WorkContext 组成，跨 1–7 天完成。
不是周级大方向（"代码开发"太粗），不是单次浏览（"读了篇文章"太细）。

## 新工作上下文
- 标题：{{wc_title}}
- 摘要：{{wc_summary}}
- 类别：{{wc_category}}

## 候选 Stream（共 {{n_candidates}} 条，同 Project 下）
{{candidates_block}}

## 决策规则
- 主题/任务**明显延续**某条候选 Stream → `existing`
- 是一件**开了头、预计还要做几天的新事情** → `new`，给 title / summary
- 只是一次性浏览、随手查 → `none`

宁可归到已有 Stream 也不要轻易新开。

## 输出
仅 JSON，见下三种之一：...
```

### 5.7 Repo 新方法

`ProjectStreamRepo`：
- `list_active_for_project(project_id, limit) -> Vec<ProjectStream>` — 已有 `list_for_project` 返回全部；新增按状态过滤版
- `insert(NewProjectStream) -> ProjectStream` — 已有
- `touch_last_active(stream_id, at)` — 已有 / 如无需新增
- `inherit_for(context_id, survivor_id)` — merge 路径；实质就是 `set_stream(context_id, survivor.stream_id)`

`WorkContextRepo`：
- `set_stream(context_id, stream_id: Option<i64>) -> Result<()>`
- `recent_titles_for_stream(stream_id, limit) -> Vec<String>`

`ProjectStreamRepo` 若已有 `list_for_project` 能返回全部 + status，复用即可，不必新增。

---

## 6. Reconcile 阶段 C 改写

原阶段 C（`services/project_reconcile/stream.rs`）= per-project batch 聚类。**完全重写**为"Stream 周级收敛"，做四件事：

### 6.1 Stream 合并

同 project 下两条看起来是同一件事的 Stream → LLM 判 + 合。

- 粗筛：对每个 project 取 `active + paused` 的 Streams，取 top-10 按 `last_active_at DESC`
- 两两配对：候选对限 top-3（每个 project 最多 3 对送 LLM）
- Prompt：新 `project_stream_merge.md`，输入两条 Stream 的 `{title, summary, 最近 5 个 WC 标题}`
- 合并执行：
  - `keep_id` 保留，`drop_id` 归档（`status='done'`）
  - `UPDATE work_contexts SET stream_id = keep_id WHERE stream_id = drop_id`
  - 合并后的 stream 用 LLM 给的新 title/summary 刷新
- 安全阀：单次 reconcile 合并 ≤ 5 对（比 Project 合并阈值 3 宽松，因为 Stream 更细）

### 6.2 Stream 状态转换（SQL）

```sql
-- active → paused：2 天无 touch
UPDATE project_streams SET status='paused', updated_at=datetime('now')
 WHERE status='active' AND last_active_at < datetime('now', '-2 days');

-- paused → done：7 天无 touch
UPDATE project_streams SET status='done', updated_at=datetime('now')
 WHERE status='paused' AND last_active_at < datetime('now', '-7 days');

-- paused → active：复活（resolver 期间可能有新归属）
UPDATE project_streams SET status='active', updated_at=datetime('now')
 WHERE status='paused' AND last_active_at >= datetime('now', '-2 days');
```

这一段放在 `status.rs`（阶段 B），复用 B 的事务；不再占 C。

### 6.3 NULL 兜底补跑

扫 `stream_id IS NULL` 且 `project_id IS NOT NULL` 的活跃 WC，对每条调 `stream_resolver::resolve`。上限：单次 reconcile 最多补 30 条 WC（防止 LLM 调用过量）。

### 6.4 Title/Summary 刷新

Stream 累计 ≥5 条 WC 且上次 `updated_at` 超 3 天未刷新时，用最近 5 条 WC 的 title/summary 喂 LLM 生成新的 Stream title/summary。上限：单次 reconcile ≤ 10 条。

这一步是优化项，v1 可以先**不做**（只做 6.1–6.3），v1.1 补。

### 6.5 阶段 C 新结构

```rust
pub async fn run(deps: &ReconcileDeps, summary: &mut ReconcileSummary) -> Result<()> {
    merge_streams(deps, summary).await?;     // 6.1
    backfill_null_streams(deps, summary).await?;  // 6.3
    // refresh_titles(deps, summary).await?;  // 6.4, v1.1
    Ok(())
}
```

---

## 7. 前端：`/now` 重构

### 7.1 新结构

```
SheetPageHeading（"NOW · 今天在推进 N 件事"）
  ├ Tabs: 今天 / 本周 / 归档
  └ 左：ProjectStreamList（Project 折叠 header，Stream 为主条目）
     右：StreamDetailPanel（Stream summary + 该 Stream 下的 WC 列表）
        WC 点击 → 右侧 Sheet 滑出 WC 详情
```

### 7.2 Kicker 计数逻辑

```
今天 tab：count = unique stream_id where last_active_at in today
本周：count = unique stream_id where last_active_at in last 7 days
归档：count = streams with status='done'
```

### 7.3 左列：ProjectStreamList

新文件 `src/pages/now/project-stream-list.tsx`，替换现有 `project-grouped-list.tsx`。

数据源：
- `useProjects(status='active')` — 取活跃 Project
- `useStreamsGrouped(tab)` — 新 hook，返回 `{ project_id → ProjectStream[] }`（见 §7.6）

渲染：
```
▼ Corivo 主应用
    • 改 /now 成 Stream 视图          4 WC · 2h ago
    • 写 stream hot-path spec         2 WC · 刚刚
    • 修 resolver 超时日志            3 WC · 昨天
▼ GUM 论文阅读
    • 读 arXiv:2505.10831 §3          5 WC · 3d ago
```

- Stream 条目显示：title + WC 数量徽标 + last_active 相对时间
- Project 折叠态持久化到 `ui-store.expandedProjects: Record<number, boolean>`
- 没有活跃 Stream 的 Project 不显示（避免空组）
- 孤儿兜底：`project_id=NULL` 或 `stream_id=NULL` 的 WC 聚到底部"未归类"section

### 7.4 右列：StreamDetailPanel

新文件 `src/pages/now/stream-detail-panel.tsx`，替换 `work-context-detail-panel.tsx`（后者降级为 Sheet 浮层组件，见 §7.5）。

内容：
- 顶部：Stream title + 状态 pill（active/paused/done）+ Project 面包屑
- 中部：Stream summary
- 下部：该 Stream 下所有 WC，按 `last_seen_at DESC` 列表（标题 + 时间）。条目点击 → 触发 Sheet

当 `selected` 为 null（首次进入、无任何 Stream）：显示引导文案。

### 7.5 WC Sheet 浮层（分级折叠）

复用 shadcn `Sheet` 组件（`side="right"`）。把现 `WorkContextDetailPanel` 的内容**重新分三级**——最重要的直接露，次要的折叠，完全不丢数据。

**一级（始终展开）**：
- Header：标题 · 状态 pill · archive 按钮（保持 header 可见）
- Summary 段（核心文案）
- 时间：`started_at → last_seen_at`、持续时长、相对"刚刚/2h ago"

**二级（`<details>` 或 Accordion 折叠，默认收起）**：
- "信号" —— `signals` 列表（app / url / file / repo 等）
- "相关 Project / Stream" —— 面包屑式链接

**三级（再内一层或 Accordion 次级）**：
- "轨迹" —— `episode-timeline`（具体 observation / screenshot 追溯）

路由级不变，URL 不反映 Sheet 开关状态（v1 简化）。

### 7.6 Hooks

新增 `src/hooks/use-streams.ts`：

```ts
// 扁平：用于 Stream 详情、跳转
useStream(streamId: number | null)

// 分组：/now 左列主查询
useStreamsGrouped(tab: 'today' | 'week' | 'archived'): {
  projects: Project[];
  streamsByProject: Record<number, ProjectStream[]>;
  workcontextCountByStream: Record<number, number>;
  orphanWorkContexts: WorkContext[]; // stream_id 或 project_id null
}

// Stream 下的 WC 列表：用于右侧 detail panel
useStreamWorkContexts(streamId: number | null): WorkContext[]
```

新 Tauri command `list_streams_grouped(tab)`（`src-tauri/src/commands/reconcile.rs` 或新文件 `commands/stream.rs`）：服务端一次 join 返回前端所需全部 shape，避免 N 次 round-trip。响应：

```rust
#[derive(Serialize)]
pub struct StreamsGroupedResponse {
    pub projects: Vec<Project>,
    pub streams: Vec<ProjectStream>,  // 含 project_id
    pub wc_counts: Vec<(i64, u32)>,   // (stream_id, count)
    pub orphans: Vec<WorkContext>,    // project_id=NULL 或 stream_id=NULL 的活跃 WC
}
```

### 7.7 删除 / 降级

- **删** `src/pages/now/focus-card.tsx`（从 `now-page.tsx` 移除引用）
- **降** `src/pages/now/project-grouped-list.tsx` → 保留一版本迭代，下一轮清掉
- **降** `work-context-detail-panel.tsx` → 重命名为 `work-context-detail-sheet.tsx`，包裹在 Sheet 里

### 7.8 Tabs 重命名

现 `进行中 / 今天 / 归档` → 新 `今天 / 本周 / 归档`（顺序保持，语义稍调）。过滤逻辑从 WC.status 改成 "该 tab 的 Streams"，见 §7.2。

---

## 8. 前端：Settings 记忆维护区

### 8.1 位置

新 section 加在 `src/pages/settings/settings-page.tsx` 的 **推送** / **捕获** 之后、危险区之前。

标题："记忆维护"，副标题："Corivo 自动整理你的 Project 和 Stream，也可以手动触发一次。"

### 8.2 内容

```
├ 上次整理：2d ago · 合并 2 个 Project · 新增 3 条 Stream · 归档 1 条
│     ["展开摘要"] → 折叠显示 summary_json 的结构化字段
├ [立即整理]  (button, primary)  调 useReconcileNow
│     loading 态：按钮变 spinner + 文案"正在整理…"
│     失败：toast 显示 errors[]
├ [全量重算归属]  (button, secondary)  调 useBackfillAttribution
│     适用场景：旧数据全是"未归属"，想一次扫完 project_id=NULL 或
│     stream_id=NULL 的所有活跃 WC，不受 reconcile 30 条/次上限约束
│     确认对话框："预计会调 N 次 LLM（N=未归属 WC 数），确认继续？"
│     loading 态：显示"已处理 X / N"进度（通过 tauri 事件推）
├ [查看历史]  (button, secondary)  弹出 Sheet 列出近 5 次 ReconcileLogEntry
```

### 8.3 组件

新文件 `src/pages/settings/reconcile-maintenance-card.tsx`。

### 8.4 全量重算归属后端

新 Tauri command：`backfill_attribution_now() -> BackfillSummary`。放在 `commands/reconcile.rs` 或 `commands/stream.rs`。

行为：
- 取所有 `status='active'` 且 (`project_id IS NULL` OR `stream_id IS NULL`) 的 WC
- 依次调用 `project_resolver::resolve`（若 project_id 为 NULL）、`stream_resolver::resolve`（若 stream_id 为 NULL）
- **不受 reconcile §6.3 的 30 条上限**
- 串行执行，防止 LLM 过载
- 每处理 10 条发一次 `attribution-backfill-progress` 事件（`{processed, total}`）
- 与 `ReconcileService.running` mutex 共享，防止和定时 reconcile 并发

响应：

```rust
#[derive(Serialize)]
pub struct BackfillSummary {
    pub scanned: u32,
    pub project_assigned: u32,
    pub stream_assigned: u32,
    pub failed: u32,
    pub duration_ms: i64,
}
```

### 8.3 组件

新文件 `src/pages/settings/reconcile-maintenance-card.tsx`：

```tsx
export function ReconcileMaintenanceCard() {
  const { data: logs } = useReconcileLog(5);
  const { mutateAsync, isPending } = useReconcileNow();
  const last = logs?.[0];
  // ... 卡片渲染
}
```

---

## 9. Config

已有 `ReconcileConfig` 不动（`enabled / merge_enabled / stream_enabled / interval_hours / min_workcontexts_per_project_for_stream`）。

`min_workcontexts_per_project_for_stream` 在阶段 C 新版里**不再使用**（Stream 已 hot-path 生成，不做 batch 聚类）。保留字段以免 migration，但逻辑无视之。注释里标 deprecated。

新增 `StreamResolverConfig`（如需开关调试）：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct StreamResolverConfig {
    /// 总开关（false → stream_id 永远 NULL，reconcile 阶段 C 也跳过）
    pub enabled: bool,
    /// LLM 超时
    pub llm_timeout_secs: u64,
    /// 候选集上限
    pub candidate_limit: u32,
}

impl Default for StreamResolverConfig {
    fn default() -> Self {
        Self { enabled: true, llm_timeout_secs: 8, candidate_limit: 8 }
    }
}
```

挂到 `Config::stream_resolver`。

---

## 10. 执行顺序

1. **Prompt**：写 `prompts/project_stream_resolve.md`（hot path）+ `prompts/project_stream_merge.md`（reconcile 阶段 C §6.1）
2. **Repo**：`ProjectStreamRepo::list_active_for_project`、`WorkContextRepo::set_stream` / `recent_titles_for_stream` / `inherit_stream_from`
3. **hot path**：新 `services/work_context/stream_resolver.rs` + 快路径测试 + LLM path 测试
4. **pipeline 串入**：`pipeline.rs::process_batch` 里 project_resolver 之后调 stream_resolver
5. **Reconcile 阶段 C 重写**：`services/project_reconcile/stream.rs` 改为 merge + backfill；B 阶段加 Stream 状态 SQL
6. **Config**：新增 `StreamResolverConfig`
7. **新 command**：`list_streams_grouped` + 注册
8. **前端 types/hooks**：`ProjectStream` 类型（若缺）+ `use-streams.ts`
9. **前端 /now 重构**：`ProjectStreamList` / `StreamDetailPanel` / WC Sheet；删 FocusCard
10. **前端 Settings 记忆维护卡**
11. **手动 QA**：`pnpm tauri dev` 本地跑半天，观察新 WC 是否即时得到 stream_id、/now 新视图是否可读、"立即整理"是否能触发
12. **清理**：删除 `prompts/project_stream_cluster.md`、旧 `project-grouped-list.tsx`、`focus-card.tsx`

---

## 11. 测试

### 11.1 Rust 单测

| 位置 | Case |
|---|---|
| `stream_resolver` | 候选 0 → new（快路径，不调 LLM） |
| `stream_resolver` | 单活跃候选 + 24h 内 → inherit（快路径） |
| `stream_resolver` | 多候选 + LLM 返 existing → set_stream + touch |
| `stream_resolver` | 多候选 + LLM 返 new → insert + set_stream |
| `stream_resolver` | LLM 超时 → 返 None，stream_id 留 NULL |
| `stream_resolver` | merge 幸存者已有 stream_id → inherit 不调 LLM |
| `reconcile::stream` | 2 条相似 Stream + LLM 合并 → drop 归档、WC 迁移 |
| `reconcile::stream` | backfill NULL：30 条上限 |
| `reconcile::status` | Stream 2 天无 touch → paused |
| `reconcile::status` | Stream 7 天无 touch → done |
| `reconcile::status` | paused + 新 touch → 回 active |

### 11.2 前端（Vitest）

| 位置 | Case |
|---|---|
| `use-streams` | tab=today 过滤正确 |
| `project-stream-list` | 空 Project 不渲染 |
| `project-stream-list` | 折叠态持久化到 ui-store |

### 11.3 集成（手动）

本地 `pnpm tauri dev` 跑 2 小时产出数据，肉眼验证：
- 每个新 WC 都得到 `stream_id`（DB 查）
- `/now` 左列每个 Project 下的 Stream 数在 1–5
- "立即整理"按钮能触发 + 结果卡片更新

---

## 12. 风险 & 缓解

| 风险 | 影响 | 缓解 |
|---|---|---|
| LLM hot path 每个 WC 加一次调用 | 成本翻倍（project + stream 各一次） | 快路径（§5.4）覆盖 30%+；失败吞错不阻塞；总调用频率仍在 1 次/分钟量级 |
| Stream 膨胀（LLM 倾向新建） | 左列过长 | prompt 强调"宁合不新"；reconcile 合并兜底 |
| Stream 碎片化（每条 1–2 个 WC） | 信息噪声 | reconcile 合并 + 3 天未刷新的 title 重生（v1.1）|
| 快路径误归（单活跃 Stream 场景不同主题被 inherit） | 错归 | 用户后续 WC 会触发 LLM 路径 + reconcile 合并纠正；实际危害低 |
| 迁移风险（旧 stream_id 是周级语义，新 hot path 语义不同） | 数据混合 | reconcile 合并会把语义漂移大的合掉；极端情况可加 one-shot migration 14 清 project_streams 表（本 spec v1 不做） |
| Settings "立即整理"被用户频繁点 | LLM 账单 | mutex 已有；UI 上 loading 态禁重复点 |

---

## 13. 回退

- `Config.stream_resolver.enabled = false` → hot path 全部跳过，`stream_id` 永远 NULL，`/now` 退回 Project > WC 视图的兜底桶（`orphanWorkContexts`）
- `Config.reconcile.stream_enabled = false` → 阶段 C 跳过
- 整套 `reconcile.enabled = false` → 保留 hot path，只停 daily 任务

任意一层都可单独关，无需回滚代码。
