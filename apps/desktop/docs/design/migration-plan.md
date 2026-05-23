# Step 3 · Redesign Migration Plan

> 把 [system-v0](system-v0.html) 的 token、[screens-v0](screens-v0.html) 的形态,从 HTML mockup 落进真实代码。

## 现状(recon 摘要)

代码侧比预想的干净:

- **Tailwind v4 + `@theme` in CSS** —— [packages/ui/src/styles/globals.css](../../../packages/ui/src/styles/globals.css)。
- **shadcn 基础组件全走语义 token**(`bg-primary` / `text-foreground` / `border-input` 等),没有写死的色值。这意味着**改值不用改组件**。
- **Token 名跟 shadcn 对齐**:`--background` / `--foreground` / `--primary` / `--accent` / `--muted` / `--border` / `--ring` 等。
- **暗色模式未实现** —— `apps/desktop/src/hooks/use-theme-sync.ts` 是空占位,`Config.app.theme` 是 no-op。是干净起点,无包袱。
- **Quick Ask** 有独立 token 系统([quick-ask.css](../../src/styles/quick-ask.css)),靠 macOS `NSVisualEffectView` 做原生 blur,不是 CSS `backdrop-filter`。这是对的,保留。
- 当前主色 `--primary: #f0885e`(coral)即用户判断"太像 Claude"的那个色,需要替换为 `#A86B22`(印染琥珀)。
- 字体:Inter / Lora / JetBrains Mono / Caveat 都通过 `@fontsource-variable` 加载。Lora、Caveat 现在用不上,且违背"去 Serif"方向。
- 有一处需要清理:[island.css](../../src/styles/overlay/island.css) 用了硬编码十六进制,不走 token 系统。

## 策略 · 同名换值

不重命名 token,**只改值**。这样:
- shadcn 组件零改动
- 现有 `bg-primary` / `text-foreground` 等 utility 自动获得新视觉
- 暗色靠 `[data-theme="dark"]` 选择器叠加,不动现有结构

## Token 映射表

shadcn 名 → 新值。**Light 是默认(`:root`),Dark 是 `[data-theme="dark"]` 重写**。

| shadcn token | 当前值 | 新 · Light | 新 · Dark | 对应 system-v0 |
|---|---|---|---|---|
| `--background` | `#fdfaf6` | `#F8F7F3` | `#161410` | `--bg` |
| `--foreground` | `#1a1206` | `#1A1916` | `#F5F2E9` | `--fg` |
| `--card` | `#ffffff` | `#FFFEFB` | `#1E1B16` | `--bg-elevated` |
| `--card-foreground` | `#1a1206` | `#1A1916` | `#F5F2E9` | `--fg-strong` |
| `--popover` | `#ffffff` | `#FFFEFB` | `#1E1B16` | `--bg-elevated` |
| `--primary` ★ | `#f0885e` | `#A86B22` | `#C0894A` | `--accent` |
| `--primary-foreground` | `#ffffff` | `#FFFFFF` | `#161410` | `--fg-on-accent` |
| `--secondary` | `#faf4ee` | `#F0EEE6` | `#272319` | `--bg-subtle` |
| `--muted` | `#faf4ee` | `#F0EEE6` | `#272319` | `--bg-subtle` |
| `--muted-foreground` | `#8a7a6a` | `#5A574F` | `#CFCABE` | `--fg-muted` |
| `--accent` | `#fef0e8` | `#ECDFC5` | rgba(192,137,74,.14) | `--accent-soft` |
| `--accent-foreground` | `#a84b22` | `#6B4216` | `#D5A878` | `--accent-fg-on-soft` |
| `--border` | `#edd8c8` | rgba(28,26,23,.07) | rgba(245,242,233,.07) | `--border` |
| `--input` | `#edd8c8` | rgba(28,26,23,.13) | rgba(245,242,233,.13) | `--border-strong` |
| `--ring` | `#f0885e` | `#5A574F` | `#CFCABE` | `--fg-muted` |
| `--destructive` | `#c53030` | `#B85450` | `#E68682` | — |
| `--radius` | `1rem` (16) | `0.625rem` (10) | 同 | `--r-md` |

旧自定义品牌 token 也要改:

| 旧 | 新 · Light | 新 · Dark |
|---|---|---|
| `--corivo-amber` | `#A86B22` | `#C0894A` |
| `--corivo-amber-deep` | `#6B4216` | `#8E5A1F` |
| `--color-amber-light` | `#ECDFC5` | rgba(192,137,74,.14) |
| `--color-amber-text` | `#6B4216` | `#D5A878` |
| `--color-cta` | `#1A1916` | `#F5F2E9` |
| `--color-fg-mid` | `#5A574F` | `#CFCABE` |
| `--color-fg-muted` | `#7A746A` | `#98917E` |
| `--color-fg-faint` | `#B5AE9C` | `#5A574F` |
| `--color-surface-warm` | `#ECDFC5` | rgba(192,137,74,.10) |
| `--color-surface-muted` | `#F0EEE6` | `#272319` |

## 阶段(按风险从低到高)

### Phase 1 · Token + 主题基础设施(最低风险)
**目标**:换底色但 UI 不动 —— 现有页面在新色板下"还活着"。

