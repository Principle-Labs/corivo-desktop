# Project Reconcile + Stream 聚类 Spec（周级收敛）

> 版本：draft-0 · 依赖：`project-layer-spec.md` 已合入 · 无后续阻塞

---

## 1. 问题陈述

上一份 spec 让 Project 层"能跑"，但它是**同步归属 + 一次性决策**：

- Project 可能被误建（冷启动阶段 LLM 上下文少，会把"阅读 GUM 论文"和"Corivo 主应用"分成两个其实应该合起来的 project）
- Project 可能永远不闭环（项目做完了，`last_active_at` 不再更新，但状态还是 `active`）
- **没有 Stream 层**——用户心智里同一 Project 下会切换"用户调研 / 技术调研 / 代码开发"，这层在 v1 完全不存在

Reconcile 是周级/日级的异步收敛过程，**不上 hot path**。它解决三件事：

1. **Project 合并**：两个在 LLM 眼里实质相同的 Project 合成一个
2. **Project 归档**：长期 dormant 的 Project 关闭
3. **Stream 聚类**：在 active Project 里按"推进阶段"切分 WorkContext

---

## 2. 非目标

- **不做 WorkContext 级别的 reconcile**。粒度修复已由 `work-context-merge-spec.md` 在同步路径解决
- **不做跨用户 / 多机同步**。Corivo 是本机应用
- **不做 LLM 驱动的 Project 拆分**（一个 Project 判定太大时拆两份）。拆分比合并危险得多，v1 先只做合并/归档；真需要时由 Stream 层自然承担拆分语义

---

## 3. 触发机制

### 3.1 何时跑

每 24 小时一次，在本地时间凌晨 3 点附近（避开用户活跃时段）。实现方式：

- 新 service `project_reconcile::spawn(cx)`，在 `lib.rs::run()` 启动时拉起
- 内部是 `tokio::time::interval(Duration::from_secs(60 * 15))`，每 15 分钟 tick 一次
- 每次 tick 比较 `now()` 和上次 run 的时间（存 `app_state.json` 或一个简单的 `reconcile_log` 表），过了 24h 才跑
- 好处：用户中途不开机也没关系，下次开机后很快就会补跑

### 3.2 手动触发

Tauri command `reconcile_now() -> ReconcileSummary`，用于开发期按钮触发和后续"设置页手动触发"。

### 3.3 单实例保护

一次 reconcile 内部串行跑；用 `tokio::Mutex` 防重入。不允许两次 reconcile 并发（LLM 调用多、写操作多）。

---

## 4. Reconcile 流程（总）

```
enter reconcile
  ├── 阶段 A：Project 合并（LLM）
  ├── 阶段 B：Project 归档 / 状态转换
  ├── 阶段 C：Stream 聚类（LLM，per-project）
  └── 写 reconcile_log（ran_at, summary_json）
exit
```

每个阶段独立可跳过（Config 里各有 `enabled` 开关）。

---

## 5. 阶段 A：Project 合并

### 5.1 候选

只考虑 `status='active'` 的 project，按 `last_active_at DESC` 取 top-20。超出 20 个的场景先不处理（数据量早期不会到）。

### 5.2 两两配对策略

不是真 $O(N^2)$。做法：

1. **粗筛**：对每个 project 用 `title + summary` 去 `work_contexts_fts`（复用 v11 的 FTS）查相似项，把 FTS 命中率高的 project 对列为候选对
2. 候选对限 top-5（最多 5 对送给 LLM）

### 5.3 LLM 判断

新 prompt：`project_merge_decide.md`，输入两个 project 的 `{title, summary, 最近 10 个 WorkContext 标题}`，输出：

```json
{"decision": "merge", "keep_id": 42, "title": "...", "summary": "..."}
```
或
```json
{"decision": "keep_separate"}
```

合并规则：
- `keep_id` 指定保留的 project（LLM 选活跃度更高 + 名称更合适的那个）
- 另一个 project 所有 WorkContext 通过 `UPDATE work_contexts SET project_id = keep_id WHERE project_id = drop_id` 迁过来
- 被合并的 project → `status='archived'`（不删，保留历史可追溯）
- 合并后的 project 用 LLM 给的新 title/summary 刷新

### 5.4 安全阀

单次 reconcile 合并 ≤ 3 对。超过说明 LLM 在飙合并，应让人看一眼。用日志告警。

---

## 6. 阶段 B：Project 状态转换

纯 SQL，不调 LLM：

