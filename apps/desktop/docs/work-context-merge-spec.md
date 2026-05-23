# WorkContext 近邻合并 Spec（Pipeline 粒度修复）

> 版本：draft-0 · 依赖：无 · 阻塞：`project-layer-spec.md`

---

## 1. 问题陈述

当前 `/now` 列表里大量重复条目，典型形态：

```
阅读"论文截图时间选择分析"对话，停留在回复框……
在 Claude 连续阅读"论文截图时间选择分析"对话，重点比较……
持续在 Claude 阅读并编辑中文分析，重点完善"我的判断"……
在 Claude 阅读"论文截图时间选择分析"，重点查看 GUM 策略……
```

这四条本应是**同一个 WorkContext + 四个 episode**，但现在是**四个独立 WorkContext 各挂一个 episode**。

### 根因

[`process_batch`](../src-tauri/src/services/work_context/pipeline.rs) 的去重只依赖一条路径：

```rust
active_ids_matching_signals(&new_signals)  // 重叠 ≥1 → 合并
```

失败模式：
- LLM 每次从略不同的观察切片里提取 `signals`，命中同一个 `(kind, value)` 元组的概率远低于直觉
- 当 signals 是 `{app: Claude}` 这种高共享但低特异性的时候，反而会误合并；而 `{entity: GUM 策略}` 这种每次措辞都漂的，又完全错过
- 结果：WorkContext 降格成了"分钟级的 episode"，episode 降格成了"batch 级的快照"

### 目标

让 WorkContext 稳定在**小时级**——同一个人连着 30 分钟看同一个 Claude 对话，就是 1 个 WorkContext + N 个 episode。

保留现有 signal 重叠路径（它对"同一份代码继续改"这种强证据仍然最可靠），但在它失败时用 **FTS 近邻**兜底。

---

## 2. 非目标

- **不引入 embedding**。FTS5 + jieba 够用，先不增加向量表和模型依赖
- **不改 signal schema、不改 episode schema**。只改合并决策
- **不做跨 category 合并**。`project` 不会合并进 `task`

---

## 3. 合并决策的新形态

```
draft
  ├─ 1. SQL: active_ids_matching_signals(draft.signals) ≥ 1
  │     命中 → 合并到最大重叠的那条（现有行为）
  │
  ├─ 2. 未命中 → FTS: 最近 T_MERGE_WINDOW 内、同 category、status=active
  │     以 draft.title + "\n" + draft.summary 为查询文本
  │     取 top-1；非空则合并
  │
  └─ 3. 仍未命中 → 新建（现有行为）
```

命中合并后的写入动作（三种来源统一）：
1. `touch(id, reactivate=true)`（同今天）
2. `upsert_signal` 全部新 signals（把 signal 池并起来，下次重叠路径更可能命中）
3. `insert_episode` 一个新 episode（同今天）
4. `link_observation` 全部本批 observations（同今天）
5. **可选**：若新 draft 的 `summary` 长度比旧 summary 多 ≥ 20%，调 `update_text` 刷新 title/summary——用最新一批的描述替代（旧的往往更泛、新的进展更具体）。这一步有风险（可能抖动），**v1 先不做**，留给后续观察

### 参数

| 名称 | 值 | 说明 |
|---|---|---|
| `T_MERGE_WINDOW` | 30 min | FTS 候选窗口。小于 TTL、大于一般 batch 间隔 |
| FTS top-K | 1 | 只取最佳候选；命中即合并 |
| 最小 token 数 | 2 | 查询 token 少于 2 个时跳过 FTS（避免"Claude"这种单词误匹配） |

窗口 30 min 的理由：task 的 default TTL 是 30 min，这意味着"30 min 内还活着"的 active task 本来就应该被视作同一件事；超过 30 min 属于"又回到这件事"，走 signal 重叠重新命中更合适。

---

## 4. Schema 变更（migration v11）

新文件：`src-tauri/src/db/migrations/011_work_contexts_fts.sql`

