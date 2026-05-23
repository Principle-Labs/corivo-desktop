# Session-Scoped Notification Decision Log Design

## 1. Goal

为 screenshot connector 流程中的“是否值得提醒并发送系统通知”阶段增加一份高保真决策日志，并把它保存在当前 session 的运行目录下。

完成后，每一次 batch 的通知判断都应该可以被完整回放：

- 这次判断的输入是什么
- 总结出来的当前活动是什么
- 从记忆里搜到了什么
- 哪些结果被过滤掉，为什么
- 给 LLM 的判断 prompt 是什么
- LLM 原始输出了什么
- 系统如何解析它
- 最终为什么发了或没发通知
- 如果失败，失败在哪一层

## 2. Why This Change

当前通知链路已经在 [`src-tauri/src/services/mvp_pipeline.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/mvp_pipeline.rs) 中跑通：

1. Gemini 生成当前 batch summary
2. 写入或更新 session 对应的 memory document
3. 搜索相关历史记忆
4. 构造 judgment prompt
5. 调用 LLM 判断 `should_push`
6. 调用通知服务发送通知

但现在这个阶段只有零散的 `tracing` 日志，没有一份稳定、可回放、跟 session 目录绑定的结构化审计记录。结果是：

- 很难回答“为什么这次发了通知”
- 很难回答“为什么这次没有发通知”
- 很难定位“是检索没搜到、过滤掉了、LLM 判断为 false，还是发送失败”
- 出现错误时，很难把截图、summary、related memories 和最后判断串起来看

根据 Corivo 的记忆，详细总结的价值在于尽可能还原现场。同样的原则也适用于通知判断：日志必须优先还原完整决策上下文，而不是只保留一个 `should_notify = true/false`。

## 3. Non-Goals

- 不做前端日志查看页面
- 不做跨 session 的统一日志索引
- 不做数据库持久化或 SQLite 查询入口
- 不做多 provider 审计聚合
- 不做通知历史中心
- 不做日志压缩、上传或清理策略
- 不改动现有通知产品逻辑，只补充可观测性

这次只解决“每个 session 目录下，能看到每次通知决策发生了什么”。

## 4. Chosen Approach

### 4.1 Session-Scoped JSONL Audit Log

每个 session 目录新增一个 append-only 日志文件：

`captures/<session_id>/notification-decisions.jsonl`

采用 JSON Lines 的原因：

- 追加写最简单，适合 MVP
- 一次决策就是一行，天然按时间顺序排列
- 即使某条记录很长，也不影响前后记录
- 人工排查时 `tail`、`rg`、`jq -c` 都很方便

### 4.2 One Final Record Per Batch

每个 batch 最终写入一条完整记录，而不是在每个阶段分别 patch 同一条记录。

原因：

- append-only 更稳，不需要更新已有文件内容
- 可以避免“写了一半程序就 return，留下半条状态”的复杂性
- 对这条链路来说，batch 级的一次完整审计记录比事件流更有价值

### 4.3 Pipeline-Owned Logging

日志由 [`src-tauri/src/services/mvp_pipeline.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/mvp_pipeline.rs) 负责组织和写入。

不把日志放到通知 provider 或前端层，原因是：

- summary、related memories、judgment prompt 只在 pipeline 内部最完整
- provider 层只知道最终 `NotificationPayload`
- 前端层根本拿不到完整上下文

因此最小且正确的边界是：

`MvpPipeline -> NotificationDecisionLogger -> JSONL file`

## 5. Storage Layout

当前 session 目录结构保持不变，只新增一个文件：

```text
captures/
  <session_id>/
    meta.json
    0001.jpg
    0002.jpg
    ...
    notification-decisions.jsonl
```

如果某个 session 从未走到通知判断阶段，则该文件可以不存在。

第一次需要写日志时再创建文件。

## 6. Record Lifecycle

一条日志记录覆盖一次 `process_batch()` 中的通知决策阶段。

建议在以下信息都收集完成后再一次性落盘：

1. summary 已生成或失败
2. memory search 已返回或失败
3. related memories 已过滤
4. judgment 已返回或失败
5. notification send 已成功、跳过或失败

如果流程中途 return，也要在 return 前尽量把当前已知上下文写成一条失败或跳过记录。

## 7. Data Model

### 7.1 Top-Level Shape

每一行 JSONL 是一个对象，建议结构如下：

```json
{
  "schema_version": 1,
  "pipeline": "mvp",
  "decision_id": "session-1:segment-12:2026-04-14T10:35:08Z",
  "session_id": "session-1",
  "segment_id": 12,
  "captured_at": "2026-04-14T10:35:00Z",
  "logged_at": "2026-04-14T10:35:08Z",
  "input": {},
  "retrieval": {},
  "judgment": {},
  "decision": {},
  "notification": {},
  "errors": []
}
```

顶层字段职责：

- `schema_version`: 以后扩字段时做兼容
- `pipeline`: 当前固定为 `mvp`
- `decision_id`: 一次决策的稳定标识，方便 grep 和后续 UI 引用
- `session_id` / `segment_id`: 绑定本地数据
- `captured_at`: 这批截图对应的业务时间
- `logged_at`: 写日志的实际时间

