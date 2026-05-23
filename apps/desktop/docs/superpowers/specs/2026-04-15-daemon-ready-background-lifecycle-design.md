# Daemon-Ready Background Lifecycle Design

## Goal

为 Corivo 补齐“后台运行 / tray / menu bar / 启动恢复 / 统一退出”这一整层产品与架构语义。

这次设计的目标不是单独增加一个系统托盘菜单，而是把 Corivo 明确定义成：

- 一个可以在后台持续运行的单进程桌面 app
- 主窗口、tray / menu bar、notification overlay 都只是这个 app 暴露给用户的 surface
- 当前继续采用单进程 Tauri 架构实现
- 但边界按未来可拆分为 `UI shell + background daemon` 的形状来设计

完成本 spec 后，Corivo 的关闭、隐藏、开机启动、后台捕获、overlay、生存期恢复、显式退出都应有统一语义，不再散落在各模块中各自解释。

这份 spec 不是从零开始定义 tray / menu bar，而是要在已经落地的最小预览版之上继续演进：

- 预览设计见 [2026-04-15-menubar-preview-design.md](/Users/airbo/Developer/corivo/corivo-app/docs/superpowers/specs/2026-04-15-menubar-preview-design.md)
- 预览实现计划见 [2026-04-15-menubar-preview.md](/Users/airbo/Developer/corivo/corivo-app/docs/superpowers/plans/2026-04-15-menubar-preview.md)

也就是说，这次设计的任务不是“重新发明 tray”，而是把这个 preview 纳入正确的大生命周期边界。

## Non-Goals

- 这次不直接引入双进程 / 守护进程 / 本地 IPC
- 不把 capture loop、segment pipeline、memory provider 全量重写成服务总线
- 不做复杂 tray UI、二级菜单、快捷跳转到具体页面
- 不做后台音效、badge、红点、通知中心聚合
- 不做 Windows / Linux 特有托盘怪癖的深度适配；先定义跨平台语义，再允许平台实现细化
- 不改变现有 notification overlay 的视觉语言
- 不把所有前端状态改成事件推送模型；与本 spec 无直接冲突的现有轮询接口可以继续保留

## Current State

当前文档和代码里，后台与关闭语义已经出现，但尚未被统一收束：

- [base.md](/Users/airbo/Developer/corivo/corivo-app/docs/superpowers/specs/base.md) 已把 Corivo 定义为“在用户本地后台运行，定期截图并调用 Gemini 生成活动总结”
- [spec-09-settings-complete.md](/Users/airbo/Developer/corivo/corivo-app/docs/superpowers/specs/spec-09-settings-complete.md) 已引入 `auto_start` 与 `minimize_to_tray`
- [spec-10-onboarding.md](/Users/airbo/Developer/corivo/corivo-app/docs/superpowers/specs/spec-10-onboarding.md) 已向用户承诺“你可以继续让它在后台运行”
- 当前 [lib.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/lib.rs) 在 `setup` 阶段初始化数据库、capture store、config service、notification overlay，并在窗口关闭时只做局部 shutdown
- 当前 [capture_loop.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/capture_loop.rs) 已经拥有 `running / paused_idle / stopped` 这类后台工作语义
- 当前 [panel.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/panel.rs) 与 [overlay.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/providers/notification/overlay.rs) 已经让 overlay panel 成为独立于主窗口的可见 surface
- 最新的 [2026-04-15-menubar-preview-design.md](/Users/airbo/Developer/corivo/corivo-app/docs/superpowers/specs/2026-04-15-menubar-preview-design.md) 与对应实现已经补上一个最小 macOS menu bar 预览：
  - 使用现有 [`menubar.png`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/icons/menubar.png) 作为 template icon
  - 点击菜单栏图标会恢复主窗口
  - 菜单里已有 `Open Corivo / Quit Corivo`
  - `minimize_to_tray = true` 时关闭主窗口会隐藏而不是直接退出