```sql
-- active → dormant：近 7 天无 WorkContext 归属
UPDATE projects SET status = 'dormant', updated_at = datetime('now')
 WHERE status = 'active'
   AND last_active_at < datetime('now', '-7 days');

-- dormant → archived：近 30 天无 WorkContext 归属
UPDATE projects SET status = 'archived', updated_at = datetime('now')
 WHERE status = 'dormant'
   AND last_active_at < datetime('now', '-30 days');

-- dormant → active：复活（reconcile 期间可能有新归属让 last_active_at 刷了）
UPDATE projects SET status = 'active', updated_at = datetime('now')
 WHERE status = 'dormant'
   AND last_active_at >= datetime('now', '-7 days');
```

三步依次跑。archived 不会被 resolver 当作候选（见 §4.1 `status='active'` 过滤），等同于关闭。

---

## 7. 阶段 C：Stream 聚类

### 7.1 Stream 是什么

一个 Project 内部的"推进阶段 / 线头"，周级稳定。典型例子（Corivo 项目下）：
- `用户调研`
- `技术调研 / GUM 论文阅读`
- `代码开发 / 核心管线`
- `产品打磨 / UI 页面`

一个 Project 可能同时有 1–4 条活跃 Stream；WorkContext 归到其中一条（或 null）。

### 7.2 数据模型（migration 013）

```sql
CREATE TABLE project_streams (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id      INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  title           TEXT    NOT NULL,
  summary         TEXT    NOT NULL,
  status          TEXT    NOT NULL DEFAULT 'active'
                           CHECK (status IN ('active','paused','done')),
  started_at      TEXT    NOT NULL,
  last_active_at  TEXT    NOT NULL,
  created_at      TEXT    NOT NULL DEFAULT (datetime('now')),
  updated_at      TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_project_streams_project ON project_streams(project_id, last_active_at DESC);

ALTER TABLE work_contexts ADD COLUMN stream_id INTEGER
  REFERENCES project_streams(id) ON DELETE SET NULL;
CREATE INDEX idx_work_contexts_stream ON work_contexts(stream_id, last_seen_at DESC);
```

`stream_id` nullable：WorkContext 可以只有 project 没有 stream。

### 7.3 聚类算法

对每个 `status='active'` 且**近 14 天有 ≥ 5 条 WorkContext** 的 Project：

```
1. 拉该 project 近 14 天所有 WorkContext（title + summary + category + last_seen_at）
2. 拉该 project 当前所有 active streams（id, title, summary）
3. 调 LLM：
   - 输入：WorkContexts 列表 + 已有 Streams 列表
   - 输出：每个 WorkContext 的归属 `stream_id`（existing / new_stream_index / null）
     + 可能新增的 streams 数组 [{title, summary}]
     + 可能要 paused / done 的 stream ids（没有新成员且 summary 描述"已完成"）
4. 应用决策：
   - 新 stream → insert project_streams
   - 每个 WorkContext → UPDATE stream_id
   - paused/done → UPDATE project_streams.status
```

### 7.4 Prompt

新 prompt：`project_stream_cluster.md`。核心要点：

- 告诉 LLM："这是用户在这个 Project 下最近两周做的 N 件事。请帮他把它们按'推进阶段'分成 1–4 条 Stream"
- 示例给 2–3 个：研究项目下的 `文献阅读 / 实验 / 写作`；软件项目下的 `用户调研 / 设计 / 实现 / 测试`
- 强调 Stream 数量**不宜多**：同一时段 4 条已经很满；新 Stream 要有明确的"阶段"意义，不是单个 WorkContext 的放大

### 7.5 门槛

- 单 Project 的 WorkContext 数少于 5 → 跳过 stream 聚类（不够样本，维持 `stream_id=null`）
- 新 Stream 数 > 6 → reject（LLM 在碎片化），保留原始归属，日志告警

### 7.6 Stream 状态

- `active`：有 WorkContext 在近 7 天内 touch 过
- `paused`：超过 7 天没 touch，但 LLM 没判定 done
- `done`：LLM 明确判定（基于 summary 文本，如"完成"、"已上线"、"合入"）

纯 SQL 维护 active/paused；done 只有 LLM 能给。

---

## 8. 前端影响

### 8.1 `/now` 页面

二级分组：

```
▼ Project: Corivo 主应用
    ▸ Stream: 代码开发（3）
        [ WorkContext ]
        [ WorkContext ]
        [ WorkContext ]
    ▸ Stream: 产品打磨（2）
        [ WorkContext ]
        [ WorkContext ]
    ▸ 未归入 Stream（1）
        [ WorkContext ]
▼ Project: GUM 论文阅读
    ▸ 未归入 Stream（2）  # 成员少，没聚类
```

- Stream 折叠状态也进 `ui-store`
- `project_id = null` 的 WorkContext 继续在页尾"未归属"组
- Stream 的 title 直接显示，附带成员数

### 8.2 新 commands / hooks

