# Tauri NSPanel Panel-First Overlay Design

## 1. Goal

把 Corivo 当前的通知 overlay 从“一个被手工改造成像 panel 的 Tauri window”重构为“一个真正由 `tauri-nspanel` 驱动的 macOS panel”。

这次设计的目标不是重写通知 UI，而是重写 overlay 的原生容器层：

- 用 `tauri-nspanel` 接管 `notification-overlay` 的 macOS panel 壳
- 保留现有 React overlay 内容层和 compact / expanded 动画语言
- 保留当前的 notch-aware 多屏几何逻辑
- 把 panel 生命周期、层级、Space 行为、激活策略收回到独立 panel 管理层

## 2. Current State

当前实现已经具备一套单窗口顶部 island：

- [`src-tauri/src/providers/notification/overlay.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/overlay.rs) 负责创建 `notification-overlay` window、设置透明置顶属性并发送通知事件
- [`src-tauri/src/providers/notification/screen_metrics.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/screen_metrics.rs) 负责前台窗口所在屏选择、notch 检测和 compact / expanded 几何生成
- [`src/overlay/overlay-app.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-app.tsx) 与 [`src/overlay/overlay-model.ts`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-model.ts) 负责 compact / expanded 状态机和 island 内容渲染

当前 macOS panel 行为主要靠手写 AppKit patch 完成：

- `NonactivatingPanel`
- `NSMainMenuWindowLevel + 2`
- `CanJoinAllSpaces / FullScreenAuxiliary / Stationary / IgnoresCycle`
- `orderFront`
- `hidesOnDeactivate = false`

这意味着当前 overlay 在产品上已经像 panel，但在架构上仍然是“一个普通 window + 一层手写原生 patch”。

## 3. Problem

当前方式的问题不是不能用，而是边界不够稳：

- 原生 panel 语义和通知业务混在一个文件里
- macOS 专属窗口细节被写死在 provider 中，后续继续演进 overlay 会越来越难维护
- panel 生命周期、show / hide、窗口层级、space 行为没有独立所有者
- 手写 AppKit patch 会和未来更深的 panel 能力互相缠绕

如果后续还要继续把顶部 island 做得更接近系统表面，这个边界迟早要重构。

## 4. Chosen Approach

### 4.1 Panel-First Architecture

采用 panel-first 重构：

- `notification-overlay` 仍然是同一个可见实体
- 但它的原生容器不再被视为普通 `WebviewWindow`
- 在 macOS 上，容器层改为由 `tauri-nspanel` 统一管理

换句话说：

- 现在：`overlay business owns panel behavior`
- 改后：`panel layer owns panel behavior`

### 4.2 What Stays vs What Changes

保留：

- React overlay UI
- compact / expanding / expanded / collapse-delay 状态机
- notch-aware 几何和多屏定位
- 通知 provider 链路

重构：

- overlay 的原生容器创建方式
- panel 的 setup / show / hide / reuse 生命周期
- macOS panel 层级、全屏辅助、激活策略

## 5. High-Level Architecture

改造后的主链路：

`NotificationService -> OverlayPanelProvider -> PanelController -> tauri-nspanel Panel -> React overlay content`

其中：

- `OverlayPanelProvider` 仍然是通知 provider 的业务入口
- `PanelController` 是新引入的原生容器层
- React overlay 继续是可视内容层，不直接负责 panel 语义

### 5.1 Layer Responsibilities

#### Notification layer

负责：

- 接收通知 payload
- 计算本次 overlay 的 notch-aware 几何
- 构建前端 view model
- 决定何时 show / hide / update frame

不负责：

- 手写 AppKit panel 行为

#### Panel layer

负责：

- 创建或复用 `notification-overlay` panel
- 使用 `tauri-nspanel` 设置 non-activating、level、space 行为、show / hide 行为
- 管理 panel 初始化与复用

不负责：

- notch-aware 几何推导
- React 动画状态

#### React overlay layer

负责：

- compact / expanded 视图状态
- hover 展开 / collapse-delay
- island 壳体动画和内容布局

不负责：

- panel 生命周期
- macOS panel 行为配置

## 6. Panel Lifecycle

### 6.1 Create at App Setup

panel 在 app setup 阶段创建，而不是等第一条通知来了再创建。

原因：

- 首条通知更稳
- 避免首次 show 时的原生窗口构造抖动
- panel 行为在启动期统一初始化

### 6.2 Reuse, Don’t Recreate

panel 作为常驻壳存在：

- app 启动时 create
- 无通知时 hidden
- 收到通知时 reuse 并 show
- 通知结束后 hidden

不在每条通知结束后销毁 panel。

### 6.3 Visibility Ownership

panel 层拥有：

- create
- show
- hide
- order / focus policy

React 层拥有：

- 是否处于 compact / expanded
- hover 是否触发内容展开

## 7. State Model

### 7.1 Window-Level State

panel 只需要两种原生状态：

- hidden
- visible

### 7.2 Content-Level State

React 继续保持：

- `compact-idle`
- `expanding`
- `expanded`
- `collapse-delay`

### 7.3 Boundary-Only Geometry Switching

panel frame 切换只发生在状态边界：

- 准备显示 compact 通知时 -> 设置 compact frame
- 进入 expanded 边界时 -> 设置 expanded frame
- collapse 回 compact 时 -> 切回 compact frame
- 通知结束时 -> hide panel

不在动画每一帧同步原生窗口几何。

## 8. Notch-Aware Geometry

panel-first 重构不改变现有 notch-aware 几何原则。

继续保留：

- 多显示器按前台窗口所在屏显示
- 真刘海屏使用 measured notch width + widening scale
- 无刘海屏继续 simulated island
- expanded 顶边固定，只向下生长

这意味着：

- [`src-tauri/src/providers/notification/screen_metrics.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/screen_metrics.rs) 继续保留
- panel 层只消费 compact / expanded frame 结果，不自己推导 notch 几何

