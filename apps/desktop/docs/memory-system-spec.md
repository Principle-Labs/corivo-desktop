# Memory System Spec

> 版本：draft-0 · 状态：设计阶段，未实现
> 依赖：[corivo-architecture-v3-spec.md](corivo-architecture-v3-spec.md)（frames-as-truth 基础）
> 影响面：`apps/desktop/src-tauri/src/db/schema.sql`、`services/exec_agent/local_context.rs`、新增 `services/memory/`、新增 `services/persona/`、`packages/shared-types`、`/ask` & Quick Ask 的工具集

---

## 1. 问题陈述

Corivo 现在有两条原始数据源：

1. **行为流**——`frames` 表，由 capture_pipeline 写入，AX/OCR/adapter 抽取的文本 + 截图 + adapter_payload。这一层已经稳定。
2. **对话流**——`chat_threads` + `chat_messages`，`/ask` 和 Quick Ask 的会话历史，`cited_frame_ids` 让单条消息能反向引用具体 frame。

但用户期待的 "Corivo 是有记忆的" 不止于此。从用户视角，记忆要做到三件事：

1. **记得我做过的事**（行为）
2. **记得我说过的话**（陈述，且区分"随口说"和"必须记住"）
3. **记得我是个什么样的人**（画像、习惯、偏好）

第三件事现在完全没有；第二件事虽然有原始数据但没有"重要性"概念——所有消息平权，agent 只能靠 recall 命中重提。这份 spec 给这三件事一个统一模型、统一存储、统一召回入口。

**非目标**：

- 跨设备同步记忆（隐私优先，本期完全本地）
- 给 web/api 端暴露记忆查询接口（desktop-only）
- 重做 frames 抽取管线（继续复用 v3）

---

## 2. 三层记忆模型

用经典的情节-陈述-语义三分映射：

| 用户视角 | 心理学名 | Corivo 落地 | 现状 |
|---|---|---|---|
| 记得做过的事 | episodic | `frames` | 已有 |
| 记得说过的话 | declarative | `chat_messages` + **新增 `notes`** | 部分 |
| 记得我是谁 | semantic | **生成 `auto-persona.md`** | 无 |

**核心认知**：三层不是平级三张表，是**沉淀深度不同**。

- `frames` —— 流水，量大、廉价、易朽（90 天 retention 后清掉）
- `chat_messages` —— 中间层，量中、未升格前只是聊天历史
- `notes` —— 用户/agent 显式声明要长记的，量小、贵、权威
- `auto-persona.md` —— 从前三者蒸馏出的自然语言画像，量小、计算产物、可被 note 推翻

升格关系：`message → note`（agent 在线调用 `save_note` 或会话闲置学习）、`(frames + messages + notes) → auto-persona.md`（后台蒸馏）。

---

## 3. `notes` 表

### 3.1 Schema

Step 1 只需要 `notes` 基表，先跑通"用户让我记住 → 下次自动生效"的闭环。
全文检索用的 `search_tokens` / `notes_fts` 放到 Step 2，沿用现有 `frames_fts`
模式：Rust 侧用 `services::tokenize::tokenize_for_index` 写入空格分隔 token，SQLite
FTS5 继续用 `unicode61`，**不依赖 SQLite 内注册 `jieba` tokenizer**。

```sql
CREATE TABLE notes (
  id                  TEXT    PRIMARY KEY,             -- ULID
  content             TEXT    NOT NULL,                -- 自然语言内容
  scope               TEXT    NOT NULL                 -- 见 §3.2
                              CHECK (scope IN ('global','project','session')),
  scope_ref           TEXT,                            -- project_id / thread_id; global 时为 NULL
  source_type         TEXT    NOT NULL                 -- 见 §3.3
                              CHECK (source_type IN ('user_explicit','agent_inferred')),
  source_message_id   TEXT,                            -- FK → chat_messages.id（可空：用户直接在设置面板创建时为 NULL）
  source_thread_id    TEXT,                            -- FK → chat_threads.id
  confidence          REAL    NOT NULL DEFAULT 1.0,    -- 0..1，agent_inferred 时 < 1
  status              TEXT    NOT NULL DEFAULT 'active'
                              CHECK (status IN ('active','suggested','superseded','contradicted','archived')),
  superseded_by       TEXT,                            -- FK → notes.id；某条 note 被另一条覆盖时指向新版本
  created_at          TEXT    NOT NULL,                -- DbInstant
  updated_at          TEXT    NOT NULL,
  last_referenced_at  TEXT,                            -- 最后一次被召回命中的时间
  expires_at          TEXT                             -- 可选硬过期，默认 NULL = 永不过期
);

CREATE INDEX idx_notes_scope_status ON notes(scope, status);
CREATE INDEX idx_notes_scope_ref ON notes(scope, scope_ref) WHERE scope_ref IS NOT NULL;
```

Step 2 再补全文索引：

```sql
ALTER TABLE notes ADD COLUMN search_tokens TEXT;

-- FTS5 索引，与 frames_fts 一样索引 Rust 侧 jieba 预分词结果
CREATE VIRTUAL TABLE notes_fts USING fts5(
  search_tokens,
  scope UNINDEXED,
  source_type UNINDEXED,
  content='notes', content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2'
);

-- 触发器同步 notes ↔ notes_fts
CREATE TRIGGER notes_ai AFTER INSERT ON notes BEGIN
  INSERT INTO notes_fts(rowid, search_tokens, scope, source_type)
  VALUES (new.rowid, COALESCE(new.search_tokens, ''), new.scope, new.source_type);
END;
CREATE TRIGGER notes_ad AFTER DELETE ON notes BEGIN
  INSERT INTO notes_fts(notes_fts, rowid, search_tokens, scope, source_type)
  VALUES ('delete', old.rowid, COALESCE(old.search_tokens, ''), old.scope, old.source_type);
END;
CREATE TRIGGER notes_au AFTER UPDATE ON notes BEGIN
  INSERT INTO notes_fts(notes_fts, rowid, search_tokens, scope, source_type)
  VALUES ('delete', old.rowid, COALESCE(old.search_tokens, ''), old.scope, old.source_type);
  INSERT INTO notes_fts(rowid, search_tokens, scope, source_type)
  VALUES (new.rowid, COALESCE(new.search_tokens, ''), new.scope, new.source_type);
END;
```

### 3.2 `scope` 语义

| scope | 注入策略 | 例子 | scope_ref |
|---|---|---|---|
| `global` | **永久注入** system prompt（§6.1） | "我叫 Larry"、"用中文回我"、"别再问要不要确认" | NULL |
| `project` | 仅在当前 project 上下文出现时召回 + 注入 | "这个仓库用 4 空格缩进" | project_id |
| `session` | 仅在当前 thread 内有效 | "今天我们只讨论性能" | thread_id |

`project` 层依赖未来的 project 模型；本期暂用 NULL 占位，预留字段以便未来不改 schema。

### 3.3 `source_type` 语义

| source_type | 来源 | 默认 confidence |
|---|---|---|
| `user_explicit` | 用户在对话里明说"帮我记下/以后都用 X"，agent 识别后写入 | 1.0 |
| `agent_inferred` | agent 或后台会话学习判定"这是个长期偏好"，主动建议升格 | 0.6（默认 `status='suggested'`，不硬注入） |

**`source_type` 决定召回权重**，见 §7.4。`agent_inferred` 默认只是候选记忆，
不进入 persistent 注入，除非后续被用户确认为 `active`。

### 3.4 Notes 写入通路

不做"每个 turn 结束都跑一次升格判定"。那会带来固定 LLM 成本，而且只能看单轮上下文，
容易漏掉跨轮逐渐形成的偏好。本期改成两条写入通路：

#### 3.4.1 在线保存：`save_note` 工具

给 corivo-agent 暴露一个 native tool：

```ts
save_note({
  content: string,
  scope?: "global" | "session" | "project",
  source_message_id?: string,
  confidence?: number,
  reason?: string
})
```

同时把"什么时候应该保存记忆"写进 `system_prompt_extra` / Agent prompt：

- 用户明确说"记住"、"以后都"、"别再"、"我喜欢/我不喜欢" → 调 `save_note`，服务端写 `source_type='user_explicit'`、`status='active'`
- agent 觉得这是长期偏好但用户没有明确要求保存 → 可以调 `save_note`，但服务端写 `source_type='agent_inferred'`、`status='suggested'`
- 一次性事实、当前任务状态、临时讨论约束 → 不保存，除非用户明确要求

工具调用发生在正常对话过程中，不需要等 turn_end。写入失败只返回工具错误，不中断主回答。

#### 3.4.2 离线学习：会话闲置 / 结束后的 agent 任务

