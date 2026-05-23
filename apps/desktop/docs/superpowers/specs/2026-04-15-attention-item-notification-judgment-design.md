# Attention-Item Notification Judgment Design

## 1. Goal

把 Corivo 当前一次性的“summary -> memory search -> should_push”通知判断，升级成更像助理的 `Attention Item + structured state + LLM judgment` 框架。

完成后，系统应该具备下面的行为：

- 能把多条相关 memory 归并成“一件值得持续盯着的事”
- 能先形成内部推荐“用户现在最该做什么”，再决定要不要打断用户
- 能在事项进入 Attention 后持续重新评估，而不是推一次就结束
- 能基于历史提醒、时间逼近、用户反馈和当前进展决定是否再次提醒
- 不使用盲目的 `memory_id` 去重规则
- 保留现有通知审计能力，并把运行时控制状态明确落到本地

这次设计的目标不是做一个数据库驱动的死规则系统，而是做一个“有记忆、有状态、有判断”的推送控制面。

## 2. Why This Change

当前通知链路已经在 [`src-tauri/src/services/mvp_pipeline.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/mvp_pipeline.rs) 中跑通：

1. 用 Gemini 生成当前 batch summary
2. 更新 session document 到 Supermemory
3. 用 summary 搜索相关记忆
4. 构造 judgment prompt
5. 调用 LLM 返回 `should_push`
6. 调用通知服务发送通知

这条链路的问题不是“不能发通知”，而是它只会做一次性的判断，缺少“持续盯住一件事”的中间层。

因此它解决不了下面这些真实场景：

- 同一件重要事项，虽然已经提醒过，但由于离时间点更近了，仍然应该再提醒
- 同一件事在记忆库里可能表现为多条 memory，不能简单用 `memory_id` 当提醒主键
- 用户说“今天别再提醒”或“1 小时后提醒”，这些反馈需要影响后续判断
- 当前 summary 里出现了新进展，系统应该自动降低提醒强度
- 系统应该先判断“现在推荐用户做什么”，再判断“要不要立刻打断用户”

用户已经明确提出：系统应该更像助理，而不是“某个 ID 推过一次就再也不推”。这意味着：

- `memory` 更适合作为证据层
- “值得持续关注的一件事”必须有自己的本地状态
- LLM 不应该盲猜历史，而应该基于结构化状态做判断

## 3. Principles

### 3.1 Memory Is Evidence, Not Reminder Identity

Supermemory 返回的 recall 结果是证据，不是提醒主对象。

提醒主对象应该是本地的 `Attention Item`，即“系统当前持续盯住的一件事”。

### 3.2 Recommendation Before Interruption

系统不应直接问“要不要推”。

它必须先判断：

> 如果我是助理，我现在最推荐用户去做什么？

只有当这个推荐值得打断用户时，才把它转成通知。

### 3.3 History Is Input, Not Hard Gate

提醒历史、忽略次数、上次提醒时间、用户反馈，这些都应该成为判断输入。

它们不应该变成类似下面这样的硬规则：

- `push_count > 0 -> 不再提醒`
- `memory_id 重复 -> 不再提醒`

### 3.4 Rules Guard Explicit Intent And Safety

规则层只做两类事：

- 尊重用户明确意图，例如 `never_notify`、`suppress_until`
- 防止明显的通知爆炸，例如 60 秒内完全相同的重复通知

除此之外，绝大多数提醒决策都由 LLM 基于状态判断。

### 3.5 Runtime Control State Stays Local

根据 Corivo 的记忆，Supermemory 更适合做统一记忆底座和 recall 面，不适合承担本地运行时推送状态。

因此：

- 远端 memory 继续承担证据和召回职责
- 本地 SQLite 负责 Attention Item 和通知控制状态
- 现有 JSONL decision log 继续承担审计和回放职责

## 4. Non-Goals

- 不在这次设计中把 Corivo 变成完整任务管理器
- 不要求完美抽取所有待办、会议、承诺和截止日期
- 不把所有 memory 都升级成结构化 obligation
- 不在第一版里依赖日历、邮件、IM 等外部系统集成
- 不用纯规则替代 LLM judgment
- 不在第一版里把 overlay 反馈直接做成“远端 memory 删除器”
- 不替换现有 `notification-decisions.jsonl` 审计日志

这次只解决：

> 如何让“值得持续提醒的一件事”拥有自己的状态，并由 LLM 基于该状态做更像助理的提醒判断。

## 5. Current State And Constraints

### 5.1 Current Pipeline Is Memory-Centric And One-Shot

[`src-tauri/src/services/mvp_pipeline.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/mvp_pipeline.rs) 当前是围绕“本轮 summary + recall 结果”做一次性判断的。

现状里还不存在“这件事上轮已经提醒过、这轮是否值得继续跟进”的中间层。