### 7.2 Input Section

`input` 用来描述“系统看到了什么”。

建议字段：

```json
{
  "image_count": 5,
  "paths": [
    "/abs/path/0001.jpg",
    "/abs/path/0002.jpg"
  ],
  "summary_prompt": "...",
  "summary_text": "...",
  "time_signals": [
    "今天下午3点开会"
  ],
  "canonical_memory_id": "mem-new",
  "session_document_updated": true
}
```

要求：

- `summary_text` 必须完整保存，不能只截断前 100 个字符
- `paths` 保留当前 batch 的绝对路径，便于回看具体截图
- `summary_prompt` 保存固定 prompt 原文，便于 prompt drift 排查
- `time_signals` 保存提取结果，便于判断通知是否和时间线索相关
- `canonical_memory_id` 保存本次写入或更新后的 document id

### 7.3 Retrieval Section

`retrieval` 用来描述“系统从记忆库取回了什么，以及如何过滤”。

建议字段：

```json
{
  "search_query": "...summary text...",
  "search_limit": 5,
  "raw_result_count": 3,
  "filtered_result_count": 2,
  "related_text_for_prompt": "1. [04-10 11:00] ...",
  "related_memories": [
    {
      "memory_id": "mem-1",
      "occurred_at": "2026-04-10T03:00:00Z",
      "content": "...",
      "tags": ["auto"]
    }
  ],
  "filtered_out": [
    {
      "memory_id": "mem-new",
      "content": "...",
      "reason": "same_canonical_memory"
    }
  ]
}
```

要求：

- 同时保留 `raw_result_count` 和 `filtered_result_count`
- `filtered_out.reason` 必须是结构化枚举，而不是自然语言
- `related_text_for_prompt` 保存真正送进 judgment prompt 的拼接文本
- `related_memories.content` 必须完整保留

### 7.4 Judgment Section

`judgment` 用来描述“系统如何向 LLM 提问，以及 LLM 回了什么”。

建议字段：

```json
{
  "prompt": "...",
  "raw_output": "```json\n{\"should_push\":true,...}\n```",
  "cleaned_output": "{\"should_push\":true,...}",
  "parsed": {
    "should_push": true,
    "title": "相关记忆",
    "body": "你之前决定先做 MVP 闭环"
  },
  "parse_error": null
}
```

要求：

- `prompt` 必须完整保留
- `raw_output` 必须完整保留，即使不是合法 JSON
- `cleaned_output` 保存去掉 code fence 之后的版本
- `parse_error` 在失败时保存具体错误字符串
- 如果 judgment 根本没有执行，整段仍然存在，但字段为空并在 `decision` 标明跳过原因

### 7.5 Decision Section

`decision` 用来描述“系统对这次判断的最终结论”。

建议字段：

```json
{
  "should_notify": true,
  "decision_reason": "llm_should_push_true",
  "skip_reason": null
}
```

这里必须显式区分三类情况：

1. 真正决定发送通知
2. judgment 已执行，但结论是不发
3. judgment 根本没执行，因为前面的条件不成立或失败

### 7.6 Notification Section

`notification` 用来描述“最终通知动作是否发生”。

建议字段：

```json
{
  "attempted": true,
  "title": "相关记忆",
  "body": "你之前决定先做 MVP 闭环",
  "level": "info",
  "send_result": "sent",
  "send_error": null
}
```

要求：

- `attempted` 表示是否真的调用了通知服务
- `send_result` 是结构化状态，不是布尔值
- 如果发送失败，`send_error` 保存 provider 层错误文本
- 如果因为配置禁用而没有发送，也要记录在这里

### 7.7 Errors Section

`errors` 用来补充整个过程中出现的非终止性错误。

建议结构：

```json
[
  {
    "stage": "judgment_call",
    "message": "llm timeout"
  }
]
```

这个字段用于保留多个错误，而不是把所有错误都塞进 `decision.skip_reason` 或 `notification.send_error`。

## 8. Structured Enums

为了后续稳定分析，以下字段应使用固定枚举值。

### 8.1 `filtered_out.reason`

- `same_canonical_memory`
- `same_summary_content`
- `empty_content`
- `unsupported_result`

当前实现至少需要前两个，后两个作为保守扩展位。

### 8.2 `decision.decision_reason`

- `llm_should_push_true`
- `llm_should_push_false`
- `skipped_before_judgment`
- `failed_before_judgment`

### 8.3 `decision.skip_reason`

- `no_related_memories`
- `all_related_filtered_out`
- `judgment_returned_false`
- `judgment_parse_error`
- `judgment_call_failed`
- `notification_disabled`
- `notification_send_failed`

`decision_reason` 和 `skip_reason` 不重复承担职责：

- `decision_reason` 说明最终分支类型
- `skip_reason` 说明“不发通知”的直接原因

### 8.4 `notification.send_result`

- `not_attempted`
- `sent`
- `disabled`
- `failed`

## 9. Failure Handling

日志记录不能反过来影响主 pipeline。

明确要求：

- 如果 judgment 调用失败，仍然写失败记录
- 如果 judgment 返回非法 JSON，仍然写记录并保留 `raw_output`
- 如果 notification send 失败，仍然写记录
- 如果日志写入失败，只做 `tracing::error!`，不能让 `process_batch()` 再失败一次

这是一个观察层，不是控制层。

## 10. Ownership and Boundaries

建议新增一个很薄的日志模块，例如：

- [`src-tauri/src/services/notification_decision_log.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/notification_decision_log.rs)

