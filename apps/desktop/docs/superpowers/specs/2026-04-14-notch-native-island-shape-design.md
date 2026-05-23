# Notch Native Island Shape Design

## 1. Goal

为顶部 notification island 补充一套更接近 `CodeIsland` 方向的形状设计，明确三件事：

- 圆角应该如何分配
- 如何处理刘海区域与屏幕顶部安全边界
- 展开后的轮廓应该长成什么样

这份设计只约束 overlay island 的形状语言，不改变既有通知状态机、provider 链路、或 Tauri 单窗口模型。

## 2. Reference Direction

参考项目：[`wxtsky/CodeIsland`](https://github.com/wxtsky/CodeIsland)

从公开描述可确认的方向：

- `Notch-native UI`
- `Expands from the MacBook notch`
- `Auto-detects notch displays`

本设计不复制其实现，但采用相同的视觉原则：

- 岛不是“悬浮在刘海下方的 toast”
- 岛应当看起来像“从刘海/菜单栏底边长出来的系统对象”

## 3. Core Shape Decision

### 3.1 Base Metaphor

选择“顶部硬连接、底部软展开”的形状，而不是完整胶囊。

用户感知应该是：

- compact 态：一片从顶部切下来的短挂片
- expanded 态：同一片挂片向下展开成更大的通知面

不应该是：

- 一个普通 pill 从小放大
- 一张完整矩形卡片突然掉下来

### 3.2 Corner Rules

顶部两个角始终不参与圆角动画：

- `top-left = 0`
- `top-right = 0`

底部两个角承担视觉软化：

- compact 态：底部较大圆角
- expanded 态：底部中等圆角

推荐参数：

- compact：`bottom-left = bottom-right = 18..24`
- expanded：`bottom-left = bottom-right = 12..16`

如果 compact 需要更强的“岛”感，可以短暂使用更大底角，但 expanded 不应继续维持胶囊级大圆角。

### 3.3 Top Edge

顶部边必须保持一条稳定直线：

- 不上拱
- 不做半圆顶
- 不随动画漂移

这样才能建立“岛挂在系统顶部”的感觉。

## 4. Notch Avoidance

### 4.1 Principle

“避开刘海”不等于简单下移。

正确目标是：

- 在视觉上贴近刘海/菜单栏底边
- 在命中和内容上不侵入刘海主体

也就是说：

- 视觉锚点贴顶部
- 展开内容只向下生长
- 交互边界不要假装刘海区域是可用内容区

### 4.2 Positioning Rules

统一使用主显示器顶部中心作为锚点。

有刘海屏：

- compact 宽度应接近刘海主体视觉宽度，不要明显大一圈
- island 与顶部边形成“嵌入感”，而不是悬浮感

无刘海屏：

- 保持相同顶部挂片造型
- 不额外引入平台分叉视觉

### 4.3 Safe Expansion

expanded 态允许宽度超过 compact，但要遵守：

- 展开只向左右和下方发生
- 顶边位置不变
- 顶部两个角仍保持直角

这会让 expanded panel 看起来像从 notch 下沿展开，而不是整块气泡整体放大。

## 5. Expanded Shape

### 5.1 Form

expanded 态采用“hanging panel”轮廓：

- 顶边平直
- 左右边从上到下轻微外扩
- 底边圆润
- 整体重心下沉

它应该像一个从顶部挂下来的面板，而不是一个更大的胶囊。

### 5.2 Animation Character

动画的主观感受应是：

- compact island 固定挂在顶部
- 内容面从底边向下抽出
- 收起时内容面被吸回 compact island

因此：

- 原生窗口 resize 只负责边界切换
- 视觉上的连续放大/收起交给窗口内部 motion 动画

## 6. Implementation Guidance

### 6.1 Shape Ownership

形状控制放在前端 island motion 层：

- 顶部角半径固定为 `0`
- 底部角半径随 compact / expanded 变化
- 宽高变化通过 motion 驱动

### 6.2 Window Ownership

Tauri window 只负责：

- compact 边界尺寸
- expanded 边界尺寸
- 顶部中心定位

窗口本身不承担连续视觉动画。

### 6.3 CSS / Motion Split

静态视觉资产放在 overlay 独立样式文件中：

- 背景
- 阴影
- 文本层级

动态形态放在 motion 对象中：

- width
- height
- bottom corner radius
- opacity / y / scale

## 7. Non-Goals

- 不为不同机型做单独视觉主题
- 不在这次设计里做真实刘海几何测量
- 不引入与系统截图完全一致的硬件仿真

v1 目标是建立“notch-native”的形态语言，不是逐像素复刻 Apple Dynamic Island。

## 8. Chosen Recommendation

最终采用：

- 顶部两角永久直角
- 底部两角负责圆角动画
- island 贴顶部中心
- 展开后使用“下垂挂片”而不是大胶囊
- 外层窗口只切边界，内部 motion 负责连续生长

这是在 Corivo 当前 Tauri overlay 约束下，最接近 `CodeIsland` 方向、同时命中边界和实现复杂度最平衡的方案。
