# Suggestion Generator (B 阶段) — 设计文档

**创建于**：2026-04-19
**所在阶段**：Proactive Push Roadmap 的 B 阶段（最小增量）
**关联文档**：[../plans/2026-04-18-proactive-push-roadmap.md](../plans/2026-04-18-proactive-push-roadmap.md)
**对应论文**：GUM/GUMBO §4.3.1 "Discovering Suggestions"（[arXiv:2505.10831](https://arxiv.org/abs/2505.10831)）

---

## 1. 背景与目标

Corivo 现有推送链路只做"命题被修订 → 推命题文本"。GUMBO 论文指出主动助手应该推**建议**而非**知识**——即把抽象命题翻译成对用户当下有意义的、可对话的句子。

B 阶段目标：在 `push_decider` 决定要推之后，多一步 LLM 调用产出 3 条候选建议，把第一条作为 overlay body 推出去，3 条都存库。

**显式不做的事**（留给 C 阶段）：
- 不引入 Mixed-Initiative E[U] 评分或 ContextAgent 式 P_S 主动性分数（仍用现有四道硬 gate）
- 不引入 token-bucket 全局速率限制
- 不做反馈回流到 GUM 命题模型
- 不引入 chat / action handoff（建议只描述情境 + 轻量提议，不实际执行任何动作）

---

## 2. 架构总览

新增 service 模块 `services/suggestion_generator.rs`，在 `push_pipeline::handle_event` 里 `push_decider` 判定 `Push` **之后**、`notification_service.send` **之前** 同步调用。失败走 fallback，不阻断推送。

```
PropositionRevised broadcast
            ↓
   push_pipeline::handle_event
            ↓
     push_decider.decide()
            ├─ Skip → 记日志，结束（不烧 LLM token）
            └─ Push
                    ↓
        suggestion_generator.generate(&proposition)
                    ├─ retrieval::query → 相关命题 G
                    ├─ render prompts/suggest.md
                    ├─ LLM.complete_json → [{text, reasoning}; 3]
                    ├─ suggestions_repo.insert_batch → 3 行
                    └─ Ok(SuggestionBundle) | Err(_)
                    ↓
            ┌───────┴───────┐
        Ok(bundle)        Err(_)
            ↓                 ↓
   payload = 建议          payload = 命题（fallback）
   suggestion_id=Some     suggestion_id=None
            └────────┬────────┘
                     ↓
        notification_log.insert
                     ↓
        notification_service.send (overlay)
```

**新模块定位**：`suggestion_generator` 是纯 service（无 IO 副作用之外的状态），便于 mock LLM 单测。

---

## 3. 数据流细节

### 3.1 检索 G

调用现有 `user_model::retrieval::query`：

| 参数 | 取值 |
|------|------|
| `text` | `Some(proposition.text)`（FTS-style match_bonus 拉相关命题）|
| `limit` | 5 |
| `start_time` / `end_time` | None |
| `include_observations` | false |

返回的 `Vec<ScoredProposition>` 在传给 prompt 前过滤掉**当前 `proposition.revision_group` 内的所有版本**——这同时排除了自身和自己的旧版本，避免 LLM 看到重复的自己。

### 3.2 LLM 调用

| 项 | 取值 |
|----|------|
| Provider | 复用 `config.summary.provider`（与 PROPOSE/SIMILAR/REVISE 同源），B 阶段不加新配置 |
| Method | `LlmProviderExt::complete_json::<Vec<RawSuggestion>>` |
| Temperature | 由 provider 决定：`complete_json` 路径 → `TEMPERATURE_STRUCTURED`（0.1，强制结构化）。建议文本会偏稳定/略缺创意，B 阶段先接受；C 阶段如果觉得太"模板化"再考虑切 `complete_text` + 手动解析 |
| Schema | JSON array，恰好 3 个对象，每个 `{text: string, reasoning: string}` |

### 3.3 失败 fallback

任何一步失败（retrieve / LLM / JSON / insert）：
1. `tracing::warn!(suggestion_failed=true, reason=…)` 记日志
2. NotificationPayload 用老格式：title=`"Corivo 学到了一条新判断"`, body=`proposition.text`
3. `notification_log.suggestion_id = NULL`（C 阶段做评估时能区分降级 vs 真建议）
4. push 不被阻断 —— 用户永远收到 **something**

---

## 4. 数据模型

### 4.1 新表 `suggestions`

```sql
CREATE TABLE suggestions (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    proposition_id   INTEGER NOT NULL REFERENCES propositions(id) ON DELETE CASCADE,
    text             TEXT    NOT NULL,
    reasoning        TEXT    NOT NULL,
    position         INTEGER NOT NULL CHECK (position >= 0 AND position <= 2),
    created_at       TEXT    NOT NULL  -- ISO8601 UTC
);

CREATE INDEX idx_suggestions_proposition_id ON suggestions(proposition_id);
```

字段说明：
- `position`：LLM 输出顺序（0 = 推出去的那条；1/2 留作 C 阶段备用 / 评估）
- `reasoning`：LLM 自评 "为啥推荐这条"，仅入库不展示，B 阶段用于 prompt 调试
- 入库 3 条用同一个 transaction，要么全入要么不入（在 repo 层封装）

### 4.2 改 `notification_log`

```sql
ALTER TABLE notification_log ADD COLUMN suggestion_id INTEGER NULL
    REFERENCES suggestions(id) ON DELETE SET NULL;
```

`NULL` 表示本次推送是降级路径（LLM 失败用了原命题文本）。

### 4.3 Migration

新增 `db/migrations/007_suggestions.sql`，`schema_version` 6 → 7（当前 main 已到 v6，由 `006_propositions_fts.sql` 引入 jieba FTS）。同步更新 `db/schema.sql`（fresh install 路径）。

### 4.4 新 repo `db/repos/suggestions.rs`

```rust
pub trait SuggestionRepo: Send + Sync {
    async fn insert_batch(
        &self,
        proposition_id: i64,
        items: Vec<NewSuggestion>,
    ) -> Result<Vec<Suggestion>>;

    async fn by_id(&self, id: i64) -> Result<Option<Suggestion>>;

    async fn by_proposition_id(&self, proposition_id: i64) -> Result<Vec<Suggestion>>;
}

pub struct NewSuggestion {
    pub text: String,
    pub reasoning: String,
    // position 由 insert_batch 按 items 顺序自动分配 0/1/2
}

pub struct Suggestion {
    pub id: i64,
    pub proposition_id: i64,
    pub text: String,
    pub reasoning: String,
    pub position: u8,
    pub created_at: DateTime<Utc>,
}
```

---

## 5. Prompt 设计

新文件 `src-tauri/prompts/suggest.md`，与现有 prompt 同款**纯字符串替换**（不是 Handlebars——`services/user_model/prompts.rs` 的 `render_*` 函数都是 `.replace("{{key}}", value)`）。列表用单个占位符在 Rust 端预渲染：

````markdown
<!-- version: 1 | GUM §4.3.1 Discovering Suggestions (B 阶段) -->

你是 Corivo 的贴心观察者。Corivo 是一个长期观察用户行为、为用户建立"通用用户模型"的助手。
现在 Corivo 对用户产生了一条新的判断（命题），它打算用一句温和、自然的话告诉用户。

新命题：
- 内容：{{proposition_text}}
- 依据：{{proposition_reasoning}}
- 置信度：{{proposition_confidence}}/10

相关的已有命题（按相关性排序，作为情境上下文）：
{{related_propositions}}

请生成 3 条候选建议文本，要求：
- 用第二人称对用户说话（"你最近……"），口吻温和、不评判
- 描述 Corivo 观察到的情境，可以轻量提议（"要不要……"），但不许声称替用户做事
- 每条不超过 60 字
- 避免健康 / 关系 / 政治等敏感话题
- reasoning 字段写一句话解释你为什么这么建议（中文，30 字内）

仅输出 JSON 数组，恰好 3 个对象：

```json
[
  {"text": "...", "reasoning": "..."},
  {"text": "...", "reasoning": "..."},
  {"text": "...", "reasoning": "..."}
]
```

不要 Markdown 包裹，不要任何额外文字。
````

`{{related_propositions}}` 在 Rust 端按 G 是否为空预渲染：

```rust
let related_block = if related.is_empty() {
    "（暂无相关命题）".to_string()
} else {
    related
        .iter()
        .map(|p| format!("- {}（置信度 {}）", p.text, p.confidence.unwrap_or(0)))
        .collect::<Vec<_>>()
        .join("\n")
};
```

`render_suggest` 函数加在 `services/user_model/prompts.rs` 里（与 `render_propose` 等并列）。

---

## 6. UI 改动

仅改 NotificationPayload 内容，**前端零改动**：

| 字段 | 旧值 | 新值 |
|------|------|------|
| title | "Corivo 学到了一条新判断" | **"Corivo 想跟你说一句"** |
| body | `proposition.text` | `suggestions[0].text` |
| reasoning | `proposition.reasoning` | `proposition.reasoning`（不变 — 这是命题依据，不是建议依据） |
| confidence | `proposition.confidence` | `proposition.confidence`（不变） |
| 按钮 | 已处理 / 不再提醒 / 今天不再 / 不相关 | 不变 |

降级路径下 title/body 回落到旧值（兜底语义对得上）。

---

## 7. 配置

B 阶段**不引入** `SuggestionConfig`，常量硬编码在 `suggestion_generator.rs` 顶部：

```rust
const SUGGESTION_COUNT: usize = 3;
const RETRIEVAL_LIMIT: u32 = 5;
const MAX_TEXT_CHARS: usize = 80; // 60 字 + 富余
// Temperature 不在本模块控制——complete_json 走 provider 内置 TEMPERATURE_STRUCTURED
```

唯一例外：现有 `PromptDebugConfig` 已有 `summary_override` / `push_judgment_override`（让 prompt 调试页能热改 prompt），按一致性同步加：

```rust
pub struct PromptDebugConfig {
    pub summary_override: Option<String>,
    pub push_judgment_override: Option<String>,
    pub suggest_override: Option<String>,  // 新增
}
```

`suggestion_generator` 渲染 prompt 时优先读 `suggest_override`，未设置才读 `prompts/suggest.md`。前端 prompt 调试页对应增加一个 textarea。

C 阶段再视需要把硬编码常量提升到正式的 `SuggestionConfig`。

---

## 8. 测试策略

### 8.1 单元测试 `services/suggestion_generator.rs`

用 `providers/llm/mock.rs` 注入：

- ✅ Happy path：LLM 返回合法 JSON → 3 条都入库 → 返回第一条
- ✅ Retrieval 返回空 G → 仍能出建议（prompt 里 `related_propositions` 为空 list）
- ✅ LLM 返回非法 JSON → 返回 Err → 调用方走 fallback
- ✅ LLM 返回 < 3 条对象 → 返回 Err
- ✅ LLM 调用超时 → 返回 Err
- ✅ DB insert 失败 → 返回 Err（确保 transaction：3 条要么全入要么不入）

### 8.2 集成测试 `tests/suggestion_generator_end_to_end.rs`

新建：用真 SQLite + mock LLM 验证一次完整 PropositionRevised → suggestions 表 3 行 → notification_service.send 被调用且 payload 含建议文本。

### 8.3 改动现有 `tests/push_pipeline_end_to_end.rs`

加一个 case：mock LLM 失败 → notification 走降级 → suggestion_id=NULL。

### 8.4 Snapshot 测试 `prompts/suggest.md`

对 Handlebars 渲染做 snapshot（`insta` crate，仓库已在用），覆盖：
- 有 G 的情况
- G 为空的情况
- 命题 reasoning 含特殊字符的情况

---

## 9. 文件清单

### 新增

- `src-tauri/src/services/suggestion_generator.rs`
- `src-tauri/src/db/repos/suggestions.rs`
- `src-tauri/src/db/migrations/007_suggestions.sql`
- `src-tauri/prompts/suggest.md`
- `src-tauri/tests/suggestion_generator_end_to_end.rs`

### 修改

- `src-tauri/src/db/migrations.rs`（注册 v7 迁移；`LATEST_VERSION` 常量从 6 改 7）
- `src-tauri/src/db/schema.sql`（新表 + `notification_log` 新列）
- `src-tauri/src/db/repos/notification_log.rs`（`NewNotificationLog` / `NotificationLog` struct 加 `suggestion_id` 字段）
- `src-tauri/src/services/push_pipeline.rs`（`push_decider` Push 之后调 suggestion_generator）
- `src-tauri/src/lib.rs`（构造 `SuggestionRepo` + 注入 push_pipeline 依赖）
- `src-tauri/src/domain/config.rs`（`PromptDebugConfig` 加 `suggest_override` 字段）
- `src/lib/types.ts`（前端 `PromptDebugConfig` 同步加 `suggest_override`）
- `src/pages/settings/sections/prompt-debug-section.tsx`（加一个 textarea 让用户热改 suggest prompt）
- `src-tauri/tests/push_pipeline_end_to_end.rs`（加降级 case）

---

## 10. 风险与缓解

| 风险 | 缓解 |
|------|------|
| LLM 失败率高，用户大多数时候看到的还是命题文本（B 退化成 no-op） | tracing log 加 `suggestion_failed` 计数；上线 1 周后看 dismiss/ack 率对比 |
| 3 条建议 token 成本翻倍 | gated by push_decider 已经过滤掉 70-90% 命题；预计每天新增 LLM 成本 < 现有 PROPOSE 的 20% |
| LLM 偶尔吐敏感推断 | prompt 里明确禁止健康/关系/政治；reasoning 字段帮人工抽查 |
| `suggestions` 表无限增长 | C 阶段加 retention（如 90 天前的 LLM 自评 reasoning 字段可清空，保留 text） |
| schema_version 6→7 迁移在升级时失败 | 现有 `migrations.rs` 是阶梯式 + transaction 包裹，失败回滚；测试覆盖空库 / v6 库两种起点 |
| 命题文本极短或极长导致 prompt match_bonus 失效 / token 超限 | retrieval 已有 `match_bonus` fallback 机制（无 match 时仍按 confidence/decay 排序）；命题文本长度上限由 PROPOSE prompt 早就限制过 |

---

## 11. 与 C 阶段的衔接预留

- `suggestions` 表已有 `position` 字段，C 阶段加 P_S 分数列即可（无破坏性 migration）
- `notification_log.suggestion_id` 已存在，C 阶段评估时能 join 到具体推的是哪条建议
- `suggestion_generator` 是纯 service，C 阶段把 LLM prompt 加一个 `proactive_score` 输出字段就能复用
- 降级与正常路径在 `notification_log.suggestion_id` 已有 NULL/Some 区分 —— C 阶段做"建议 vs 原命题"的对比评估直接用这个字段分组

---

## 12. 评审清单（自检）

- [x] 所有外部依赖（LLM、DB、retrieval）都有 mock 路径
- [x] 失败模式覆盖：每一步失败的行为都已定义
- [x] DB migration 是 additive，不破坏现有数据
- [x] UI 零改动 —— 前端代码不动，降低集成测试面
- [x] Prompt 文件版本化（`<!-- version: 1 -->`），未来改 prompt 时容易追溯
- [x] 与 C 阶段的衔接路径清晰，不需要重做 schema