会话学习不是一次性问答——agent 需要自己决定看 thread 哪一段、要不要回查更早的相关消息、要不要交叉对照现有 notes 避免重复。因此**这条通路走 §11 后台 agent 任务**，而不是 stuff 整段对话给单次 LLM：

```
thread inactive ≥ idle_threshold / closed / app backgrounded
  → scheduler.rs 排入 FIFO 队列（§11.6）
  → BackgroundAgentTask::run<SessionMemoryLearningTask>()
       · system_prompt:      prompts/session_memory_learning.md
       · initial_user_message:
           "学习 thread <id> 自 <last_checkpoint_message_id> 之后的对话，
            找出值得长期记住的偏好。"
       · tools (§11.3):       chat_thread_get, note_list(scope='global')
       · max_turns:           10
       · output_schema:       [{ content, scope, source_type, confidence,
                                  source_message_id, reason }, ...]
  → Rust 侧 consume_output：校验 → 写 notes
       · source_type='user_explicit'  → status='active'
       · source_type='agent_inferred' → status='suggested'
  → 更新 checkpoint (thread_id, last_learned_message_id)
```

触发：

- thread 连续无操作 ≥ `Config.session_learner.idle_threshold_minutes`（默认 25 min）
- 用户显式关闭 / 归档 thread
- app 退出或进入后台时做 best-effort 调度（合并去重）

agent 自主决定要不要回查更早消息、查到什么程度，不局限于单次窗口；成本从"每 turn 固定一次"变成"每个有效会话最多若干次"。失败按 §11.5 落日志，checkpoint 不前进，下次 idle 自动重试同一窗口。

---

## 4. `auto-persona.md` 画像文档

### 4.1 为什么是 Markdown 文档

这层的主要消费方是 prompt，而不是查询系统。把 `communication_style`、
`active_apps`、`work_schedule` 这类维度固化成数据库标签，v0 阶段收益很低：

- agent 需要读的是一段自然语言上下文，而不是按标签查询字段
- 维度边界会变，过早建表会把产品理解锁死
- 用户需要看懂、校正、删除的是"Corivo 觉得我是怎样的人"，Markdown 比结构化行更直观
- 如果未来真的需要过滤 / 统计 / 排序，再从稳定的文档结构或独立 evaluator 中抽结构化层

所以 v0 不建 `traits` 表。`auto-persona.md` 是画像层的唯一产物，也是 prompt 注入源。

### 4.2 文档契约

- 路径：`app_data_dir/auto-persona.md`
- 来源：`frames` / `chat_messages` / `notes` 的后台蒸馏结果
- 语义：软画像，不是硬约束；`notes` 永远优先
- 更新：每日一次 + 启动时检查"距上次生成 > 24h 则跑"
- 版本：文件头写 `last_updated`、`evidence_window`、`generator_version`
- UI：设置页直接渲染 Markdown；用户校正时写入一条 `user_explicit` note，下次生成时覆盖旧画像

推荐结构（同时也是蒸馏 prompt 必须引导 LLM 产出的维度模板）：

```markdown
<!-- generated_by: corivo-persona-v1 -->
<!-- last_updated: 2026-05-13T03:00:00Z -->
<!-- evidence_window: 2026-04-13T00:00:00Z..2026-05-13T00:00:00Z -->

# 你的记忆画像

## 明确告诉过我的事
<!-- 来源：scope=global & source_type=user_explicit 的 active notes；
     是硬事实，prompt 装配走 persistent block，这里只是镜像展示，便于用户在设置页一站式审阅。 -->
- 用中文回复。
- 不要为了常规代码改动反复确认。

## 常用软件与用途
<!-- 从 frames.adapter_payload / active_apps 统计，挑使用频次最高 / 最有信息量的 5–10 个；
     一条 = 软件名 —— 用途 / 典型场景；不要罗列所有窗口。 -->
- Cursor / VS Code —— 主力代码编辑器，Rust 和 TypeScript 都在这里写。
- iTerm2 + zsh —— 命令行；常用 `pnpm`、`cargo`、`gh`。
- Chrome —— 文档查阅与 PR review，标签页通常超过 30 个。
- Linear —— 任务追踪，主要看自己 cycle 内的 issue。
- 飞书 / Slack —— 团队沟通；飞书占主导。

## 喜好倾向
<!-- 从 chat_messages + notes 中归纳的"做事偏好 / 表达偏好 / 工具偏好"；
     和"明确告诉过我的事"区分——这里允许是推测，但要给出可被反驳的具体描述。 -->
- 偏好直接、可执行、带文件路径的方案，不喜欢泛泛解释。
- 中文沟通；术语和代码标识符保持英文。
- 增量小步实现比一次大改更被接受。
- 倾向 pnpm + monorepo，避免引入新包管理器。

## 工作时间范围
<!-- 从 frames.captured_at 的分布估算活跃时段；标注时区；
     如果数据稀疏（< 2 周或 < N 个活跃日）则写"证据不足"，不要硬编。 -->
- 主要活跃时段：工作日 10:00–19:00 (Asia/Shanghai)，午休约 12:30–13:30。
- 晚间 21:00–23:00 偶有深度编码。
- 周末活动明显减少，仅零散提交。

## 性格习惯
<!-- 从交互模式归纳——回复长度倾向、对确认的容忍度、对错误的反应方式等；
     用"倾向 / 通常 / 不喜欢"这类软词，避免下定论。 -->
- 注重一次到位：失败时倾向追根因，而不是绕过。
- 不喜欢反复确认的交互；除非动作有不可逆代价才会接受二次确认。
- 喜欢把决策写成 spec 再实现，而不是边写边设计。
- 文档 / 注释偏少；让代码自解释。

## 我推测出的工作方式
<!-- 兜底章节：放不进上面 4 个维度但仍有信号的内容；保持 ≤ 5 条。 -->
- 你经常在 macOS 桌面应用、Rust/Tauri、agent 工具链之间切换。
- 最近主要责任在 corivo desktop 的核心服务层。

## 近期上下文
<!-- 仅最近 1–2 周的高密度活动，给 agent 一个"现在在干什么"的入口；
     这一节会随每次蒸馏明显变动，是过期最快的内容。 -->
- 最近频繁处理 Corivo desktop 的记忆系统、模型配置、connector 相关工作。
```

章节标题是给人和 LLM 读的，不是数据库 enum——renderer 可以按章节组织内容，但不要把章节当成长期稳定 API。但 **"常用软件与用途 / 喜好倾向 / 工作时间范围 / 性格习惯" 这四个维度是 v0 prompt 必须引导 LLM 主动覆盖的**，证据不足时显式输出"证据不足"而不是省略章节，这样 UI 也能反馈"再用一段时间画像会更准"。

### 4.3 蒸馏管线：agent 主动查证

画像不是从一份固定 evidence 摘要"翻译"出来的，是 agent 在只读工具支持下**自主决定要看什么、看多深**的产物。所以这条通路也走 §11 后台 agent 任务，而不是单次 LLM 调用——`prompts/persona_distill.md` 给目标和六个必覆盖的章节，agent 自己用 `frame_stats` / `frame_search` / `chat_history_search` 等只读工具拼出画像所需的证据。

```
services/persona/
├── mod.rs           -- PersonaService（启动时检查 last_run > 24h，调度入口）
├── task.rs          -- impl BackgroundAgentTask for PersonaDistillTask（§11.4）
├── conflict.rs      -- 与 notes 的冲突检测（§5）
└── renderer.rs      -- consume_output：冲突检测 → 原子替换 auto-persona.md
prompts/
└── persona_distill.md  -- agent 的 system prompt（要求覆盖 §4.2 六章节）
```

`PersonaDistillTask` 的 §11.4 契约具体值：

| 字段 | 值 |
|---|---|
| `KIND` | `SystemTaskKind::PersonaDistill` |
| `system_prompt` | `include_str!("../../prompts/persona_distill.md")` |
| `initial_user_message` | "生成本期画像。证据窗口：`<start>..<end>`（默认最近 30 天）。当前时区：`Asia/Shanghai`。上一版画像 token：`<persona_id>`，需要参考其有用片段并修订过期内容。" |
| `tool_whitelist` | `frame_search`, `frame_get`, `frame_stats`, `chat_history_search`, `chat_thread_get`, `note_list`, `auto_persona_get_previous`（见 §11.3） |
| `max_turns` | 30（画像任务交互轮次较多，比 session learner 大） |
| `output_schema` | 不强约束 JSON——直接要求最终一条 assistant message 是完整 Markdown |
| `consume_output` | 调 `conflict::remove_or_rewrite_conflicting_persona_lines` → `renderer::atomic_write(auto_persona_path)` |

