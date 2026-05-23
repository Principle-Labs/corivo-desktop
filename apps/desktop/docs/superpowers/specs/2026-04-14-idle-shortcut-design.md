# Idle Shortcut Design

## 1. Goal

为 Corivo 当前的截图 -> summary -> memory -> notification 链路增加一个保守的 `idle shortcut`。

完成后，当一个 batch 高置信度地表示“用户长时间没有输入”时，pipeline 应该：

- 继续保留原始截图文件
- 创建一个本地 `idle segment`
- 跳过昂贵的 LLM 调用
- 不写 Supermemory
- 不做 memory search
- 不做 notification judgment
- 不发通知

这次改动的目标是优先节省 token，同时尽量不误伤“用户虽然没有输入，但仍在认真阅读消息、代码或文档”的高价值时段。

## 2. Why This Change

当前 [MvpPipeline](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/mvp_pipeline.rs) 对每个 batch 都会执行完整链路：

1. Gemini 图片总结
2. 更新 session document 到 Supermemory
3. 用当前 summary 搜索相关记忆
4. 再跑一次 LLM 做 push judgment

在默认配置下，Corivo 每 `30s` 截一张图、每 `5` 张图触发一次 batch，[config.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/domain/config.rs) 和 [capture_loop.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/capture_loop.rs) 共同决定了这意味着大约每 `2.5` 分钟就会跑一轮完整推理。

这有两个问题：

- 用户离开电脑、锁屏前短时间发呆、长时间 idle 时，LLM 调用通常没有信息增量
- 真正昂贵的不是单次 summary，而是“summary + search + judgment”的整条链路重复触发

因此第一版最值得做的优化，不是暂停截图，而是在进入 LLM 之前先做一个保守的 idle gate。

## 3. Non-Goals

- 不在这次改动里暂停截图或自动停止 capture loop
- 不引入 OCR、图像相似度、屏幕稳定性等二级判定信号
- 不把 idle batch 写入 Supermemory session document
- 不改变正常非 idle batch 的现有行为
- 不在这次改动里暴露用户可调 idle 阈值设置
- 不顺手统一当前 filesystem capture store 与 SQLite screenshots 表的全部架构分叉

## 4. Current State And Constraints

### 4.1 Capture Path

[CaptureLoop](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/capture_loop.rs) 当前只负责：

- 截图
- JPEG 编码
- 把图片写入 [CaptureStore](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/capture_store.rs)
- 在 batch 满时把 `session_id + images + paths` 交给 pipeline

当前 `CapturedBatch` 不包含：

- 每张截图的 `captured_at`
- 每张截图的输入空闲秒数
- 每张截图的本地 sequence / metadata

因此，想在 pipeline 中做 idle 判定，必须先把“截图时观察到的输入空闲信息”带到 batch 里。

### 4.2 DB Path

SQLite schema 里已经存在 [screenshots](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/db/schema.sql) 表和 repo，但当前 runtime capture 主路径并没有把每张截图写进 SQLite。

也就是说：

- 当前截图元数据主要存在 filesystem + `sessions.json/meta.json`
- segment / session / memory 主要存在 SQLite

这个分叉对 idle shortcut 的影响是：

- 第一版不应该依赖“先把 screenshot rows 全量写入 SQLite”才能工作
- 否则 scope 会从“加一个 idle gate”膨胀成“补齐整条 screenshot persistence 双写架构”

### 4.3 Session Document Aggregation

根据现有实现，session document 是由该 session 的全部 segment summary 聚合而成，[mvp_pipeline.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/mvp_pipeline.rs) 和 [commands/memory.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/commands/memory.rs) 都使用这一模型。

如果 idle segment 也像普通 segment 一样参与聚合，那么后续正常 batch 更新 session document 时，就会把“用户在空闲”这种低价值内容写进远端记忆。

这与本次目标冲突，因此 idle segment 必须被显式排除在 session document 聚合之外。

## 5. Chosen Approach

### 5.1 High-Level Behavior

在 `process_batch()` 的最前面增加一个本地 `idle gate`：

1. capture 继续正常截图，不改变当前采集频率
2. 每张截图额外采集 `idle_seconds_at_capture`
3. 当 batch ready 时，把这些字段和图片一起传给 pipeline
4. pipeline 先执行 `assess_idle_batch()`
5. 如果命中 idle：
   - 创建一个本地 idle segment
   - 记录 shortcut 判定元数据
   - 直接返回
6. 如果未命中 idle：
   - 继续走原有 Gemini -> Supermemory -> search -> judgment 链路

### 5.2 Why This Approach

这是目前最小、最稳的方案：

