# Corivo Notch-Aware Overlay Design

## 1. Goal

为 Corivo 当前顶部通知 overlay 增加一层真正可落地的 notch-aware 能力，同时保持现有通知链路、单窗口模型和 island 动画语言不分叉。

这次设计要解决四件事：

- 多显示器时，通知应该出现在用户当前正在看的那块屏
- 真刘海屏上，compact 态要更像从硬件 notch 长出来
- 无刘海屏上，继续保留当前“假刘海 / island”语义，不退化成普通 toast
- expanded 态保持顶边固定，只向下生长

## 2. Current State

当前实现已经具备单窗口顶部 island 的基本形态：

- 后端通过 [`src-tauri/src/providers/notification/overlay.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/overlay.rs) 创建并驱动透明置顶 overlay 窗口
- 前端通过 [`src/overlay/overlay-app.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-app.tsx) 和 [`src/overlay/overlay-model.ts`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-model.ts) 渲染 compact / expanded island
- 窗口几何目前仍带有“主屏固定顶部 + 前端本地推算 x/y”的假设

现状问题不是视觉基础不存在，而是几何来源过于简单：

- 只看主屏，不看用户当前工作屏
- 不区分真刘海和无刘海屏
- 前端仍依赖 `window.screen.width` 做居中推算
- island 形状虽然已接近 notch-native，但没有真正利用硬件 notch 几何

## 3. Product Decisions

这次设计采用以下已确认决策：

- 多显示器时，通知出现在前台窗口所在屏
- 无刘海屏不做退化处理，继续保留当前 island / 假刘海语言
- 真刘海屏 compact 宽度不严格等于真实刘海，而是比真实刘海略宽一圈
- expanded 顶边始终钉在屏幕顶部，不整体下沉
- 继续保留单 overlay provider、单 overlay window、单状态机

## 4. Chosen Approach

### 4.1 Geometry-Aware Island

采用“几何感知 island”方案，而不是轻量定位补丁或双模式 overlay。

核心思路：

- 后端增加一层屏幕几何解析
- 继续复用现有 notification provider 和 overlay window
- 前端根据后端传来的屏幕几何，切换 island 的 compact / expanded 参数

这条路径能同时满足：

- 不分叉通知体系
- 保持现有 React island 结构和状态机
- 在真刘海屏上做更可信的硬件贴合
- 在无刘海屏上保留现有产品语言

### 4.2 Ownership Split

职责边界明确分成两层：

- Rust 侧负责“出现在什么屏、窗口边界多大、是否有刘海、刘海几何是多少”
- React 侧负责“岛长什么样、compact / expanded 怎么动、内容如何绕开 notch 感”

不允许让前端继续自行猜测屏幕几何。

## 5. Architecture

通知主链路保持不变：

`MvpPipeline / commands -> NotificationService -> OverlayNotificationProvider -> overlay window -> React overlay`

这次只在 `OverlayNotificationProvider` 内增加一层屏幕几何解析，不引入新的 provider 类型，也不引入第二个 overlay window。

建议新增：

- [`src-tauri/src/providers/notification/screen_metrics.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/screen_metrics.rs)

该模块负责：

- 获取前台普通窗口 bounds
- 从 monitor 列表中选出目标屏
- 检测目标屏是否有刘海
- 计算 `notch_width`、`top_safe_height`、目标屏尺寸
- 产出 compact / expanded 两组窗口几何

[`src-tauri/src/providers/notification/overlay.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/overlay.rs) 只保留：

- provider 入参处理
- 调用 `screen_metrics`
- 组装 `OverlayNotificationViewModel`
- 发送事件给 overlay renderer

## 6. Screen Selection

### 6.1 Primary Rule

目标屏的选择规则为：

1. 取前台应用最前层普通窗口 bounds
2. 用该 rect 与所有 monitor frame 做命中 / overlap 计算
3. 命中中心点优先，overlap 面积次之
4. 如果无法解析前台窗口，则回退主屏

这样通知会跟着用户当前工作区走，而不是永远停在主屏。

### 6.2 Fallback Principle

notch-aware 是增强层，不是阻断层。

如果前台窗口解析失败：

- 仍必须显示通知
- 直接回退到主屏
- 不允许因为目标屏判定失败而丢通知

## 7. Notch Detection and Metrics

在 macOS 上，真刘海检测优先使用 `NSScreen` 的 safe area / auxiliary area 信息。

建议输出的屏幕几何字段：

- `has_notch: bool`
- `anchor_mode: "notch" | "simulated"`
- `screen_width_px: u32`
- `screen_height_px: u32`
- `top_safe_height_px: u32`
- `notch_width_px: Option<u32>`

其中：

- 真刘海屏：`has_notch = true`，`anchor_mode = "notch"`
- 无刘海屏：`has_notch = false`，`anchor_mode = "simulated"`

如果刘海测量失败，则直接按 `simulated` 处理。

## 8. Window Geometry Rules

### 8.1 Top Anchor

compact 和 expanded 共用同一条顶部锚线：

- 顶边位置不漂移
- expanded 只向下生长
- 中心线始终与目标屏中线对齐

### 8.2 True-Notch Compact Geometry