`prompts/persona_distill.md` 必须显式要求 LLM **主动调用工具**而不是凭空推断，并把"常用软件与用途 / 喜好倾向 / 工作时间范围 / 性格习惯"这四节作为**硬要求维度**，证据不足时输出"证据不足"绝不省略章节。每个维度建议的取证路径（写进 prompt 引导 agent，不是 Rust 侧的强约束）：

| 章节 | 建议工具序列 | 取证规范 |
|---|---|---|
| 常用软件与用途 | `frame_stats(group_by='bundle_id', time_range=window)` → 对 top N 跑 `frame_search(bundle_id, limit=2)` 取窗口标题样本 | 按使用时长降序取 5–10 个；用窗口标题样本推断"用途"而非凭名字猜 |
| 喜好倾向 | `note_list(scope!='global', status='active')` + `chat_history_search(高互动话题关键词)` | 给可被反驳的具体描述；禁止"勤奋 / 聪明"这类空话 |
| 工作时间范围 | `frame_stats(group_by='hour_of_week', time_range=window)` | 显式给出时区；样本天数 < 14 或活跃日 < 7 时输出"证据不足" |
| 性格习惯 | `chat_history_search(关键词='不要 / 别 / 不喜欢')` + 抽样 `chat_thread_get` 看回复长度 / 追问模式 | 用"倾向 / 通常 / 不喜欢"软词；不下绝对结论 |
| 我推测出的工作方式 | 兜底——agent 看完前四节后自行总结剩余信号 | ≤ 5 条 |
| 近期上下文 | `chat_history_search(time_range=最近 14 天)` + `frame_stats(group_by='bundle_id', time_range=最近 7 天)` | 明确是"会快速过期"的章节 |

`conflict.rs` 的冲突检测按章节做：用户在 `notes` 中明确否定的内容（如 "我不用 Linear"）必须把对应章节里冲突的那一条删掉或改写，而不是只过滤"明确告诉过我的事"。

调度细节：

- 每日一次（默认凌晨 3 点本地时区，`Config.persona_distill.schedule_hour_local`）+ 启动时检查 `last_run > 24h`
- 一次完整任务的预期 token：5k–15k（取决于 evidence window 大小和 agent 决定回查的深度）
- 失败按 §11.5 落日志，下个调度周期重试；用户也能在设置页"诊断"标签里点"立即重跑"

---

## 5. 优先级与冲突

**单向偏序：`notes > auto-persona.md`。**

理由：note 是用户显式声明，persona 是统计 / LLM 推断，前者权威性永远高。

### 5.1 写入时检测

`conflict::remove_or_rewrite_conflicting_persona_lines(draft)` 在写入 `auto-persona.md` 前执行：

```
for paragraph in persona_draft.paragraphs:
    for note in active_authoritative_notes:
        if llm_judge_conflict(note.content, paragraph):
            remove_or_rewrite(paragraph, note)
```

`llm_judge_conflict` 是一个轻量 LLM 调用，prompt 在 `prompts/persona_note_conflict.md`，
输出 boolean + 建议 rewrite。本期不优化为本地小模型，复用主 llm_service；persona 蒸馏是每日级，开销可接受。

### 5.2 用户改 note 时的反向触发

`notes` 表写入/更新后，不做即时细粒度重算；标记 persona dirty。下一次 idle / daily
persona job 重新生成 `auto-persona.md`。如果用户显式修改了与当前 persona 冲突的 note，
可以触发一次低优先级后台重生成，但不要阻塞当前对话。

### 5.3 用户校正画像（反向通道，v1）

UI 在 `auto-persona.md` 的段落旁提供"不对"按钮。点击 → 在 `notes` 表追加一条
`scope=global, source_type=user_explicit, content="不是：<persona paragraph>；实际是：<user correction>"`。
下一次 persona 生成时会依据这条 note 删除或改写对应段落。

本期（v0）先不上段落级 UI 反向通道；设置页提供普通 note 新增 / 删除即可。

---

## 6. 三块 prompt 上下文

```
┌─────────────────────────────────────────────────────────────┐
│  system_prompt_extra 装配                                   │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────────────────┐                                │
│  │ Persistent (always-on)  │  notes.scope='global'          │
│  │                         │  + Agent.md  + Tools.md        │
│  │   不查询，每 turn 都拼   │  hard cap: 1000 tokens         │
│  └─────────────────────────┘                                │
│                                                             │
│  ┌─────────────────────────┐                                │
│  │ Persona (generated)     │  auto-persona.md                │
│  │                         │  soft context, cap 800 tokens  │
│  └─────────────────────────┘                                │
│                                                             │
│  ┌─────────────────────────┐                                │
│  │ Recalled (on-demand)    │  memory::recall(user_msg)      │
│  │                         │  返回 frames/notes/messages    │
│  │   每 user turn 召一次   │  按层 budget 截断（§7.6）       │
│  └─────────────────────────┘                                │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### 6.1 Persistent 通路

- **不参与召回**，无条件拼接
- 内容来源：`scope='global' AND status='active'` 且来源权威的 notes，按 `created_at` 排序
  - 包含：`user_explicit`
  - 排除：`agent_inferred` 默认产生的 `status='suggested'` 候选
- Token 上限：1000 tokens，超出截断最旧的（用户应被告知）
- 在 prompt 中以独立 block 标识，例如：

  ```
  <persistent_memory>
  <!-- 用户显式声明、永久生效 -->
  - 我叫 Larry
  - 用中文回我
  - 别问要不要确认，直接做
  </persistent_memory>
  ```

### 6.2 Recalled 通路

- 每个 user turn 触发一次 `memory::recall`，结果作为另一个 block 拼接：

  ```
  <relevant_memory>
  <!-- 与本轮可能相关，按相关性召回 -->
  ## 你可能记得的事
  - [2 周前] 调试 supermemory config 时，你提到偏好用 jj 而不是 git
  </relevant_memory>
  ```

- 三个 block 在 prompt 中**语义分开**：persistent 是硬事实，persona 是软画像，recalled 是本轮相关线索。

---

## 7. 召回层 `services/memory/`

### 7.1 模块组织

```
services/memory/
├── mod.rs            -- MemoryService + 公共类型 MemoryItem / RecallRequest / RecallResult
├── query.rs          -- query rewrite: user_msg → fts_query（jieba）
├── recall/
│   ├── mod.rs        -- 编排各 recaller
│   ├── frames.rs     -- 调 FrameRepo::fts_search；未来有 hybrid_search 再替换
│   ├── notes.rs      -- notes_fts MATCH + scope 过滤
│   └── messages.rs   -- chat_messages.content_text FTS（新增 FTS 表）
├── scorer.rs         -- 统一打分公式（§7.4）
├── budget.rs         -- 分层 token 截断（§7.6）
└── render.rs         -- Vec<MemoryItem> → prompt block markdown
```

与现有 `services/recall/` 的关系：

- 当前 `services/recall/` 基本只剩流式事件封装；不要再引用已经撤掉的
  `hybrid_search` / `tool_defs` / `tools` 模块。
- `services/memory/` 负责 **跨表统一召回**，被 `local_context.rs`（system-level 注入）和新工具 `memory_search`（tool-level 主动召回）共同使用。
- `memory::recall::frames` v0 直接 call `FrameRepo::fts_search`。如果后续重新引入 hybrid search，再把这里替换成共享后端。

### 7.2 数据契约

```rust
pub enum MemoryItem {
    Frame    { id: FrameId,   excerpt: String, score: f32, ts: DateTime<Utc> },
    Note     { id: NoteId,    content: String, score: f32, scope: NoteScope, source_type: NoteSourceType },
    Message  { id: MessageId, content: String, score: f32, thread_id: ThreadId, ts: DateTime<Utc> },
}

pub struct RecallRequest {
    pub user_message: String,
    pub thread_id: Option<ThreadId>,    // 用于 scope='session' 过滤
    pub project_id: Option<ProjectId>,  // 预留
    pub budget: RecallBudget,
}

pub struct RecallResult {
    pub items: Vec<MemoryItem>,
    pub diagnostics: RecallDiagnostics,  // 命中数、被 budget 截掉的数、耗时
}
```

`MemoryItem` 经 ts-rs 导出到 `@corivo/shared-types`，前端能识别召回来源（用于"Corivo 因为记得 X 所以这么回答"的可解释 UI）。

### 7.3 Query 构造

直接拿 user_message 喂 FTS5 会被停用词淹没。流程：

```
user_message
  ↓ jieba 分词（复用 services/tokenize.rs::tokenize_for_query）
  ↓ 过滤纯标点 / 空白 token（沿用 tokenize.rs 现有规则）
  ↓ 关键词加权：实体 > 动词 > 名词（v1，可先不做）
  ↓
