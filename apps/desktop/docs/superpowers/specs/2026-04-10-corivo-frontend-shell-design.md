# Corivo Frontend Shell — Design Doc

**Date:** 2026-04-10  
**Scope:** UI Shell only (mock data, placeholder Tauri invoke calls)  
**Stack:** Tauri 2 + React 19 + TypeScript + Vite

---

## 1. Goal

Build the complete frontend shell for the Corivo app: four pages (Overview, Capture, Connections, Settings) with full layout, routing, state management, and component structure. All Tauri backend calls are mocked — real Rust commands will be wired in a later phase.

---

## 2. Dependencies to Install

| Package | Purpose |
|---|---|
| `tailwindcss` + `@tailwindcss/vite` | Styling |
| `shadcn@latest` | Component library (New York style / Neutral theme) |
| `react-router-dom` | Client-side routing |
| `zustand` | Global runtime state |
| `@tanstack/react-query` | Async data management with caching |
| `lucide-react` | Icons (shadcn default, auto-installed) |
| `date-fns` | Date formatting with zhCN locale |
| `react-markdown` | Markdown rendering for AI-generated summaries |

shadcn config: `--style new-york`, `--base-color neutral`, `--radius 0.625rem`

shadcn components to install: `button card badge tabs accordion dialog dropdown-menu input label textarea toast sonner`

---

## 3. File Structure

```
src/
  pages/
    overview/index.tsx
    capture/index.tsx
    connections/index.tsx
    settings/index.tsx
  components/
    layout/
      AppLayout.tsx          # Left nav + right content shell
      NavItem.tsx            # Single nav item with icon + label + active state
      CaptureStatusIndicator.tsx  # Bottom of nav, subscribes to zustand
    shared/
      StatCard.tsx           # Metric card used in Overview grid
      TimelineCard.tsx       # Segment card in Overview timeline
      ConnectionCard.tsx     # Connection card in Connections page
  lib/
    tauri.ts                 # All mock invoke calls — single source of truth
    store.ts                 # zustand store
    query-client.ts          # tanstack-query QueryClient instance
    utils.ts                 # date-fns helpers, cn() utility
  types.ts                   # All shared TypeScript types
  components/ui/             # shadcn auto-generated, do not hand-edit
  App.tsx                    # QueryClientProvider + RouterProvider
  main.tsx
```

**Key constraint:** Pages never import `invoke` directly. All Tauri calls go through `lib/tauri.ts`.

---

## 4. Types (`src/types.ts`)

```ts
export type Segment = {
  id: string
  startTime: string   // ISO string
  endTime: string
  summary: string
  screenshots: string[]  // base64 or file URLs
}

export type Connection = {
  id: string
  name: string
  icon: string
  status: 'connected' | 'expired' | 'error'
  lastSyncAt: string
  type: 'oauth' | 'api_key' | 'mcp'
}

export type AppConfig = {
  geminiApiKey: string
  supabaseUrl: string
  supabaseApiKey: string
}

export type Session = {
  id: string
  date: string
  screenshotCount: number
  durationMinutes: number
}
```

---

## 5. Mock Tauri Layer (`src/lib/tauri.ts`)

All functions are designed with real Rust command signatures in mind. Swapping to real `invoke()` calls later requires only changing this file.

```ts
export const startCapture = async (): Promise<void> => {}
export const stopCapture = async (): Promise<void> => {}
export const listSegments = async (date: string): Promise<Segment[]> => mockSegments
export const listSessions = async (): Promise<Session[]> => mockSessions
export const generateSummary = async (params: {
  startTime: string
  endTime: string
  prompt: string
}): Promise<string> => mockSummaryText
export const saveConfig = async (config: Partial<AppConfig>): Promise<void> => {}
export const testGeminiConnection = async (apiKey: string): Promise<boolean> => true
export const testSupabaseConnection = async (url: string, key: string): Promise<boolean> => true
export const listConnections = async (): Promise<Connection[]> => mockConnections
```

Mock data arrays are defined in the same file.

---

## 6. Global State (`src/lib/store.ts`)

zustand manages runtime state only — data that changes during app use without needing caching.

```ts
type AppState = {
  capture: {
    isRunning: boolean
    startedAt: Date | null
    screenshotCount: number
  }
  stats: {
    todayScreenshots: number
    monthlyTokens: number
    monthlyCost: number
  }
  startCapture: () => Promise<void>
  stopCapture: () => Promise<void>
  refreshStats: () => Promise<void>
}
```

tanstack-query handles server data (segments, sessions, connections) with caching and refetch logic.

---

## 7. Routing (`src/App.tsx`)

```
/              → Overview page
/capture       → Capture page
/connections   → Connections page
/settings      → Settings page
```

All routes share the `AppLayout` wrapper. `App.tsx` wraps everything in `QueryClientProvider` + `RouterProvider`.

---

## 8. Layout (`AppLayout` + `NavItem` + `CaptureStatusIndicator`)