内部包含：

- `NotificationDecisionLogEntry`
- `NotificationDecisionInput`
- `NotificationDecisionRetrieval`
- `NotificationDecisionJudgment`
- `NotificationDecisionResult`
- `NotificationDecisionLogger`

职责拆分：

- `MvpPipeline`: 负责收集业务上下文
- `NotificationDecisionLogger`: 负责找到 session 目录、序列化、append 写入

不建议把这部分塞进 [`src-tauri/src/services/notification_service.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/notification_service.rs)，因为那个层级拿不到：

- summary
- related memories
- judgment prompt
- parse error
- filter reason

## 11. Session Directory Resolution

日志文件必须落在当前 session 的截图目录下，因此 logger 需要稳定拿到：

`captures/<session_id>/notification-decisions.jsonl`

建议不要让 `MvpPipeline` 自己拼路径字符串，而是复用或扩展 capture storage 层已有的 session 目录解析逻辑，避免路径规则分叉。

可选实现：

1. 给 [`src-tauri/src/services/capture_store.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/capture_store.rs) 增加 `session_dir(session_id)` 只读辅助方法
2. 或新增一个共享的 `CapturePaths` 辅助类型，由 `CaptureStore` 和 logger 共用

不建议在多个地方重复硬编码 `captures/<session_id>`。

## 12. Integration Plan

最小侵入接法如下：

### 12.1 Before Search

在 summary 和 canonical memory 写入完成后，初始化日志对象，填充：

- `session_id`
- `segment_id`
- `image_count`
- `paths`
- `summary_prompt`
- `summary_text`
- `time_signals`
- `canonical_memory_id`

### 12.2 After Search

记录：

- 原始结果数量
- 过滤后数量
- 每条 related memory 的内容
- 每条 filtered out 结果及原因
- `related_text_for_prompt`

### 12.3 After Judgment

记录：

- 完整 judgment prompt
- raw output
- cleaned output
- parsed result
- parse error

### 12.4 After Notification Action

记录：

- 是否尝试发送
- 最终 title/body
- 发送结果
- 发送错误
- `decision_reason`
- `skip_reason`

### 12.5 On Early Returns

这些分支都要尽量落日志：

- search 失败
- no related memories
- all related filtered out
- judgment 调用失败
- judgment parse 失败
- notifier send 失败

## 13. Testing Requirements

至少补齐以下测试用例。

### 13.1 Notification Sent

当存在相关记忆且 judgment 返回 `should_push = true` 时：

- 写出一条 JSONL 记录
- `decision.should_notify = true`
- `notification.send_result = "sent"`
- 记录完整 prompt 和 raw output

### 13.2 Filtered to Empty

当 search 结果只有刚写入的 canonical memory 或相同 summary 内容时：

- 写出一条记录
- `decision.should_notify = false`
- `skip_reason = "all_related_filtered_out"`

### 13.3 No Related Memories

当 search 返回空数组时：

- 写出一条记录
- `skip_reason = "no_related_memories"`
- `judgment.prompt` 为空

### 13.4 Invalid Judgment JSON

当 judgment 返回非 JSON 文本时：

- 写出一条记录
- 保存 `raw_output`
- `parse_error` 非空
- `skip_reason = "judgment_parse_error"`

### 13.5 Judgment Call Failure

当第二次 LLM 调用直接失败时：

- 写出一条记录
- `errors` 包含 `stage = "judgment_call"`
- `skip_reason = "judgment_call_failed"`

### 13.6 Notification Send Failure

当 notifier 返回错误时：

- 写出一条记录
- `notification.send_result = "failed"`
- `notification.send_error` 有值
- `skip_reason = "notification_send_failed"`

### 13.7 Log Write Failure Does Not Break Pipeline

当 session 目录缺失或追加写失败时：

- 主 pipeline 行为不应被二次中断
- 仅记录 `tracing::error!`
- 现有通知发送结果不受影响

## 14. Success Criteria

- 每个 session 目录下都可以找到该 session 的通知决策日志
- 单次 batch 的通知判断可以从日志中完整回放
- 可以明确区分“没搜到”“搜到了但被过滤”“LLM 判 false”“发送失败”
- judgment prompt 和 raw output 都被保留，便于排障
- 日志写入失败不会影响 screenshot pipeline 主流程

## 15. Open Question Deferred

这次设计明确只做 session 内落盘，不做全局查询入口。

如果后续要支持“按时间看全部通知判断历史”或“在 UI 上查看日志”，再单独设计：

- 全局索引
- 日志裁剪与保留策略
- 前端读取协议
- 隐私遮罩规则