fts_query = "\"并发\" OR \"concurrency\" OR \"文档\" OR \"docs\""
```

LLM rewrite 路径（v1 引入，本期不做）：当 jieba 召回率不达标时，加一道"LLM 把 user_message 改写成 1-3 个 FTS query"。

### 7.4 打分公式

```
score = bm25_normalized(fts)        -- FTS5 自带，归一到 [0, 1]
      × layer_weight                -- notes 3.0 / frames 1.0 / messages 0.5
      × recency_decay               -- exp(-Δdays / half_life)
      × scope_boost                 -- scope=current_project 时 ×1.5
      × source_boost                -- user_explicit 1.0 / agent_inferred 0.6
```

每项：

- **`layer_weight`**：硬编码常量，调参靠 §15 评测
- **`recency_decay`**：half_life 因层而异——frames 7 天、messages 30 天、notes 365 天
- **`scope_boost`**：`scope='global'` 的 note 本来就走 persistent 通路，召回通路里不参与；`scope='project'` 命中当前 project 时加权
- **`source_boost`**：仅对 notes 适用，其他层为 1.0

### 7.5 Persona 不参与召回

`auto-persona.md` 是 prompt 注入文档，不通过 `memory::recall` 查询，也不参与 FTS 打分。
需要给 agent 的画像上下文由 `local_context.rs` 直接读取并截断注入。这样避免"用户问几点工作"
这种问题被迫映射到预设标签。

### 7.6 Token Budget

```
total = 3500 tokens（可由 Config 调）
  ├── persistent (scope=global notes)  hard cap 1000  -- 见 §6.1
  ├── persona document                   ≤ 800
  ├── recalled notes (非 global)        ≤ 600
  ├── recalled frames                    ≤ 1000
  └── recalled messages                  ≤ 500
```

召回层独立选 top-N、独立截断；persistent / persona 走各自 hard cap。
不做全局合并 top-K，否则 frames 因数量优势会挤掉 notes。

Token 计数复用 corivo-agent sidecar 的 tokenizer——desktop 端用近似估算（4 字符 ≈ 1 token，中文 2 字符 ≈ 1 token），不要求精确。

### 7.7 双通路：system-level + tool-level

**System-level**（粗召）：

- 触发点：每个 user turn，`local_context.rs` 在装配 `system_prompt_extra` 时调用
- 输入：user_message
- 用途：覆盖广、廉价，把"显然相关"的塞进 prompt

**Tool-level**（精召）：

- 暴露为新工具 `memory_search(query: string, layers?: string[], scope?: string)`
- 注册位置：
  - `packages/agent/src/native-tools/` 新增 `memory-search.ts`
  - `packages/agent/src/native-tools/index.ts` 注册工具名
  - `apps/desktop/src-tauri/src/services/exec_agent/rpc_server.rs` 增加 `memory_search` dispatch
- 用途：agent 在思考中发现需要查更精确的东西时主动调用
- 调用同一个 `memory::recall` 后端，区别只是 query 由 agent 而非用户构造

两条都要。System-level 解决"用户没主动提但需要知道"，tool-level 解决"agent 中途发现需要"。

---

## 8. System Prompt 装配改造

改 `services/exec_agent/local_context.rs::load`：

**Before**（现状）：

```rust
pub fn load(app_data_dir: &Path) -> Option<String> {
    let soul = ensure_or_read(soul_path, DEFAULT_SOUL_MD);
    let agent = ensure_or_read(agent_path, DEFAULT_AGENT_MD);
    let tools = ensure_or_read(tools_path, DEFAULT_TOOLS_MD);
    Some([soul, agent, tools].join("\n\n"))
}
```

**After**：

```rust
pub fn load(
    app_data_dir: &Path,
    memory: &MemoryService,
    req: &RecallRequest,
) -> Option<String> {
    let persistent = memory.persistent(req.project_id);   // scope=global notes
    let persona    = read_auto_persona(app_data_dir);      // soft generated markdown
    let recalled   = memory.recall(req);                   // 其他

    let agent  = ensure_or_read(agent_path, DEFAULT_AGENT_MD);
    let tools  = ensure_or_read(tools_path, DEFAULT_TOOLS_MD);

    Some(render_prompt_extra(persistent, persona, recalled, agent, tools))
}
```

`Soul.md` 不再被读取（见 §9）。装配顺序是 **persistent notes → persona document → recalled memory → Agent → Tools**。
硬记忆先于软画像，软画像先于本轮相关线索。

调用点要拿到 `user_message`。当前 corivo-agent sidecar 已经是每个 turn 通过
`SidecarInput.system_prompt_extra` 接收 prompt extra，不需要新增
`set_system_prompt_extra` RPC。实现上只要把 `exec_agent_send` 里构造
`extra_system_prompt` 的位置改为调用新的 `local_context::load(..., req)`。

---

## 9. `Soul.md` 处置

### 9.1 删除文件角色

- `Soul.md` 不再被 `local_context.rs` 读取
- 用户对 `Soul.md` 的手工编辑**不再生效**；旧文件不会被读取或改写，只作为用户本地历史文件保留
- `Agent.md` / `Tools.md` 保留原样，这两个不是用户记忆，是 agent 行为约定与工具说明
- `prompts/default_soul.md` 可保留作历史默认文案，但不再 `include_str!` 到 `local_context.rs`

### 9.2 引入 `auto-persona.md`

- 具体文档契约见 §4.2。
- 文件系统层面允许写，但下次 persona 蒸馏完成会覆盖；UI 显示"自动生成"标识。
- 设置页渲染此文件，并提供新增 / 删除 notes 的入口；v0 不做段落级编辑。
- prompt 注入时包在独立 block 中，明确这是推断画像：

  ```markdown
  <persona_memory>
  <!-- 自动生成的软画像；如与 persistent_memory 冲突，以 persistent_memory 为准。 -->
  ...auto-persona.md...
  </persona_memory>
  ```

---

## 10. 数据库与跨边界改动

### 10.1 Schema 版本

- `TARGET_SCHEMA_VERSION` bump 到当时主线的下一个可用版本；本 spec 不锁死具体数字
  （数据库相关任务并行进行时，以落地时的 `schema.sql` / `migrations.rs` 为准）
- migration 模式仍是 **purge-and-apply**（`db/migrations.rs` 现有约定）
- legacy-drop 列表无需扩——`notes` 是新表，旧 DB 没有

### 10.2 新增 SQL

落在 `db/schema.sql`（追加，不动 frames 段落；`chat_threads` 段落 ALTER 加 5 列 + 1 张 FTS 虚表，`chat_messages` 段落改 CHECK 约束）：

- Step 1a：
  - `notes` 基表（§3.1）
- Step 1b：
  - `chat_threads` 增列 `kind` + `system_task`（§11.2）——后台 agent 任务和用户对话共表
  - `chat_threads` 增列 `summary` + `summary_topics` + `summary_updated_at` + `thread_summaries_fts` virtual table（§12.2）——session learner 副产物入口，供 `thread_search` 工具使用
  - `chat_messages.role` 的 CHECK 约束由 `('user','assistant')` 扩到 `('user','assistant','system')`——后台 agent 任务失败时落 `role='system'` 错误记录用（§11.5）
- Step 2：`notes.search_tokens` + `notes_fts`，以及 `chat_messages` 的 search-token FTS
- Persona 文档不需要新表；写入 `app_data_dir/auto-persona.md`

每一个 step 落地时按 §10.1 的约定单独 bump 一次 `TARGET_SCHEMA_VERSION`。

`chat_threads_list`（现有命令）需要改成默认 `WHERE kind = 'user'`，避免后台 thread 漏到 `/ask` 侧栏；新增 `chat_threads_list_system(task?, limit?)` 给诊断面板专用。

### 10.3 packages/agent 改动

System-level memory 不需要改 sidecar 协议：`SidecarInput.system_prompt_extra`
和 `SidecarInput.tools.native` 都已经是 per-turn 输入。后台 agent 任务通过**收紧 `tools.native` 列表**实现工具白名单（§11.3），不需要新增 sidecar 协议字段。

Step 1 的在线写入需要新增一个 native tool：

- `packages/agent/src/native-tools/save-note.ts`
- `packages/agent/src/native-tools/index.ts` 注册 `save_note`
- `apps/desktop/src-tauri/src/services/exec_agent/rpc_server.rs` dispatch 到 `NotesRepo`
- `local_context.rs` / Agent prompt 增加"何时调用 `save_note`"的记忆保存规则
- **`save_note` 只对用户对话开放**：后台 agent 任务的 `tool_whitelist` 不包含它，由 `BackgroundAgentTaskRunner` 在装配 `SidecarInput.tools.native` 时过滤掉

Step 2 的 tool-level recall 需要新增另一个 native tool：

- `packages/agent/src/native-tools/memory-search.ts`
- `packages/agent/src/native-tools/index.ts` 注册 `memory_search`
- `apps/desktop/src-tauri/src/services/exec_agent/rpc_server.rs` dispatch 到 `MemoryService`
- **同时弃用现存的 `recall-screen-history` native tool**：它仅查 frames，是 `memory_search` 的真子集。删除 `packages/agent/src/native-tools/recall-screen-history.ts`、`index.ts` 注销、`rpc_server.rs` 移除 dispatch，并把 Agent prompt 里的工具引用替换为 `memory_search`。

Step 3（后台 agent 任务）需要的只读工具——这些工具**用户对话也能用**，区别只在白名单是否包含：

| 工具 | dispatch 实现 | 用户对话 | 后台任务 |
|---|---|---|---|
| `frame_search` | `FrameRepo::fts_search` | ✅（已通过 `memory_search` 间接暴露） | ✅ |
| `frame_get` | `FrameRepo::get_by_id` | ✅ | ✅ |
| `frame_stats` | 新增聚合 query：`group_by IN ('bundle_id', 'hour_of_week', 'day_of_week')` | ✅ | ✅ |
| `chat_history_search` | `ChatRepo::fts_search`（**强制 `kind='user'`**，后台 thread 不可见即不可被搜到） | ✅ | ✅ |
| `chat_thread_get` | `ChatRepo::get_thread_with_messages` | ✅ | ✅ |
| `thread_search` | `ChatRepo::thread_summary_fts_search`（§12.4.1，**强制 `kind='user'`**） | ✅ | ❌（§12.5） |
| `note_list` | `NotesRepo::list` | ✅ | ✅ |
| `auto_persona_get_previous` | 读 `app_data_dir/auto-persona.md` | ✅ | ✅ |

`packages/agent/src/native-tools/` 各加一个 thin shim 文件，dispatch 到 `rpc_server.rs`。

这些不是新的长连接协议，只是沿用现有 native tool → Rust RPC 通路。

### 10.4 ts-rs 导出

新增类型 `#[derive(TS)]`：

