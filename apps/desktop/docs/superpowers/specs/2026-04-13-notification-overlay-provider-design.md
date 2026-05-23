# Notification Overlay Provider Design

## 1. Goal

把当前仅支持系统通知的实现，改成一个可扩展的通知 provider 架构，并先落地一个新的 `overlay` provider：

- 业务层不再直接依赖系统通知插件
- 通知发送有稳定的 provider 接口，后续可扩展到 email、Telegram 等实现
- v1 先用屏幕底边 overlay 取代当前系统通知
- overlay 通知显示在屏幕最底端，带由下到上、由纯色到透明的渐变
- 通知内容显示在屏幕中间，具备单入单出和轻微呼吸效果

## 2. Current State

当前通知链路已经存在，但抽象层很薄，仍然直接绑定系统通知：

- [`src-tauri/src/services/mvp_pipeline.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/mvp_pipeline.rs) 定义了 `NotificationSender` trait，并在推送决策命中时调用 `self.effects.send(&title, &body)`
- [`src-tauri/src/lib.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/lib.rs) 中的 `AppNotificationSender` 是当前唯一实现，内部直接调用 `tauri_plugin_notification`
- [`src-tauri/src/commands/notification.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/commands/notification.rs) 的测试命令同样直接走系统通知
- [`src/pages/settings/sections/test-section.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/pages/settings/sections/test-section.tsx) 的“发送测试通知”按钮只是在验证系统通知插件链路
- [`src-tauri/tauri.conf.json`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/tauri.conf.json) 目前只声明了主窗口，没有专用通知窗口

现状的问题是：

- 没有真正的 `providers/notification` 模块
- `MvpPipeline` 虽然依赖 trait，但并没有接到可扩展的 provider 装配模式
- “系统通知”是硬编码的实现细节，不适合作为未来 email / Telegram / overlay 的统一入口
- 没有单独的渲染窗口来承载更强的视觉表现

## 3. Non-Goals

- 不在这次设计里实现多 provider 同发
- 不在这次设计里实现通知队列、通知中心、通知历史
- 不在这次设计里实现通知交互按钮
- 不在这次设计里覆盖多屏定位策略
- 不在这次设计里保留系统通知作为并行 fallback
- 不在这次设计里引入高自由度的视觉配置系统

本次明确采用“单一 active provider”的模式。未来如果要一条通知同时发到多路，再在 `NotificationService` 之上增加 fan-out dispatcher。

## 4. Chosen Approach

### 4.1 Four-Layer Notification Stack

通知能力按四层拆分：

1. `domain/config`
2. `providers/notification`
3. `services/notification_service`
4. `overlay window controller`

这套分层刻意贴近现有 [`memory_service.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/memory_service.rs) 和 [`llm_service.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/llm_service.rs) 的风格，避免通知体系成为特殊路径。

### 4.2 Single Active Provider

配置层只允许选择一个通知 provider：

- `overlay`
- 未来可扩展 `system`
- 未来可扩展 `email`
- 未来可扩展 `telegram`

v1 只实现 `overlay`。这意味着：

- 上层业务只关心 `send(payload)`
- provider 的选择由 `NotificationService` 根据配置决定
- `MvpPipeline` 不再持有临时 trait 实现，而是依赖一个稳定的 service

### 4.3 Overlay First

通知的第一实现不是系统通知，而是屏幕底边 overlay。原因：

- 这是当前产品明确想要的展示方式
- 它最依赖本地桌面渲染和窗口控制，越早确定边界越好
- 一旦 overlay provider 打通，后续再加 email / Telegram 只是 provider 扩展，不需要改动业务通知决策逻辑

## 5. Architecture

### 5.1 Config Layer

在 [`src-tauri/src/domain/config.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/domain/config.rs) 中新增：

- `NotificationConfig`
- `NotificationProviderKind`
- `OverlayNotificationConfig`

建议结构：

```rust
pub struct NotificationConfig {
    pub enabled: bool,
    pub provider: NotificationProviderKind,
    pub overlay: OverlayNotificationConfig,
}

pub enum NotificationProviderKind {
    Overlay,
}

pub struct OverlayNotificationConfig {
    pub duration_ms: u64,
    pub animation_ms: u64,
    pub height_px: u32,
    pub color: String,
}
```

设计约束：

- `enabled` 保留现有总开关语义，替代当前 `app.notifications_enabled` 的承载角色
- `provider` 用于单选 provider
- `overlay` 子配置只保留必要视觉和时序参数
- 不在 v1 暴露过多样式自由度，避免界面失控

### 5.2 Provider Layer

新增目录：

- [`src-tauri/src/providers/notification/mod.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/mod.rs)
- [`src-tauri/src/providers/notification/types.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/types.rs)
- [`src-tauri/src/providers/notification/overlay.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/overlay.rs)

其中：

- `types.rs` 定义 `NotificationPayload`、`NotificationError`
- `mod.rs` 定义 `NotificationProvider` trait 并导出类型
- `overlay.rs` 实现 `OverlayNotificationProvider`