### 5.2 Decision Log Already Exists, But It Is Audit-Only

[`src-tauri/src/services/notification_decision_log.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/notification_decision_log.rs) 已经能记录：

- 输入 summary
- recall 结果
- judgment prompt
- judgment 原始输出
- notification send 结果
- feedback 文本

但它不是运行时控制面，不适合承担高频查询：

- 这件事上次什么时候提醒
- 用户是否说过今天别再提醒
- 这件事下一次最早什么时候再检查

### 5.3 Overlay Feedback Exists, But It Is Still Freeform

当前 overlay 反馈入口已经存在：

- [`src/overlay/overlay-feedback-area.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-feedback-area.tsx)
- [`src/overlay/use-overlay-feedback.ts`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/use-overlay-feedback.ts)

但它目前更像“补一句备注”，还没有升级成结构化控制信号。

### 5.4 Prompt Debug Exists, But Only For Summary And Push Judgment

当前 Prompt Debug 设置已经存在：

- [`src/pages/settings/sections/prompt-debug-section.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/pages/settings/sections/prompt-debug-section.tsx)

它目前只覆盖：

- summary prompt
- push judgment prompt

新的混合框架会额外引入一个 Attention resolution 判断面，因此后续需要扩展 prompt debug。

### 5.5 Local SQLite Is The Right Runtime State Layer

当前项目已经有稳定的本地 SQLite 和 repo 层，适合承接：

- Attention Item 身份
- 控制状态
- 反馈结果
- 事件历史

因此第一版不应把运行时控制状态塞回 Supermemory metadata，也不应依赖 JSONL 扫描做主查询路径。

## 6. Chosen Approach

### 6.1 High-Level Flow

推荐的混合框架如下：

1. summary：生成当前 batch summary
2. recall：用 summary 去 Supermemory 召回相关 memory
3. attention resolution：把多条 recall 结果归并成 0..N 个 `Attention Item`
4. state load：加载每个 item 的本地控制状态
5. recommendation judgment：判断“现在最推荐用户做什么”
6. interruption judgment：判断“是否要现在打断用户”
7. notification：如果值得打断，生成通知并发送
8. state update：回写控制状态和事件历史
9. feedback loop：用户反馈再次更新状态，影响后续判断

这条链路的关键不是“多加几张表”，而是增加一个真正的中间语义层：

> `memory -> attention item -> recommendation -> notification`

### 6.2 Attention Item Is The Reminder Unit

一个 `Attention Item` 表示的是：

> 系统当前认为值得持续盯住的一件事

例如：

- 今天 `17:00` 评审会，需要准备 demo
- 需要给老王回电话
- 今晚前发版本说明

它不是某条 memory，也不是某次通知，而是系统内部持续存在的一条事项线程。

### 6.3 Recommendation And Notification Are Different Decisions

系统对每个 Attention Item 先形成内部推荐：

- 用户现在最该做什么
- 为什么现在推荐它
- 如果现在不做，风险是什么
- 下一次最早什么时候再检查

在这之后，系统再单独判断：

- 这个推荐是否值得打断用户
- 是否应该立刻发通知
- 如果通知，要如何组织标题和正文

这能避免系统把“内部建议”和“外部打断”混为一谈。

### 6.4 One Batch Produces At Most One Foreground Notification

第一版为了控制噪音，每个 batch 最多允许一条 foreground notification。

即使同一轮里识别到多个 Attention Item，系统也应该：

- 对多个 item 形成内部推荐
- 最终只挑出最值得现在打断用户的一个
- 其余 item 留在本地 Attention 状态中，等待后续重新评估

## 7. Attention Item Lifecycle

### 7.1 Discovery

系统从当前 summary + recall 结果中发现：

- 某个事项和当前上下文明显相关
- 或某个事项重要且临近时间点

这时创建或重新激活一个 Attention Item。

### 7.2 Watching

事项进入 Attention 后，不代表一定立刻提醒。

系统可能只形成内部推荐：

- 先继续观察
- 先不打断
- `next_check_at = 15 minutes later`

### 7.3 Notifying

当系统判断：

- 事项很重要
- 时间更近了
- 当前未见处理进展
- 打断收益高于打断成本

才把内部推荐转成通知。

### 7.4 Re-Notifying

再次提醒的理由不是“它是同一条 memory”，而是：

- 事项仍未解决
- 风险升高
- 距离关键时间点更近
- 上次提醒后没有看到新进展

### 7.5 Snoozed Or Suppressed

如果用户说：

- 稍后提醒
- 今天别再提醒
- 这条不相关

系统会把事项调到：

- `snoozed`
- `suppressed`
- `deprioritized`

### 7.6 Done Or Stale

如果后续 summary 显示：

- 用户已经开始处理
- 用户已经完成
- 事项已经过期且不再有价值

该 Attention Item 可以结束或归档。

## 8. Attention Resolution

### 8.1 Inputs

Attention resolution 使用以下输入：

- 当前 batch summary
- 当前时间
- time signals
- recall 到的 related memories
- 现有 open Attention Item 的摘要视图

### 8.2 Output

resolution 阶段的目标不是决定要不要发通知，而是回答：

- 这批 recall 结果里包含几件值得持续关注的事
- 每件事的标题是什么
- 大致属于什么类型
- 是否带有时间点或 deadline
- 哪些 memory 是这件事的证据

### 8.3 Resolution Strategy

推荐做成一个轻量的文本 LLM 调用，返回 0..3 个候选 item。

每个候选 item 至少包含：

- `title`
- `kind`
- `normalized_goal`
- `subjects`
- `importance`
- `due_at`
- `supporting_memory_ids`
- `why_relevant`

### 8.4 Deterministic Canonical Key Generation

为了避免完全依赖 LLM 生成 item key，系统应该用 resolution 输出的结构化字段，本地生成稳定 `canonical_key`。

建议 key 由下列信息归一化后拼接：

- `kind`
- `normalized_goal`
- `subjects`
- `due_at` 的合理时间 bucket

这样可以做到：

- 同一事项即使 evidence memory 变化，也尽量映射到同一个 item
- 不把 `memory_id` 当主键

## 9. Structured Control State

### 9.1 What State Needs To Be Remembered

对于每个 Attention Item，系统必须能回答：

- 什么时候第一次看到它
- 最近一次看到是什么时候
- 最近一次提醒是什么时候
- 一共提醒过多少次
- 用户忽略过多少次
- 用户明确压制过没有
- 当前是否处于 snooze / suppress 状态
- 最近有没有看到处理进展
- 下一次最早什么时候再检查

### 9.2 Why This State Exists

这份状态不是为了做死规则，而是为了给 LLM 提供上下文。

例如同一个事项：

- `notify_count = 2`
- `last_notified_at = 16:00`
- `due_at = 17:00`
- `last_progress_at = null`

这组状态应该让 LLM 倾向于说：

> 虽然提醒过，但现在更临近且仍无进展，允许再次提醒。

### 9.3 Minimal Persisted State

第一版建议最少保存：

- `first_seen_at`
- `last_seen_at`
- `last_notified_at`
- `notify_count`
- `ignore_count`
- `dismiss_count`
- `snooze_until`
- `suppress_until`
- `never_notify`
- `importance`
- `due_at`
- `last_progress_at`
- `next_check_at`
- `last_judgment`

## 10. LLM Judgment Responsibilities

### 10.1 Recommendation Judgment

对每个 active Attention Item，LLM 要先判断：

- 当前最推荐用户做什么
- 这件事现在的 urgency
- 为什么是现在
- 当前是否已经看到进展
- 如果不打断，下一次应该什么时候再看

### 10.2 Interruption Judgment

在 recommendation 之后，LLM 再判断：

- 哪一个 item 值得现在打断用户
- 是否应该发通知
- 标题和正文该如何表达

### 10.3 Judgment Outputs

根据 Corivo 的记忆，notification judgment 输出应该包含：

- `why_now`
- `urgency`
- `supporting_memory_ids`
- `next_check_at`
- `reminder_strategy`

在新的混合框架里，这些字段应该保留，并且围绕 `Attention Item` 扩展，而不是只围绕单条 memory。

### 10.4 Hard Rules Outside The LLM

规则层只做：

- `never_notify = true` 时硬拦
- `suppress_until > now` 时硬拦
- `snooze_until > now` 时硬拦
- 极短时间窗口内完全相同通知的 anti-spam guard

规则层明确不做：

- `memory_id` 重复硬拦
- `notify_count > 0` 硬拦
- `ignore_count > N` 硬拦

## 11. Prompt Contracts

### 11.1 Attention Resolution Output

建议 resolution prompt 输出 JSON：

```json
{
  "items": [
    {
      "title": "17:00 评审会准备",
      "kind": "meeting_prep",
      "normalized_goal": "prepare demo for 17:00 review",
      "subjects": ["Maggie", "review"],
      "importance": "high",
      "due_at": "2026-04-15T17:00:00+08:00",
      "supporting_memory_ids": ["mem_1", "mem_2"],
      "why_relevant": "当前未看到评审准备动作，但事项临近。"
    }
  ]
}
```

### 11.2 Attention Judgment Output

建议 judgment prompt 输出 JSON：

```json
{
  "items": [
    {
      "item_id": "attn_123",
      "should_notify_now": true,
      "urgency": "high",
      "why_now": "距离 17:00 仅剩 25 分钟，且当前未见准备进展。",
      "recommended_action": "暂停当前编码，先整理 demo 和评审材料。",
      "supporting_memory_ids": ["mem_1", "mem_2"],
      "next_check_at": "2026-04-15T16:45:00+08:00",
      "reminder_strategy": "repeat_until_due"
    }
  ],
  "should_push": true,
  "chosen_item_id": "attn_123",
  "title": "17点评审会",
  "body": "还没看到准备进展，建议现在开始整理 demo。",
  "why_now": "事项临近且未开始处理。",
  "urgency": "high",
  "supporting_memory_ids": ["mem_1", "mem_2"],
  "next_check_at": "2026-04-15T16:45:00+08:00",
  "reminder_strategy": "repeat_until_due"
}
```

### 11.3 Prompt Debug Extension

当前 prompt debug 只有 summary 和 push judgment 两类 override。

新的框架需要新增至少一个：

- `attention_resolution_override`

这样才方便在不改代码的前提下调试：

- 多条 memory 是否被错误拆成多件事
- 事项标题和 due_at 是否稳定
- resolution 输出是否足够支撑 judgment

## 12. User Feedback Loop

### 12.1 Feedback Should Become Structured Control Signals

overlay 反馈不应该继续只是自由文本备注。

第一版至少需要支持下面几类结构化动作：

- `handled`
- `remind_later`
- `not_relevant`
- `stop_for_today`
- `never_remind_this`

### 12.2 Feedback Effects

这些反馈应当转成控制状态：

- `handled` -> 标记事项完成或进入 done candidate
- `remind_later` -> 设置 `snooze_until`
- `not_relevant` -> 增加 `dismiss_count` 并降低 importance
- `stop_for_today` -> 设置 `suppress_until = end_of_day`
- `never_remind_this` -> 设置 `never_notify = true`

### 12.3 Freeform Feedback Still Matters

自由文本反馈仍然应保留，用于：

- 写入 decision log
- 后续人工排障
- 为下一阶段更细粒度的 memory adjustment 提供素材

但自由文本不应是唯一控制方式。

## 13. Integration With Current Code

### 13.1 MvpPipeline

[`src-tauri/src/services/mvp_pipeline.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/mvp_pipeline.rs) 需要从：

