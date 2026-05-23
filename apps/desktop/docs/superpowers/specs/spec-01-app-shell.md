# spec-01-app-shell.md

## 一、目标

搭建 Corivo 的 app-shell。完成本 spec 后，仓库里应该有一个能正常 `dev / build` 的 Tauri 桌面应用骨架，作为后续所有模块 spec 的承载层，包含：
- Tauri 2 + React 18 + TypeScript + Vite 的基础工程
- 与 [base.md](/Users/airbo/Developer/corivo/corivo-app/docs/superpowers/specs/base.md) 一致的前后端目录结构
- 左导航 + 右内容区的 AppLayout
- TanStack Router 的 code-based 路由树，接好 4 个主页面和 1 个连接详情页占位路由
- TanStack Query、Zustand、shadcn/ui、暖化主题 token 的初始化
- 一个统一的 Tauri command 封装入口，以及一个仅用于联调的 demo command

## 二、不做什么

- 不实现截图采集、分段、总结、记忆查询等任何业务逻辑
- 不接入 SQLite、migrations、repo 层、日志落盘
- 不接入 tauri-plugin-store、keychain、真实配置结构
- 不实现 MemoryProvider、LlmProvider、ConnectorRegistry
- 不实现任何真实页面 UI，只做占位骨架
- 不做系统托盘、开机自启、通知、恢复捕获等启动流程能力
- 不做暗色模式验收；暗色 token 可以保留，但 P0 验收只看浅色

## 三、成功标准

完成本 spec 后，运行 `pnpm tauri dev` 应该满足：
1. Tauri 窗口正常打开，标题为 `Corivo`，尺寸符合约定。
2. 左侧为固定宽度 180px 的导航栏，包含 Logo、4 个导航项和底部状态占位。
3. 点击「概览 / 记忆 / 连接 / 配置」时，URL 与右侧内容区同步切换。
4. 当前激活导航项有暖色 accent 高亮，hover 态和默认态都清晰。
5. 全局视觉使用 base.md 锁定的暖化 token：奶油米白背景、暖棕文字、暖浅边框、无阴影。
6. 前端通过统一封装成功调用一个测试 Tauri command（`greet`）并收到返回值。
7. TanStack Router Devtools 与 React Query Devtools 在开发环境可见。
8. `pnpm tauri build` 可以成功产出可分发应用。

## 四、与 base.md 对齐的锁定约束

### 4.1 目标边界

- 本 spec 只负责顶层骨架，不提前实现后续 spec 的业务。
- 目录命名、模块分层、路由结构必须直接继承 base.md，不在本 spec 内重新发明结构。

### 4.2 前端结构约束

- `src/app` 只放布局和路由装配。
- `src/routes` 只放 TanStack Router 路由声明。
- `src/pages` 只放页面级 UI。
- `src/components/layout` 放 Sidebar、NavItem、StatusIndicator 等骨架组件。
- `src/lib/tauri.ts` 是唯一允许直接 `invoke` 的地方。

### 4.3 路由约束

- 使用 TanStack Router 的 code-based 路由，不用 file-based 插件方案。
- 需要 URL 同步的状态必须通过 `validateSearch + zod` 定义。
- P0 阶段只给 `/` 和 `/memory` 配 search params。

### 4.4 视觉约束

- 使用 shadcn stone 风格作为基底，但必须覆写为 base.md 锁定的暖化 token。
- 不使用阴影表达层次，统一用边框、底色和间距。
- 圆角、字号、间距按 base.md 中的 token 走，不各页自定义一套。

### 4.5 联调约束

- `greet` 仅用于验证前后端通信是否通，不属于正式业务接口。
- 后续真实 command 在对应 spec 中补入；本 spec 只搭桥，不扩展业务命令面。

## 五、目录结构（完成态）

