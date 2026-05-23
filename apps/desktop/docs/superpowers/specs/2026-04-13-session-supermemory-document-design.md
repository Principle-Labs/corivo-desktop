# Session-Scoped Supermemory Document Design

## 1. Goal

把当前“每个 batch 总结都 `add` 成一条新 Supermemory document”的行为，改成“一个 session 对应一个 Supermemory document；同一 session 后续 batch 只更新这一个 document”。

完成后，Corivo 的记忆层对单个 session 的语义应该是稳定的：

- 一个 session 在 Supermemory 中只有一个 canonical document
- 后续 batch 只会更新 canonical document，不会继续裂出多条
- 本地数据库能明确知道 `session_id -> supermemory_document_id`
- session document 的内容可以完全由本地 segment 数据重建，不依赖远端追加行为

## 2. Why This Change

当前实现位于 [mvp_pipeline.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/mvp_pipeline.rs)。`process_batch()` 每次拿到一批截图后都会直接调用 `memory.add(memory_input)`，而 [MemoryProvider](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/memory/mod.rs) 只有 `add/get/list/search/delete`，没有 session 级的幂等写入或更新能力。

结果是：

- 同一个 session 会被拆成多个 document
- session 的“完整上下文”分散在多条记忆里
- 后续 UI 或通知逻辑如果要理解整个 session，只能再拼装多条 document
- 历史数据迁移和去重会越来越困难

## 3. Non-Goals

- 不重构整个 MemoryProvider 抽象层
- 不引入新的 provider 或多 provider 策略
- 不在这次改动里顺手修复 Supermemory search 的 schema drift
- 不做复杂的历史智能合并；只做按 session 的 deterministic 回填
- 不改 capture 的基础行为，不改 Gemini prompt，不改通知判断策略

## 4. Chosen Approach

### 4.1 Canonical Identity

每个 session 使用固定的外部身份：

`customId = corivo:session:{session_id}`

第一次写入时带着这个 `customId` 调用 Supermemory 的 add/ingest 接口，后续更新则直接使用本地保存的 `document_id` 调用 `update-document`。

这样做的原因：

- `customId` 解决首次写入和补偿重试时的幂等性
- `document_id` 解决后续精确更新和排障
- 两者结合后，本地状态和远端状态都可追踪

### 4.2 Local Source of Truth

session document 的真实内容由本地 SQLite 里的 segment 数据决定，而不是由远端 document 反向回读再 append。

换句话说：

- segment 是 session 内的原子事件
- session document 是 segment 的聚合投影
- 任何时候都可以从本地重新构建出“正确的 session document 内容”

这是这次改动最重要的边界。只要本地 segment 数据在，远端 document 就只是一个可重建的投影。

### 4.3 Sync Strategy

每次 batch 完成后：

1. 先把这次 batch 的结果落成一个 local segment
2. 再按 `session_id` 读取该 session 的所有 segment
3. 组装成一份完整的 session document
4. 如果该 session 还没有 `supermemory_document_id`，调用 add 创建 document
5. 如果已经有 `supermemory_document_id`，调用 update-document 覆盖更新

不采用“远端 append 一段文本”的原因：

- append 很难保证格式稳定
- 重试可能造成重复段落
- 删除、重排、补偿写入都更难做

## 5. Data Model Changes

### 5.1 Sessions Table

给 [schema.sql](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/db/schema.sql) 的 `sessions` 表新增两列：

- `supermemory_document_id TEXT`
- `supermemory_custom_id TEXT`

语义：

- `supermemory_document_id`：远端 canonical document id
- `supermemory_custom_id`：本地固定幂等键，通常是 `corivo:session:{session_id}`

### 5.2 Session Repo API

给 [sessions.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/db/repos/sessions.rs) 增加：

- 读出 session 的 supermemory 绑定信息
- 写入或更新某个 session 的 `document_id/custom_id`

### 5.3 Segment Repo API

当前 [segments.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/db/repos/segments.rs) 已经有表和 repo，但实际 pipeline 还没有接入。需要新增：

- `list_by_session(session_id)`，按时间顺序拿到该 session 的全部 segment

并把 `process_batch()` 真的接到 segment 创建与更新流程上。

## 6. Provider Contract Changes

### 6.1 Minimal Trait Extension

在 [types.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/memory/types.rs) 的 `MemoryInput` 上新增：

- `external_id: Option<String>`

语义是“调用方希望这条记忆在上游系统里拥有稳定外部身份”。这比直接暴露 `customId` 更通用，未来别的 provider 可以忽略或映射。