- 这个 preview 当前主要接在 [lib.rs](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/lib.rs) 里，通过 `AppLifecycleControl`、tray setup helper 和 close policy helper 实现，证明方向是对的，但还不是完整生命周期 owner

问题在于：这些语义目前还没有由一个统一 owner 解释。

典型后果：

- “关闭主窗口”到底是隐藏还是退出，不是全局唯一答案
- 主窗口、capture loop、overlay panel、store/db shutdown 没有同一条退出序列
- 旧 session 的收口与异常退出恢复没有被明确纳入生命周期模型
- 当前的 menu bar preview 仍然只是最小可用接线，还没有被定义成完整后台 app 的一等入口

## Problem

如果继续把当前的 tray / menu bar preview 当作一个局部功能点往上堆，会出现两类更深层问题：

1. 产品语义继续分散  
用户会同时面对：

- 关闭窗口
- 最小化到托盘
- 真正退出
- 开机启动但不弹窗
- 后台仍在捕获
- overlay 仍可能继续出现

如果这些动作没有统一状态机，任何一个新功能都会继续把行为写散。

2. 架构边界会越来越难拆 daemon  
如果主窗口、tray、overlay、capture、store/db flush 彼此直接调用，将来从单进程升级为双进程时，迁移成本会接近重写。

所以，这次要解决的根问题不是“没有 tray 菜单”，而是“当前 preview 还没有被吸纳进 daemon-ready 的生命周期边界”。

## Chosen Approach

采用 **daemon-ready single-process** 方案：

- 当前版本继续保持单进程 Tauri app
- 新增一个统一的 `LifecycleCoordinator`
- 所有 shell surface 只能向 coordinator 发控制请求
- 所有后台工作状态只能由 runtime owner 修改
- 进程内先按“未来会换成 IPC”的接口形状组织

并明确把这件事拆成增量演进，而不是推翻现有 preview：

- **Phase 0（已完成）**：最小 macOS menu bar preview，验证 template icon、`Open Corivo / Quit Corivo`、close-to-tray 方向
- **Phase 1（本 spec 的近期实现）**：把 preview 中写在 `lib.rs` 的 tray / close / quit 接线收束到统一 lifecycle 边界
- **Phase 2（本 spec 的完整目标）**：补齐 `Recovering`、统一 shutdown budget、overlay/runtime 协调与 daemon-ready 接口
- **Phase 3（未来可选）**：在不改变产品语义的前提下，把 runtime 从进程内调用替换成独立 daemon + IPC

这意味着：

- 现在：`Coordinator -> Runtime` 是进程内调用
- 未来：`Coordinator -> RuntimeDaemon` 可以替换为 IPC

变的是 transport，不是产品语义。

### Why Not Dual Process Now

双进程 / 守护进程并非错误方向，但当前阶段成本明显过高：

- 需要 IPC 协议
- 需要 daemon 存活检测和重连
- 需要前后台版本兼容
- 需要升级替换两个可执行体
- 需要更复杂的 crash / permission / logging 策略

当前 Corivo 还没有强到必须为这些复杂度付费的程度；但它已经需要把边界先切对。

## Architecture

### Layer 1: Shell Layer

Shell 负责所有用户可见、可操作的 surface：

- main window
- tray / menu bar
- notification overlay
- 开机启动后的首屏可见性策略

Shell 只负责发请求，不拥有后台真状态。

它可以表达：

- `show_main_window`
- `hide_main_window`
- `request_start_capture`
- `request_stop_capture`
- `request_quit`

它不允许：

- 直接结束 session
- 直接停止 worker
- 直接写 store / db 退出标记
- 直接决定 overlay 的底层生命周期

### Layer 2: Runtime Layer

Runtime 负责所有后台工作能力：

- capture loop
- idle pause / resume
- session 生命周期
- pipeline 恢复与可重试状态
- store / db flush
- 崩溃恢复与 stale session 收口

