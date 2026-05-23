# Top Island Notification Design

## 1. Goal

把当前“屏幕底边 overlay 通知”替换成一个更接近 macOS 灵动岛语义的顶部通知形态：

- 通知默认显示为贴在屏幕顶部中央的小黑块
- 默认态只显示图标和标题
- 用户将鼠标移到小黑块上时，通知以连续、丝滑的动画展开
- 展开后显示标题和正文
- 鼠标移出后不立即收起，而是延迟一小段时间再收起
- 整个交互保持为单窗口、单实体、单入单出，不做多弹窗切换

## 2. Why Replace Bottom Overlay

当前实现位于 [`src-tauri/src/providers/notification/overlay.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/overlay.rs) 和 [`src/overlay/overlay-app.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-app.tsx)。

现状特点：

- 通知窗口直接按主屏幕尺寸贴到底边
- 内容区域使用底边渐变带和中间正文
- 动画模型是底边进入、停留呼吸、底边退出

这个方案的问题不是单纯的 Dock 遮挡，而是整体交互语义已经不符合想要的产品方向：

- 用户希望的是一个顶部的小黑块实体，而不是底边氛围带
- 需要 hover 后展开的交互，而不是“直接显示全文”
- 需要一个更像系统表面的对象，而不是底边提示条

因此这次不是“修补 Dock safe area”，而是彻底替换通知形态。

## 3. Non-Goals

- 不在这次设计里保留底边 overlay 作为并行样式
- 不在这次设计里引入多通知队列
- 不在这次设计里实现按钮、回复、关闭等复杂交互
- 不在这次设计里支持多屏 hover 策略
- 不在这次设计里做平台特化外观分支
- 不在这次设计里引入通知历史或持久化面板

v1 目标是把单条通知变成顶部“岛”式通知，而不是做桌面消息中心。

## 4. Chosen Approach

### 4.1 Single Window, Two Visual States

通知继续使用独立的 Tauri overlay 窗口，但只保留一个窗口，不再拆成：

- 一个 compact 窗口
- 一个 expanded 窗口

同一个窗口承载两种视觉状态：

- `compact`
- `expanded`

用户感知到的是同一个对象在变形，而不是两个窗口互相切换。

### 4.2 Top-Centered Island

通知窗口固定锚定在主屏幕顶部中央。

compact 态是一块小黑胶囊，紧贴屏幕上边缘；expanded 态则在同一锚点基础上向下生长，形成更高的展开内容区。

关键约束：

- 顶部锚点不漂移
- 展开方向只向下
- 左右中心线保持稳定

### 4.3 Hover-Driven Expansion

通知的主要交互是 hover：

- 鼠标进入小黑块 -> 开始 expand
- 鼠标停留在展开区域内 -> 保持 expanded
- 鼠标离开 -> 进入 delayed-collapse
- 若在 delay 结束前重新进入 -> 取消收起
- 若没有重新进入 -> 执行 collapse

这比点击展开更符合“顶部系统岛”的语义，也更适合桌面环境。

## 5. Window Model

### 5.1 Window Placement

通知窗口放在主屏幕顶部中央，位置由窗口控制器统一计算。

compact 态：

- 宽度接近一个系统级胶囊岛
- 高度控制在标题一行 + 安全留白

expanded 态：

- 宽度在 compact 基础上适度扩大
- 高度向下扩张以容纳正文

不允许的行为：

- 不允许从顶部中线偏移到左上或右上
- 不允许展开时换成新的窗口位置
- 不允许“悬在离顶部很远的位置”

### 5.2 Dynamic Window Size

虽然是单窗口，但窗口尺寸必须跟状态同步变化：

- `compact` 时窗口只包住小黑块
- `expanded` 时窗口只包住展开后的岛

原因：

- 避免在屏幕顶部留下看不见的大命中层
- 避免视觉和交互边界不一致
- 保证用户 hover 的感知范围和实际 hit area 一致

### 5.3 Transparent Overlay Shell

窗口本身仍然是：

- 透明背景
- 无边框
- 置顶
- 不进入任务栏

但与之前底边版本不同的是，窗口现在不承担“全宽覆盖层”的语义，而是只包裹岛形内容本身。

## 6. State Machine

### 6.1 Notification States

建议状态机：

`hidden -> compact-enter -> compact-idle -> expanding -> expanded -> collapse-delay -> collapsing -> compact-idle -> hidden`

各状态语义：

- `hidden`
  当前没有可见通知
- `compact-enter`
  小黑块初次出现
- `compact-idle`
  小黑块静止显示，等待 hover 或生命周期结束
- `expanding`
  从 compact 向 expanded 生长
- `expanded`
  正文可见，hover 持续保持
- `collapse-delay`
  鼠标已移出，但还在延迟容错期
- `collapsing`
  从 expanded 收回 compact

### 6.2 Two Timers

这次需要明确拆成两套计时：

1. 通知总生命周期
2. hover 收起延迟

#### Total Lifetime

控制一条通知何时彻底消失，即：

- 从 `compact-idle` 进入 `hidden`
- 或在 expanded 完成后回到 compact 再最终隐藏

这个计时不应该因为用户多次 hover 而无限延长。

#### Collapse Delay

只负责 expanded 态的 hover 容错：

- 鼠标刚离开，不立即收起
- 给用户一个短暂“走错一点也不会闪退”的缓冲

这是产品已经明确选择的交互。

## 7. Visual Language

### 7.1 Compact State

compact 态是一个近黑色高圆角胶囊：

- 颜色不是灰卡片，而是接近硬件表面的深黑
- 半径较大，接近系统岛形
- 边缘有极轻微高光或内阴影
- 厚度明显，不做细条状提示