涉及文件:
- [packages/ui/src/styles/globals.css](../../../packages/ui/src/styles/globals.css) —— 替换 `:root` 中的所有色值,新增 `[data-theme="dark"]` 块
- [apps/desktop/src/hooks/use-theme-sync.ts](../../src/hooks/use-theme-sync.ts) —— 实装(读 Config + matchMedia + 写 `data-theme`)
- 新增 `apps/desktop/src/components/theme-provider.tsx` —— mount 在 AppBoot
- [apps/desktop/src/app/app-boot.tsx](../../src/app/app-boot.tsx) —— 包一层 `<ThemeProvider>`
- [packages/shared-types](../../../packages/shared-types) + Rust `Config` —— `theme: "light" | "dark" | "system"`(目前 string,改 enum)
- [packages/ui/src/components/](../../../packages/ui/src/components/) —— 新增 `focus-mark.tsx`
- 字体:删除 Lora、Caveat 的 `@fontsource` 引用,字号 stack 把 SF Pro 系列前置

**验证**:
- 现有 Ask、Onboarding、Settings 页面打开 —— 视觉应该已经从 coral 变印染琥珀
- 切换 Settings → General → Theme(Light / Dark / System)生效且持久化
- 重启 app,主题保留

### Phase 2 · Quick Ask 对齐
**目标**:浮层的色温跟主窗口同源,但保留毛玻璃。

涉及文件:
- [apps/desktop/src/styles/quick-ask.css](../../src/styles/quick-ask.css) —— `--qa-*` token 重新映射到主系统
- [apps/desktop/src/styles/overlay/island.css](../../src/styles/overlay/island.css) —— 替换硬编码十六进制为变量
- Tauri 端 `apply_quick_ask_vibrancy` —— 检查 NSVisualEffectView material 在新色温下是否仍然合适

**验证**:Quick Ask 召唤后,跟主窗口"明显是一家"

### Phase 3 · IA 落地
**目标**:把 [ia-v0](ia-v0.html) 的结构性决定写进代码。

- 删除 `/timeline` 路由 + [apps/desktop/src/pages/timeline/](../../src/pages/timeline/) 目录
- 删除 sidebar 中的 Timeline nav item
- Sidebar 重构:从两个 nav tab → 会话列表(品牌位 / 新建 / ⌘K 搜索 / 列表 / 用户卡 / 采集状态)
- 新增 Frame Drawer 组件 —— Sheet 的右侧变体,从 cited frame chip 触发
- Settings 重组:
  - 合并 `capture` + `permissions` → `Capture & Privacy`,排到二号位
  - 新增 "最近 24h 抓到的" Inspector 卡片
  - General 节加入 Theme 切换器(已在 Phase 1 做)

**验证**:走完 Flow 01–04(从 [ia-v0](ia-v0.html) §03)无障碍

### Phase 4 · 各页精修
按 [screens-v0](screens-v0.html) 的稿子逐张:
- Onboarding 三步(Welcome / Permission / Done)
- Ask page(空状态 / cited frame chip 形态 / tool call viz / 流式光标)
- Settings · Capture & Privacy 内容(Inspector 卡片含 24 柱小图)

**验证**:逐张跟 [screens-v0](screens-v0.html) 对比,Light/Dark 两套都过

### Phase 5 · 收尾
- 删未用 token / 字体 / 依赖(Lora、Caveat、可能的旧 amber 别名)
- 微动效 spec 落地(hover 120ms / dialog 280ms / 浮层 spring 220ms)
- 走一遍组件库,空/加载/错误三态完整

## 不在这次做的事

- 多窗口 / 多账号
- 导出 / 分享会话
- 自定义快捷键
- Frame 全文搜索独立入口
- Notification overlay 的产品形态(代码保留,不动)

## 风险与回退

| 风险 | 缓解 |
|---|---|
| Phase 1 换值后某处颜色读不出来(硬编码) | grep `#f0885e` / `#fdfaf6` / `#1a1206` 等旧值,逐处替换 |
| 暗色下某 shadcn 组件对比度不够 | 走一遍组件库,逐个 audit;必要时给特定组件加 dark 重写 |
| Quick Ask 在新色温下原生 blur 显得脏 | NSVisualEffectView material 切到 `Sidebar` / `HudWindow` 之间试 |
| Phase 3 删 Timeline 时漏 import | TS 编译会立刻报错,跟着修;sidebar / router 各 1–2 处 |
| Settings Config schema 改 theme 字段类型 | 加迁移逻辑:旧 string 值若不在新 enum 里,fallback 到 "system" |

每一个 Phase 落完都是**可独立运行 + 可回滚**的状态,不积累中间态。

## 节奏建议

我会逐 Phase 推进,**每完成一个 Phase 暂停一次,你 review 后再开下一个**。改完一段先 `pnpm tauri:dev` 跑一下,有问题就在那个 phase 内修,不带病往下走。

第一步即 **Phase 1 · Token + 主题基础设施**。涉及的具体改动我会在动手时再列一遍 diff 摘要给你看。

如果这个计划没问题,回 "go phase 1" 我就开始。
