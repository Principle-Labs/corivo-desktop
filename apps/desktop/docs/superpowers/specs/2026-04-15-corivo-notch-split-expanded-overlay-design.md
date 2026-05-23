# Corivo Notch Split / Expanded Overlay Design

## 1. Goal

为 Corivo 顶部 notification overlay 明确一套新的 notch-aware 内容布局合同：

- compact 态像 `CodeIsland` 一样围绕刘海分配左右内容区
- expanded 态获得更大的展示宽度，但顶部仍保留刘海空槽
- 真刘海屏增强布局能力，无刘海屏和异常刘海指标继续走当前 simulated 路径

这份设计只调整 overlay 的几何合同和内容布局，不改变现有通知链路、单窗口模型、或状态机语义。

## 2. Reference Direction

参考项目：[`wxtsky/CodeIsland`](https://github.com/wxtsky/CodeIsland)

本设计借用的是它的两条核心原则，而不是照搬实现：

- 窗口应贴在顶部，让 island 看起来像从刘海/菜单栏长出来
- 内容不应只在刘海正下方堆叠，而应围绕刘海分配到左右可用区

Corivo 与 `CodeIsland` 的差异是：

- Corivo 仍保留现有 Tauri 单 overlay window
- Corivo 的 expanded 态要展示通知正文，因此会比 `CodeIsland` 更强调展开后的正文容量

## 3. Current State

当前工程里已经有一版 notch-aware 基础能力：

- 后端 [`src-tauri/src/providers/notification/screen_metrics.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/screen_metrics.rs) 能识别目标屏、刘海、安全区，并输出 compact / expanded window geometry
- 前端 [`src/overlay/overlay-app.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-app.tsx) 与 [`src/styles/overlay/island.css`](/Users/airbo/Developer/corivo/corivo-app/src/styles/overlay/island.css) 已能依据 `has_notch` 和 `anchor_mode` 做基础 notch-aware 样式

当前缺口不是屏幕几何不存在，而是内容布局仍偏保守：

- compact 主要还是“整体加宽 + 单侧 padding”语义
- headline 没有被明确拆成 notch 左右两个可读区
- expanded 虽然更宽，但顶部内容区没有稳定保留 notch 空槽

结果是：

- 短文本还能工作
- 长一点的标题或状态信息依然容易落到刘海中心附近
- “内容显示在刘海之外”这件事没有被结构性保证

## 4. Product Decisions

已确认的产品决策如下：

- compact 态采用 `B` 方案：headline 围绕刘海分成左区 / gap / 右区
- expanded 态采用接近 `C` 的方案：整体变宽以增加信息容量
- expanded 顶部仍保留 notch 空槽，不做完全占满的横幅
- 新增的展示宽度优先给正文区，而不是把 headline 彻底改成全宽横排
- 非刘海屏、外接无刘海屏、或无效刘海指标继续走 simulated 模式，不改变现有产品语言

## 5. Chosen Approach

### 5.1 Compact: Split Around Notch

compact 态不再只表达“宽度略大于 notch”，而是明确表达：

- 左侧有最小可读区
- 中间有 notch gap
- 右侧有最小可读区

compact headline 结构固定为三列：

`left content | notch gap | right content`

其中：

- `left content` 承载主标题或应用名
- `right content` 承载状态、时间、或短摘要
- `notch gap` 是不可放内容的视觉空槽

这能保证 compact 态的信息天然落在刘海外侧，而不是依赖 padding 微调。

### 5.2 Expanded: Wider Body, Same Notch Language

expanded 态不是 compact headline 的简单横向拉伸，而是双层合同：

- headline 继续保留 `left | gap | right`
- body 改为全宽单列，拿走主要新增空间

这让 expanded 同时满足两件事：

- 视觉上仍像从 compact notch island 生长出来
- 正文获得接近全宽的展示容量

### 5.3 Why Not Full-Width Headline

不采用“expanded 顶部也彻底占满”的方案，原因是：

- 会破坏 compact 到 expanded 的形态连续性
- 会让 expanded 更像普通顶部横幅，而不是从刘海生长出来的 island
- 标题通常比正文短，真正需要新增宽度的是 body

因此，本设计选择“headline 继续避让 notch，body 负责吃掉新增宽度”。

## 6. Geometry Contract

### 6.1 Ownership

Rust 侧继续是窗口几何和 notch 几何的唯一真源。

前端不得自行通过 `window.screen.width` 或内建 heuristics 推测可用区域。

后端负责：

- 选择目标屏
- 判断是否为真刘海模式
- 计算 compact / expanded 窗口边界
- 提供 notch 相关度量给前端布局消费

前端负责：

- 根据当前 phase 选择 compact 或 expanded 表现
- 根据 notch 合同切换 compact headline、expanded headline、expanded body 的布局

### 6.2 Compact Width Rule

真刘海模式下，compact 宽度不再只满足“比 notch 稍宽”。

它至少要满足：

`compact width >= left readable zone + notch gap + right readable zone`

也就是说，compact 的最小值应由三部分共同决定：

- `notch_gap_px`
- `min_left_zone_px`
- `min_right_zone_px`

如果现有配置宽度更大，则取更大值；如果屏幕宽度不足，则按屏幕宽度上限裁切。

### 6.3 Expanded Width Rule

expanded 宽度遵守：

- `expanded width > compact width`
- 顶边保持不动
- 中轴继续与屏幕中轴对齐

expanded 的新增宽度主要用于：

- 正文 body 的单列展示
- 更舒展的 headline 左右可读区

但 expanded headline 仍然保留 notch gap，不改成完全占满。

### 6.4 Recommended Additional Field

为了避免前端反推 notch 可用区，本设计建议在 view model 中显式新增：

- `notch_gap_px`

它表示前端在 notch-aware headline 中应保留的中间空槽宽度。

如果已有 `notch_width_px`，也可以在后端直接将两者设为同一产品语义；关键是前端消费一个明确合同，而不是自行乘系数猜。

## 7. Frontend Layout Contract

### 7.1 Compact Headline

compact 态 headline 使用显式 grid，而不是 padding hack。

推荐结构：

- grid columns: `minmax(0, 1fr) notch_gap_px minmax(0, 1fr)`
- 左侧右对齐
- 右侧左对齐
- 中间列为空槽，不允许承载文案

这样在视觉上会形成明显的“刘海一分为二”的效果。

### 7.2 Expanded Headline

expanded 态 headline 继续使用同样的三列结构，但允许：

- 左右列比 compact 更宽
- 行高和内边距更舒展
- 右侧显示更完整的状态信息

headline 的任务是维持 notch 语言连续性，而不是承担主要信息容量。

### 7.3 Expanded Body

expanded 态 body 改为全宽单列：

- 不再按 notch 左右拆分
- 直接使用 expanded 内层宽度
- 继续沿用当前 body 动画和层级，只改布局宽度合同

这样新增宽度可以真正转化为更高的文本可读性。

## 8. Fallback Behavior

以下情况全部继续走 simulated 路径：

- `has_notch = false`
- `anchor_mode = simulated`
- `top_safe_height_px` 无效
- `notch_width_px` 缺失或异常

在 fallback 模式下：

- 不启用 compact 三列 notch-aware headline
- 不启用 expanded 的 notch gap headline 合同
- 保持当前普通 island 行为

这次设计的原则是增强真刘海屏表现，而不是重写全部 overlay 样式。

## 9. File Responsibilities

### 9.1 Rust

[`src-tauri/src/providers/notification/screen_metrics.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/screen_metrics.rs)

- 调整 true-notch compact 最小宽度合同
- 调整 expanded 相对 compact 的增宽合同
- 如采用建议，补充 `notch_gap_px`

[`src-tauri/src/providers/notification/overlay.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/overlay.rs)

- 将新的 notch gap / geometry 字段装配进 view model

[`src-tauri/src/providers/notification/types.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/types.rs)

- 对齐新增输出字段

### 9.2 Frontend

[`src/lib/types.ts`](/Users/airbo/Developer/corivo/corivo-app/src/lib/types.ts)

- 对齐新的 notch-aware 布局字段

[`src/overlay/overlay-app.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-app.tsx)

- compact 态渲染三列 headline
- expanded 态渲染保留 notch gap 的 headline
- expanded body 切为全宽单列

[`src/styles/overlay/island.css`](/Users/airbo/Developer/corivo/corivo-app/src/styles/overlay/island.css)

- 明确 headline grid、左右对齐、gap 宽度变量
- 去掉仅靠 padding 模拟 notch 的关键路径

[`src/overlay/overlay-model.ts`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-model.ts)

- 如需，增加 notch-aware headline / body 布局辅助函数

## 10. Testing

### 10.1 Rust Geometry Tests

[`src-tauri/tests/notification_overlay_geometry.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/tests/notification_overlay_geometry.rs) 需要覆盖：

- true-notch compact 至少能容纳左右内容区与 notch gap
- expanded 明显宽于 compact，但顶边仍保持不动
- invalid notch metrics 回退 simulated
- 多屏与外接屏选择逻辑不回归

### 10.2 Frontend Overlay Tests

[`src/overlay/overlay-app.test.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-app.test.tsx) 需要覆盖：

- compact headline 使用 notch-aware 三列结构
- expanded headline 仍保留 notch gap
- expanded body 为全宽单列
- simulated 模式不会进入这些 notch-aware 布局

## 11. Non-Goals

- 不引入第二个 overlay window
- 不改通知状态机时序
- 不改 compact / expanded 的基本动画语义
- 不追求逐像素复刻 Apple Dynamic Island

## 12. Recommendation

采用“compact 分栏避让 notch + expanded 增容但保留顶部 gap”的方案。

这是当前约束下最稳的折中：

- compact 真正保证内容显示在刘海之外
- expanded 真正增加正文可读面积
- 整体视觉语言仍连续，不退化成普通顶部横幅
