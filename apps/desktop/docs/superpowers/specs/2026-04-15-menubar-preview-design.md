# Menu Bar Preview Design

## Goal

给 Corivo 增加一个最小可预览的 macOS menu bar 图标入口，让当前这张 [`menubar.png`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/icons/menubar.png) 先以 template icon 的方式挂到状态栏，方便直接看顶部栏里的真实观感。

这次只解决“先看到样子并具备最小后台入口”，不把完整 daemon-ready 生命周期一次做完。

## Context

- 当前代码里还没有真正的 tray / menu bar 实现。
- [`spec-09-settings-complete.md`](/Users/airbo/Developer/corivo/corivo-app/docs/superpowers/specs/spec-09-settings-complete.md) 已经定义了 `minimize_to_tray`，但运行时代码尚未兑现。
- [`2026-04-15-daemon-ready-background-lifecycle-design.md`](/Users/airbo/Developer/corivo/corivo-app/docs/superpowers/specs/2026-04-15-daemon-ready-background-lifecycle-design.md) 已经定义了完整后台语义。这次实现只拿其中最小、不会走错方向的一段落地。

## Scope

### In Scope

- 在 app 启动时创建一个 menu bar tray icon
- 使用现有 `src-tauri/icons/menubar.png` 作为图标资源
- 在 macOS 上将 tray icon 标记为 template icon
- 提供两个最小菜单动作：`Open Corivo`、`Quit Corivo`
- 点击 menu bar 图标时恢复主窗口
- 当 `minimize_to_tray = true` 时，关闭主窗口改为隐藏窗口而不是退出

### Out of Scope

- 不做完整后台生命周期协调器
- 不做 capture start/stop、状态文案、动态 badge、二级菜单
- 不做 Windows / Linux 的 tray 细节适配
- 不调整现有 overlay 行为

## Chosen Approach

在 [`src-tauri/src/lib.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/lib.rs) 中直接完成最小 tray 接入：

- `setup` 阶段创建 tray menu 和 tray icon
- 通过 Tauri 2 的 `TrayIconBuilder` 设置图标与事件
- 使用 `icon_as_template(true)` 让 macOS 以单色状态栏图标方式渲染
- 抽出少量纯函数辅助：
  - 主窗口关闭时应该 `HideToTray` 还是 `Shutdown`
  - tray menu item id 到动作的映射

这样可以用单元测试覆盖最关键的产品语义，而 Tauri 运行时接线保留为最小实现。

## UX Behavior

- App 启动后，菜单栏出现 Corivo 图标
- 左键点击图标时，主窗口显示并获得焦点
- 打开 tray 菜单可见：
  - `Open Corivo`
  - `Quit Corivo`
- 主窗口点关闭：
  - 若 `minimize_to_tray = true`，窗口隐藏，tray 仍保留
  - 若 `minimize_to_tray = false`，沿用原有退出路径

## Testing Strategy

- Rust 单元测试覆盖：
  - `minimize_to_tray` 对关闭动作的决策
  - tray menu item id 的解析
- `cargo test` 验证新旧单元测试
- `cargo build` 验证 Tauri tray 接线能编译通过
- 如本机 GUI 环境允许，再手动启动 app 看 menu bar 图标实际效果

## Risks

- 当前 `menubar.png` 是 513x512 彩色资源。template 模式下系统会只取其 alpha 轮廓，实际观感取决于透明边缘和内部留白。
- 如果原图 alpha 结构不适合 menu bar，图标会显得过粗或过糊。这一步先验证真实效果，再决定是否另做专用单色资产。
- 当前 app 还没有统一的“显式退出”协调器；本次 `Quit Corivo` 只能复用现有最小退出路径。