## 9. File Plan

### 9.1 Keep

保留以下前端文件为内容层：

- [`src/overlay/overlay-app.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-app.tsx)
- [`src/overlay/overlay-model.ts`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-model.ts)
- [`src/overlay/use-overlay-notification.ts`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-hooks.ts)
- [`src/styles/overlay/island.css`](/Users/airbo/Developer/corivo/corivo-app/src/styles/overlay/island.css)

### 9.2 Add

建议新增：

- [`src-tauri/src/providers/notification/panel.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/panel.rs)

职责：

- `ensure_panel`
- `show_panel`
- `hide_panel`
- `update_panel_frame`
- macOS `tauri-nspanel` panel 配置

### 9.3 Refactor

重构：

- [`src-tauri/src/providers/notification/overlay.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/overlay.rs)

让它只承担：

- provider 入参处理
- 调用 `screen_metrics`
- 构建 view model
- 调用 `panel.rs` 执行 panel 生命周期动作
- 发事件给前端

### 9.4 Update

更新：

- [`src-tauri/src/lib.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/lib.rs)

在 setup 阶段：

- 预创建 overlay panel
- 不再直接调用手写 window patch 逻辑

### 9.5 Dependencies

更新：

- [`src-tauri/Cargo.toml`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/Cargo.toml)

引入：

- `tauri-nspanel`

并尽量收缩我们自己维护的 AppKit patch 代码。

## 10. Testing Strategy

### 10.1 Rust Geometry Tests

保留并继续运行：

- [`src-tauri/tests/notification_overlay_geometry.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/tests/notification_overlay_geometry.rs)

它继续锁定：

- 目标屏选择
- notch-aware compact / expanded 几何
- fallback 逻辑

### 10.2 Frontend Overlay Tests

保留并继续运行：

- [`src/overlay/overlay-app.test.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-app.test.tsx)

它继续锁定：

- compact / expanded 状态机
- backend-owned window geometry
- notch-aware 前端展示

### 10.3 Manual macOS Verification

panel-first 重构新增重点手测项：

- panel show / hide 不抢焦点
- panel 不出现在任务栏
- panel 在 Spaces / Mission Control 下行为稳定
- 全屏应用上方能作为辅助 panel 显示
- 多屏 / notch 屏切换稳定

## 11. Rollout Order

推荐按四步落地：

1. 接入 `tauri-nspanel`，先把普通 window 改成 panel 壳
2. 抽出独立 `panel.rs`，收拢 panel 生命周期
3. 把 notch-aware 几何切换接回 panel-first 模型
4. 删除旧的手写 AppKit panel patch 路径

这样每一阶段都可以独立验证，不需要一次性改穿整个 overlay。

## 12. Non-Goals

本次明确不做：

- 重写 React overlay UI
- 改动通知业务状态机
- 改动 notch-aware 几何模型本身
- 引入通知队列
- 引入 panel 级复杂交互代理

v1 目标是“panel-first 容器重构”，不是“通知系统重写”。

## 13. Chosen Recommendation

最终方案：

- 使用 `tauri-nspanel` 作为 macOS overlay 的原生 panel 容器
- 保留现有 React island 内容层
- 保留现有 notch-aware 几何层
- 新增独立 panel 管理层
- 把当前 `overlay.rs` 中的原生 panel 行为迁出到 panel 层
- app setup 时预创建 panel，通知期间复用 panel，仅在状态边界切 frame

这是把当前 Corivo overlay 从“像 panel 的 window”推进到“真正 panel-first 架构”的最小正确重构路径。