```
corivo/
├── src/
│   ├── main.tsx
│   ├── app/
│   │   ├── layout.tsx
│   │   └── router.tsx
│   ├── routes/
│   │   ├── __root.tsx
│   │   ├── overview.tsx
│   │   ├── memory.tsx
│   │   ├── connections.index.tsx
│   │   ├── connections.screenshot.tsx
│   │   └── settings.tsx
│   ├── pages/
│   │   ├── overview/overview-page.tsx
│   │   ├── memory/memory-page.tsx
│   │   ├── connections/connections-page.tsx
│   │   ├── connections/screenshot-detail-page.tsx
│   │   └── settings/settings-page.tsx
│   ├── components/
│   │   ├── ui/
│   │   └── layout/
│   │       ├── sidebar.tsx
│   │       ├── nav-item.tsx
│   │       └── status-indicator.tsx
│   ├── lib/
│   │   ├── tauri.ts
│   │   ├── types.ts
│   │   ├── utils.ts
│   │   └── query-client.ts
│   ├── stores/
│   │   └── ui-store.ts
│   ├── hooks/
│   └── styles/
│       └── globals.css
├── src-tauri/
│   ├── src/
│   │   ├── main.rs
│   │   ├── lib.rs
│   │   └── commands/
│   │       ├── mod.rs
│   │       └── demo.rs
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   └── capabilities/
│       └── default.json
├── public/
├── index.html
├── package.json
├── pnpm-lock.yaml
├── tsconfig.json
├── vite.config.ts
├── tailwind.config.js
├── postcss.config.js
├── components.json
└── README.md
```

## 六、技术方案

### 6.1 创建基础工程

使用 `pnpm create tauri-app` 创建 React + TypeScript + pnpm 的 Tauri 2 项目。先确认默认欢迎页可以启动，再开始清理模板。

### 6.2 清理默认模板

- 删除 Tauri 默认欢迎页 UI、无关 demo 资源和样式文件。
- 前端入口切换到 `src/main.tsx + RouterProvider`。
- Rust 侧保留一个最小 demo command，但挪到 `src-tauri/src/commands/demo.rs`，不要继续把 demo 逻辑塞在 `lib.rs` 顶层。

### 6.3 安装依赖

运行时依赖：
- `@tanstack/react-router`
- `@tanstack/react-query`
- `zustand`
- `zod`
- `lucide-react`
- `date-fns`
- `clsx`
- `tailwind-merge`
- `sonner`

开发依赖：
- `tailwindcss`
- `postcss`
- `autoprefixer`
- `@types/node`
- `@tanstack/router-devtools`
- `@tanstack/react-query-devtools`

### 6.4 Tailwind 与 shadcn

- 初始化 Tailwind。
- 初始化 shadcn/ui，风格选择 `New York`，base color 选择 `Stone`，启用 CSS variables。
- `components.json` 与 alias 必须对齐 `@/* -> ./src/*`。
- shadcn 会生成默认变量；生成后必须用 base.md 的暖化 token 覆盖，不保留默认冷灰体系。

### 6.5 暖化主题 token

`src/styles/globals.css` 里使用 base.md 锁定的 OKLCH token：
```css
@tailwind base;
@tailwind components;
@tailwind utilities;

@layer base {
  :root {
    --background: oklch(0.985 0.008 85);
    --card: oklch(1 0 0);
    --popover: oklch(1 0 0);
    --muted: oklch(0.96 0.01 85);
    --foreground: oklch(0.25 0.02 60);
    --muted-foreground: oklch(0.50 0.015 65);
    --card-foreground: oklch(0.25 0.02 60);
    --popover-foreground: oklch(0.25 0.02 60);
    --border: oklch(0.92 0.012 80);
    --input: oklch(0.92 0.012 80);
    --primary: oklch(0.55 0.08 55);
    --primary-foreground: oklch(0.98 0.005 85);
    --accent: oklch(0.93 0.025 75);
    --accent-foreground: oklch(0.30 0.04 55);
    --destructive: oklch(0.55 0.18 25);
    --destructive-foreground: oklch(0.98 0.005 85);
    --radius: 0.75rem;
  }

  .dark {
    --background: oklch(0.18 0.015 60);
    --card: oklch(0.22 0.018 60);
    --popover: oklch(0.22 0.018 60);
    --muted: oklch(0.25 0.015 60);
    --foreground: oklch(0.95 0.01 85);
    --muted-foreground: oklch(0.72 0.012 75);
    --border: oklch(0.30 0.015 60);
    --primary: oklch(0.75 0.06 65);
    --primary-foreground: oklch(0.20 0.02 60);
    --accent: oklch(0.30 0.02 60);
    --accent-foreground: oklch(0.92 0.02 75);
  }

  * {
    @apply border-border;
  }

  body {
    @apply bg-background text-foreground;
    font-family: -apple-system, BlinkMacSystemFont, "PingFang SC",
      "Hiragino Sans GB", "Microsoft YaHei", sans-serif;
    font-feature-settings: "rlig" 1, "calt" 1;
  }
}
```

