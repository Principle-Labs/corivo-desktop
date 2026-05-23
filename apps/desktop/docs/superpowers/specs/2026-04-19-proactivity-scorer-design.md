# Proactivity Scorer (C 阶段) — 设计文档

**创建于**：2026-04-19
**所在阶段**：Proactive Push Roadmap 的 **C 阶段第一子系统（P_S 主动性分数）**
**关联文档**：
- [Proactive Push Roadmap](../plans/2026-04-18-proactive-push-roadmap.md)
- [B 阶段 Suggestion Generator Spec](2026-04-19-suggestion-generator-design.md)

**对应论文**：
- ContextAgent §4.2（[arXiv:2505.14668](https://arxiv.org/abs/2505.14668)）—— P_S 1-5 主动性分数 + θ 阈值机制
- GUMBO §4.3.2（[arXiv:2505.10831](https://arxiv.org/abs/2505.10831)）—— Mixed-Initiative 把决策让给模型

---

## 1. 背景与目标

B 阶段已经把推送 payload 从"命题文本"升级到"3 条 LLM 生成的建议"。但**决定要不要推**还由 4 道硬阈值 gate 把守：

```rust
struct PushConfig {
    base_cooldown_secs: u32,
    dismiss_penalty_secs: u32,
    min_confidence: u32,
    push_on_contradict: bool,
}
```

这种硬规则有两个根本问题：
1. **同一条命题，confidence=5 时该不该推？** —— 取决于内容是否相关，不是数字大小能判定的
2. **"7 天 cooldown"是用户偏好还是绝对真理？** —— 用户兴趣会随时间变化，硬规则无法适应

C 阶段第一子系统：**用 LLM 评分（P_S, 1-5）替代硬阈值**，θ 默认 4，可由用户调节。保留一条不可逾越的硬规则：**用户主动 dismiss 过的 revision_group 永久封禁**——确保用户的明确否决永远不被覆盖。

**显式不做**（留给后续子系统）：
- 不做 token-bucket 全局速率限制（push history 已喂给模型让它判 spam）
- 不做反馈回流到 GUM（独立子系统）
- 不做 P_S 评估指标 dashboard（C+ 阶段，等数据积累后做）

---

## 2. 架构总览

```
PropositionRevised broadcast
        ↓
push_pipeline::handle_event
        ↓
[硬规则] notification_log 里同 revision_group 是否曾 dismissed？
        ├─ 是 → SKIP（不烧 LLM）
        └─ 否
                ↓
proactivity_scorer.score(proposition):
   ├─ 拉 G（top 5 相关命题，复用 retrieve_related）
   ├─ 拉最近 3 条 observations
   ├─ 拉 push 历史（同组最近 3 条 + 全局最近 1h）
   ├─ render prompts/score.md
   ├─ LLM.complete_json → {p_s: 1-5, reasoning: string}
   └─ proactivity_scores_repo.insert(...)（无论后续推不推都存）
        ↓
P_S ≥ θ ？（θ 来自 config.user_model.push.proactivity_threshold，默认 4）
        ├─ 否 → SKIP（但 score 已入库）
        └─ 是
                ↓
suggestion_generator.generate (B 阶段已有，不动)
        ↓
notification_log.insert (含 suggestion_id + proactivity_score_id)
        ↓
notification_service.send (overlay)
```

**关键架构决策**：

1. `services/push_decider.rs` 整个模块**删除** —— 旧四道硬 gate 全没了，"硬规则 + scorer"逻辑直接长在 `push_pipeline.rs`。
2. `proactivity_scorer` 是**独立 service**，不复用 suggestion_generator，但**两者共享 `retrieve_related` 函数**（B 阶段把它埋在 suggestion_generator 内部，C 阶段抽到公共 module）。
3. **scorer 失败不存表**——抛 Err 上去，push_pipeline 跳过本次推送、记 `tracing::warn!(scorer_failed=true)`。`proactivity_scores` 表只存"模型真说了话"的 P_S，便于评估指标干净。
4. **`notification_log.proactivity_score_id`** 反向链接到 score 行——推送历史页能直接看到"为啥推了它"。

---

## 3. 数据流细节

### 3.1 硬规则：dismissed-ever 检查

新加 repo 方法：

```rust
// db/repos/notification_log.rs
async fn group_ever_dismissed(&self, group: &str) -> Result<bool>;
```

SQL：`SELECT 1 FROM notification_log WHERE revision_group = ?1 AND outcome = 'dismissed' LIMIT 1`

判 `true` 直接 return Ok(()) 不做后续。日志：`tracing::info!(outcome="skip", reason="group_dismissed_ever", ...)`。

### 3.2 Scorer 输入（中等档）

| 信号 | 来源 | 检索方法 |
|------|------|---------|
| 当前命题 | `proposition.text/reasoning/confidence/decay` + `contradicts_proposition_id` | 已有 |
| G top 5 | `retrieve_related`（B 阶段抽出来） | 复用 |
| 最近 3 条 observations | 新加 `observation_repo.recent(3)` | 新方法 |
| 同组最近 3 条 push | `notification_log.recent_in_group(group, 3)` | 新方法 |
| 全局最近 1 小时 push | `notification_log.recent_window(Duration::hours(1))` | 新方法 |

push 历史只带 `pushed_at / outcome / suggestion_id 是否非空`——**不带具体文本**（省 token + 减少隐私泄露面）。

### 3.3 Scorer 失败 → skip

任何一步抛 Err（retrieve / LLM / JSON parse / P_S 越界 1-5）：

1. `tracing::warn!(scorer_failed=true, reason=…, proposition_id=...)`
2. **不写 `proactivity_scores` 表**
3. 直接返回，不调 suggestion_generator，不写 notification_log
4. 用户当次收不到这条命题的推送

### 3.4 P_S 评分流程

```rust
// services/proactivity_scorer.rs
pub async fn score(&self, proposition: &Proposition) -> Result<ProactivityScore>
```

成功路径：

1. 并发拉 G / observations / push_history（`tokio::join!` 三个 query）
2. 渲染 `prompts/score.md`
3. `complete_json::<RawScore>` 返回 `{p_s, reasoning}`
4. 校验 `p_s ∈ 1..=5`，否则 Err
5. 写 `proactivity_scores` 表，返回带 id 的 `ProactivityScore`
6. push_pipeline 拿这个 id 后续 insert notification_log 时关联

### 3.5 push_pipeline.handle_event 改写

```
1. by_id(proposition_id) 拿命题
2. 硬规则：group_ever_dismissed → SKIP
3. proactivity_scorer.score → ProactivityScore | Err → SKIP
4. p_s >= θ ？否 → SKIP（但 score 已入库）
5. suggestion_generator.generate → bundle (Some) | Err → bundle = None
6. notification_log.insert(suggestion_id, proactivity_score_id, ...)
7. notification_service.send：
   - bundle = Some → title = "Corivo 想跟你说一句", body = bundle.surfaced.text
   - bundle = None → title = "Corivo 学到了一条新判断", body = proposition.text（B 阶段降级保留）
```

**关键不变量**：C 阶段只改"是否要推"的决策（步骤 3-4 是新的），不改"已决定推之后的文案 fallback"（步骤 5-7 完全沿用 B 阶段）。即：scorer 通过 + suggestion 失败 → 仍然推（降级文案 + `notification_log.suggestion_id = NULL` + `proactivity_score_id = Some`）。

---

## 4. 数据模型

### 4.1 新表 `proactivity_scores`

```sql
CREATE TABLE proactivity_scores (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    proposition_id    INTEGER NOT NULL REFERENCES propositions(id) ON DELETE CASCADE,
    p_s               INTEGER NOT NULL CHECK (p_s >= 1 AND p_s <= 5),
    reasoning         TEXT    NOT NULL,
    threshold_at_time INTEGER NOT NULL,
    scored_at         TEXT    NOT NULL
);

CREATE INDEX idx_proactivity_scores_proposition_id ON proactivity_scores(proposition_id);
CREATE INDEX idx_proactivity_scores_scored_at ON proactivity_scores(scored_at DESC);
```

不存 `was_pushed`——通过 `notification_log` 是否存在指向这条 score 的行直接 derive，避免双写竞态。

### 4.2 改 `notification_log`

```sql
ALTER TABLE notification_log
    ADD COLUMN proactivity_score_id INTEGER NULL
        REFERENCES proactivity_scores(id) ON DELETE SET NULL;
```

`NULL` 表示这条推送是 **C 阶段上线前的旧记录**（B 阶段没 P_S）。

### 4.3 简化 `PushConfig`

**之前**（B 阶段）：
```rust
pub struct PushConfig {
    pub base_cooldown_secs: u32,
    pub dismiss_penalty_secs: u32,
    pub min_confidence: u32,
    pub push_on_contradict: bool,
}
```

**现在**（C 阶段）：
```rust
pub struct PushConfig {
    pub proactivity_threshold: u8,  // 1..=5, default 4
}
```

`#[serde(default)]` 让旧 user config JSON 里残留的 4 个字段被 serde 静默忽略——**零用户侧迁移**。`proactivity_threshold` 缺失时用 default 4。

`Config::validate()` 改：删 4 个旧检查，加 `proactivity_threshold in 1..=5`。

### 4.4 新 repo 方法

`db/repos/notification_log.rs` 新增：
- `group_ever_dismissed(group: &str) -> Result<bool>`
- `recent_in_group(group: &str, limit: u32) -> Result<Vec<NotificationLogEntry>>`
- `recent_window(window: Duration) -> Result<Vec<NotificationLogEntry>>`（全局最近 1h）

`db/repos/observations.rs`（如果还没有）新增：
- `recent(limit: u32) -> Result<Vec<Observation>>`

新 repo 文件 `db/repos/proactivity_scores.rs`：

```rust
#[async_trait]
pub trait ProactivityScoreRepo: Send + Sync {
    async fn insert(&self, new: NewProactivityScore) -> Result<ProactivityScore>;
    async fn by_id(&self, id: i64) -> Result<Option<ProactivityScore>>;
    async fn by_proposition_id(&self, proposition_id: i64) -> Result<Vec<ProactivityScore>>;
}

pub struct NewProactivityScore {
    pub proposition_id: i64,
    pub p_s: u8,
    pub reasoning: String,
    pub threshold_at_time: u8,
}

pub struct ProactivityScore {
    pub id: i64,
    pub proposition_id: i64,
    pub p_s: u8,
    pub reasoning: String,
    pub threshold_at_time: u8,
    pub scored_at: DateTime<Utc>,
}
```

### 4.5 抽公共函数 `retrieve_related`

B 阶段 `suggestion_generator.retrieve_related` 是私有的。C 阶段 scorer 也要用同样的逻辑。**移到 `services/user_model/retrieval.rs`**（已有的公共 retrieval 模块），暴露为：

```rust
// services/user_model/retrieval.rs
pub async fn related_for_proposition(
    repo: &dyn PropositionRepo,
    cfg: &RetrievalConfig,
    prop: &Proposition,
    limit: u32,
) -> Result<Vec<ScoredProposition>>;
```

逻辑：调 `query()` + 过滤掉同 `revision_group` 的版本（包括自身）。`SuggestionGenerator::retrieve_related` 删除，改为调用这个公共函数。`ProactivityScorer` 也调它。

### 4.6 删除旧代码

- `services/push_decider.rs` 整个文件**删除**（`PushDecider` trait、`DefaultPushDecider`、`PushConfig` 内部副本、`SkipReason` 枚举、`MockClock`、`SystemClock` —— `SystemClock` 还有用，迁移到 `services/mod.rs` 或 `push_pipeline.rs`）
- `tests/push_decider.rs` 删除
- `tests/push_pipeline_dismiss_penalty.rs` 删除（dismiss penalty 概念没了）

### 4.7 Migration v8

新增 `db/migrations/008_proactivity.sql`：
```sql
CREATE TABLE proactivity_scores (...);
CREATE INDEX idx_proactivity_scores_proposition_id ...;
CREATE INDEX idx_proactivity_scores_scored_at ...;
ALTER TABLE notification_log ADD COLUMN proactivity_score_id INTEGER NULL ...;
INSERT OR IGNORE INTO schema_version (version) VALUES (8);
```

`migrations.rs` 注册 `apply_v8`，`LATEST_VERSION = 8`。

---

## 5. Prompt 设计

新文件 `src-tauri/prompts/score.md`：

````markdown
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
````

### 5.1 占位符填充规则

| 占位符 | 空时填 | 非空格式 |
|--------|--------|----------|
| `{{contradicts_clause}}` | `""` | `（与命题"X..."冲突）` |
| `{{related_propositions}}` | `（暂无相关命题）` | 一行一条：`- {text}（置信度 {c}）` |
| `{{recent_observations}}` | `（暂无最近活动）` | 一行一条：`- [{HH:MM}] {content截断 200 字}` |
| `{{group_push_history}}` | `（这是该主题首次推送）` | 一行一条：`- {pushed_at}: {outcome 或 "未响应"}` |
| `{{global_recent_summary}}` | `过去 1 小时无推送` | `过去 1 小时推送 N 条（acknowledged X / dismissed Y / 未响应 Z）` |

### 5.2 渲染函数

加在 `services/user_model/prompts.rs`（与 `render_propose / render_similar / render_revise / render_suggest` 并列）：

```rust
pub fn render_score(input: &ScoreInput) -> String { ... }

pub struct ScoreInput {
    pub proposition_text: String,
    pub proposition_reasoning: String,
    pub proposition_confidence: i32,
    pub proposition_decay: i32,
    pub contradicts_text: Option<String>,
    pub related: Vec<SuggestRelated>,  // 复用 B 阶段的类型
    pub recent_observations: Vec<RecentObservation>,
    pub group_push_history: Vec<PushHistoryRow>,
    pub global_last_hour: PushFrequencySummary,
}

#[derive(Debug, Clone)]
pub struct RecentObservation {
    pub created_at: DateTime<Utc>,  // for [HH:MM] prefix
    pub content: String,            // truncated to 200 chars at fill time
}

#[derive(Debug, Clone)]
pub struct PushHistoryRow {
    pub pushed_at: DateTime<Utc>,
    pub outcome: Option<String>,    // None => "未响应"; Some(s) => s as-is
}

#[derive(Debug, Clone)]
pub struct PushFrequencySummary {
    pub total: u32,
    pub acknowledged: u32,
    pub dismissed: u32,
    pub no_response: u32,           // total - acknowledged - dismissed
}
```

`PushFrequencySummary` 由 `proactivity_scorer` 在拉完 `recent_window(Duration::hours(1))` 后用 outcome 字段聚合得到（不是 repo 层的工作）。

### 5.3 LLM 调用

| 项 | 取值 |
|----|------|
| Provider | 复用 `config.summary.provider`（与 PROPOSE/SIMILAR/REVISE/SUGGEST 同源） |
| Method | `LlmProviderExt::complete_json::<RawScore>` |
| Temperature | provider 内置 `TEMPERATURE_STRUCTURED` (0.1)——P_S 应该确定性高 |
| Schema | JSON object，`{p_s: number 1-5, reasoning: string}` |

---

## 6. 配置 + UI

### 6.1 PromptDebugConfig 加 score_override

跟 B 阶段 suggest_override 一样的 pattern：

```rust
pub struct PromptDebugConfig {
    pub summary_override: Option<String>,
    pub push_judgment_override: Option<String>,
    pub suggest_override: Option<String>,
    pub score_override: Option<String>,  // 新增
}
```

`normalize()` 也对 `score_override` 调一次 `normalize_optional_prompt_override`。

### 6.2 Frontend types.ts

```typescript
export interface PushConfig {
  proactivity_threshold: number; // 1-5
}

export interface PromptDebugConfig {
  summary_override: string | null;
  push_judgment_override: string | null;
  suggest_override: string | null;
  score_override: string | null;
}
```

### 6.3 Settings UI

**位置**：`src/pages/settings/sections/general-section.tsx` 的"窗口与通知"FieldGroup。

**删除**：现有"冲突时仍推送"ToggleRow（push_on_contradict 没了）。

**新增**：主动性滑块（用原生 `<input type="range">`，避免引入 shadcn Slider 依赖）：

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

### 6.4 Prompt 调试页加 score_override

`src/pages/settings/sections/prompt-debug-section.tsx`：跟 B 阶段 suggest_override 完全一样的 pattern——加一个新 FieldGroup "评分 Prompt"，默认值常量 `DEFAULT_SCORE_PROMPT` 复制 `prompts/score.md` 内容。

### 6.5 测试桩同步

`src/lib/config-tauri.test.ts` 里 PushConfig 测试 fixture 改：

```typescript
push: {
  proactivity_threshold: 4,
}
```

PromptDebugConfig fixture 加 `score_override: null`。

---

## 7. 测试策略

### 7.1 单元测试

**`src-tauri/tests/proactivity_scores_repo.rs`**（新）
- `insert_persists_row_with_threshold_snapshot`
- `by_id_round_trips`
- `by_proposition_id_returns_in_scored_at_order`
- `check_constraint_rejects_p_s_zero_and_six`

**`src-tauri/tests/proactivity_scorer.rs`**（新，用 `MockLlm`）
- `happy_path_inserts_score_and_returns_it`
- `score_with_empty_g_observations_history_still_works`
- `invalid_json_returns_err_no_db_write`
- `p_s_out_of_range_returns_err`
- `llm_exhausted_returns_err`
- `db_insert_failure_returns_err`

**`src-tauri/tests/notification_log_repo.rs`**（扩展）
- `group_ever_dismissed_returns_true_when_any_dismissed_in_group`
- `group_ever_dismissed_returns_false_for_acknowledged_or_ignored`
- `recent_in_group_returns_ordered_by_pushed_at_desc`
- `recent_window_returns_only_within_duration`

**`src-tauri/tests/observations_repo.rs`**（如不存在则新建）
- `recent_returns_n_newest_observations`

**`src-tauri/tests/config_validate.rs`**（扩展）
- `proactivity_threshold_zero_rejected`
- `proactivity_threshold_six_rejected`
- `score_override_blank_normalizes_to_none`

### 7.2 Snapshot 测试

`src-tauri/tests/prompt_snapshots.rs` 新增：
- `score_basic`
- `score_empty_context`
- `score_with_contradict`
- `score_high_frequency`

### 7.3 集成测试 `src-tauri/tests/push_pipeline_proactivity.rs`（新）

- `dismissed_ever_group_skipped_without_llm_call`
- `score_below_threshold_writes_score_but_no_push`
- `score_above_threshold_pushes_with_link`
- `scorer_failure_skips_no_score_no_push`
- `scorer_pass_but_suggestion_generator_fails_still_pushes_degraded`
- `threshold_change_uses_current_value_at_score_time`

### 7.4 Migration 测试

`src-tauri/tests/migrations_v8.rs`（新）
- `v8_creates_proactivity_scores_table_and_adds_notification_log_column`
- `v8_is_idempotent_when_run_twice`

### 7.5 删除的测试文件

- `src-tauri/tests/push_decider.rs` → 删
- `src-tauri/tests/push_pipeline_dismiss_penalty.rs` → 删
- `src-tauri/tests/push_pipeline_end_to_end.rs` 里的 cooldown case → 删；保留 `first_push_surfaces_suggestion_text_with_new_title` 和 `fallback_path_uses_proposition_text_and_null_suggestion_id`，但补上 scorer mock

### 7.6 现有测试 fixture 更新

任何旧 `PushConfig { base_cooldown: ..., dismiss_penalty: ..., min_confidence: ..., push_on_contradict: ... }` 字面量都要替换成 `PushConfig { proactivity_threshold: 4 }`。Plan 阶段先 grep 全仓库枚举触点。

### 7.7 前端

- `pnpm exec tsc --noEmit` 干净
- `src/lib/config-tauri.test.ts` fixture 更新
- 手动 UI smoke：滑块拖动 → 值持久化 → 重启后还在

---

## 8. 文件清单

### 新建（8 个）

- `src-tauri/src/db/migrations/008_proactivity.sql`
- `src-tauri/src/db/repos/proactivity_scores.rs`
- `src-tauri/src/services/proactivity_scorer.rs`
- `src-tauri/prompts/score.md`
- `src-tauri/tests/migrations_v8.rs`
- `src-tauri/tests/proactivity_scores_repo.rs`
- `src-tauri/tests/proactivity_scorer.rs`
- `src-tauri/tests/push_pipeline_proactivity.rs`

### 删除（3 个）

- `src-tauri/src/services/push_decider.rs`
- `src-tauri/tests/push_decider.rs`
- `src-tauri/tests/push_pipeline_dismiss_penalty.rs`

### 修改（约 16 个）

- `src-tauri/src/db/migrations.rs`（注册 v8，LATEST_VERSION = 8）
- `src-tauri/src/db/repos/mod.rs`（exports）
- `src-tauri/src/db/repos/notification_log.rs`（3 个新方法 + `proactivity_score_id` 字段）
- `src-tauri/src/db/repos/observations.rs`（`recent` 方法，如已存在则跳）
- `src-tauri/src/services/mod.rs`（exports）
- `src-tauri/src/services/push_pipeline.rs`（重写 handle_event，删 PushDecider 依赖，加 ProactivityScorer）
- `src-tauri/src/services/suggestion_generator.rs`（`retrieve_related` 抽出去）
- `src-tauri/src/services/user_model/prompts.rs`（加 `render_score` + 类型）
- `src-tauri/src/services/user_model/mod.rs` 或 `retrieval.rs`（接收 `retrieve_related`）
- `src-tauri/src/domain/config.rs`（PushConfig 简化 + score_override + validate）
- `src-tauri/src/lib.rs`（wire ProactivityScorer + 删 DefaultPushDecider 构造）
- `src-tauri/tests/prompt_snapshots.rs`（4 个 score_* snapshot）
- `src-tauri/tests/notification_log_repo.rs`（3 个新方法测试）
- `src-tauri/tests/config_validate.rs`（threshold 范围 + score_override normalize）
- `src-tauri/tests/push_pipeline_end_to_end.rs`（删 cooldown 测试 + 更新 fixture）
- `src-tauri/tests/suggestion_generator.rs` / `suggestion_generator_end_to_end.rs`（grep PushConfig 字面量替换）
- `src/lib/types.ts`（PushConfig + PromptDebugConfig）
- `src/lib/config-tauri.test.ts`（fixture）
- `src/pages/settings/sections/general-section.tsx`（删 toggle + 加滑块）
- `src/pages/settings/sections/prompt-debug-section.tsx`（加 score_override textarea）

---

## 9. 风险与缓解

| 风险 | 缓解 |
|------|------|
| 删 4 道 hard gate 后用户体验突变（推太多/太少） | θ 默认 4 保守；UI 滑块让用户立即调整；上线前先在自己机器跑 1-2 天观察 P_S 分布 |
| scorer LLM 失败 = 用户收不到推送 | tracing 计数 `scorer_failed`；失败率 > 5% 时再考虑 retry/fallback；proactivity_scores 表能反向 audit |
| dismiss = 永久封禁过严（"今天不再"被当成永久） | 当前先承担；后续 C+ 阶段加"被屏蔽话题管理"页面 + outcome 子类型 |
| 每条命题都烧 scorer token（成本上升） | dismiss-ever 硬规则短路一部分；prompt 紧凑（中等档 ~400 token）；上线后 1 周对比 B 阶段 LLM cost |
| schema_version 7→8 升级失败 | 阶梯式 migration + transaction 包裹（已有 pattern）；测试覆盖 v7 + 空库两种起点 |
| θ 改变后历史 score 可比性混乱 | `threshold_at_time` 快照保留 |
| push history 在 prompt 里泄露隐私 | 设计**不带 suggestion/proposition 文本**，只带 timestamp + outcome + revision_group——plan 阶段守住这点 |
| `retrieve_related` 重构破坏 B 阶段 suggestion_generator | 抽出去后两边 import；B 测试不变（行为兼容） |

---

## 10. 与未来阶段的衔接

- **C+ 反馈回流**（roadmap 里的另一件事）：`outcome` → 合成 observation 写回 GUM。本设计的 dismiss-ever 硬规则**与之兼容**——回流后即使模型自己学会"避开"，硬规则仍是兜底。
- **C+ token-bucket**：roadmap 提到的全局 1/min 速率限制。本设计里 prompt 已经把"全局最近 1h 推送数"喂给模型，token-bucket 可作为额外的硬护栏（"用户偏好级硬上限" + "模型软评分"双层）。
- **C+ 评估指标 dashboard**（Acc-P / MD / FD）：`proactivity_scores` 表 + `notification_log.outcome` 是数据基础，后续做评估页直接 join 这两张表。

---

## 11. 自检清单

- [x] 所有外部依赖（LLM、DB、retrieval）都有 mock 路径
- [x] 失败模式覆盖：scorer / suggestion_generator / DB 每一步都有定义
- [x] DB migration additive（不破坏 v7 数据），ALTER 加列不丢老行
- [x] `push_decider` 删除带来的 ripple（lib.rs 接线、测试 fixture）已枚举
- [x] θ 用户可调（UI 滑块），默认 4 保守
- [x] `threshold_at_time` 快照让历史可解释
- [x] dismiss-ever 硬规则保护用户的明确否决
- [x] Prompt 文件版本化（`<!-- version: 1 -->`）便于追溯
- [x] 与 C+ 阶段（反馈回流 / token-bucket）的衔接路径清晰
