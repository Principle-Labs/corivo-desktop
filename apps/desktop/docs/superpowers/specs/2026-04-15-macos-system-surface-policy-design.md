# macOS System Surface Policy Design

## Goal

为 Corivo 定义一套明确的 macOS 系统表面策略，使它在主窗口隐藏后真正表现为一个 menu bar only 的后台应用，而不是“主窗口没了，但系统仍把它当成一个普通前台 app 挂在后台”。

这份 spec 只解决 macOS 上最关键的系统语义：

- Dock icon 什么时候出现
- menu bar only 什么时候生效
- notification overlay 在隐藏态下不能把 app 重新拉回前台 regular app
- `Regular <-> Accessory` 的切换规则

这份 spec 是当前实现优先级更高、也更直接解决现有痛点的一份设计文档。  
如果与更宽泛的 [2026-04-15-daemon-ready-background-lifecycle-design.md](/Users/airbo/Developer/corivo/corivo-app/docs/superpowers/specs/2026-04-15-daemon-ready-background-lifecycle-design.md) 在 macOS surface 细节上有冲突，以这份文档为准。

## Non-Goals

- 不在这次设计里引入双进程 / daemon / IPC
- 不在这次设计里扩 tray menu 的控制面
- 不重写 capture、memory、overlay 的业务逻辑
- 不覆盖 Windows / Linux 的托盘策略
- 不静态把整个 app 永久变成 `LSUIElement` 菜单栏应用
- 不在这次设计里解决所有后台生命周期问题；重点只放在 macOS 系统层面对 app 的“呈现身份”

## Current State

当前 Corivo 已经有一个最小的 macOS menu bar preview：

- [2026-04-15-menubar-preview-design.md](/Users/airbo/Developer/corivo/corivo-app/docs/superpowers/specs/2026-04-15-menubar-preview-design.md) 已定义 preview 方向
- 当前 [lib.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/lib.rs) 已经创建了 tray / menu bar icon
- 当前使用 [`menubar.png`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/icons/menubar.png) 作为 template icon
- 当前菜单至少已有 `Open Corivo / Quit Corivo`
- 当前 `minimize_to_tray = true` 时，主窗口关闭会隐藏而不是直接退出

同时，根据 Corivo 的记忆，当前真实问题已经比较明确：

- 主窗口隐藏后，Corivo 并没有真正进入“menu bar only 后台态”
- notification overlay / panel 的存在仍会让系统把它视作一个普通 app 在后台挂着
- 根因不在 tray icon 本身，而在 activation policy 和 Dock icon 可见性的切换规则没有被正式定义和正确接入

另外，当前 overlay 仍是 persistent panel 模型：

- panel 在 app setup 阶段就创建
- 不活动时隐藏
- 后续复用

这本身不是问题。问题在于：**persistent panel 不能自动等于“允许 app 继续以 Regular activation policy 悬在后台”**。

## Problem

如果只做 menu bar preview，而不把 macOS app 的系统身份切对，用户感知会出现三个严重偏差：

1. 用户以为 Corivo 已经“退到后台了”，但系统层面它仍然是一个普通 app  
表现为：

- Dock icon 还在
- Cmd-Tab / App Switcher 里仍把它当 regular app
- 菜单栏里有图标，但 Dock 里也像一个没关干净的 app

2. overlay 把“后台 app”语义破坏掉  
即便主窗口隐藏，只要 overlay 出现，系统又像看到一个普通 app 仍在后台。

3. 后续生命周期设计会继续建立在错误前提上  
如果不先把 `Regular` 和 `Accessory` 的切换语义写死，后面无论是 tray menu、recovering、甚至 daemon-ready 架构，都会继续建立在一个“系统身份含混”的实现上。

所以这次要解决的根问题是：

**Corivo 在 macOS 上什么时候是 foreground app，什么时候是 menu bar only background app，必须由一套正式策略统一决定。**

## Chosen Approach

采用 **dynamic activation policy switching** 方案：

- 当 Corivo 需要作为正常桌面 app 与用户交互时，使用 `Regular`
- 当 Corivo 主窗口隐藏，只保留 menu bar icon 与 overlay 等后台 surface 时，切换到 `Accessory`
- 不使用静态 `LSUIElement = true` 把整个 app 永久做成菜单栏 app

选择这个方案的原因：

- Corivo 既需要一个正常主窗口，也需要真正的 menu bar only 后台态
- 静态菜单栏 app 不适合“有时像正常 app，有时像后台 agent”这种产品形态
- 当前问题本质上就是缺少 `Regular <-> Accessory` 的动态切换，而不是缺少 tray 图标

## Terms

### Activation Policy

本 spec 使用 macOS AppKit 的两个系统身份：

- `Regular`
  - 有 Dock icon
  - 会出现在 App Switcher
  - 适合作为前台主窗口 app