Runtime 不关心按钮、窗口层级或菜单结构。

### Layer 3: Lifecycle Coordinator

Coordinator 是这次设计的核心。

它的职责是：

- 接收 shell 发来的控制请求
- 驱动 app lifecycle 状态转换
- 驱动 runtime work 状态转换
- 把 runtime 状态变化广播回 shell
- 在启动恢复、显式退出、异常退出这三类复杂流程里担任唯一 owner

这个 owner 必须是未来 daemon 化后仍然成立的语义中心。

### Relation To The Existing Preview

当前 preview 里的 `AppLifecycleControl` 是一个正确但临时的前驱物。

它已经证明了三件事：

- tray / menu bar 的最小入口可以先落地
- `minimize_to_tray` 的运行时语义值得保留
- `Quit Corivo` 需要走统一退出路径，而不是让每个 surface 自己直接结束进程

但它还不应该成为最终 architecture 的长期 owner。  
本 spec 的目标是把这部分从“写在 `lib.rs` 的预览接线”提升为“正式的 lifecycle 边界”。

## State Model

本 spec 采用两层状态机。

### AppLifecycleState

只回答“Corivo 这个 app 现在以什么形态活着”：

- `Booting`
- `Recovering`
- `RunningVisible`
- `RunningHidden`
- `Quitting`
- `Terminated`

语义：

- `Booting`：进程已启动，正在初始化 config/store/db/runtime/surfaces
- `Recovering`：正在收口上次异常退出留下的状态
- `RunningVisible`：主窗口可见，app 正常运行
- `RunningHidden`：主窗口隐藏，但 app 正常运行，tray / menu bar 可用
- `Quitting`：已进入统一退出流程，不再接受新的控制请求
- `Terminated`：进程退出终态，只在逻辑上存在

### RuntimeWorkState

只回答“后台工作现在在干嘛”：

- `Stopped`
- `Running`
- `PausedIdle`
- `Recovering`

语义：

- `Stopped`：当前没有活跃 capture
- `Running`：当前正在捕获 / 处理中
- `PausedIdle`：capture 因 idle policy 暂停，但 runtime 仍在
- `Recovering`：正在恢复上次未完成的后台任务与 session

### Why Two Layers

以下组合都是真实而必要的：

- `RunningVisible + Stopped`
- `RunningVisible + Running`
- `RunningHidden + Running`
- `RunningHidden + PausedIdle`
- `Quitting + Running`

如果只用一层状态机，这些语义会被压成含糊的“正在运行”，导致 tray、overlay、退出、恢复无法各自定义边界。

## Event Model

所有控制都必须通过 coordinator 暴露的命令入口，而不是任意模块直接互相调用。

### Control Requests

推荐定义统一控制请求：

- `show_main_window`
- `hide_main_window`
- `request_start_capture`
- `request_stop_capture`
- `request_quit`
- `request_open_from_system_surface`
- `request_toggle_main_window`
- `resolve_launch_policy`

这些名字是 daemon-ready 的。
今天是 Rust 进程内调用，未来可以直接映射成 IPC message。

### State Events

推荐统一的只读事件：

- `app-lifecycle-changed`
- `runtime-work-state-changed`
- `capture-status-changed`
- `shutdown-started`
- `shutdown-finished`
- `runtime-recovery-started`
- `runtime-recovery-finished`

前端、tray/menu bar、overlay 只消费这些状态，不自己推导后台真相。

## Lifecycle Rules

### Manual Launch

用户主动启动 Corivo：

`Booting -> Recovering -> RunningVisible`

例外：

- 如果显式传入“以隐藏方式打开”参数，可进入 `RunningHidden`

### Autostart Launch

开机自启默认：

`Booting -> Recovering -> RunningHidden`

这里不主动弹主窗口。

`auto_start`、`launch_hidden_on_autostart`、`start_capture_on_launch` 必须拆成三条规则，不允许揉成一句“开机后台启动”。