真刘海屏上，compact 宽度按“真实刘海宽度乘产品系数”得到，而不是严格贴齐真实 notch。

规则：

- `baseNotchWidth = measured notch width`
- `compactWidth = clamp(baseNotchWidth * notchScale, minCompact, maxCompact)`
- `compactHeight = clamp(topSafeHeight + visualPadding, minHeight, maxHeight)`

其中 `notchScale` 为大于 1 的轻微放大系数，用来实现“比真实刘海略宽一圈”的产品效果。

### 8.3 Simulated Compact Geometry

无刘海屏继续使用当前的 compact island 语言，不额外做平台分叉。

规则：

- 继续从配置读取 compact 尺寸
- 仅通过 `anchor_mode = "simulated"` 告知前端这是非真实 notch 模式

### 8.4 Expanded Geometry

expanded 几何遵守以下约束：

- 顶边位置不变
- 宽度可在 compact 基础上适度扩展
- 高度仅向下增加

即：

- `expandedY = compactY`
- `expandedX = screen.midX - expandedWidth / 2`

不允许 expanded 整体下沉，不允许从顶部锚点漂开。

## 9. Window Geometry Ownership

当前前端 `overlay-model` 仍在根据 `window.screen.width` 推导窗口几何，这不适用于 notch-aware 和多显示器。

本次改动后，窗口几何的最终所有权归 Rust：

- Rust 计算 compact window geometry
- Rust 计算 expanded window geometry
- 前端只按当前 phase 选择其中一组同步到窗口

建议新增到 `OverlayNotificationViewModel`：

- `compact_window_x`
- `compact_window_y`
- `compact_window_width`
- `compact_window_height`
- `expanded_window_x`
- `expanded_window_y`
- `expanded_window_width`
- `expanded_window_height`

这样前端不再自行“猜”显示器位置。

## 10. Frontend Shape Rules

前端仍沿用当前顶部 island 的形态语言：

- 顶边平直
- 顶角固定为 `0`
- compact 底角更大
- expanded 底角更收敛
- 内容只向下展开

但在 notch-aware 模式下增加一条规则：

- 真刘海屏上，headline 区和内容布局必须保留更强的中轴留白感，避免文字和图标压在 notch 中心的视觉位置上

这不要求逐像素复刻真实硬件 cutout，但要求肉眼可感知“内容围着 notch 长”。

## 11. File Changes

### 11.1 Rust

需要修改或新增：

- [`src-tauri/src/providers/notification/screen_metrics.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/screen_metrics.rs)
- [`src-tauri/src/providers/notification/types.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/types.rs)
- [`src-tauri/src/providers/notification/overlay.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/overlay.rs)

### 11.2 Frontend

需要修改：

- [`src/lib/types.ts`](/Users/airbo/Developer/corivo/corivo-app/src/lib/types.ts)
- [`src/overlay/overlay-model.ts`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-model.ts)
- [`src/overlay/use-overlay-notification.ts`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-hooks.ts)
- [`src/overlay/overlay-app.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-app.tsx)

## 12. Testing

测试分三层：

### 12.1 Rust Geometry Tests

覆盖：

- 无刘海单屏
- 真刘海单屏
- 多屏命中副屏
- 前台窗口解析失败回退主屏
- safe area / notch width 异常值

重点验证：

- 目标屏选择
- compact / expanded 窗口边界计算
- fallback 行为稳定

### 12.2 Frontend Model Tests

覆盖：

- `has_notch = false` 仍走 simulated island
- `has_notch = true` 切到 notch-aware 壳体参数
- compact phase 选 compact geometry
- expanded / collapse-delay phase 选 expanded geometry

重点是防止前端重新引入本地屏幕猜测逻辑。

### 12.3 Manual QA

至少验证：

- 内建刘海屏通知显示在前台窗口所在屏
- 真刘海屏 compact 比真实刘海略宽
- hover 展开时顶边不动，只向下长
- 外接无刘海屏仍显示 island，不退化成普通 toast
- 目标屏解析失败时通知仍能回退主屏显示

## 13. Failure Fallback

必须遵守以下回退规则：

- 前台窗口所在屏解析失败 -> 回退主屏
- notch 测量失败 -> 回退 `simulated` 模式
- 几何字段异常 -> 回退现有配置尺寸
- 任一增强逻辑失败 -> 不影响通知显示

最差情况应当是“退回今天的 overlay 行为”，而不是“通知消失”。

## 14. Non-Goals

本次明确不做：

- 新 notification provider
- 双窗口 overlay
- 真 / 假刘海两套完全不同的产品模式
- 多通知队列
- 复杂通知交互
- 通知中心或持久化历史

## 15. Chosen Recommendation

最终采用：

- 单 overlay provider
- 单 overlay window
- 前台窗口所在屏作为目标屏
- 真刘海屏使用真实测量 + 轻微放大系数生成 compact island
- 无刘海屏继续使用当前 simulated island 语言
- expanded 顶边固定，只向下展开
- 窗口几何由 Rust 统一计算，前端只负责渲染和动画

这是在 Corivo 当前通知架构下，最能贴合你现有实现、同时把 notch-aware 做成真正产品能力的方案。
