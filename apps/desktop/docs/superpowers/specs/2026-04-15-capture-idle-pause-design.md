# Capture Idle Pause Design

## 1. Goal

为 Corivo 当前的 screenshot capture 增加一个更激进的“无活动暂停”机制：

- 当连续 `4` 张截图都满足 `idle_seconds_at_capture >= 15` 时，停止继续截图
- 不结束当前 session
- 将 capture 状态切换为 `paused_idle`
- 在后台持续监听输入恢复
- 一旦检测到任意新的键盘或鼠标输入，自动恢复截图

这次改动的目标是：

- 在明显无活动时更早停止截图
- 减少无价值截图和后续 pipeline 成本
- 让前端能明确显示“无活动暂停”状态

## 2. Non-Goals

- 不移除现有 pipeline 层的 `idle shortcut`
- 不改变已有 `idle shortcut` 的保守阈值和 segment 行为
- 不引入手动“继续捕获”按钮
- 不把 per-screenshot `captured_at` 持久化到本地 store 或 SQLite
- 不把 capture 状态改成事件推送模型，前端继续走轮询

## 3. Current State

当前 [CaptureLoop](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/capture_loop.rs) 只有“运行 / 停止”两态：

- `running = true` 时不断截图
- `running = false` 时停止 worker

当前 [get_capture_status](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/commands/capture.rs) 只返回：

- `running`
- `current_session_id`

当前前端的状态展示也只有两态：

- [status-indicator.tsx](/Users/airbo/Developer/corivo/corivo-app/src/components/layout/status-indicator.tsx)
- [connections-page.tsx](/Users/airbo/Developer/corivo/corivo-app/src/pages/connections/connections-page.tsx)

当前 pipeline 层已存在一个更保守的 `idle shortcut`，它发生在 batch 进入 LLM 之前，命中后只跳过重型推理，但不会停截图。

## 4. Chosen Behavior

### 4.1 Capture Phase

新增 capture phase：

- `stopped`
- `running`
- `paused_idle`

语义：

- `stopped`：没有活跃 capture session
- `running`：正常截图中
- `paused_idle`：当前 session 仍活跃，但因无活动自动暂停

### 4.2 Pause Rule

每次截图成功后，用该帧的 `idle_seconds_at_capture` 更新一个连续计数：

- 当 `idle_seconds_at_capture >= 15`，`consecutive_idle_frames += 1`
- 当 `idle_seconds_at_capture < 15`，`consecutive_idle_frames = 0`
- 当 `idle_seconds_at_capture` 缺失，`consecutive_idle_frames = 0`

只有当 `consecutive_idle_frames >= 4` 时，才触发自动暂停。

这条规则严格基于“连续四张截图”，而不是 watcher 轮询次数，也不是单次 idle 秒数。

### 4.3 Pause Behavior

进入 `paused_idle` 时：

- 停止继续调用截图逻辑
- 保持当前 session 不变
- `current_session_id` 不清空
- session 仍保持 `active`
- 启动一个轻量 idle watcher

### 4.4 Resume Rule

idle watcher 不截图，只轮询系统 idle 状态。

一旦检测到任意新的输入，就自动恢复：

- capture phase 切回 `running`
- `consecutive_idle_frames = 0`
- 继续往同一个 session 写截图

本次不增加手动“继续”入口。

## 5. State And API Changes

### 5.1 Backend State

[CaptureLoop](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/capture_loop.rs) 需要新增：

- `phase`
- `pause_reason`
- `consecutive_idle_frames`
- `idle watcher` task handle

推荐新增：

```rust
enum CapturePhase {
    Stopped,
    Running,
    PausedIdle,
}

enum PauseReason {
    Idle,
}
```

### 5.2 Capture Status Payload

[CaptureStatus](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/commands/capture.rs) 需要扩成：

```rust
pub struct CaptureStatus {
    pub running: bool,
    pub current_session_id: Option<String>,
    pub phase: String,
    pub pause_reason: Option<String>,
    pub consecutive_idle_frames: u32,
}
```

其中：

- `running` 保留，用于兼容当前前端逻辑
- `phase` 作为新前端状态判断依据
- `pause_reason` 当前只有 `idle`