建议 trait：

```rust
pub trait NotificationProvider: Send + Sync {
    fn send(&self, payload: NotificationPayload) -> Result<(), NotificationError>;
}
```

建议 payload：

```rust
pub struct NotificationPayload {
    pub title: String,
    pub body: String,
    pub level: NotificationLevel,
    pub duration_ms: Option<u64>,
}
```

设计原因：

- 保留 `title/body`，而不是直接简化成一条字符串，避免未来 email / Telegram 再返工
- `level` 虽然 v1 视觉上未必有明显差异，但保留分类能力
- `duration_ms` 支持单次发送覆盖默认配置

### 5.3 Service Layer

新增 [`src-tauri/src/services/notification_service.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/notification_service.rs)。

职责：

- 读取当前通知配置
- 根据 `NotificationProviderKind` 解析 provider
- 应用全局开关和默认时长
- 向上暴露统一的 `send(payload)` 接口

建议模式与 [`memory_service.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/memory_service.rs) 保持一致：

- `NotificationProviderFactory`
- `DefaultNotificationProviderFactory`
- `NotificationService`

这样后续扩展 `email`、`telegram` 时，只需要：

- 在 config 中增加枚举值
- 在 provider factory 中增加分支
- 实现新的 provider

### 5.4 Overlay Window Controller

这是 `OverlayNotificationProvider` 的内部实现细节，而不是独立业务层。

职责：

- 创建或复用专用 overlay window
- 设置窗口透明、置顶、无边框、不可交互
- 将 `NotificationPayload` 转成前端需要的 `OverlayNotificationViewModel`
- 通过 Tauri event 发送到 overlay renderer
- 控制窗口显示、隐藏和生命周期

调用链为：

`MvpPipeline -> NotificationService -> OverlayNotificationProvider -> Overlay Window + Renderer`

## 6. Overlay Window Behavior

### 6.1 Window Model

overlay 使用独立窗口，不复用主应用窗口路由。原因：

- 主窗口负责主产品界面，不适合承载常驻透明覆盖层
- overlay 需要透明、置顶、不可交互等特殊窗口属性
- 把通知 UI 放到独立窗口后，视觉实现和主界面路由逻辑完全解耦

建议窗口属性：

- transparent
- decorations = false
- always_on_top = true
- skip_taskbar = true
- focus = false
- visible = false
- resizable = false
- closable = false for user flow

### 6.2 Positioning

v1 只支持主屏幕。

定位原则：

- 窗口贴底部
- 宽度铺满目标屏幕
- 高度固定为配置值，例如 `160px` 到 `220px`
- 原点直接对齐屏幕底边，不做右下角 toast 样式

这意味着通知的视觉语义是“底边氛围带”，而不是“角落气泡”。

### 6.3 Visual Layout

界面结构只保留两层：

1. 底部全宽色带
2. 屏幕中间文案区

具体要求：

- 底部区域使用纯色到底边，然后向上渐变为透明
- 正文居中，优先显示 `body`
- `title` 作为次级信息，可弱化显示或仅用于可访问性/调试
- v1 不放图标、按钮、头像、操作区

### 6.4 Motion

动画采用单入单出：

- enter: 从下方向上轻微浮起并淡入
- breathe: 在停留阶段做轻微呼吸
- exit: 呼吸结束后下沉并淡出

设计约束：

- 呼吸动画只在停留阶段运行
- 退出前要先停止呼吸，避免视觉冲突
- 整体节奏应当安静，不做高频抖动

### 6.5 Lifecycle

v1 仅允许一个活动通知：

- 新通知到来时，覆盖当前通知
- 不维护排队列表
- 新内容到来后重新开始完整动画周期

这是一个明确的产品选择。它降低实现复杂度，并保持“当前最重要的一条提醒”这一语义。

## 7. Data Contracts

### 7.1 Rust Payload

后端统一的通知输入模型：

```rust
pub struct NotificationPayload {
    pub title: String,
    pub body: String,
    pub level: NotificationLevel,
    pub duration_ms: Option<u64>,
}
```

### 7.2 Overlay View Model

provider 发给前端窗口的数据建议另起一个 view model，而不是把 Rust payload 原样透传：

```rust
pub struct OverlayNotificationViewModel {
    pub title: String,
    pub body: String,
    pub color: String,
    pub height_px: u32,
    pub animation_ms: u64,
    pub duration_ms: u64,
    pub sequence: u64,
}
```

`sequence` 的作用是：

- 明确区分“同一个窗口收到的新通知”
- 帮助前端在替换通知时重置动画状态机

### 7.3 Frontend State Machine

前端 overlay renderer 接到通知后进入：

`idle -> entering -> breathing -> exiting -> idle`

状态机放在前端 renderer 的原因：

- CSS / Web Animations 更适合处理细腻的渐变和呼吸效果
- Rust 层只需要管理窗口和消息，不应直接承担动画时序细节