`summary -> search -> judge -> send`

扩展成：

`summary -> recall -> resolve attention items -> load state -> judge recommendation/interruption -> send -> update state`

### 13.2 Notification Decision Log

[`src-tauri/src/services/notification_decision_log.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/notification_decision_log.rs) 保持 audit 角色。

但后续需要扩展记录：

- chosen attention item
- recommendation fields
- next_check_at
- reminder_strategy

### 13.3 Overlay Feedback

[`src/overlay/overlay-feedback-area.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-feedback-area.tsx) 应从单文本输入，升级成：

- 快捷动作按钮
- 可选文本补充

### 13.4 Settings

[`src/pages/settings/sections/prompt-debug-section.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/pages/settings/sections/prompt-debug-section.tsx) 后续应增加 Attention resolution prompt 的 override 入口。

## 14. Rollout Plan

### 14.1 Phase 1

- 新增 Attention Item resolution
- 新增本地 control state
- 新增 recommendation + interruption judgment
- 保持单条 foreground notification
- overlay 反馈升级为结构化控制动作

### 14.2 Phase 2

- 更细粒度的 progress detection
- 更稳定的 item canonicalization
- 更强的 repeat strategy 细分
- 将部分 feedback 反向作用到 remote memory hygiene

### 14.3 Phase 3

- 更强的 obligation extraction
- 与日历或外部时间源结合
- 跨 session 的 Attention 追踪和趋势判断

## 15. Testing Strategy

### 15.1 Resolution Tests

验证多条相关 memory 能稳定归并成同一 item：

- 同一会议多条证据 -> 一条 Attention Item
- 两件明显不同的事 -> 两条 Attention Item

### 15.2 Re-Reminder Tests

验证再次提醒逻辑：

- 已提醒但无进展且更临近 -> 可以再次提醒
- 已提醒且用户开始处理 -> 不应升级提醒
- `stop_for_today` 后 -> 当天不再提醒

### 15.3 Feedback Tests

验证 overlay 反馈会更新后续判断：

- `remind_later` 后，在 snooze 结束前不提醒
- `never_remind_this` 后硬拦
- `handled` 后事项结束

### 15.4 Audit Tests

验证 decision log 仍能完整记录：

- resolution 结果
- chosen item
- judgment 输出
- feedback 回写后的状态变化

## 16. Open Questions Deferred From This Spec

- 是否在第一版里直接支持 remote memory 删除
- 是否在第一版里对 Attention Item 暴露前端历史中心
- 是否需要把 item resolution 结果也单独持久化成可回放日志
- 是否需要把 `recommended_action` 暴露给 UI，而不仅仅暴露 notification 文案

这些问题都重要，但不会阻塞第一版逻辑落地。