## 6. Data Flow

### 6.1 Running Path

`running` 状态下：

1. 继续走现有截图逻辑
2. 保存截图文件
3. 读取这张图的 `idle_seconds_at_capture`
4. 更新 `consecutive_idle_frames`
5. 如果未达到 `4`，继续正常工作
6. 如果达到 `4`，切换到 `paused_idle`

### 6.2 Paused Path

`paused_idle` 状态下：

1. 不执行截图
2. 不往 batch 中追加 frame
3. watcher 定期调用 `current_idle_seconds()`
4. 一旦检测到输入恢复，切回 `running`

## 7. Frontend Changes

### 7.1 Sidebar Status

[status-indicator.tsx](/Users/airbo/Developer/corivo/corivo-app/src/components/layout/status-indicator.tsx) 改成三态：

- `running` -> `捕获中`
- `paused_idle` -> `无活动暂停`
- `stopped` -> `未启动`

推荐颜色：

- `running`：绿色
- `paused_idle`：黄色
- `stopped`：灰色

### 7.2 Connections Page

[connections-page.tsx](/Users/airbo/Developer/corivo/corivo-app/src/pages/connections/connections-page.tsx) 同步使用相同状态文案：

- `捕获中`
- `无活动暂停`
- `未启动`

按钮逻辑：

- `running`：显示 `停止`
- `paused_idle`：显示 `停止`
- `stopped`：显示 `开始捕获`

本次不新增“继续”按钮。

## 8. Edge Cases

### 8.1 Missing Idle Signal

如果某张截图的 `idle_seconds_at_capture = None`：

- 不推进暂停计数
- 直接清零
- 保持正常截图

### 8.2 Resume Reset

从 `paused_idle` 恢复到 `running` 后：

- 必须清零 `consecutive_idle_frames`

否则会出现刚恢复就再次暂停的错误行为。

### 8.3 Manual Stop Wins

手动调用 `stop_capture` 时：

- 必须停止截图 worker
- 必须停止 idle watcher
- 必须阻止 watcher 再次自动恢复

手动停止优先级高于自动恢复。

### 8.4 Session Continuity

同一个 session 可以经历：

- `running -> paused_idle -> running`

本次不在暂停时结束 session，也不在恢复时新建 session。

### 8.5 Partial Batch Handling

进入 `paused_idle` 时，如果当前 batch 未满：

- 保留当前未满 batch，恢复后继续积累

这是最小改动方案，也能保留暂停前已经拍到的有效截图。

## 9. Relationship To Existing Idle Shortcut

本次新增的是 capture 层 pause/resume。

现有 pipeline 层 `idle shortcut` 保持不变，继续负责：

- 当已经拍到的 batch 明显长期 idle 时
- 跳过 summary / memory / judgment / notification

两者关系是：

- capture 层：尽早少拍图
- pipeline 层：对已拍到的图尽量少跑大模型

本次不把两者合并成一个统一状态机。

## 10. Testing

至少覆盖以下测试：

### 10.1 Backend

- 连续 `4` 张截图都满足 `idle >= 15` 时，状态切为 `paused_idle`
- 中间任意一张 `< 15` 时，计数清零，不暂停
- `idle = None` 时不暂停
- `paused_idle` 后检测到新输入时会恢复到 `running`
- 恢复后 `consecutive_idle_frames` 被清零
- 手动 `stop` 后 watcher 不会再自动恢复
- 同一个 session 在 pause/resume 前后保持同一 `session_id`

### 10.2 Frontend

- `phase = running` 时显示 `捕获中`
- `phase = paused_idle` 时显示 `无活动暂停`
- `phase = stopped` 时显示 `未启动`
- connections 页面与侧栏状态文案保持一致

## 11. Acceptance Criteria

完成后应满足：

- 连续四张截图满足 `idle_seconds_at_capture >= 15` 时，Corivo 自动停止截图
- 自动暂停不会结束当前 session
- 用户一旦恢复键盘或鼠标输入，Corivo 自动恢复截图
- 前端侧栏和连接页都能看到 `无活动暂停`
- 原有 pipeline 层 `idle shortcut` 行为不回归