- `Note` / `NoteScope` / `NoteSourceType` / `NoteStatus`
- `MemoryItem` / `RecallRequest` / `RecallResult` / `RecallDiagnostics`
- `AutoPersona`（设置页渲染用的扁平视图）
- `ChatThreadKind`（`'user' | 'system'`）/ `SystemTaskKind`（`'persona_distill' | 'session_memory_learning'`）
- `SystemThreadSummary`（诊断面板列表项：thread_id / task / started_at / duration_ms / status / token_used）

全部 `export_to = "../../../../packages/shared-types/src/generated/"`，记得加进 `packages/shared-types/src/index.ts` barrel。

### 10.5 新增 Tauri 命令

`commands/memory.rs`：

| 命令 | 用途 |
|---|---|
| `list_notes(scope?, project_id?)` | 设置页列表 |
| `create_note(content, scope, scope_ref?)` | 手动添加 |
| `update_note(id, content?, status?)` | 编辑/归档 |
| `delete_note(id)` | 硬删 |
| `get_auto_persona()` | 设置页显示生成画像 |
| `regenerate_auto_persona()` | 手动触发一次画像重生成（dispatch §11 PersonaDistillTask） |
| `recall_preview(query)` | 调试 UI：输入查询、看召回结果 |

`commands/background_agent_task.rs`（v1 加，给设置页诊断标签用）：

| 命令 | 用途 |
|---|---|
| `list_system_threads(task?, limit?)` | 列出后台 agent 任务 thread（`chat_threads_list_system` 的 typed wrapper） |
| `get_system_thread_detail(thread_id)` | 拿完整对话 + 工具调用记录，复用 `/ask` message renderer |
| `rerun_system_task(task)` | 强制立刻跑一次（dev 入口，prod 默认隐藏） |

在 `lib.rs::invoke_handler!` 注册；前端在 `lib/tauri.ts` 加 typed wrapper；React Query 用 `hooks/use-memory.ts` 和 `hooks/use-background-tasks.ts`。

---

## 11. 后台 agent 任务通路

### 11.1 为什么走 sidecar 而不是单次 LLM 调用

`session_memory_learning`（§3.4.2）和 `persona_distill`（§4.3）这两个任务**不是一次性问答**：

- 它们要看的素材是 frames + chat_messages + notes 的组合，规模随用户使用时长线性增长。把"猜需要的素材"全部 stuff 进单次 prompt 永远会漏，要么超 token、要么丢关键 evidence。
- 正确做法是给 agent 一个**目标 + 只读工具**，让 agent 自己规划"先看哪些 bundle_id 高频、再抽几段对应时段的 chat、最后比对上版 persona"——这是 agent loop 的本职。
- 复用现有 corivo-agent sidecar 的 native-tool / MCP 调度，不要再造一套后台用的 tool 通路。

但和用户对话相比，这条通路必须做几件事不一样：

| 维度 | 用户对话（chat_send / quick_ask） | 后台 agent 任务 |
|---|---|---|
| 触发 | UI / hotkey / 用户输入 | 调度器（每日 / idle / 启动） |
| thread 可见性 | 默认进 `/ask` 侧栏 | **永不进侧栏**，仅诊断面板可见 |
| 工具集 | 全集（含 `save_note` 等写工具） | **只读子集**，按任务白名单收紧 |
| 流式输出 | `StreamEmitter` → 前端 | 无前端订阅，事件仅落日志 |
| 失败处理 | 报错给用户 | warn 日志，下个调度周期重试 |
| 用户中断 | 可点停止 | 不可见即不可中断；任务自带 `max_turns` 硬上限 |

### 11.2 thread 区分：`chat_threads.kind`

显式给 `chat_threads` 加二分字段，比 session_id 前缀约定干净，列表 SQL 也能直接 index：

```sql
ALTER TABLE chat_threads ADD COLUMN kind TEXT NOT NULL DEFAULT 'user'
  CHECK (kind IN ('user', 'system'));
ALTER TABLE chat_threads ADD COLUMN system_task TEXT;
-- system_task 在 kind='system' 时必填，便于诊断 / 限流 / 评测：
--   'persona_distill' | 'session_memory_learning' | 未来更多
-- kind='user' 时为 NULL

CREATE INDEX idx_chat_threads_kind_created ON chat_threads(kind, created_at DESC);
```

- `chat_threads_list`（现有 command，§10.5 邻居）默认 `WHERE kind = 'user'`，前端 `/ask` 侧栏 0 改动
- 新增 `chat_threads_list_system(task?, limit?)`，只服务设置页"诊断"标签
- 现有"归档 / 置顶"语义和 `kind='system'` 正交：系统 thread 不允许置顶；归档对系统 thread 无意义但字段保留

### 11.3 工具白名单

每个后台任务声明自己的 native tool 子集，写工具一律不进白名单：

**`persona_distill` 白名单：**
- `frame_search(query, time_range?, limit?)` —— FTS 查 frames（共享 §7 召回后端）
- `frame_get(frame_id)` —— 单帧详情（ax_text / ocr_text / adapter_payload）
- `frame_stats(group_by, time_range)` —— 按 `bundle_id` / 小时 / 星期 聚合，给"常用软件"和"工作时间"专用
- `chat_history_search(query, time_range?)` —— 跨 thread 查历史 chat（强制 `kind='user'`，不能查到其它系统 thread）
- `chat_thread_get(thread_id)` —— 拿单 thread 全文
- `note_list(scope?, status?)` —— 现有 notes
- `auto_persona_get_previous()` —— 上一版 `auto-persona.md`

**`session_memory_learning` 白名单：**
- `chat_thread_get(thread_id)` —— 当前学习窗口的对话全文
- `note_list(scope='global', status='active')` —— 检测是否和已有 note 重复
- 不需要 frame 工具，session learner 的输入域就是单 thread 对话

**所有后台任务显式禁用：**
- 任何写工具（`save_note`、`update_note`、`delete_note`）
- `memory_search`（user-facing 召回工具，后台任务用更细的 `frame_search` / `chat_history_search`）
- MCP 服务器（`SidecarInput.tools.mcp_servers` 传空数组）

写入是任务**返回 JSON 结果后由 Rust 侧做的**——agent 不直接落库。`session_memory_learning` 返回 `{ notes: [...], thread_summary: { content, topics } }`（§12.3），Rust 校验后分别写 `notes` 和 `chat_threads.summary`；`persona_distill` 返回完整 markdown，Rust 跑 §5 冲突检测后原子替换 `auto-persona.md`。

### 11.4 执行器：`services/background_agent_task/`

```
services/background_agent_task/
├── mod.rs            -- BackgroundAgentTask trait + Runner
├── runner.rs         -- 实际跑 sidecar 的封装
├── registry.rs       -- 任务白名单 / 工具白名单 / 默认 max_turns
└── scheduler.rs      -- 调度入口（daily tick / idle hook / startup check）
```