```sql
-- v11: 给 work_contexts 加 FTS5 索引，让 pipeline 在 signal 重叠失败时走
-- 语义近邻兜底。与 propositions_fts (v6) 同款结构：contentless，rowid 与
-- work_contexts.id 对齐；中文分词由 jieba 在应用层完成，写入 tokens 列。
CREATE VIRTUAL TABLE work_contexts_fts USING fts5(
    tokens,
    content='',
    tokenize='unicode61 remove_diacritics 0'
);

-- 回填现有行（开发阶段数据量小，直接在 migration 里做；生产迁移时 Rust
-- 侧 apply_v11 会用 jieba 重新生成 tokens，这里只建空索引，apply 时回填）
INSERT OR IGNORE INTO schema_version (version) VALUES (11);
```

对应 `db/migrations.rs::apply_v11`：
1. 执行上面的 SQL
2. `SELECT id, title, summary FROM work_contexts` → 对每行调 `tokenize_for_index(title + "\n" + summary)` → `INSERT INTO work_contexts_fts(rowid, tokens) VALUES (?, ?)`

---

## 5. Repo 变更

文件：`src-tauri/src/db/repos/work_contexts.rs`

### 5.1 写路径同步 FTS

在以下方法里追加 FTS 写入（模仿 propositions repo 的套路）：

| 方法 | FTS 动作 |
|---|---|
| `insert` | `INSERT INTO work_contexts_fts(rowid, tokens) VALUES (?, ?)` |
| `update_text` | `UPDATE work_contexts_fts SET tokens = ? WHERE rowid = ?` |
| `archive` | 不变（archived 也允许被检索，但 FTS 候选查询会过滤 status） |

trait 不变；实现体加几行即可。单测：对 insert/update_text 后查询 FTS 能命中。

### 5.2 新方法：`fts_candidates_within`

```rust
#[async_trait]
pub trait WorkContextRepo: Send + Sync {
    // ... 现有方法

    /// 在时间窗 + status=active + 可选 category 内，用 FTS 找和 `query_text`
    /// 最相近的 work_contexts。返回按 FTS bm25 排序的 top-N。pipeline 在
    /// signal 重叠未命中时用它兜底合并。
    async fn fts_candidates_within(
        &self,
        query_text: &str,
        since: DateTime<Utc>,
        category: Option<WorkContextCategory>,
        limit: u32,
    ) -> Result<Vec<WorkContext>>;
}
```

SQL 形如：

```sql
SELECT c.{SELECT_COLUMNS}
  FROM work_contexts_fts f
  JOIN work_contexts c ON c.id = f.rowid
 WHERE work_contexts_fts MATCH ?1
   AND c.status = 'active'
   AND c.last_seen_at >= ?2
   AND (?3 IS NULL OR c.category = ?3)
 ORDER BY bm25(work_contexts_fts) ASC
 LIMIT ?4
```

查询字符串用 `tokenize_for_query`（已存在，见 `user_model/tokenize.rs`）生成。返回空 → 跳过 FTS。

---

## 6. Pipeline 变更

文件：`src-tauri/src/services/work_context/pipeline.rs`

### 6.1 注入新参数

`DistillerCtx` 不用变（`work_context_repo` 已经是 Arc）。

在文件顶 加常量：

```rust
const FTS_MERGE_WINDOW_MINUTES: i64 = 30;
const FTS_MIN_QUERY_TOKENS: usize = 2;
```

### 6.2 改 `process_batch` 的决策段

原本是：

```rust
let matches = ctx.work_context_repo.active_ids_matching_signals(&new_signals).await?;
let merged_into = matches.into_iter().find(...).map(|(id, _)| id);
```

改成：

```rust
let merged_into = resolve_merge_target(ctx, &draft, &new_signals).await;
```

`resolve_merge_target` 签名：