内容仅包含：

- 一枚小图标或状态点
- 一行标题

标题必须单行截断，不允许换行。

### 7.2 Expanded State

expanded 态保留同样的黑色主体，只是在底部自然延长出正文区。

显示内容：

- 标题
- 正文
- 可选的小图标或来源点

不显示：

- 操作按钮
- 次级描述标签
- 卡片边框
- 多余装饰图形

正文建议显示 2 到 4 行，超过则截断。

### 7.3 Motion Character

动画风格必须更接近系统硬件界面，而不是普通 toast：

- 连续
- 有黏性
- 没有明显断点
- 不做弹跳式过度夸张动画

compact -> expanded 的重点不是“快”，而是“像同一个物体在长大”。

## 8. Hover Behavior

### 8.1 Hover Entry

当鼠标进入 compact 态小黑块：

- 立即进入 `expanding`
- 标题和图标位置平滑调整
- 正文透明度与尺寸变化同步出现

不采用：

- 先扩大容器，再突然出现文字
- hover 后等待很久才开始展开

### 8.2 Hover Retention

expanded 态下，只要鼠标还在整个岛区域内，就保持展开。

这里的“整个岛区域”包括：

- 原始 compact 胶囊区域
- 向下展开后的正文区域

### 8.3 Delayed Collapse

鼠标离开后：

- 不立即收起
- 先进入 `collapse-delay`
- 若鼠标在 delay 期内返回，则取消收起
- 若未返回，则执行 `collapsing`

这层 delay 的目标是“容错”，不是“拖延”。它应该短而自然。

## 9. Hit Area Strategy

### 9.1 Compact Hit Area

compact 态只有小黑块本体可命中。

明确不做：

- 顶部整条透明 hover 区
- 隐形超大命中框

原因：

- 会造成误触发展开
- 会让用户觉得顶部存在一块看不见的层
- 和视觉边界不一致

### 9.2 Expanded Hit Area

expanded 态的整块岛形区域都应算 active hit area。

这样鼠标从标题移动到正文区域时，不会错误触发收起。

### 9.3 Window and Hit Area Alignment

这次一个重要边界是：

- 视觉范围
- 窗口范围
- hover 命中范围

三者必须尽量一致。

这是选择“单窗口动态尺寸”而不是“固定大透明窗口”的核心原因。

## 10. Data Model Changes

现有通知 payload 继续保留：

- `title`
- `body`
- `level`
- `duration_ms`

新增一个轻量展示字段即可：

- `icon_kind`
  或
- `accent_kind`

语义：

- compact 态显示什么小图标
- 不承担复杂业务逻辑

这次不引入：

- 富文本正文
- 自定义按钮
- 多段内容块
- 图片预览

## 11. Integration Changes

### 11.1 Replace Bottom Layout Strategy

现有 [`overlay.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/overlay.rs) 里的定位逻辑是按主屏幕底边计算：

- 用主屏尺寸直接贴底
- 窗口高度是底边带高度

新设计中需要改成：

- 顶部中线锚定
- compact / expanded 两套尺寸
- 按状态重设窗口位置与尺寸

### 11.2 Replace Bottom Visual Renderer

现有 [`overlay-app.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/overlay/overlay-app.tsx) 里还是底边布局：

- 全宽 stage
- 底边渐变
- 中下方文案

新设计中要替换成：

- 顶部岛形容器
- compact / expanded 双布局
- hover 驱动的展开与收起

## 12. Testing Strategy

### 12.1 Rust / Window Controller

需要验证：

- compact 和 expanded 状态下窗口位置都锚定在顶部中央
- 状态切换时窗口尺寸更新正确
- 新通知到来时窗口状态可重置到 compact-enter

### 12.2 Frontend Renderer

需要验证：

- compact 态只显示标题和图标
- hover 后进入 expanding / expanded
- 鼠标离开后进入 collapse-delay
- delay 内重新进入会取消收起
- delay 结束后才进入 collapsing

### 12.3 Manual QA

人工验收需要确认：

- 小黑块是否真正贴顶部中央
- compact 态是否足够克制
- 展开时是否像同一个对象生长
- 正文是否清晰可读
- 移出后延迟收起是否自然
- 再次 hover 时是否能稳定恢复展开

## 13. Risks

### 13.1 Hover Stability Requires Tight Window Geometry

如果窗口尺寸、hover 区、视觉边界不同步，会出现：

- 鼠标明明还在岛上却误收起
- 或鼠标离开很远仍被判定为 hover

因此状态与窗口几何必须同步更新。

### 13.2 Over-Animating Will Make It Feel Fake

如果动画用了太强的缩放、弹跳、阴影变化，会更像网页卡片，而不是系统表面对象。

这次应该更偏“克制的系统感”。

### 13.3 Total Lifetime vs Hover Lifetime Can Conflict

若总生命周期和 hover 生命周期没有清晰分离，容易出现：

- 用户 hover 导致通知永远不消失
- 或通知在用户阅读正文时突然被 total lifetime 强制关掉

实现上必须明确这两套规则的优先级。

## 14. Acceptance Criteria

完成后应满足：

- 通知不再位于底边，而是位于屏幕顶部中央
- 默认态是小黑块，只显示图标和标题
- hover 后同一个岛平滑展开
- 展开后显示标题和正文
- 鼠标离开后进入 delayed-collapse，而不是立即收起
- 在收起延迟内重新 hover 会取消收起
- 交互由单窗口承载，不引入第二个展开窗口
- 整体视觉更接近系统表面对象，而不是 toast 或卡片