核心契约：

```rust
pub trait BackgroundAgentTask {
    const KIND: SystemTaskKind;                  // 落到 chat_threads.system_task
    fn system_prompt(&self) -> &'static str;     // include_str!("../../prompts/<task>.md")
    fn initial_user_message(&self) -> String;    // 给 agent 的目标描述 + 输入指针（如 thread_id）
    fn tool_whitelist(&self) -> &'static [&'static str];
    fn max_turns(&self) -> u32 { 20 }            // 硬上限，避免无限循环烧 token
    fn output_schema(&self) -> Option<&'static str>;  // JSON schema，结构化输出走 schema-guided 解码
    async fn consume_output(&self, output: String, deps: &TaskDeps) -> Result<()>;
}
```

`runner::run<T: BackgroundAgentTask>()`：

1. `INSERT INTO chat_threads (id, kind, system_task, …)`，拿到 thread_id
2. 构造 `SidecarInput`：
   - `session_id = thread_id`
   - `system_prompt_extra = task.system_prompt()`
   - `tools.native = filter(all_native_tools, task.tool_whitelist())`
   - `tools.mcp_servers = []`
   - 用 `initial_user_message` 作为 turn 的 user message
3. 调 `exec_agent::run_turn()`，**不传 `StreamEmitter`**（或传一个 no-op emitter，只把事件转写到日志文件）
4. 循环 → max_turns 或 agent 自己声明结束
5. `task.consume_output(final_text, deps)` 完成实际副作用（写 notes / 写 persona.md）
6. 标记 thread `archived_at = now()`（也可以加 `completed_at`，但 `archived_at` 已存在，复用即可）

### 11.5 日志与诊断

- **jsonl 持久化**复用 sidecar 现有路径 `${sessions_dir}/${thread_id}.jsonl`；不区分目录，靠 `chat_threads.kind` 反查
- **chat_messages** 仍正常写入，包括工具调用记录——这样诊断面板能复用 `/ask` 的渲染组件
- **失败**：在 `chat_messages` 写一条 `role='system'` 的 error 记录 + `tracing::warn!`，不弹用户通知（依赖 §10.2 Step 1b 把 `chat_messages.role` 的 CHECK 约束扩到包含 `'system'`；现行约束只允许 `'user' | 'assistant'`，不扩则插入会被拒）
- **诊断 UI**（设置页"记忆 → 诊断"子标签，v1 加）：
  - 列出最近 N 次系统 task：kind、started_at、duration、token_used、status
  - 点开能看完整工具调用链（复用 `/ask` 的 message renderer）
  - 提供"立即重跑"按钮（dev 专用，prod 默认隐藏）

### 11.6 调度

| 任务 | 触发 | 频率 / 冷却 |
|---|---|---|
| `persona_distill` | 每日凌晨低活动时段 + 启动时检查 `last_run > 24h` | 每日至多 1 次 |
| `session_memory_learning` | thread idle ≥ 20–30 min / 用户关闭归档 / app 退到后台 | 每个 thread 每段未学习窗口 1 次，checkpoint 记录已学到 message_id |

**并发约束：单进程同一时刻至多一个后台 agent task 在跑。**理由：

- sidecar 进程虽然便宜，但同时跑两个会让 LLM API 配额 / rate limit 互踩
- 用户主动 `/ask` 必须优先；后台任务在用户开始新 turn 时主动 yield（不强杀，等当前 turn 跑完再让出）

`scheduler.rs` 维护 FIFO 队列 + 信号量，多个 idle thread 触发时按时间排队。

### 11.7 Config

```rust
pub struct BackgroundAgentTaskConfig {
    pub enabled: bool,                           // 总开关，关掉 = 不跑任何后台 agent task
    pub persona_distill: PersonaDistillConfig,
    pub session_learner: SessionLearnerConfig,
}

pub struct PersonaDistillConfig {
    pub enabled: bool,
    pub schedule_hour_local: u32,                // 默认 3
    pub evidence_window_days: u32,               // 默认 30
}

pub struct SessionLearnerConfig {
    pub enabled: bool,
    pub idle_threshold_minutes: u32,             // 默认 25
}
```

设置页"记忆 → 后台任务"开关直接绑到 `enabled`，与"清空自动画像并停止蒸馏"（§14.1）保持一致语义。

---

## 12. 跨 thread working memory

### 12.1 问题陈述

每个 thread 是 chat history 的孤立窗口。靠 `chat_messages` 召回（§7）能命中"具体哪句话"，但召回粒度是 message：

- 用户说"上次那个 bug"，message 召回会拉出几条相关 message，但 agent 看不到该 thread **整体在讨论什么、最后结论是什么**
- agent 想跳转到"昨天我们聊的那个 thread"没有专门工具——只能靠 message FTS 间接命中
- FTS 对短问句（"上次那个 bug" / "昨天那个事"）基本无效，命中靠运气

`chat_messages` 召回是细粒度的，但跨 thread 的"语义连接"需要一层 thread-level 表征 + 一个专门的查询工具。这是 working memory：不像 `notes` 那样升格为长期事实，也不像 `auto-persona.md` 那样蒸馏为画像——它就是"thread 自己长什么样"的高密度概要。

### 12.2 数据：`chat_threads.summary` + topics

不另起表，单一 ULID 关系够用，在 `chat_threads` 加三列：

```sql
ALTER TABLE chat_threads ADD COLUMN summary TEXT;             -- 自然语言摘要（150–300 字）
ALTER TABLE chat_threads ADD COLUMN summary_topics TEXT;      -- 空格分隔关键词（FTS 用）
ALTER TABLE chat_threads ADD COLUMN summary_updated_at TEXT;  -- DbInstant；NULL = 未生成
```

FTS5（沿用 frames_fts 模式，Rust 侧 jieba 预分词写 `summary_topics`）：

```sql
CREATE VIRTUAL TABLE thread_summaries_fts USING fts5(
  summary,
  summary_topics,
  kind UNINDEXED,
  content='chat_threads', content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2'
);
-- 同步触发器参照 frames_fts；仅对 kind='user' 的 thread 入索引，
-- kind='system'（§11.2 后台 task thread）永不进入工作记忆召回。
```

`summary` 直接给人和 LLM 读；`summary_topics` 是密度更高的关键词索引，让短查询（"那个 bug"）也能命中。两列都参与 FTS。

### 12.3 生成时机：session learner 副产物

不另起任务。`SessionMemoryLearningTask`（§3.4.2 / §11.4）在产出 notes 的同时输出一份 thread summary，`output_schema` 扩展为：

```json
{
  "notes": [ ... ],                                 // §3.4.2 原有
  "thread_summary": {
    "content": "150–300 字概要（话题 / 关键决策 / 未决项）",
    "topics": ["bug", "useEffect", "race condition", "焦点切换", ...]
  }
}
```

`prompts/session_memory_learning.md` 显式要求 agent 同时产出两类：

- **notes**：升格陈述（§3.4.1 规则）
- **thread_summary**：写给"未来某个 agent 用来快速回到这个上下文"，要写清楚 *这次聊的什么 / 得出了什么结论 / 还有什么没解决*

`consume_output`：

- `notes` 走 §3.4.2 路径
- `thread_summary` 写到 `chat_threads.{summary, summary_topics, summary_updated_at}`

复用 session learner 的 checkpoint：summary 和 notes 同进同退。thread 又有新消息时下次 learner 会重写 summary（不增量 patch，整段覆盖——thread 短，重写比 diff 便宜也准）。

### 12.4 召回通路

#### 12.4.1 Tool-level：`thread_search`（本期）

新增 user-facing native tool：

```ts
thread_search({
  query: string,
  time_range?: { from?: string; to?: string },
  limit?: number      // 默认 5，最多 20
})
```

返回 `[{ thread_id, started_at, summary, score }, ...]`。

`rpc_server` dispatch：FTS5 在 `thread_summaries_fts` 上查，**强制 `kind='user'`**（后台 thread 不可被搜到，§11.2 边界），跨 `summary` + `summary_topics` 两列匹配，BM25 排序，再叠 recency_decay（half_life 60 天，比 message 召回宽松）。

典型用法：

```
user: "继续上次那个 bug"
agent: thread_search({ query: "bug" }) → [thread_01HX..., thread_01HY...]
agent: chat_thread_get(thread_01HX...) → 看具体细节
agent: <在新 thread 中接续讨论>
```

`thread_search` 不替代 `chat_history_search`——前者粗筛 thread 找入口，后者细查具体 message。两个工具搭配使用。

#### 12.4.2 System-level：暂不做（v1+）

每 turn 自动注入 thread summary 会污染上下文，而且大多数 turn 不需要跨 thread 跳转。让 agent 通过 `thread_search` 按需主动查。