- 不会漏掉原始截图
- 不依赖额外模型
- 不把 scope 扩大到自动 pause/resume capture
- 和 Dayflow 的成功思路一致，但阈值更保守、更适配短 batch

## 6. Idle Signal Source

### 6.1 Signal Definition

新增 `idle_seconds_at_capture`，语义是：

> 当前这张截图拍下时，距离上一次键盘或鼠标输入已经过去了多少秒。

实现方式参考 Dayflow 的同类思路，在 macOS 上读取 HID 空闲秒数。

### 6.2 Capture Data Shape

把当前只有 `images` 和 `paths` 的 batch 结构，升级为“每帧携带元数据”的结构。

建议新增：

```rust
pub struct CapturedFrame {
    pub image: ImageInput,
    pub path: PathBuf,
    pub captured_at: DateTime<Utc>,
    pub idle_seconds_at_capture: Option<u64>,
}

pub struct CapturedBatch {
    pub session_id: String,
    pub frames: Vec<CapturedFrame>,
}
```

为了减少改动，第一版可以保留 `images()` / `paths()` 这样的辅助访问方法，避免调用方一次性大改。

## 7. Idle Classification Rules

本次采用“保守 shortcut”规则。必须同时满足全部条件，才把 batch 判成 idle。

### 7.1 Eligibility Guard

以下任一条件不满足时，直接判定为 `not idle`：

- `screenshot_count < 4`
- 任一 frame 缺失 `idle_seconds_at_capture`

原因：

- 样本太少时容易误判
- 第一版不做缺失值插补，缺失就宁可放过，不做 shortcut

### 7.2 Sample Threshold

必须满足：

- 至少 `90%` 的截图满足 `idle_seconds_at_capture >= 120`
- 最后一张截图也满足 `idle_seconds_at_capture >= 120`

原因：

- `120s` 是明显长于“短暂停顿/读一段内容”的阈值
- 最后一张必须也 idle，防止刚恢复操作的 batch 被旧 idle 值误覆盖

### 7.3 Break-Glass Guard

如果任意一张截图满足：

- `idle_seconds_at_capture < 45`

则直接判定为 `not idle`。

这是刻意偏保守的保护条款，用来保住这些场景：

- 用户在认真读代码，但中途有过少量输入
- 用户在看消息，间歇性回复
- 用户在浏览文档，偶尔滚动或复制

### 7.4 Coverage Threshold

对每张 frame，构造一个输入空闲覆盖区间：

`[captured_at - idle_seconds_at_capture, captured_at]`

然后：

1. 把区间裁剪到当前 batch 的实际时间范围
2. merge 所有重叠区间
3. 计算覆盖总时长 / batch 总时长

只有当 coverage ratio >= `0.85` 时，才满足覆盖条件。

原因：

- 仅看样本占比，容易被离散的长 idle 值误导
- 覆盖率约束能更稳地表达“这一整段时间几乎都没有输入”

## 8. Idle Segment Behavior

### 8.1 Segment Creation

命中 idle 后，仍然创建一条 segment，但它不是正常的 summarized segment。

建议行为：

- `status = done`
- `activity_type = "idle_shortcut"`
- `prompt_used = NULL`
- `model = NULL`
- `input_tokens = NULL`
- `output_tokens = NULL`
- `cost_usd = 0`
- `memory_id = NULL`
- `summary = "用户在这段时间内处于空闲状态。"`

### 8.2 Idle Metadata

为了后续排障和调阈值，segment 需要保存 shortcut 判定结果。

当前 `segments` 表没有 metadata 字段。建议新增：

- `shortcut_metadata TEXT NULL`

内容为 JSON，至少包含：

```json
{
  "shortcut_type": "idle_v1",
  "screenshot_count": 5,
  "qualified_idle_ratio": 1.0,
  "coverage_ratio": 0.91,
  "min_idle_seconds_at_capture": 124,
  "max_idle_seconds_at_capture": 301,
  "last_idle_seconds_at_capture": 124,
  "thresholds": {
    "qualified_idle_seconds": 120,
    "break_glass_seconds": 45,
    "required_qualified_ratio": 0.9,
    "required_coverage_ratio": 0.85
  }
}
```

### 8.3 Screenshot Association

命中 idle 的 batch 仍应把截图与该 segment 关联起来。

这样做的原因：

- 后续调试时可以回看“为什么这段被判成 idle”
- 如果将来要做二次回填或重新跑 classifier，有明确的数据归属

## 9. Session Document And Memory Rules

idle shortcut 的核心边界是：

- `idle segment` 是本地流水记录
- 它不是 session memory document 的一部分
- 它不能触发远端记忆更新

因此需要明确加入以下规则：

### 9.1 `build_session_document()` 排除 Idle Segment