### Main Window Close

关闭主窗口不是退出 app。

当 `minimize_to_tray = true` 时：

- `RunningVisible -> RunningHidden`
- 不退出进程
- 不停止 capture
- 不销毁 tray/menu bar
- 不销毁 runtime

当 `minimize_to_tray = false` 时：

- 不允许直接跳过 coordinator 关闭进程
- 必须转成 `request_quit`

### Open From System Surface

当用户从 tray / menu bar 点 `Open Corivo`：

- `RunningHidden -> RunningVisible`
- 只显示并聚焦主窗口
- 不重新初始化 runtime
- 不新建第二个进程

### Explicit Quit

当用户从 tray / menu bar 点 `Quit Corivo`：

- `RunningVisible|RunningHidden -> Quitting`
- 只有这个入口才表示真正退出 app

## System Surface Semantics

### Tray / Menu Bar As First-Class Surface

只要 Corivo 进程还活着，`SystemSurface` 就必须存在：

- macOS：菜单栏图标
- Windows / Linux：系统托盘图标

它代表的是“Corivo 这个后台 app 仍在运行”，不是“主窗口的补充按钮”。

### First-Version Menu Scope

第一版菜单保持克制，只放：

- `Open Corivo`
- `Hide Corivo`
- `Capture Status: Running / Paused / Stopped`
- `Start Capture`
- `Stop Capture`
- `Quit Corivo`

不放：

- 主题切换
- 语言切换
- 数据清理
- 深层页面跳转
- 高级通知设置

这些都留在主窗口设置页。

### Main Window And System Surface Relation

- 主窗口只是控制台
- tray / menu bar 是后台入口
- `Hide` 与 `Quit` 是两个完全不同的动作
- 菜单文案必须明确区分，不允许用户靠猜

## Overlay Semantics

overlay 不依赖主窗口是否可见，只依赖：

- app 当前不在 `Quitting`
- 通知策略允许显示
- runtime / decision 层确实产生了可显示通知

因此，以下场景都应该成立：

- `RunningVisible + Running`：允许显示 overlay
- `RunningHidden + Running`：也允许显示 overlay
- `RunningHidden + PausedIdle`：原则上不主动发起新的 overlay
- `Quitting`：overlay 必须立即收起并停止后续 timer

这条规则确保 overlay 是 app 的 surface，而不是主窗口的附属品。

## Recovery Model

### Recovering Is A Formal State

每次启动都必须先经过 `Recovering`，而不是把恢复逻辑散落在各 service 初始化里。

`Recovering` 需要完成：

1. 检查上次 shutdown 是否完整
2. 收口 stale active session
3. 收口未完成的后台任务
4. 强制所有 surface 回到干净初始态

### Stale Active Session Rule

任何上一进程遗留的 `active` session，新的 runtime 都不能继承并继续使用。

必须满足：

- 旧 `active` session 不能继续保持 `active`
- 新 runtime 只能创建新 session
- 旧 session 必须被显式结束为异常结束态

推荐持久化：

- `ended_reason = user_stop | user_quit | unexpected_exit | system_shutdown`
- `recovered_at`
- `recovered_by_run_id`

### In-Flight Work Recovery

如果 segment / summary / memory write 在上一次退出时半途而废：

- 不能永久卡在 `processing`
- 必须被回退成可重试状态
- 恢复后由 runtime 重新调度

恢复过程里，主窗口可以延迟显示，但 tray / menu bar 不应缺失。

### Clean Surface Start

恢复完成前：

- overlay 一律 hidden
- 主窗口是否显示由启动策略决定
- tray / menu bar 可以先挂出，但不暴露误导性状态

## Shutdown Model

### request_quit Is The Only Exit Path

无论退出是从哪里触发：

- tray / menu bar
- 主窗口菜单
- 快捷键
- 系统退出事件

都必须统一转成 `request_quit`。