后续如果 §15 评测发现 agent 经常忘记跨 thread 上下文，再考虑：

- 在 §6 三块 prompt 上下文加 `<related_threads>` block（thread-level 召回结果）
- 或在 §7 召回打分公式加 thread-summary 通道（layer_weight 介于 notes 和 messages 之间）

### 12.5 工具白名单更新（接续 §11.3）

| 工具 | 用户对话 | session_learner | persona_distill |
|---|---|---|---|
| `thread_search` | ✅ | ❌（learner 看的就是当前 thread） | ❌（distill 用 `chat_history_search` 按内容查，比按 summary 更细） |

后台任务**不需要** `thread_search`：session learner 的输入域就是单 thread；persona distill 直接对内容做检索，不依赖 summary 的二次压缩。

### 12.6 与 notes / persona 的关系

三层产物互不重叠：

| 产物 | 粒度 | 寿命 | 注入方式 |
|---|---|---|---|
| `notes` | 单条事实/偏好 | 长期（直至用户删除或 superseded） | persistent block（global）/ 召回（其它 scope） |
| `auto-persona.md` | 整体画像 | 中期（每日重生成） | persona block，soft context |
| `chat_threads.summary` | 单 thread 概要 | 与 thread 同寿；thread 有新消息会重写 | **不进 prompt**——只通过 `thread_search` 工具按需暴露 |

一个 thread 可以同时产出若干 notes + 一份 summary，由 session learner 一次性输出，不冲突。

### 12.7 隐私 / 老化

- `chat_threads.summary` 跟着 `chat_threads` 走，没有独立 retention——thread 被用户删除时 summary 自动消失
- "清空所有 notes"按钮（§14.1）不影响 summary；提供独立的"清空所有 thread summary"开关，触发后下次 idle session learner 会重生成
- thread `archived_at IS NOT NULL` 的，summary 仍可被 `thread_search` 命中——归档语义是"不在主侧栏列出"，不是"不可被搜索"

---

## 13. 实现阶段

**强制按序**，每个阶段独立可用、独立可回滚。

### Step 1a：notes + 永久注入

最小可用形态：用户能"让 Corivo 记住"，agent 下次能用上。这一步**完全同步**——只走"用户在对话里触发 `save_note` → 写库 → 下个 turn 装配进 persistent block"的闭环，没有后台任务、没有跨 thread。可独立发布，可独立回滚。

Schema / repo：
- [ ] `db/schema.sql` 加 `notes` 基表（§3.1），bump schema version
- [ ] `db/repos/notes.rs` CRUD

用户对话侧：
- [ ] `save_note` native tool + Rust RPC 写入路径；`agent_inferred` 默认写 `status='suggested'`
- [ ] Agent prompt 增加"什么时候调用 `save_note`"的规则
- [ ] `commands/memory.rs` 暴露 list/create/update/delete
- [ ] 设置页"记忆" tab（最简：列表 + 增删）
- [ ] `local_context.rs` 改造为读 active + global 的 notes（不再读 Soul.md，§9）
- [ ] ts-rs 类型导出：`Note` / `NoteScope` / `NoteSourceType` / `NoteStatus`

**完成后**：用户在对话里说"以后用中文回我"，重启后下个 thread 仍能继承——note → persistent block 这一通路单独跑通。

### Step 1b：后台 agent 任务框架 + session learner + thread_search

引入"后台 agent task"通路并接上第一个任务——session_memory_learning。Step 1a 的 `notes` 表在此多了一条写入来源（agent 离线学习），同时加上 thread 级 working memory（§12）。

Schema / repo：
- [ ] `db/schema.sql` 加 `chat_threads.kind` + `system_task` 两列（§11.2）
- [ ] `db/schema.sql` 加 `chat_threads.summary` + `summary_topics` + `summary_updated_at` 三列 + `thread_summaries_fts` virtual table（§12.2）
- [ ] `db/schema.sql` 扩 `chat_messages.role` 的 CHECK 约束加上 `'system'`（§11.5 错误记录用）
- [ ] bump schema version
- [ ] `db/repos/chat.rs` 增 `update_thread_summary` / `thread_summary_fts_search` 方法
- [ ] `chat_threads_list` 默认 `WHERE kind='user'`；新增 `chat_threads_list_system`

后台执行通路（§11）：
- [ ] `services/background_agent_task/` 骨架：`BackgroundAgentTask` trait + `runner.rs`（不传 StreamEmitter / 工具白名单过滤 / max_turns / 错误落 `chat_messages.role='system'`）
- [ ] `scheduler.rs` 单进程 FIFO + 信号量（§11.6 并发约束）
- [ ] `prompts/session_memory_learning.md` + `services/persona/.../session_learner_task.rs` 实现 trait；prompt 同时要求产出 `notes` 和 `thread_summary`（§12.3）
- [ ] `consume_output` 拆两路：notes → `notes` 表；thread_summary → `chat_threads.summary` 列
- [ ] thread idle / closed / app backgrounded hook → 入队
- [ ] checkpoint（thread_id + last_learned_message_id）避免重复学习；summary 与 notes 同步推进

用户对话侧：
- [ ] `thread_search` native tool（§12.4.1）+ Rust RPC dispatch 到 `ChatRepo::thread_summary_fts_search`，强制 `kind='user'`
- [ ] Agent prompt 增加"什么时候调用 `thread_search`"的规则
- [ ] auto-persona.md v0 占位文件（仅渲染"明确告诉过我的事"，Step 3 由 distill 任务覆盖）
- [ ] ts-rs 类型导出：`ChatThreadKind` / `SystemTaskKind` / `SystemThreadSummary`

**完成后**：偏好不止靠用户显式说"记住"——一段普通对话里也能被后台 agent 任务"悄悄学到"再升格；`thread_search` 让用户说"继续上次那个 bug"时 agent 能定位历史 thread。后台执行器已在线，Step 3 的 persona distill 可直接复用。

### Step 2：memory::recall 统一召回

引入 recalled 通路，frames + notes + messages 合并召回。

- [ ] `services/memory/` 模块骨架
- [ ] `query.rs` jieba 改写
- [ ] `notes.search_tokens` + `notes_fts`（沿用 frames 的 Rust-side jieba 预分词模式）
- [ ] `chat_messages.search_tokens` + `chat_messages_fts`
- [ ] `recall/frames.rs` 调 `FrameRepo::fts_search`（没有现成 hybrid 模块就先自己封装）
- [ ] `recall/notes.rs` + `recall/messages.rs`
- [ ] `scorer.rs` + `budget.rs`
- [ ] `local_context.rs` 接入：persistent + recalled 双 block
- [ ] 新工具 `memory_search` 注册到 `packages/agent/src/native-tools/`，Rust `rpc_server` 增加 dispatch
- [ ] `recall_preview` 命令 + 调试 UI

**完成后**：agent 能"想起"前几周对话里说过的具体事情；显式 notes 永久生效，隐式聊天历史按相关性出现。

### Step 3：persona 文档蒸馏

加入画像层，生成可读的 `auto-persona.md` 并注入 prompt。**复用 Step 1 已搭好的后台 agent 任务执行器，不再造一套调用通路。**

- [ ] `frame_stats` / `frame_search` / `chat_history_search` / `chat_thread_get` / `auto_persona_get_previous` 这几个只读 native tool（如果 Step 1/2 未实现的，统一在这里补齐）
- [ ] `services/persona/task.rs`：`impl BackgroundAgentTask for PersonaDistillTask`（§4.3 / §11.4 契约表）
- [ ] `services/persona/mod.rs`：每日 03:00 + 启动 `last_run > 24h` 调度入队
- [ ] `prompts/persona_distill.md`：要求 agent 覆盖 §4.2 六章节、按 §4.3 表格的取证路径主动调工具；"四个硬维度"证据不足时输出"证据不足"
- [ ] `services/persona/conflict.rs`：notes ↔ persona 段落冲突检测 / rewrite（§5）
- [ ] `services/persona/renderer.rs`：consume_output → 冲突检测 → 原子写入 auto-persona.md
- [ ] `local_context.rs` 注入 `<persona_memory>` block
- [ ] `get_auto_persona` / `regenerate_auto_persona` 命令 + 设置页显示
- [ ] `list_system_threads` / `get_system_thread_detail` 命令 + 设置页"诊断"标签（v1，可滞后）
- [ ] §15 评测集建立、跑一次基线

**完成后**：agent 行为体现出"知道这个用户是个怎样的人"；用户能在设置页看到自然语言画像，并通过 notes 校正它。

---

## 14. 风险、取舍、未决问题

### 14.1 隐私