```
┌─────────────────────────────────────────────┐
│  [Logo]                                      │
│  Nav items:                                  │
│    Home        → /                           │
│    Camera      → /capture                    │
│    Plug        → /connections                │
│    Settings    → /settings                   │
│                                              │
│  [CaptureStatusIndicator]  ← bottom of nav   │
├─────────────────────────────────────────────┤
│  <Outlet />   (page content)                 │
└─────────────────────────────────────────────┘
```

- Nav width: `w-[180px]`, border-right
- Active route: `bg-accent` highlight via `NavLink` from react-router-dom
- `CaptureStatusIndicator`: green dot + "捕获中 · 2h15m" when running, grey dot + "未运行" when stopped

---

## 9. Page Designs

### 9.1 Overview (`/`)

**Layout (top to bottom):**
1. Header: Large "今天" + `format(new Date(), 'yyyy年M月d日 · EEEE', { locale: zhCN })`
2. Stats grid: `grid grid-cols-4 gap-3` — four `StatCard` components
   - 捕获状态 (from zustand)
   - 今日截图数 (from zustand stats)
   - Token 用量 (from zustand stats)
   - 本月花费 (from zustand stats)
3. Timeline: `useQuery(['segments', today])` → list of `TimelineCard`
   - Each card: time range + summary text, `hover:bg-accent/50 cursor-pointer`
   - Click → shadcn `Dialog` with: full summary, screenshot grid, "重新生成" button
   - "重新生成" calls `generateSummary` via `useMutation`
4. Connected sources: horizontal `Badge` row, `variant="secondary"`, green dot prefix

**StatCard props:** `{ label: string; value: string | number; description?: string }`

### 9.2 Capture (`/capture`)

**Layout:**
1. Header: "捕获"
2. Control Card: flex row
   - Left: `Button size="lg"` — "开始捕获" / "停止捕获" (zustand state + actions)
   - Right: status text "已运行 2h 15m · 270 张截图 · 间隔 30s"
3. shadcn `Tabs` with three tabs:

**Tab 1 — 正在运行:**
- Grid of latest 5 screenshot thumbnails (mock image placeholders)
- Polling: `useQuery` with `refetchInterval: 30000`

**Tab 2 — 历史会话:**
- `useQuery(['sessions'])` → list grouped by date
- shadcn `Accordion` — each item expands to show session details

**Tab 3 — 自定义总结:**
- Two `Input type="datetime-local"` for time range
- `Textarea` with default template prompt
- "生成总结" Button → `useMutation(generateSummary)`
- Loading state: `Loader2` icon with `animate-spin`
- Result: `Card` with `react-markdown` rendered content

### 9.3 Connections (`/connections`)

**Layout:**
1. Header: "连接" + subtitle "接入外部数据源和 MCP 服务"
2. "已连接" section: `grid grid-cols-2 gap-3` — `ConnectionCard` per connection
   - Status dot (green/yellow/red based on `status`)
   - Last sync time
   - `DropdownMenu` "⋯" button with: 同步 / 配置 / 断开
3. "可添加" section: `grid grid-cols-3 gap-3` — dashed border cards
   - `border-dashed border-2` placeholder cards
   - Click → `Dialog` for OAuth or config flow (mock)
4. Bottom: "+ 自定义 MCP Server" `Button variant="outline"` → `Dialog` with URL/stdio input

**ConnectionCard props:** `{ connection: Connection; onSync; onConfigure; onDisconnect }`

### 9.4 Settings (`/settings`)

**Layout:** Two-column, `flex gap-6`

**Left nav** (`w-32`): vertical list, local `useState` for active item
- 常规 / API Keys / 存储 / 隐私 / 关于
- Active: `bg-accent`, inactive: `text-muted-foreground`

**Right content:** renders based on active section

**API Keys section (primary focus):**
```
Gemini API Key
[••••••••••••••••] [测试]
用于截图总结与信号抽取

Supabase URL
[https://...        ] [测试]

Supabase API Key
[••••••••••••••••] [测试]

[保存配置]
```
- `Input type="password"` for key fields, `type="text"` for URL
- 测试 button calls `testGeminiConnection()` / `testSupabaseConnection()`
- Result displayed via shadcn `Sonner` toast (success/failure)
- 保存 button calls `saveConfig()`

Other sections (常规/存储/隐私/关于): placeholder content with section title only.

---

## 10. Theme & Styling Constants

| Token | Value |
|---|---|
| Style | New York |
| Base color | Neutral |
| Border radius | `--radius: 0.625rem` |
| Font | System default |
| Spacing unit | `p-4` / `p-6` multiples |

---

## 11. Out of Scope (this phase)

- Real Tauri Rust commands (startCapture, generateSummary, etc.)
- Actual screenshot capture or AI API calls
- OAuth flows for connections
- Data persistence (config saving, segment storage)
- Tauri event listeners (`listen('segment-updated', ...)`)
- Error boundaries or offline handling