禁止：

- 某个模块自己直接 `app.exit`
- 主窗口关闭事件绕过 coordinator 直接销毁进程

### Ordered Graceful Shutdown

推荐退出顺序：

1. coordinator 接收 `request_quit`
2. `AppLifecycleState -> Quitting`
3. 拒绝新的 show/hide/start/stop/overlay 请求
4. 主窗口隐藏或失活
5. overlay 收起并停止 timer
6. runtime 停止接收新截图
7. 当前 session 落结束态
8. 将未完成远端任务落成“下次可恢复”
9. flush store / db / checkpoint
10. 退出进程

### Bounded Shutdown Budget

退出必须是有界的 graceful shutdown。

推荐定义总 budget：

- `5s` 到 `8s`

策略：

- 先尽力优雅退出
- 超时则写入“需要恢复”的持久化标记
- 然后终止进程

### Remote Work Must Not Block Exit Forever

正在进行中的远端工作，例如：

- Gemini 请求
- Supermemory 写入

不能无限阻塞 `Quit Corivo`。

正确策略是：

- 退出时停止接受新工作
- 允许短时间收尾
- 超时则把当前任务落成“下次恢复”

这条是 daemon-ready 设计的关键边界。

## Config Model

这次设计建议把启动 / 隐藏 / 自动工作语义拆成三条配置：

- `auto_start`
- `launch_hidden_on_autostart`
- `start_capture_on_launch`

它们的职责分别是：

- `auto_start`：系统登录后是否启动 Corivo 进程
- `launch_hidden_on_autostart`：开机自启时是否默认隐藏主窗口
- `start_capture_on_launch`：app 启动完成后是否自动开始捕获

如果当前配置结构里还没有 `launch_hidden_on_autostart`，本 spec 建议补齐，而不是继续复用 `minimize_to_tray` 混合表达两种不同语义。

## Incremental Rollout From Preview

为了避免“已经落地的 preview 变成 throwaway code”，实现顺序应明确建立在当前 menubar preview 基线上：

### Step 1: Preserve The Preview Contract

先把当前 preview 已经兑现的行为视为不可回退的基线：

- template icon 仍可正常显示
- 点击图标仍能恢复主窗口
- `Open Corivo / Quit Corivo` 仍然存在
- `minimize_to_tray` 仍然兑现为 hide 而不是直接退出

### Step 2: Move Ownership, Not Product Behavior

下一阶段的重点不是改用户行为，而是迁移 ownership：

- 现在：preview helper 主要在 `lib.rs`
- 目标：由 `LifecycleCoordinator` 成为统一 owner

也就是说：

- 对用户看见的行为尽量不改
- 对内部调用边界进行重组

### Step 3: Fill In Missing Lifecycle Semantics

在 preview 的基础上补齐当前仍缺的正式语义：

- `Recovering`
- stale session 收口
- shutdown budget
- in-flight remote work recovery
- overlay 与 `Quitting` 的统一收尾

### Step 4: Expand System Surface Only After Ownership Is Stable

只有当 ownership 稳定以后，再考虑扩 tray / menu bar 的控制面，例如：

- capture status 文案
- `Start Capture / Stop Capture`
- 更丰富的后台入口

不建议反过来先把菜单堆大，再回头补生命周期。

## File Plan

### New

- `src-tauri/src/services/lifecycle_coordinator.rs`

职责：

- 维护 `AppLifecycleState`
- 接收 shell 控制请求
- 驱动 runtime / shell 状态转换
- 统一启动恢复与退出顺序

### Modify

- `src-tauri/src/lib.rs`
  - setup 阶段接入 coordinator
  - 把当前 preview 中的 `AppLifecycleControl`、tray setup、close policy、quit helper 收束到统一 owner
  - 把窗口关闭、开机启动、overlay 初始化、退出序列接入统一 owner，而不是继续在 `lib.rs` 上堆更多分支

