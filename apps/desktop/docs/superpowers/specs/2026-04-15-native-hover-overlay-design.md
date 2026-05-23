# Native Hover Overlay Design

## Goal

把 Corivo 当前顶部 notification overlay 的 hover 交互从“前端 DOM 事件驱动”改成“macOS 原生 panel hover 驱动”，让 overlay 在应用未获得焦点时仍能稳定感知 hover，并保持现有单 panel、单通知、compact/expanded 岛形语义不分叉。

## Current State

当前 overlay 由两层状态共同驱动：

- Rust panel 层只管理原生容器可见性与 compact / expanded frame
- React overlay 层通过 `mouseenter` / `mouseleave` 维护 `compact-idle -> expanding -> expanded -> collapse-delay`

现有实现的几个关键点：

- panel 在 app setup 阶段预创建，后续复用，不按通知重建
- Rust 已经拥有 compact / expanded 两套窗口几何，并负责 show / hide / set frame
- 前端收到 `overlay-notification` 后启动 lifetime timer、animation timer、collapse timer
- hover 信号只来自 `top-island` DOM 元素

这套设计在“Corivo 自己拿到前端 hover 事件”的前提下工作正常，但它不保证应用失焦时仍可靠。

## Problem

当前 hover 真相源在前端，存在三个边界问题：

1. WebView 没收到 DOM hover，前端就无法进入 expanded
2. 原生 panel 已经是 non-activating / unfocused 配置，失焦场景下前端 hover 不能被视为可靠前提
3. 视觉 shell 与 DOM hit area 不是同一套几何，hover 体感会比用户看到的黑色岛更窄

因此，这次要解决的不是“前端 hover 灵敏度微调”，而是“hover 权威归属错误”。

## Chosen Approach

采用“native hover owns phase”方案：

- macOS panel 层新增原生 hover tracking
- Rust 成为 overlay UI phase 的唯一 owner
- 前端不再自行根据 DOM 事件推进 phase
- 前端只消费通知内容和后端发来的权威 phase

这次设计继续复用：

- 当前单 `notification-overlay` panel
- 当前 compact / expanded 几何模型
- 当前 React 岛形视觉与动画语言

不做：

- 双窗口 overlay
- 异形 path hit test
- 多通知队列
- 平台分叉 hover 策略

## State Model

### Native Panel Phase

原生 panel 继续只保留三态：

- `Hidden`
- `Compact`
- `Expanded`

它们只表达容器几何与可见性，不表达完整 UI 生命周期。

### UI Phase

Rust 新增权威 UI phase：

- `hidden`
- `compact_idle`
- `expanding`
- `expanded`
- `collapse_delay`

前端和后端都消费这套 phase，但只有后端能写。

### Ownership

- Rust 负责：hover、timer、phase、frame 切换
- React 负责：根据 phase 渲染 compact / expanding / expanded / collapse-delay 视觉状态

## Native Hover Tracking

### Attachment Point

tracking 挂在 macOS panel 的原生内容视图层，而不是 WebView DOM。

panel 创建完成后，panel adapter 负责安装 tracking area，并在 compact / expanded frame 切换时同步更新 tracking bounds。

### Tracking Semantics

第一版 tracking 不做异形 path hit test，直接采用 panel 当前原生矩形 bounds：

- compact 态：tracking bounds = compact panel bounds
- expanded 态：tracking bounds = expanded panel bounds

理由：

- 复用当前 backend-owned 几何真相
- 命中区与原生窗口 frame 一致
- 不再依赖前端 SVG shell 外扩
- 实现复杂度远低于异形 hit path

这意味着第一版 hover 的语义是“进入 panel 当前原生矩形区域”，而不是“进入 SVG 黑色路径”。

### Event Flow

原生 tracking 只需要两类事件：

- `hover entered`
- `hover exited`

这些事件先到 panel adapter，再转给 Rust overlay 状态机。

## Timer Model

所有 phase timer 从前端迁移到后端。

### Lifetime Timer

控制通知总生命周期：

- 新通知到来时启动
- 不因为反复 hover 而无限续命

到期行为：

- `compact_idle` -> `hidden`
- `expanding` / `expanded` -> `collapse_delay`
- `collapse_delay` 保持不变，等待收尾

### Animation Timer

控制：

- `expanding` 持续 `animation_ms` 后 -> `expanded`

它只负责 phase 推进，不负责逐帧几何同步。

### Collapse Delay Timer

控制：

- `collapse_delay` 持续 `collapse_delay_ms` 后
  - 若 lifetime 未过期 -> `compact_idle`
  - 若 lifetime 已过期 -> `hidden`

### Hover Cancellation

`collapse_delay` 期间若再次收到 native `hover entered`：

