# Notification Overlay Feedback Design

## Goal

为 Corivo 顶部 notification overlay 增加一个符合灵动岛语义的单行反馈输入能力：

- 所有通知在 expanded 态都显示单行输入框
- 用户可以按 `Enter` 或点击发送按钮提交反馈
- 提交成功后显示短暂“已记录”确认，再自动缩回并隐藏
- 用户反馈写回当前通知对应的 `notification-decisions.jsonl` 记录

这次设计的重点不是做完整消息回复系统，而是在现有单 panel 顶部岛形通知上增加一个非常轻的 quick feedback 交互。

## Current State

当前 overlay 已经具备：

- 单 `notification-overlay` panel
- backend-owned compact / expanded phase
- 原生 hover 驱动展开与收起
- `notification-decisions.jsonl` 作为现有通知决策日志

当前通知 payload 只承载：

- 标题、正文、颜色
- timing 参数
- compact / expanded 几何

它还没有：

- 用户反馈输入能力
- 通知与 decision log entry 的显式关联 id
- 对 JSONL entry 的回写能力

## Problem

你要求的是“在 panel 里直接给用户一个 quick reply 式输入框，并把输入写进通知 JSONL 日志”。

当前缺口有两类：

1. UI 缺口
- expanded 态没有输入槽
- 没有提交、失败、确认这类轻交互状态

2. 数据缺口
- overlay notification 当前没有 `decision_id`
- `NotificationDecisionLogger` 当前只有 append，没有按 `decision_id` 更新 JSONL entry 的能力

所以这次不是简单加个输入框，而是要把“通知实例”和“日志 entry”打通。

## Chosen Approach

采用“轻量内嵌输入”方案：

- 所有通知在 expanded 态都显示单行输入框
- 输入框位于正文下方，和岛形内容融为一体
- 提交时不引入新窗口，不跳出 settings，不打开对话框
- 后端通过 `decision_id` 把反馈写回现有 `notification-decisions.jsonl`

不采用：

- 多行表单
- 二段式“点一下再展开输入框”
- 单独 feedback 日志文件
- 前端直接改 JSONL

## UX Model

### Expanded State

expanded 态结构变成：

- 标题
- 正文
- 单行反馈输入槽
- 极简发送按钮

输入槽视觉要求：

- 更像灵动岛内部的低对比 reply tray，而不是传统表单
- 边界轻、内凹、和黑色岛体融为一体
- 宽度随 expanded 正文区自然伸展
- 不引入高对比 label 或重表单边框

### Submission

用户可以：

- `Enter` 提交
- 点击发送按钮提交
- `Escape` 清空当前输入

不支持：

- 多行回车换行
- 长文反馈

### Confirmation

提交成功后：

- 输入框切到 `submitted` 轻确认态
- 文案例如“已记录”
- 大约 0.8 秒后自动缩回 compact
- 然后按现有隐藏路径退出

提交失败时：

- 回到 expanded 输入态
- 保留原输入
- 在输入框附近给一个轻量错误提示

## State Model

### Overlay Phase

现有 overlay phase 继续由后端拥有：

- `hidden`
- `compact_idle`
- `expanding`
- `expanded`

不为了反馈再加一套全局 overlay phase。

### Local Feedback UI State

反馈交互使用前端局部子状态：

- `idle`
- `submitting`
- `submitted`
- `error`

这些状态只影响 expanded 面板内部的输入区域，不接管 panel 几何 owner。

这样边界保持清晰：

- Rust：拥有 overlay panel phase
- React：拥有 feedback input 的局部交互态

## Data Contract

### Overlay Notification Payload

`OverlayNotificationViewModel` 需要增加：

- `decision_id: string | null`
- `session_id: string | null`
- `segment_id: number | null`

正式提交只要求 `decision_id`，后两项主要用于调试与后续追踪。

### Feedback Payload

用户提交到后端的请求最小化为：

```ts
{
  decisionId: string;
  text: string;
}
```

### JSONL Feedback Shape

每条 decision entry 增加：

```json
{
  "feedback": {
    "text": "用户输入内容",
    "submitted_at": "2026-04-15T12:34:56Z",
    "source": "overlay_panel"
  }
}
```

这一版只支持单条反馈记录，不做 feedback 数组。

## Persistence Model

当前 `NotificationDecisionLogger` 是 append-only。

这次需要新增一个显式的更新 API，例如：

```rust
update_feedback(decision_id: &str, feedback: NotificationDecisionFeedback)
```

行为：

- 读取对应 session 的 `notification-decisions.jsonl`
- 逐行反序列化
- 命中 `decision_id` 那条后替换 JSON
- 原子写回文件

不新建独立 repository。

更新逻辑继续留在 `notification_decision_log.rs`，避免把日志读写边界打散。

## Backend Interface

新增 Tauri command：

```rust
submit_notification_feedback(decision_id: String, text: String)
```

它负责：

- trim 输入
- 校验非空
- 调 decision logger 更新目标 entry
- 返回 success / error

前端不直接处理文件路径、session 目录、JSONL 行替换这些细节。

## File Plan

### Modify

- `src-tauri/src/providers/notification/types.rs`
  - 给 overlay view model 增加 `decision_id/session_id/segment_id`
  - 给 decision log entry 增加 feedback 结构

- `src-tauri/src/providers/notification/overlay.rs`
  - 发通知时把 decision 关联字段带到 view model

- `src-tauri/src/services/notification_decision_log.rs`
  - 增加 feedback 数据结构
  - 增加按 `decision_id` 更新 JSONL entry 的方法

- `src-tauri/src/commands/notification.rs`
  - 增加提交反馈 command

- `src/lib/types.ts`
  - 镜像新的 overlay payload 字段
  - 定义 feedback submit request/response 类型

- `src/lib/tauri.ts`
  - 增加 `submitNotificationFeedback`

- `src/overlay/use-overlay-notification.ts`
  - 增加反馈输入本地状态与提交逻辑

- `src/overlay/overlay-app.tsx`
  - 渲染单行输入槽、发送按钮、提交成功/失败提示

- `src/styles/overlay/island.css`
  - 增加输入槽与按钮样式

### Tests

- `src-tauri/src/services/notification_decision_log.rs`
  - 增加按 `decision_id` 更新 feedback 的测试

- `src/overlay/overlay-app.test.tsx`
  - 增加 expanded 态输入渲染、回车提交、成功确认态、错误态测试

## Error Handling

### Missing Decision ID

如果某条通知没有 `decision_id`：

- 前端仍然显示输入框
- 提交后后端返回明确错误
- 前端显示“当前通知不可记录反馈”的轻量提示

这能满足“所有通知都可见输入框”的要求，同时不 silently drop 用户输入。

### Write Failure

如果 JSONL 更新失败：

- 保留原输入
- 回到 `error` 态
- 用户可以再次提交

## Motion and Style Direction

这次的输入交互要保持灵动岛语义：

- 输入槽像岛体内部自然露出的一层功能区
- 提交成功后的确认态短、干净、无冗长 toast
- 动画优先克制，不做明显表单展开弹跳
- 继续复用当前岛体几何，不拆第二层卡片

## Success Criteria

当以下行为成立时，本次设计算完成：

- 所有通知 expanded 后都显示单行输入槽
- 用户可以按 `Enter` 或按钮提交
- 提交内容能写回当前通知对应的 `notification-decisions.jsonl`
- 成功后显示短暂确认态并自动收尾
- 失败时保留输入并提示可重试
- UI 风格仍然像灵动岛式 quick feedback，而不是普通表单