额外视觉约束：
- 页面 padding 默认 `p-6`
- 卡片 padding 默认 `p-4` 或 `p-5`
- 大卡片圆角统一 `rounded-xl`
- 不使用阴影类名

### 6.6 TypeScript alias

在 `tsconfig.json` 和 `vite.config.ts` 中统一配置 `@ -> ./src`，避免相对路径层层回退。

### 6.7 路由树

路由清单固定为：
- `/`
- `/memory`
- `/connections`
- `/connections/screenshot`
- `/settings`

路由约定：
- `__root.tsx` 挂 `AppLayout`、`Outlet` 和 Router Devtools。
- `/` 的 search schema 为 `{ date?: string }`
- `/memory` 的 search schema 为 `{ q?: string, page: number }`
- 其他页面 P0 不带 search params。

示例：
```ts
const memorySearchSchema = z.object({
  q: z.string().optional(),
  page: z.number().int().min(1).default(1),
})
```

### 6.8 AppLayout 与 Sidebar

布局结构：
- Sidebar 固定宽度 `180px`
- 内容区自适应，主容器 `max-w-5xl`
- 顶层容器 `h-screen`

Sidebar 内容：
- 顶部品牌区：仅显示 `Corivo`
- 中部导航：概览 / 记忆 / 连接 / 配置
- 底部状态区：仅占位，文案例如 `未启动`

NavItem 行为：
- 默认态为 `text-muted-foreground`
- hover 态使用 `bg-accent/60`
- active 态使用 `bg-accent text-accent-foreground font-medium`
- 首页路由必须启用 `exact` 匹配，避免 `/memory` 时首页也高亮

### 6.9 页面占位符

每个页面只需要：
- 页面标题
- 一行说明当前页面用途或未来由哪个 spec 完成
- 一个 `rounded-xl border bg-card` 的占位卡片

文案建议：
- 概览页：`时间线 · 待 spec-08 实现`
- 记忆页：`记忆列表与搜索 · 待 spec-09 实现`
- 连接页：`连接器管理 · 待 spec-10 实现`
- 配置页：`配置与密钥管理 · 待 spec-11 实现`
- 截图连接详情页：`截图连接器详情 · 待 spec-10 实现`

### 6.10 React Query 与 Zustand

- 建一个共享 `QueryClient` 实例，默认 `staleTime = 30s`，`gcTime = 5min`，`retry = 1`，`refetchOnWindowFocus = false`
- `src/stores/ui-store.ts` 先只放最小占位 store，供后续 spec 追加 UI 状态
- 本 spec 不引入任何业务 store

### 6.11 Tauri command 封装

`src/lib/tauri.ts` 中统一封装：
```ts
import { invoke } from '@tauri-apps/api/core'

export async function greet(name: string): Promise<string> {
  return invoke<string>('greet', { name })
}
```

规则：
- 组件里禁止直接 `invoke`
- 未来真实 command 统一在此文件扩展

### 6.12 Rust 侧最小 command 组织

文件拆分：
- `src-tauri/src/commands/mod.rs`
- `src-tauri/src/commands/demo.rs`

建议形式：
```rust
// commands/demo.rs
#[tauri::command]
pub fn greet(name: &str) -> String {
    format!("Hello, {}! Corivo is running.", name)
}
```

```rust
// commands/mod.rs
pub mod demo;
```