- 立即取消 collapse timer
- phase 回到 `expanded`

## Phase and Frame Coordination

Rust phase 与原生 panel frame 的关系：

- `hidden` -> panel `Hidden`
- `compact_idle` -> panel `Compact`
- `expanding` -> panel `Expanded`
- `expanded` -> panel `Expanded`
- `collapse_delay` -> panel `Expanded`

这样可以保证：

- 鼠标一旦命中 compact panel 并触发展开，panel frame 立即变成 expanded
- expanded 期间正文区域本身也是 hover 可保留区域
- collapse-delay 有足够的 expanded bounds 容错空间

## IPC Contract

### Keep: Notification Payload Event

继续保留：

- `overlay-notification`

它负责发送：

- 标题、正文、颜色
- timing 参数
- compact / expanded 几何
- sequence

### Add: Phase Event

新增：

- `overlay-phase-changed`

建议 payload：

```ts
type OverlayPhaseChangedEvent = {
  sequence: number;
  phase: "hidden" | "compact_idle" | "expanding" | "expanded" | "collapse_delay";
};
```

### Remove Phase Authority from Frontend

`sync_notification_overlay_panel` 不再承担正式交互职责。

它可以保留给测试或调试 command，但正式交互链路改为：

- Rust panel hover / timer -> Rust phase
- Rust phase -> event -> React render

不再由 React hover -> invoke -> Rust frame sync。

## File Plan

### New

- `src-tauri/src/providers/notification/overlay_session.rs`

职责：

- 维护权威 UI phase
- 管理 hover entered / exited
- 管理 lifetime / animation / collapse-delay timers
- 产出 panel phase 与前端 phase

### Modify

- `src-tauri/src/providers/notification/panel.rs`
  - 增加 native hover tracking 安装、更新、清理能力
  - 暴露 panel hover callback 桥接

- `src-tauri/src/providers/notification/overlay.rs`
  - 接入 session 状态机
  - 在通知发送、hover 事件、timer 事件发生时同步 panel frame 与前端 phase

- `src-tauri/src/providers/notification/types.rs`
  - 增加 UI phase 与 phase changed event 类型

- `src-tauri/src/lib.rs`
  - setup 阶段继续预创建 panel
  - 如有需要，初始化 overlay session 生命周期 owner

- `src/overlay/use-overlay-notification.ts`
  - 从 phase owner 改为 notification/phase 订阅器
  - 删除 DOM hover 驱动 phase 的逻辑

- `src/overlay/overlay-app.tsx`
  - 删除 `onMouseEnter` / `onMouseLeave`
  - 只按后端 phase 渲染

- `src/lib/types.ts`
  - 增加 `OverlayPhaseChangedEvent`

- `src/overlay/overlay-app.test.tsx`
  - 改成 phase-event 驱动测试，而不是 DOM hover 驱动测试

### Keep Unchanged in Principle

- `src-tauri/src/providers/notification/screen_metrics.rs`
- `src/styles/overlay/island.css`
- `src/overlay/overlay-model.ts`

这些文件继续承担现有几何与视觉职责，不在本次重写 hover 真相源时扩大 scope。

## Testing Strategy

### Rust

新增状态机测试，覆盖：

- 新通知进入 `compact_idle`
- compact hover entered -> `expanding`
- animation 完成 -> `expanded`
- expanded hover exited -> `collapse_delay`
- collapse-delay re-enter -> `expanded`
- lifetime 到期时在 compact / expanded / collapse-delay 三种分支

新增 panel adapter 测试，覆盖：

- compact / expanded 切换时 tracking bounds 更新
- hidden 时 tracking 清理

### Frontend

前端测试改为验证：

- 接收权威 phase 后 UI 是否进入对应视觉状态
- sequence 切换时是否替换 active notification
- DOM hover 不再是正式测试入口

## Risks and Mitigations

### Risk: Native tracking 集成复杂

缓解：

- 第一版只做矩形 tracking，不做异形 path
- 把复杂度局限在 `panel.rs`

### Risk: 前后端 phase 不一致

缓解：

- 前端 phase 改为只读
- 所有 timer 都迁移到 Rust

### Risk: Expanded frame 过早切换导致感知突兀

缓解：

- 继续沿用当前“边界切换 frame、前端自己播视觉动画”的原则
- 不做逐帧原生几何插值

## Success Criteria

当以下行为成立时，本次改造算完成：

- Corivo 未获得焦点时，鼠标移入 compact overlay 仍能稳定触发展开
- expanded 态正文区域能稳定保留 hover
- collapse-delay 期间重新进入可取消收起
- 总生命周期不会因反复 hover 而无限续命
- 前端不再拥有 phase 真相源