[MvpPipeline::build_session_document](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/mvp_pipeline.rs) 以及 [backfill_session_documents_impl](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/commands/memory.rs) 都必须忽略：

- `activity_type == "idle_shortcut"` 的 segment

否则，一旦后续正常 batch update session document，之前的 idle 文案会被写进 Supermemory。

### 9.2 No Search / No Judgment / No Notification

命中 idle 时：

- 不调用 `memory.add/update`
- 不调用 `memory.search`
- 不构造 judgment prompt
- 不调用 notifier
- 不写 notification decision log

原因是这条 batch 根本没有进入“通知决策阶段”。

## 10. Data Model Changes

### 10.1 `segments` Table

建议对 [schema.sql](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/db/schema.sql) 的 `segments` 表新增：

- `shortcut_metadata TEXT`

并复用已有 `activity_type` 表达 segment 类型：

- 普通 segment：`NULL` 或保留现有语义
- idle shortcut：`"idle_shortcut"`

### 10.2 `Segment` Domain Model

在 [segments.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/db/repos/segments.rs) 中补齐：

- `shortcut_metadata: Option<String>` 或更类型化的 JSON 结构

### 10.3 Capture Batch Model

将 [CapturedBatch](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/capture_loop.rs) 扩展为 frame-aware 结构，携带：

- `captured_at`
- `idle_seconds_at_capture`

## 11. Pipeline Changes

### 11.1 New Stage Order

`process_batch()` 调整为：

1. 从 batch frames 提取时间范围与 idle 数据
2. 执行 `assess_idle_batch()`
3. 如果 idle：
   - 创建 idle segment
   - 关联 screenshots
   - 落 shortcut metadata
   - 返回
4. 如果非 idle：
   - 继续现有 summary 流程
   - 继续 session document update
   - 继续 search / judgment / notification

### 11.2 Failure Policy

如果 idle 判定本身出错：

- 记录 warning
- 降级为正常路径

原则是：

- `shortcut` 失败不能阻塞主链路
- 不能因为一个本地优化器异常，导致 batch 整体丢失

## 12. Risks

### 12.1 Reading Without Input

最主要的风险是把“没有输入但在认真看屏幕”的时段误判成 idle。

因此第一版用保守规则刻意降低 shortcut 命中率：

- 120 秒合格阈值
- 45 秒 break-glass
- 85% 覆盖率
- 90% 合格样本占比

### 12.2 Architecture Split Between CaptureStore And SQLite

当前 capture 元数据主要在 filesystem store，segment 状态在 SQLite。这意味着 idle 判定所需的 per-frame 元数据最好先通过 `CapturedBatch` 直接传递，而不是强依赖 SQLite screenshots 表。

这是本次设计有意接受的局部不对称。

### 12.3 Backfill Contamination

如果未来 backfill 逻辑没有排除 `idle_shortcut` segment，就会把 idle 文案误写进 session document。这是必须显式测试的回归点。

## 13. Success Criteria

- 命中 idle 的 batch 不会触发 Gemini summary
- 命中 idle 的 batch 不会写 Supermemory
- 命中 idle 的 batch 不会执行 memory search 和 notification judgment
- 命中 idle 的 batch 会生成本地 `idle segment`
- `idle segment` 能关联回该 batch 的截图
- 后续正常 batch 更新 session document 时，不会把 idle segment 内容带入远端 memory
- 非 idle batch 的现有行为保持不变

## 14. Validation Strategy

至少需要覆盖以下验证：

1. **Classifier unit tests**
   - 高 idle 覆盖 batch 命中 shortcut
   - 有一张 `idle_seconds_at_capture < 45` 的 batch 不命中
   - `screenshot_count < 4` 不命中
   - coverage 不足的 batch 不命中

2. **Pipeline unit tests**
   - idle batch 只创建 segment，不触发 fake llm / fake memory / fake notifier
   - normal batch 继续走完整链路

3. **Aggregation tests**
   - `build_session_document()` 忽略 `activity_type = idle_shortcut`
   - `backfill_session_documents_impl()` 也忽略 idle segment

4. **Manual verification**
   - 模拟离开电脑 3-5 分钟，确认生成 idle segment 且没有远端写入
   - 模拟“阅读但偶尔滚动/点击”的场景，确认不会轻易触发 idle shortcut

## 15. Deferred Ideas

以下方向明确延后到后续版本：

- 图像稳定性 / UI diff 辅助判定
- 自动暂停截图
- 用户可调 idle 阈值
- idle segment 合并策略
- 用 OCR 或窗口标题帮助区分“真 idle”和“认真阅读”

第一版只解决一个问题：

> 当一段 batch 极高概率是“无人操作电脑”时，不要浪费一整轮 summary/search/judgment token。