- `Accessory`
  - 无 Dock icon
  - 不应表现为普通前台 app
  - 适合作为 menu bar only 后台态

### System Surfaces

本 spec 把 Corivo 在 macOS 上可见的东西分成两类：

#### Foreground Surfaces

会要求 app 以普通前台 app 语义存在的 surface：

- main window
- 未来需要完整前台交互的 modal / settings window

#### Background Surfaces

允许 app 仍然保持 menu bar only 背景身份的 surface：

- menu bar icon
- tray menu
- notification overlay panel

关键规则：

**overlay panel 属于 background surface，不得单独要求 app 保持 `Regular`。**

## State Mapping

这份 spec 不重新发明生命周期状态，而是把现有 lifecycle 映射到 macOS 系统身份。

### Mapping Rules

- `RunningVisible`
  - activation policy = `Regular`
  - Dock icon visible
  - App Switcher visible

- `RunningHidden`
  - activation policy = `Accessory`
  - Dock icon hidden
  - menu bar icon 保留
  - App Switcher 不再把 Corivo当作普通前台 app

- `Recovering`
  - 如果恢复过程中不显示主窗口，则优先 `Accessory`
  - 如果恢复阶段必须显示主窗口给用户处理，则进入 `Regular`

- `Quitting`
  - 不要求额外切回 `Regular`
  - 直接进入统一退出序列

### Visibility Truth

影响 activation policy 的真实条件只有一条：

**当前是否存在需要作为前台 app 呈现给用户的 foreground surface。**

不是：

- 是否正在 capture
- 是否有 tray icon
- 是否有 overlay panel
- 是否还有 runtime 没退出

## Policy Rules

### 1. Dock Icon Visibility

Dock icon 出现的唯一条件是：

- 主窗口或其他 foreground surface 正在以用户可见的前台界面存在

Dock icon 不应因为以下原因继续存在：

- app 只是还在后台运行
- tray / menu bar icon 还在
- notification overlay 还在显示
- capture loop 还在跑

### 2. Menu Bar Only Mode

当满足以下条件时，Corivo 必须进入 menu bar only 模式：

- main window 已隐藏
- 没有其他 foreground surface
- app 仍然活着

这个模式下要求：

- activation policy = `Accessory`
- Dock icon 消失
- menu bar icon 继续存在
- 仍可通过 menu bar icon 恢复主窗口

### 3. Overlay Must Not Promote The App

当 Corivo 处于 `RunningHidden + Accessory` 时，即便 overlay 显示，也必须满足：

- 不重新显示 Dock icon
- 不把 app 切回 `Regular`
- 不把 Corivo 重新变成一个普通前台 app
- 不因为 panel show/hide/frame update 就把 app 激活

overlay 的 hover、动画、collapse、feedback 提交等流程，只能在 background surface 语义下运行。

如果未来有某个 overlay 交互真的需要完整前台 app 语义，例如复杂输入或多步操作，正确做法是：

- 明确打开主窗口
- coordinator 切回 `Regular`

而不是让 overlay 自己偷偷把整个 app 拉回 regular。

### 4. Open Main Window From Menu Bar

从 menu bar 点击 `Open Corivo` 或左键点击图标恢复主窗口时，必须按这个顺序：

1. activation policy 切到 `Regular`
2. main window `show + unminimize + focus`
3. 成功进入 `RunningVisible`

不允许：

- 在仍然 `Accessory` 的状态下强行 show 一个像前台 app 的主窗口

如果 show / focus 失败，可以回滚为：

- 重新回到 `Accessory`
- 保持 menu bar icon

### 5. Hide Main Window To Menu Bar

当 `minimize_to_tray = true` 且用户关闭主窗口时，必须按这个顺序：

1. hide 主窗口
2. 确认当前没有其他 foreground surface
3. activation policy 切到 `Accessory`
4. 保留 menu bar icon

关键点：

- `HideToTray` 不是“窗口没了但 app 还是 `Regular`”
- `HideToTray` 的语义是“主窗口收起后，app 进入真正的 menu bar only background mode”

### 6. Autostart Hidden Launch

如果未来存在：

- `auto_start = true`
- `launch_hidden_on_autostart = true`

那么 app 启动后应直接进入：

- activation policy = `Accessory`
- 主窗口不显示
- menu bar icon 存在

不能先以 `Regular` 出现一下，再退回去。

### 7. Explicit Quit

无论当前是 `Regular` 还是 `Accessory`，`Quit Corivo` 都直接进入统一退出序列。

退出前不要求额外切回 `Regular`。

原因：

- quit 是生命周期动作，不是表面切换动作
- 先切回 `Regular` 再退出只会造成 Dock icon 一闪而过的错误观感

## Overlay Policy

### Panel Role

