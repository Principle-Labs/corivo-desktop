# Overlay Panel Motion Tuning Design

**Goal**

让通知 panel 的出现和消失更接近灵动岛：出现时有明确的“撑开”感，消失时有清楚的“回吸”感，同时保持现有常驻复用 panel 生命周期不变。

**Scope**

- 只调整 `src/overlay` 的前端动效参数与阶段性表现。
- 不改 Tauri panel 的创建/销毁策略。
- 不改通知内容结构和几何同步协议。

**Design**

1. 出现动效保持现有 `compact-idle -> expanding -> expanded` 状态机，但把视觉语言拆层：
   - 外壳形变比正文更早开始，形成“先撑开外壳”的感觉。
   - 正文在 `expanding` 期间保持更低的透明度和更深的纵向偏移，等进入 `expanded` 后更明显地上浮进入。

2. 消失动效保持现有 `expanded -> collapse-delay -> compact-idle/hidden` 路径，但强化回收层次：
   - 正文在 `collapse-delay` 就开始更快淡出并轻微下沉。
   - 外壳收回 compact 时使用更利落的 easing 和更短的尾段，形成“吸回去”的感觉。

3. 不再让所有元素共享一套统一 transition：
   - 外壳使用更有存在感的形变节奏。
   - 正文和 headline 使用更轻、更快的内容节奏。

**Files**

- Modify: `src/overlay/overlay-app.tsx`
- Modify: `src/overlay/overlay-model.ts`
- Modify: `src/overlay/overlay-app.test.tsx`

**Testing**

- 为触发和消失阶段新增/更新单测，确认不同 phase 的 motion 参数。
- 跑 `src/overlay/overlay-app.test.tsx` 以及相关 tauri wrapper 测试，避免回归。