```rust
// lib.rs
mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![commands::demo::greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### 6.13 开发环境联调

为了验证桥接是否正常，在概览页挂一次开发期调用即可：
- `useEffect` 调 `greet('Corivo')`
- 把结果打印到 console 或显示在一个小段落里

这段联调逻辑必须标注为临时验证代码，进入真实业务开发时删除。

### 6.14 窗口配置

`src-tauri/tauri.conf.json` 中主窗口参数：
```json
{
  "title": "Corivo",
  "width": 1100,
  "height": 720,
  "minWidth": 900,
  "minHeight": 600,
  "resizable": true,
  "fullscreen": false
}
```

## 七、实施步骤

步骤 1：创建并验证默认 Tauri 工程
- 跑 `pnpm create tauri-app`
- 选择 React + TypeScript + pnpm
- 跑 `pnpm install`
- 跑 `pnpm tauri dev`，确认默认欢迎页能起

步骤 2：清理模板并创建目标目录
- 删除默认演示 UI、样式、素材
- 创建 `src/app`、`src/routes`、`src/pages`、`src/components/layout`、`src/lib`、`src/stores`、`src/styles`
- 创建 Rust 侧 `commands/` 目录

步骤 3：安装和配置 Tailwind、shadcn、alias
- 初始化 Tailwind
- 初始化 shadcn
- 配好 `@` alias
- 把暖化 token 写回 `globals.css`

步骤 4：装配 Router、Query、store 和入口
- 写 `QueryClient`
- 写 route tree
- 在 `main.tsx` 挂 `QueryClientProvider`、`RouterProvider`、Devtools
- 建 `ui-store.ts`

步骤 5：实现 AppLayout、Sidebar、NavItem、StatusIndicator
- 先搭框架，再做 active/hover 状态
- 确保首页 `exact` 匹配正确

步骤 6：补齐页面占位与 demo command
- 建 5 个页面占位文件
- 建 `src/lib/tauri.ts`
- 建 Rust `commands/demo.rs`
- 在概览页做一次 `greet` 联调

步骤 7：调整窗口配置并完成验收
- 调整 `tauri.conf.json`
- 跑 `pnpm tauri dev`
- 跑 `pnpm tauri build`
- 对照验收清单逐项确认

## 八、任务分解（给 Claude Code 的执行批次）

建议拆成 5 个独立会话：

会话 1：脚手架与目录
- 完成步骤 1-2
- 目标是得到一个干净的 Tauri 工程和目标目录结构

会话 2：样式体系
- 完成步骤 3
- 目标是 Tailwind、shadcn、alias、暖化 token 一次配齐

会话 3：路由与入口
- 完成步骤 4
- 目标是 Router、Query、入口装配完成

会话 4：布局与页面占位
- 完成步骤 5-6 的前半部分
- 目标是可以在 4 个主页面和 1 个详情页之间切换

会话 5：Tauri 联调与验收
- 完成步骤 6 的后半部分和步骤 7
- 目标是前后端桥接打通并通过 build 验收

## 九、验收清单

- `pnpm tauri dev` 启动无报错
- 窗口标题是 `Corivo`
- 初始尺寸为 `1100 x 720`
- 左侧 sidebar 宽度为 `180px`
- 4 个导航项文本和图标正确
- 导航点击后 URL 和内容区同步切换
- 激活项有暖色高亮
- 背景是奶油米白，不是冷灰或纯白
- 文字是暖棕，不是纯黑
- 边框是暖浅色细线
- 页面与卡片圆角统一
- 页面中没有阴影
- Router Devtools 在开发环境可见
- React Query Devtools 在开发环境可见
- `greet` 联调成功
- `pnpm tauri build` 成功

## 十、坑点预警

1. shadcn 初始化后会覆盖 CSS 变量，暖化 token 要在最后再写回去。
2. TanStack Router 必须注册 `declare module '@tanstack/react-router'`，否则类型系统不完整。
3. 不要误用 file-based router 教程；本项目锁定 code-based 路由。
4. Tauri 2 的 `invoke` 来自 `@tauri-apps/api/core`，不是旧路径。
5. 首页 Link 必须 `exact`，否则路由高亮会串。
6. 不要把正式业务 command 提前塞进 spec-01；这里只保留 demo command。
7. 不要为了“好看”加阴影，这和 base.md 的视觉原则冲突。
8. `pnpm tauri build` 在不同系统产物格式不同，macOS 可能是 `.app` 或 `.dmg`，验收看 build 成功，不写死产物后缀。

## 十一、产出物

完成本 spec 后，应得到：
- 一个能 `dev / build` 的 Tauri app-shell
- 与 base.md 一致的前后端目录骨架
- 一套已经落地的暖化主题 token
- 一个已验证的前后端 command 调用链路
- 一份可以继续承接 spec-02 及后续模块的稳定脚手架

## 十二、下一份 spec

下一份是 `spec-02-config-and-keychain.md`，负责实现配置存储、密钥保存与测试连接，因为后续 MemoryProvider 和 LlmProvider 初始化都依赖它。