- `src-tauri/src/services/capture_loop.rs`
  - 与 coordinator 的 runtime 控制接口对齐
  - 暴露 daemon-ready 的 start/stop/status API

- `src-tauri/src/services/capture_store.rs`
  - 提供 stale session recovery 所需的收口操作

- `src-tauri/src/db/repos/sessions.rs`
  - 支持 `ended_reason` / recovery 元数据（如决定落库）

- `src-tauri/src/providers/notification/overlay.rs`
  - 在 `Quitting` 时统一收起 overlay
  - 接入 lifecycle policy，而不是仅按通知逻辑独立存活

- `src-tauri/src/providers/notification/panel.rs`
  - 与 shell lifecycle 协调

- `src-tauri/src/domain/config.rs`
  - 补 `launch_hidden_on_autostart` 等配置

- `src-tauri/src/commands/settings.rs`
  - 补 lifecycle 相关配置读写接口

- `src/lib/types.ts`
  - 增加 lifecycle / runtime 状态类型

- `src/lib/tauri.ts`
  - 增加 lifecycle 控制与查询接口

### Optional Follow-Up

- `src-tauri/src/commands/lifecycle.rs`
  - 如果控制面增长较多，可单独拆命令模块

- `src-tauri/src/services/system_surface.rs`
  - 如果 tray / menu bar / main-window shell 行为继续增长，可把当前 preview 的 system surface 接线从 `lib.rs` 独立出来

## Testing Strategy

### State Machine Tests

为 coordinator 写纯 Rust 单元测试，覆盖：

- `RunningVisible -> close -> RunningHidden`
- `RunningHidden -> open -> RunningVisible`
- `request_quit -> Quitting`
- `Quitting` 期间拒绝新请求
- autostart hidden policy

### Recovery Tests

覆盖：

- stale active session 被收口
- processing work 被回退成可重试状态
- 恢复完成后 overlay 不残留

### Shutdown Tests

覆盖：

- 退出顺序正确
- shutdown budget 生效
- 远端任务超时不会永久阻塞退出

### Integration Checks

人工验收至少覆盖：

1. 手动启动 app，窗口可见，tray / menu bar 可用
2. 当前 menubar preview 的 template icon 行为不回退
3. 关闭主窗口后 app 仍在，tray / menu bar 还在
4. 从 tray / menu bar 点 `Open Corivo` 能恢复主窗口
5. `RunningHidden + Running` 时 overlay 仍可显示
6. 从 tray / menu bar 点 `Quit Corivo` 后 capture 真正结束，app 真正退出
7. 模拟异常退出后，下一次启动能自动收口旧 session
8. autostart + hidden + start_capture_on_launch 三个开关组合行为正确

## Migration Notes

这次 spec 的重点不是立刻引入 daemon，而是把当前单进程实现约束成未来可拆 daemon 的边界。

未来如果 Corivo 升级到双进程：

- `LifecycleCoordinator` 可以继续保留在 shell 侧
- `Runtime Layer` 可以迁移成独立 daemon
- 当前控制请求和状态事件可以原样映射为 IPC 协议

理想迁移路径应是：

- 保留现有状态机与语义
- 替换调用方式
- 不重写产品规则

## Success Criteria

完成本 spec 后，Corivo 应满足：

1. “关闭主窗口”和“退出应用”是两个明确区分的动作
2. 当前 menubar preview 已兑现的最小行为被保留，并成为正式生命周期实现的基线
3. 主窗口隐藏后，app 仍可通过 tray / menu bar 作为后台入口继续工作
4. overlay 不依赖主窗口可见性
5. 所有退出都通过统一 `request_quit` 进入
6. 启动时先经过 `Recovering`，不会遗留脏 active session
7. runtime、shell、coordinator 边界清晰，未来可替换为 IPC
8. 这套模型能与现有单进程 Tauri 实现兼容落地，而不要求一次性升级为守护进程