## 8. Integration Changes

### 8.1 MVP Pipeline

[`src-tauri/src/services/mvp_pipeline.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/mvp_pipeline.rs) 当前通过 `PipelineEffects` 组合了：

- `NotificationSender`
- `MemoryEventEmitter`

建议改法：

- 去掉 `NotificationSender` 这个临时 trait
- `MemoryEventEmitter` 继续保留为事件职责
- `MvpPipeline` 新增 `notification_service: Arc<NotificationService>`

这样 `handle_judgment()` 中的：

```rust
self.effects.send(&title, &body)
```

改成：

```rust
self.notification_service.send(NotificationPayload { ... })
```

而 `emit_memory_added` 仍由现有事件系统承担。

### 8.2 App Setup

[`src-tauri/src/lib.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/lib.rs) 的 `AppNotificationSender` 可以拆分掉：

- 通知能力交给 `NotificationService`
- 记忆事件发射能力单独保留一个 `AppMemoryEventEmitter`

这样 `setup()` 时会管理：

- `NotificationService`
- `MemoryEventEmitter`
- `MvpPipeline`

### 8.3 Test Notification Command

[`src-tauri/src/commands/notification.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/commands/notification.rs) 不应再直接调用系统通知插件，而应改成：

- 从 `AppState` 读取 `NotificationService`
- 构造测试 payload
- 调用当前 active provider

这样设置页里的“发送测试通知”会验证真实通知链路，而不是旧的系统通知旁路。

## 9. Frontend Renderer

### 9.1 Entry Strategy

overlay renderer 不建议复用现有主路由。建议新增一个最小前端入口，仅承载：

- 渲染通知底带
- 监听 Tauri event
- 执行动画状态机

这样做的优点：

- 不会把主应用的 layout、router、onboarding 逻辑带进 overlay 窗口
- 加载更快
- 更容易控制透明背景和专用样式

### 9.2 Rendering Rules

v1 的视觉规则固定如下：

- 背景底边是高饱和纯色带
- 向上渐变到透明
- 中间文案最大宽度受控，避免长文本横向铺满
- 文案建议只显示 1 到 2 行，超长做截断
- 不允许 overlay 抢占用户输入焦点

### 9.3 Accessibility

虽然是视觉提示，仍应保留最小可访问性支持：

- 文案层使用语义化文本节点
- 支持 `aria-live="polite"` 或等价语义
- 颜色对比度不低于可读阈值

## 10. Testing Strategy

### 10.1 Rust Tests

需要覆盖：

- `NotificationService` 在 `enabled = false` 时不会发送
- `NotificationService` 会按 `provider` 选择正确实现
- `OverlayNotificationProvider` 会正确构建 view model
- 测试通知命令会走 service，而不是绕过 provider

### 10.2 Frontend Tests

需要覆盖：

- 收到新的 `sequence` 后会重置状态机
- 组件会按 `enter -> breathe -> exit` 流转
- 长文本会截断或受约束，不突破布局

### 10.3 Manual QA

需要人工确认：

- 通知确实贴在主屏幕最底端
- 渐变方向正确，为“下实上虚”
- 文案居中
- 呼吸效果轻微且不打扰
- 单入单出符合预期
- 新通知到来时，旧通知被替换并重启动画

## 11. Risks

### 11.1 Tauri Window Behavior Across Platforms

透明、置顶、不可聚焦窗口在不同平台上行为可能不同。虽然当前桌面目标以本地开发环境为主，仍需要把窗口控制逻辑限制在最小范围。

### 11.2 Overlay Might Compete With Fullscreen Apps

底边 overlay 在全屏应用之上是否稳定显示，可能因平台和窗口管理器而异。v1 先接受这个风险，不额外引入更复杂的屏幕/空间检测逻辑。

### 11.3 Main Router Reuse Would Cause Unnecessary Coupling

如果复用主路由，overlay 会被 onboarding、layout、query client 初始化等逻辑污染，导致窗口更重、更慢、更难调。这个风险通过独立入口避免。

## 12. Implementation Order

建议按下面顺序实现：

1. 新增 notification config 数据结构和默认值
2. 新增 `providers/notification` 模块与类型
3. 新增 `notification_service.rs`
4. 在 Tauri setup 中装配 `NotificationService`
5. 把 `mvp_pipeline` 和 `send_test_notification` 改到新 service
6. 新增 overlay window controller
7. 新增 overlay 前端入口和 renderer
8. 加测试与手工验收

## 13. Acceptance Criteria

完成后应满足：

- 代码中存在真正的 `NotificationProvider` 抽象
- `MvpPipeline` 不再直接依赖系统通知插件
- 设置页测试通知走当前 active provider
- active provider 为 `overlay` 时，会显示屏幕底边通知
- 通知具有由下到上、纯色到透明的渐变
- 通知内容在屏幕中间显示
- 通知具备单入单出和停留期轻微呼吸效果
- v1 仍保持单 provider 模式，不引入 fan-out