```rust
async fn resolve_merge_target(
    ctx: &DistillerCtx,
    draft: &WorkContextDraft,
    new_signals: &[NewSignal],
) -> Option<i64> {
    // 1. Signal overlap（原路径）
    if let Some(id) = match_by_signals(ctx, new_signals).await {
        return Some(id);
    }
    // 2. FTS 兜底
    match_by_fts(ctx, draft).await
}
```

`match_by_fts` 细节：

```rust
async fn match_by_fts(ctx: &DistillerCtx, draft: &WorkContextDraft) -> Option<i64> {
    let query_text = format!("{}\n{}", draft.title, draft.summary);
    let query_expr = tokenize_for_query(&query_text)?;  // Option<String>
    // token 数门槛
    if query_expr.split(" OR ").count() < FTS_MIN_QUERY_TOKENS {
        return None;
    }
    let since = Utc::now() - chrono::Duration::minutes(FTS_MERGE_WINDOW_MINUTES);
    let candidates = ctx.work_context_repo
        .fts_candidates_within(&query_expr, since, Some(draft.category), 1)
        .await
        .ok()?;
    candidates.into_iter().next().map(|c| c.id)
}
```

命中后的 touch / upsert_signal / insert_episode / link_observation 代码**不变**——无论是 signal 路径还是 FTS 路径，后续动作完全一致。

### 6.3 事件类型

`WorkContextChangeKind` 增加 `MergedByFts` 变体（便于前端 debug 和后续 reconcile 度量），或者复用 `Touched`。**v1 复用 `Touched`**——前端不区分。Tracing 日志里打 `merge_source = "signal" | "fts"` 即可。

---

## 7. 前端影响

**无**。Now 页面不变，只是条目会变少、每条的 episode 会变多。

---

## 8. 测试

### 8.1 单测（`work_context/pipeline.rs` 同目录）

| Case | 构造 | 期望 |
|---|---|---|
| 纯 signal 命中 | 已有 ctx 含 `{file: a.rs}`；新 draft 含 `{file: a.rs}` | 合并、走 signal 路径 |
| 纯 FTS 命中 | 已有 ctx title "阅读 GUM 论文"；新 draft title "继续看 GUM 论文" + signals 完全不同 | 合并、走 FTS 路径 |
| FTS 超窗 | 同上但旧 ctx 的 `last_seen_at` 在 31 分钟前 | **不合并**，新建 |
| FTS 跨 category | 旧是 task、新 draft 是 topic 但文本相近 | **不合并**（category 不同） |
| token 过少 | draft 只有 "Claude" 一个词 | **不合并**（跳过 FTS） |
| 都失败 | signals 零重叠、文本零交集 | 新建 |

### 8.2 集成验证（手动）

在开发机上回放 1 天的 capture，比较：
- v10 数据：`/now` 列表长度 & 每条的 episode 数
- v11 数据：同样回放

目标：**列表长度降至 v10 的 30–60%，平均 episode 数提升 2–3 倍**。

---

## 9. 执行顺序

1. 写 migration 011 + `apply_v11` 回填逻辑
2. 改 repo：`insert/update_text` 写 FTS + 新方法 `fts_candidates_within`
3. 改 pipeline：抽 `resolve_merge_target` + 实现 `match_by_fts`
4. 写 6 个单测（覆盖§8.1 所有 case）
5. 本机 `pnpm tauri dev` 跑 1–2 小时，肉眼确认 `/now` 列表不再重复同一对话
6. 合入；用这批更干净的数据推进 `project-layer-spec.md`

---

## 10. 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| FTS 误合并（不同对话但文本相近） | WorkContext 把两件事揉成一个 | 窗口限 30 min + 同 category；出现再降窗口或加最小 bm25 阈值 |
| FTS 索引与主表不同步 | 老数据检索不到，新建率升高 | migration 回填 + repo 写路径覆盖三处（insert/update_text/archive 无需同步） |
| jieba 启动慢压住首次合并 | 第一分钟合并退化 | `user_model::mod.rs` 已有 warmup，work_context 复用同一个 jieba 单例，不额外付 |