在 [mod.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/memory/mod.rs) 的 trait 上新增：

- `update(&self, id: &MemoryId, memory: MemoryInput) -> Result<(), MemoryError>`

这个扩展是最小而必要的：

- `add()` 解决首次创建
- `update()` 解决后续覆盖
- 不需要把 session 概念塞进 provider trait

### 6.2 Supermemory Mapping

在 [supermemory.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/memory/supermemory.rs) 中：

- `add()` 时，如果 `external_id` 存在，就映射到 Supermemory 的 `customId`
- `update()` 调用官方 `update-document` 接口
- provider 继续负责 metadata 的 flatten/restore

## 7. Session Document Format

session document 内容必须是可重建、稳定、便于调试的纯文本。

建议结构：

```text
Session: session-20260413-1530
Started: 2026-04-13T15:30:00Z
Last Updated: 2026-04-13T15:42:10Z
Segments: 3

Session Summary
- 用户在 Cursor 中改 Corivo 的 Supermemory 会话归并逻辑
- 重点涉及 session/document identity、provider update、数据库映射

Timeline
1. [15:31-15:32] 用户定位到 MvpPipeline 每批直接 add，导致一个 session 裂成多条 document。
2. [15:35-15:36] 用户确认 Supermemory 官方支持 update-document，可用 document_id 做覆盖更新。
3. [15:40-15:42] 用户决定用 session 作为 document 粒度，后续 batch update 到同一个 document。
```

metadata 建议至少包含：

- `session_id`
- `segment_count`
- `source = "corivo_session"`
- `last_segment_id`
- `paths` 或截图数量摘要
- `time_signals_present`
- `time_signals`

## 8. Batch / Segment Flow

`process_batch()` 要从“直接写 memory”改成下面的顺序：

1. Gemini 生成本批 summary
2. 创建一个 `segments` 记录，状态先设为 `processing`
3. 把这批 screenshots 关联到该 segment
4. 写入 segment 的 summary / tokens / cost / generated_at
5. 读取该 session 全部 segment
6. 构建 session aggregate document
7. 调用 `add` 或 `update`
8. 成功后：
   - 更新 `sessions.supermemory_document_id/custom_id`
   - 更新当前 `segment.memory_id = canonical_document_id`
   - 发 `memory_added` 事件
9. 失败后：
   - segment 标记为 `partial_done` 或 `failed`
   - 错误写进 `error_message`

这里要刻意区分：

- `segment.memory_id` 存的是 canonical session document id，而不是“这批自己的独立 document id”
- `segment` 是流水记录
- `session document` 是聚合视图

## 9. Migration Strategy

历史数据迁移只做 deterministic 回填。

步骤：

1. 扫本地 `sessions`
2. 对每个 session 读取其所有 segment
3. 如果没有 segment summary，跳过
4. 生成 `external_id = corivo:session:{session_id}`
5. 如果本地已记录 `supermemory_document_id`，直接 update
6. 如果没有，就 add 一次并回填 `document_id`
7. 对于历史上同 session 已经裂出的多个 document，不做自动内容 merge；使用本地重建出的 canonical document 覆盖一个目标 document，重复 document 另外列出供人工清理

原因是自动 merge 远端历史文本既不可靠，也不必要。

## 10. Risks

### 10.1 Segment Pipeline Is Not Yet Wired

当前 segments repo 几乎没有被实际 pipeline 使用。这意味着本次改动需要先把“batch -> segment 落库”真的接通，否则 session aggregate 没有可靠来源。

### 10.2 Search Cannot Be Used As Primary Validation

根据 Corivo 的记忆，当前 Supermemory 的 search 路径存在 schema drift，写入成功但搜索可能异常。因此这次改动的验证基准不能依赖 search，优先验证：

- 本地 `sessions.supermemory_document_id`
- `get(document_id)` 能拿到更新后的内容
- 本地 segment 数量与 document timeline 数量一致

### 10.3 Retry / Duplicate Writes

如果 add 成功但本地回填 document_id 失败，下次重试必须依赖 `external_id/customId` 避免再创建第二个 canonical document。

## 11. Success Criteria

- 同一个 session 连续处理多个 batch 后，Supermemory 中只维护一个 canonical document
- 本地 `sessions` 表可以查到该 session 的 `supermemory_document_id`
- document 内容随着新 batch 到来被覆盖更新，而不是新增一条 document
- `segments` 表记录了每次 batch 的 summary 和状态
- pipeline 的失败能留下本地状态，便于补偿
- 历史回填脚本可以把旧 session 映射到新的 canonical document 模型