overlay panel 在 macOS 上是一个 persistent panel，但它的存在只代表：

- Corivo 有一个后台可复用的 notification surface

它不代表：

- Corivo 必须继续作为 `Regular` app 存在

### Non-Activating Requirement

在 `RunningHidden + Accessory` 时，overlay panel 的 show / hide / frame sync / hover tracking 必须保持 non-promoting。

更具体地说：

- panel 可以显示
- panel 可以响应 hover
- panel 可以更新 frame
- panel 可以处理最小反馈交互

但这些行为都不应触发：

- app activation
- Dock icon re-appearance
- `Accessory -> Regular` 隐式切换

### Focus Escalation Rule

如果某个 overlay 交互超出了“后台 panel 可承载的范围”，它必须显式升级为主窗口流程：

- 由 overlay 发出 `request_open_main_window`
- coordinator 切 `Regular`
- 再把交互转交给主窗口

这条规则是为了防止 panel 一点点长成一个隐形前台 app。

## Implementation Shape

### Recommended Owner

推荐新增一个 macOS 专属的 system surface owner，例如：

- `src-tauri/src/services/macos_system_surface.rs`

它的职责：

- 根据 lifecycle 和 foreground surface 集合，计算目标 activation policy
- 执行 `Regular <-> Accessory` 切换
- 管理 Dock icon 与 menu bar only 的系统层呈现策略
- 明确保证 overlay 不会提升 app 身份

它不负责：

- capture 业务
- memory 业务
- overlay 内容生成

### Core Pure Decision

建议先抽一个纯决策函数，例如：

```rust
enum MacOSSystemSurfaceMode {
    RegularForeground,
    AccessoryBackground,
}
```

输入应至少包含：

- `AppLifecycleState`
- main window 是否可见
- 是否存在其他 foreground surface

输出只回答：

- 目标系统身份应该是 `RegularForeground` 还是 `AccessoryBackground`

这样可以用纯单元测试先把最关键的系统语义锁住。

## File Plan

### New

- `src-tauri/src/services/macos_system_surface.rs`
  - 封装 macOS activation policy / Dock visibility / menu bar only 决策与执行

### Modify

- `src-tauri/src/lib.rs`
  - 不再只靠当前 preview helper 维持 menu bar 行为
  - 接入 macOS system surface owner
  - 在 close-to-tray / open-from-menu-bar / autostart hidden / quit 路径上执行正式 surface policy

- `src-tauri/src/providers/notification/panel.rs`
  - 确保 panel 的 show/hide/focus 行为不会偷偷提升 app activation policy

- `src-tauri/src/providers/notification/overlay.rs`
  - 在 hidden/background 模式下严格遵守“overlay 不提升 app 身份”的规则

- `docs/superpowers/specs/2026-04-15-menubar-preview-design.md`
  - 后续可补一条链接，说明 preview 已升级为正式 macOS surface policy 的一部分

## Testing Strategy

### Pure Tests

为 system surface 决策函数写单元测试：

- `RunningVisible + main window visible` => `RegularForeground`
- `RunningHidden + no foreground surfaces` => `AccessoryBackground`
- `RunningHidden + overlay visible` => `AccessoryBackground`
- `Recovering + hidden launch` => `AccessoryBackground`
- `Recovering + visible recovery window` => `RegularForeground`

### Runtime Behavior Checks

人工验收至少覆盖：

1. 手动启动 Corivo，主窗口可见，Dock icon 可见，menu bar icon 可见
2. 关闭主窗口且 `minimize_to_tray = true`，Dock icon 消失，但 menu bar icon 保留
3. 在隐藏态触发 overlay，Dock icon 不重新出现
4. 从 menu bar 点 `Open Corivo`，Dock icon 恢复，主窗口获得焦点
5. 从 menu bar 点 `Quit Corivo`，Dock icon 和 menu bar icon 一起消失
6. `launch_hidden_on_autostart = true` 时，开机启动后不会先闪一下 Dock icon

### Regression Checks

还要回归确认：

- 当前 menubar preview 的 template icon 效果不回退
- 当前 close-to-tray 行为不回退
- 当前 overlay hover / animation 行为不因 activation policy 切换而变坏

## Success Criteria

完成本 spec 后，Corivo 在 macOS 上应满足：

1. 主窗口可见时，它是一个正常的 `Regular` app
2. 主窗口隐藏后，它会真正变成 `Accessory` 的 menu bar only 后台 app
3. Dock icon 的出现与消失只由 foreground surface 决定
4. overlay panel 在隐藏态下不会把 app 重新拉回 `Regular`
5. 从 menu bar 打开主窗口时，会正确切回 `Regular`
6. 从隐藏态退出时，不会出现 Dock icon 闪回
7. 这套规则建立在当前 menubar preview 之上，而不是推翻重做