```rust
#[tauri::command] list_project_streams(project_id: i64) -> Vec<ProjectStream>
#[tauri::command] reconcile_now() -> ReconcileSummary  // 手动触发
```

```ts
useProjectStreams(projectId: number)
useReconcileNow()  // mutation
```

### 8.3 设置页

Settings 页加一个区块"记忆维护"：
- 显示上次 reconcile 时间
- 按钮"立即整理"（调 `reconcile_now`）

---

## 9. Config 新字段

在 `domain/config.rs` 的 `Config` 里加：

```rust
pub struct ReconcileConfig {
    pub enabled: bool,          // 总开关
    pub merge_enabled: bool,    // 阶段 A
    pub stream_enabled: bool,   // 阶段 C
    pub interval_hours: u32,    // 默认 24
    pub min_workcontexts_per_project_for_stream: u32,  // 默认 5
}
```

默认全开、默认值如上。

---

## 10. reconcile_log 表（审计）

migration 013 同文件：

```sql
CREATE TABLE reconcile_log (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  ran_at        TEXT    NOT NULL,
  duration_ms   INTEGER NOT NULL,
  summary_json  TEXT    NOT NULL,  -- ReconcileSummary 序列化
  trigger       TEXT    NOT NULL CHECK (trigger IN ('auto','manual'))
);
CREATE INDEX idx_reconcile_log_ran_at ON reconcile_log(ran_at DESC);
```

`ReconcileSummary` 结构：

```rust
pub struct ReconcileSummary {
    pub projects_merged: u32,
    pub projects_dormant: u32,
    pub projects_archived: u32,
    pub streams_created: u32,
    pub streams_done: u32,
    pub workcontexts_reassigned: u32,
    pub errors: Vec<String>,
}
```

设置页的"上次 reconcile 时间"从这里读。

---

## 11. 测试

### 11.1 单测

| 阶段 | Case |
|---|---|
| A 合并 | LLM 返回 merge → WorkContexts 迁移 + 另一 project 归档 |
| A 合并 | LLM 返回 keep_separate → 两个 project 都不动 |
| A 合并 | 单轮 ≥ 4 对 merge → 只执行前 3 对，记警告 |
| B 状态 | 7 天无活跃 → active→dormant |
| B 状态 | 30 天无活跃 → dormant→archived |
| B 状态 | dormant 期间被归属 → 该行由 `last_active_at` 更新，B 阶段把它 dormant→active |
| C 聚类 | Project WC<5 → 跳过 |
| C 聚类 | LLM 返回合法归属 → stream 正确创建 + WC.stream_id 更新 |
| C 聚类 | LLM 新建 >6 streams → reject，保留原状态 |
| 调度 | 24h 未满 → tick 跳过 |
| 调度 | 手动 `reconcile_now` → 立即跑 |
| 重入 | 两个并发调用 → 后者阻塞到前者完成 |

### 11.2 集成

手造两份"本应合并的 project"数据 + 一份 WC 超过 5 的 project，调 `reconcile_now`：
- 合并成功、归档成功
- 新 streams 数 1–4
- `reconcile_log` 有新行

---

## 12. 执行顺序

1. migration 013（project_streams + work_contexts.stream_id + reconcile_log）
2. `ProjectStreamRepo` + `ReconcileLogRepo`
3. 阶段 B（纯 SQL，最简单，先验证 migration + repo 正确）
4. 阶段 A（合并：候选粗筛 + LLM prompt + 执行）
5. 阶段 C（Stream 聚类：prompt + 应用决策）
6. `project_reconcile::spawn` 定时器
7. `reconcile_now` command + 手动触发 UI
8. `/now` 页面加二级分组
9. 本机跑一轮 + 看 `reconcile_log` summary

---

## 13. 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| LLM 在阶段 C 狂建 Stream | Stream 碎片化 | 上限 6 的 reject 阈值 + 最小 5 个 WC 门槛 |
| 阶段 A 合错（两个独立项目被合成一个） | 数据污染，难恢复 | 单轮 ≤ 3 对 + archived 不删 + 后续 UI 加"撤销合并"（v2） |
| 调度飘移（机器睡眠、日历跳变） | reconcile 不跑 | 用"距上次时间差" 而不是 cron 式固定时刻 |
| 阶段 C 的 LLM prompt 成本 | 每 project 一次调用 × 活跃 project 数 | 只对 WC ≥ 5 的 project 跑；一天一次、成本可控 |
| Stream 语义漂移（同一条 stream 每次聚类取到不同 title） | 用户看不到稳定的 stream | prompt 里先喂"已有 streams"，要求 LLM 优先归到已有，title 保持 |
| 老数据 `stream_id=null` 长期留白 | UI 始终有"未归入 Stream" | 正常现象；Project 成员够 5 条后 reconcile 会自动归；新用户前两周不强求 |