- 所有记忆**严格本地**，不进任何同步通道
- `auto-persona.md` 必须用户可读、可清空；校正通过新增 / 删除 notes 完成
- 设置页加"清空自动画像并停止蒸馏"开关，对应 `Config.persona.distillation_enabled`
- 设置页加"清空所有 notes"按钮（不可逆，二次确认）

### 14.2 老化策略

- `notes.expires_at` 默认 NULL；用户在对话里说"先记一周"agent 应能设置
- `auto-persona.md` 文件头记录 evidence window；如果证据窗口陈旧，注入时标注"画像可能过期"或跳过注入
- `chat_messages` 召回有 30 天 half_life，自然淡出

### 14.3 召回 / 学习 / 后台任务成本

- jieba 分词 + FTS5 查询本地毫秒级，不是瓶颈
- `save_note` 只在 agent 判断需要时调用；没有每 turn 固定 LLM 判定成本
- 后台 agent 任务（§11）成本按任务摊销，不是按 turn：
  - `session_memory_learning`：每个有效 thread 触发若干次，单次预算 max_turns=10（约 2k–6k token）
  - `persona_distill`：每日至多 1 次，单次 max_turns=30（约 5k–15k token）
- 并发：单进程同一时刻至多一个后台 agent task 在跑；用户主动 `/ask` 优先，后台任务在用户开始新 turn 时让出（不强杀，等当前 turn 跑完）
- 失败：不打扰用户，按 §11.5 落日志 + 下个调度周期重试

### 14.4 后台任务的可见性 / 可审计性

- 用户**永远不会**在 `/ask` 侧栏看到后台 thread——`chat_threads_list` 强制 `kind='user'`
- 但所有调用记录都在：`chat_threads` 表 + `chat_messages` 表 + `${sessions_dir}/${thread_id}.jsonl`
- 设置页"记忆 → 诊断"标签（v1）能完整看到后台 agent 的工具调用链，与 `/ask` message renderer 复用同一组件——用户想知道"画像怎么得出来的"有完整证据链
- 工具白名单（§11.3）显式不包含任何写工具：后台 agent **不能直接修改 notes / persona / config**，所有副作用经 Rust 侧 `consume_output` 校验后落库；这是安全边界，不是规范建议

### 14.5 未决问题（v1+ 再处理）

- **跨 thread working memory 的 system-level 注入**：本期只做 tool-level `thread_search`（§12.4.1）。如果评测发现 agent 经常忘记跨 thread 上下文，再加 `<related_threads>` block 或 thread-summary 召回通道（§12.4.2）
- **project 层**：scope='project' 留了字段但本期无 project 模型支撑，等 project-layer 落地后再激活
- **多 agent / 角色记忆**：同一个 Corivo 用户在不同身份下（工作/个人/学习）可能想要不同的 persona——本期不区分
- **LLM rewrite 召回**：jieba 召回率不够时引入，需要 §15 评测先确认 miss 严重

---

## 15. 验证

记忆系统最容易"看起来跑通了实际没用"。建立一个轻量评测集，每次召回公式/打分变动都跑：

```
apps/desktop/src-tauri/tests/memory_recall_eval.jsonl
```

格式：

```json
{ "query": "我之前 useEffect 怎么搞的", "expected_top_k": ["msg_01HX...", "frame_01HX..."], "k": 5 }
{ "query": "用什么语言回我", "expected_top_k": ["note_01HX..."], "k": 3 }
```

测试 `tests/memory_recall_eval.rs`：

- 加载固定 fixture 数据（`tests/fixtures/memory_eval.sql`）到内存 SQLite
- 跑 30-50 个 case，统计 Recall@K
- 失败不一定阻塞 CI，但要持续可见——`pnpm --filter @corivo/desktop test:eval` 单独跑

每次改打分公式，跑一次基线对比，避免悄悄变差。

---

## 16. 一张图收尾

```
┌─────────────────────────────────────────────────────────────────┐
│                          数据源                                 │
│                                                                 │
│   ┌──────────┐                 ┌───────────────┐                │
│   │  frames  │                 │ chat_messages │                │
│   │ (episodic│                 │ (declarative  │                │
│   │  truth)  │                 │   raw)        │                │
│   └─────┬────┘                 └───────┬───────┘                │
│         │                              │                        │
│         │              save_note / idle│                        │
│         │                   学习       ▼                        │
│         │                       ┌────────────┐                  │
│         │                       │   notes    │                  │
│         │                       │ (declar.   │                  │
│         │                       │  promoted) │                  │
│         │                       └─────┬──────┘                  │
│         │     蒸馏                    │     冲突检测            │
│         ├──────────┐                  │    ┌──────              │
│         ▼          ▼                  ▼    ▼                    │
│       ┌──────────────────────────┐                              │
│       │    auto-persona.md       │                              │
│       │  (semantic markdown)     │                              │
│       └────────────┬─────────────┘                              │
│                    │                                            │
└────────────────────┼────────────────────────────────────────────┘
                     │
                     ▼
        ┌────────────────────────────┐
        │  services/memory/recall    │
        │                            │
        │  • query rewrite (jieba)   │
        │  • per-layer FTS search    │
        │  • 统一打分 + budget       │
        └────────────┬───────────────┘
                     │
        ┌────────────┴───────────────────────┐
        │                                    │
        ▼                                    ▼
┌────────────────┐                ┌────────────────────┐
│ system-level   │                │  tool-level        │
│ 装配 prompt    │                │  memory_search 工具│
│ (每 turn 自动) │                │  (agent 主动调用)  │
└────────┬───────┘                └────────┬───────────┘
         │                                 │
         ▼                                 ▼
  ┌──────────────────────────────────────────────┐
  │   corivo-agent sidecar (kind='user' thread)  │
  │                                              │
  │  <persistent_memory>  ...  </>               │
  │  <persona_memory>     ...  </>               │
  │  <relevant_memory>    ...  </>               │
  │  <agent_md>           ...  </>               │
  │  <tools_md>           ...  </>               │
  │                                              │
  │  tools.native (用户对话全集):                 │
  │   • save_note            (写)                │
  │   • memory_search        (跨表精召)          │
  │   • thread_search        (§12 跨 thread 跳转)│
  │   • chat_thread_get / frame_* / note_list…  │
  └──────────────────────────────────────────────┘
            ▲
            │ thread_search 命中后按 thread_id
            │ 拉 chat_threads.summary（§12.2）
            │
   ┌────────┴──────────────┐
   │ chat_threads.summary  │  ← session learner 副产物（§12.3）
   │ + summary_topics(FTS) │     仅 kind='user' 入索引
   └───────────────────────┘

────── 后台 agent 任务通路（§11，永不进 /ask 侧栏）──────

  scheduler.rs (§11.6)
    • daily 03:00 / last_run > 24h            → persona_distill
    • thread idle / closed / app backgrounded → session_memory_learning
        │
        ▼
  BackgroundAgentTaskRunner（单进程 FIFO，user turn 优先）
    • 新建 chat_threads(kind='system', system_task=...)
    • SidecarInput.tools.native = task.whitelist  ← 只读子集
    • no StreamEmitter; max_turns 硬上限
        │
        ▼
  corivo-agent sidecar (kind='system' thread)
    • 工具：frame_search / frame_stats / chat_history_search /
            chat_thread_get / note_list / auto_persona_get_previous
      （不含 thread_search——后台任务不需要跨 thread 跳转）
    • 多轮自主规划取证（不预先 stuff evidence）
        │
        ▼  consume_output（Rust 侧校验后写库）
  writes back →
    • notes (active / suggested)               ← session_learner + persona_distill
    • chat_threads.summary + summary_topics    ← session_learner 副产物
    • auto-persona.md (原子替换)               ← persona_distill
```

---

## 附录 A：术语对照

| 用户视角中文 | 技术名 | 表/产物 |
|---|---|---|
| 记得做过的事 | episodic memory | `frames` |
| 记得说过的话（重要） | promoted declarative | `notes` |
| 记得说过的话（一般） | raw declarative | `chat_messages` |
| 记得我是什么样的人 | semantic memory / persona | `auto-persona.md` |
| 全局意图 / 人设 | persistent memory | `notes.scope='global'` |
| 项目级偏好 | scoped declarative | `notes.scope='project'` |
| 一次性约定 | session memory | `notes.scope='session'` |

## 附录 B：与现有 spec 的关系

- **corivo-architecture-v3-spec.md**：本 spec 完全基于 v3 的 frames-as-truth，不动 frames 表结构
- **work-context-spec.md**：work_contexts 处理"当下在做什么"（短时滑动窗口），auto-persona 处理"长期画像"——两者正交，未来 push_decider 可同时读
- **event-task-project-spec / gum-refactor-spec / project-layer-spec**：已 ripped out，persona 文档不复用任何 GUM 管线
