# Corivo Frontend Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the complete frontend shell for Corivo app with four pages (Overview, Capture, Connections, Settings), full routing, state management, and mock Tauri layer.

**Architecture:** React 19 + Tauri 2 SPA with react-router-dom for routing, zustand for runtime state, and tanstack-query for async data. All Tauri backend calls are isolated in `src/lib/tauri.ts` — pages never import `invoke` directly. shadcn/ui (New York style, Neutral theme) provides the component library.

**Tech Stack:** Tauri 2, React 19, TypeScript, Vite, Tailwind CSS v4, shadcn/ui, react-router-dom, zustand, @tanstack/react-query, date-fns (zhCN), react-markdown, lucide-react

---

## File Map

| File | Action | Purpose |
|---|---|---|
| `vite.config.ts` | Modify | Add @tailwindcss/vite plugin |
| `src/index.css` | Create | Tailwind base import + shadcn CSS vars |
| `src/types.ts` | Create | All shared TypeScript types |
| `src/lib/utils.ts` | Create | cn() + date-fns helpers |
| `src/lib/query-client.ts` | Create | TanStack QueryClient singleton |
| `src/lib/tauri.ts` | Create | All mock invoke functions + mock data |
| `src/lib/store.ts` | Create | zustand AppState store |
| `src/App.tsx` | Rewrite | RouterProvider + QueryClientProvider |
| `src/components/layout/AppLayout.tsx` | Create | Left nav + Outlet shell |
| `src/components/layout/NavItem.tsx` | Create | Single nav item with active state |
| `src/components/layout/CaptureStatusIndicator.tsx` | Create | Capture status at bottom of nav |
| `src/components/shared/StatCard.tsx` | Create | Metric card for Overview grid |
| `src/components/shared/TimelineCard.tsx` | Create | Segment card for Overview timeline |
| `src/components/shared/ConnectionCard.tsx` | Create | Connection card with dropdown menu |
| `src/pages/overview/index.tsx` | Create | Overview page |
| `src/pages/capture/index.tsx` | Create | Capture page with tabs |
| `src/pages/connections/index.tsx` | Create | Connections page |
| `src/pages/settings/index.tsx` | Create | Settings page |

---

## Task 1: Install Dependencies

**Files:**
- Modify: `package.json` (via pnpm commands)
- Modify: `vite.config.ts`
- Create: `src/index.css`

- [ ] **Step 1: Install runtime packages**

```bash
cd /Users/airbo/Developer/corivo/corivo-app
pnpm add react-router-dom zustand @tanstack/react-query date-fns react-markdown
```

Expected: packages added to `dependencies` in package.json.

- [ ] **Step 2: Install Tailwind CSS v4**

```bash
pnpm add tailwindcss @tailwindcss/vite
```

- [ ] **Step 3: Initialize shadcn**

Run this and answer prompts as shown:
```bash
pnpm dlx shadcn@latest init
```

When prompted:
- Style: **New York**
- Base color: **Neutral**
- Global CSS file: `src/index.css`
- CSS variables: **Yes**
- Tailwind config: skip (Tailwind v4 handles this)
- Import alias for components: `@/components`
- Import alias for utils: `@/lib/utils`
- React Server Components: **No**

- [ ] **Step 4: Install shadcn components**

```bash
pnpm dlx shadcn@latest add button card badge tabs accordion dialog dropdown-menu input label textarea sonner
```

- [ ] **Step 5: Configure vite.config.ts for Tailwind**

Replace the full content of `vite.config.ts`:

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "path";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
```

- [ ] **Step 6: Verify `src/index.css` has Tailwind import**

After shadcn init, `src/index.css` should contain `@import "tailwindcss"` (Tailwind v4 syntax). If it shows `@tailwind base/components/utilities` instead, replace the file with:

```css
@import "tailwindcss";

@layer base {
  :root {
    --background: 0 0% 100%;
    --foreground: 240 10% 3.9%;
    --card: 0 0% 100%;
    --card-foreground: 240 10% 3.9%;
    --popover: 0 0% 100%;
    --popover-foreground: 240 10% 3.9%;
    --primary: 240 5.9% 10%;
    --primary-foreground: 0 0% 98%;
    --secondary: 240 4.8% 95.9%;
    --secondary-foreground: 240 5.9% 10%;
    --muted: 240 4.8% 95.9%;
    --muted-foreground: 240 3.8% 46.1%;
    --accent: 240 4.8% 95.9%;
    --accent-foreground: 240 5.9% 10%;
    --destructive: 0 84.2% 60.2%;
    --destructive-foreground: 0 0% 98%;
    --border: 240 5.9% 90%;
    --input: 240 5.9% 90%;
    --ring: 240 5.9% 10%;
    --radius: 0.625rem;
  }

  .dark {
    --background: 240 10% 3.9%;
    --foreground: 0 0% 98%;
    --card: 240 10% 3.9%;
    --card-foreground: 0 0% 98%;
    --popover: 240 10% 3.9%;
    --popover-foreground: 0 0% 98%;
    --primary: 0 0% 98%;
    --primary-foreground: 240 5.9% 10%;
    --secondary: 240 3.7% 15.9%;
    --secondary-foreground: 0 0% 98%;
    --muted: 240 3.7% 15.9%;
    --muted-foreground: 240 5% 64.9%;
    --accent: 240 3.7% 15.9%;
    --accent-foreground: 0 0% 98%;
    --destructive: 0 62.8% 30.6%;
    --destructive-foreground: 0 0% 98%;
    --border: 240 3.7% 15.9%;
    --input: 240 3.7% 15.9%;
    --ring: 240 4.9% 83.9%;
  }
}

@layer base {
  * {
    @apply border-border;
  }
  body {
    @apply bg-background text-foreground;
  }
}
```

- [ ] **Step 7: Add `@types/node` for path resolve**

```bash
pnpm add -D @types/node
```

- [ ] **Step 8: Verify build compiles**

```bash
pnpm build
```

Expected: build succeeds (TypeScript + Vite). If shadcn CSS var errors appear, check that `src/index.css` is imported in `src/main.tsx`.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "chore: install tailwind, shadcn, routing, state deps"
```

---

## Task 2: Types and Utility Layer

**Files:**
- Create: `src/types.ts`
- Create: `src/lib/utils.ts`
- Create: `src/lib/query-client.ts`

- [ ] **Step 1: Create `src/types.ts`**

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

- [ ] **Step 2: Create `src/lib/utils.ts`**

```ts
import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"
import { format, formatDistanceToNow } from "date-fns"
import { zhCN } from "date-fns/locale"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

export function formatDateHeader(date: Date): string {
  return format(date, "yyyy年M月d日 · EEEE", { locale: zhCN })
}

export function formatTimeRange(startISO: string, endISO: string): string {
  const start = new Date(startISO)
  const end = new Date(endISO)
  return `${format(start, "HH:mm")} – ${format(end, "HH:mm")}`
}

export function formatLastSync(isoString: string): string {
  return formatDistanceToNow(new Date(isoString), { addSuffix: true, locale: zhCN })
}
```

Note: shadcn auto-generates `src/lib/utils.ts` with just `cn()`. If the file already exists after shadcn init, **replace** it with the content above (which adds the date helpers to the same file).

- [ ] **Step 3: Install clsx and tailwind-merge if not present**

shadcn installs these automatically. Verify:

```bash
grep -E "clsx|tailwind-merge" package.json
```

If missing:
```bash
pnpm add clsx tailwind-merge
```

- [ ] **Step 4: Create `src/lib/query-client.ts`**

```ts
import { QueryClient } from "@tanstack/react-query"

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 1000 * 60,   // 1 minute
      retry: 1,
    },
  },
})
```

- [ ] **Step 5: Commit**

```bash
git add src/types.ts src/lib/utils.ts src/lib/query-client.ts
git commit -m "feat: add shared types and utility layer"
```

---

## Task 3: Mock Tauri Layer

**Files:**
- Create: `src/lib/tauri.ts`

- [ ] **Step 1: Create `src/lib/tauri.ts` with mock data and all functions**

```ts
import type { Segment, Connection, Session, AppConfig } from "@/types"

// ─── Mock Data ────────────────────────────────────────────────────────────────

const mockSegments: Segment[] = [
  {
    id: "seg-1",
    startTime: new Date(Date.now() - 1000 * 60 * 90).toISOString(),
    endTime: new Date(Date.now() - 1000 * 60 * 60).toISOString(),
    summary: "回顾了上周的 sprint 目标，确认了三个核心功能的优先级排序。与 PM 讨论了 MVP 范围，同意将 OAuth 集成推迟到下一阶段。",
    screenshots: [],
  },
  {
    id: "seg-2",
    startTime: new Date(Date.now() - 1000 * 60 * 55).toISOString(),
    endTime: new Date(Date.now() - 1000 * 60 * 30).toISOString(),
    summary: "实现了 Tauri 截图命令的核心逻辑，集成了 screenshots-rs crate。遇到了 macOS 权限问题，在 Info.plist 中添加了 NSScreenCaptureUsageDescription。",
    screenshots: [],
  },
  {
    id: "seg-3",
    startTime: new Date(Date.now() - 1000 * 60 * 25).toISOString(),
    endTime: new Date(Date.now() - 1000 * 60 * 5).toISOString(),
    summary: "调试 Supabase 上传流程，修复了 base64 编码导致的文件大小问题。切换为直接上传二进制流，上传速度提升约 40%。",
    screenshots: [],
  },
]

const mockSessions: Session[] = [
  {
    id: "session-1",
    date: new Date(Date.now() - 1000 * 60 * 60 * 24).toISOString(),
    screenshotCount: 143,
    durationMinutes: 185,
  },
  {
    id: "session-2",
    date: new Date(Date.now() - 1000 * 60 * 60 * 48).toISOString(),
    screenshotCount: 98,
    durationMinutes: 120,
  },
  {
    id: "session-3",
    date: new Date(Date.now() - 1000 * 60 * 60 * 72).toISOString(),
    screenshotCount: 210,
    durationMinutes: 240,
  },
]

const mockConnections: Connection[] = [
  {
    id: "conn-1",
    name: "Notion",
    icon: "📝",
    status: "connected",
    lastSyncAt: new Date(Date.now() - 1000 * 60 * 15).toISOString(),
    type: "oauth",
  },
  {
    id: "conn-2",
    name: "Linear",
    icon: "🔷",
    status: "connected",
    lastSyncAt: new Date(Date.now() - 1000 * 60 * 60 * 2).toISOString(),
    type: "oauth",
  },
  {
    id: "conn-3",
    name: "GitHub",
    icon: "🐙",
    status: "expired",
    lastSyncAt: new Date(Date.now() - 1000 * 60 * 60 * 24 * 3).toISOString(),
    type: "oauth",
  },
]

const mockSummaryText = `## 工作总结

本时间段内主要完成了以下工作：

- **核心功能实现**：完成了截图捕获模块的基础框架，集成了定时触发逻辑
- **问题排查**：定位并修复了内存泄漏问题，优化了截图压缩流程
- **文档更新**：补充了 API 接口说明和错误处理规范

### 关键决策

选择使用 \`screenshots-rs\` 而非系统 API，原因是跨平台兼容性更好，且已有社区维护的 Tauri 插件。

### 下一步

- [ ] 集成 Gemini API 进行截图内容分析
- [ ] 完善 Supabase 存储策略
`

// ─── Mock Invoke Functions ─────────────────────────────────────────────────────

export const startCapture = async (): Promise<void> => {
  await new Promise((r) => setTimeout(r, 200))
}

export const stopCapture = async (): Promise<void> => {
  await new Promise((r) => setTimeout(r, 200))
}

export const listSegments = async (_date: string): Promise<Segment[]> => {
  await new Promise((r) => setTimeout(r, 300))
  return mockSegments
}

export const listSessions = async (): Promise<Session[]> => {
  await new Promise((r) => setTimeout(r, 300))
  return mockSessions
}

export const generateSummary = async (_params: {
  startTime: string
  endTime: string
  prompt: string
}): Promise<string> => {
  await new Promise((r) => setTimeout(r, 1500))
  return mockSummaryText
}

export const saveConfig = async (_config: Partial<AppConfig>): Promise<void> => {
  await new Promise((r) => setTimeout(r, 300))
}

export const testGeminiConnection = async (_apiKey: string): Promise<boolean> => {
  await new Promise((r) => setTimeout(r, 800))
  return true
}

export const testSupabaseConnection = async (
  _url: string,
  _key: string
): Promise<boolean> => {
  await new Promise((r) => setTimeout(r, 800))
  return true
}

export const listConnections = async (): Promise<Connection[]> => {
  await new Promise((r) => setTimeout(r, 300))
  return mockConnections
}
```

- [ ] **Step 2: Commit**

```bash
git add src/lib/tauri.ts
git commit -m "feat: add mock tauri layer with mock data"
```

---

## Task 4: Zustand Store

**Files:**
- Create: `src/lib/store.ts`

- [ ] **Step 1: Create `src/lib/store.ts`**

```ts
import { create } from "zustand"
import { startCapture as tauriStartCapture, stopCapture as tauriStopCapture } from "@/lib/tauri"

type CaptureState = {
  isRunning: boolean
  startedAt: Date | null
  screenshotCount: number
}

type StatsState = {
  todayScreenshots: number
  monthlyTokens: number
  monthlyCost: number
}

type AppState = {
  capture: CaptureState
  stats: StatsState
  startCapture: () => Promise<void>
  stopCapture: () => Promise<void>
  refreshStats: () => Promise<void>
}

export const useAppStore = create<AppState>((set) => ({
  capture: {
    isRunning: false,
    startedAt: null,
    screenshotCount: 0,
  },
  stats: {
    todayScreenshots: 47,
    monthlyTokens: 128_500,
    monthlyCost: 0.38,
  },
  startCapture: async () => {
    await tauriStartCapture()
    set((state) => ({
      capture: {
        ...state.capture,
        isRunning: true,
        startedAt: new Date(),
        screenshotCount: 0,
      },
    }))
  },
  stopCapture: async () => {
    await tauriStopCapture()
    set((state) => ({
      capture: {
        ...state.capture,
        isRunning: false,
      },
    }))
  },
  refreshStats: async () => {
    // Mock: in real impl would invoke Tauri command
    set({
      stats: {
        todayScreenshots: Math.floor(Math.random() * 200),
        monthlyTokens: Math.floor(Math.random() * 500_000),
        monthlyCost: parseFloat((Math.random() * 2).toFixed(2)),
      },
    })
  },
}))
```

- [ ] **Step 2: Commit**

```bash
git add src/lib/store.ts
git commit -m "feat: add zustand store for capture and stats state"
```

---

## Task 5: App.tsx — Routing and Providers

**Files:**
- Modify: `src/App.tsx`
- Modify: `src/main.tsx`

- [ ] **Step 1: Rewrite `src/App.tsx`**

```tsx
import { createBrowserRouter, RouterProvider } from "react-router-dom"
import { QueryClientProvider } from "@tanstack/react-query"
import { Toaster } from "@/components/ui/sonner"
import { queryClient } from "@/lib/query-client"
import AppLayout from "@/components/layout/AppLayout"
import OverviewPage from "@/pages/overview"
import CapturePage from "@/pages/capture"
import ConnectionsPage from "@/pages/connections"
import SettingsPage from "@/pages/settings"

const router = createBrowserRouter([
  {
    path: "/",
    element: <AppLayout />,
    children: [
      { index: true, element: <OverviewPage /> },
      { path: "capture", element: <CapturePage /> },
      { path: "connections", element: <ConnectionsPage /> },
      { path: "settings", element: <SettingsPage /> },
    ],
  },
])

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
      <Toaster />
    </QueryClientProvider>
  )
}
```

- [ ] **Step 2: Update `src/main.tsx` to import CSS**

```tsx
import React from "react"
import ReactDOM from "react-dom/client"
import App from "./App"
import "./index.css"

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
)
```

- [ ] **Step 3: Create placeholder page files so App.tsx compiles**

Create `src/pages/overview/index.tsx`:
```tsx
export default function OverviewPage() {
  return <div className="p-6">Overview</div>
}
```

Create `src/pages/capture/index.tsx`:
```tsx
export default function CapturePage() {
  return <div className="p-6">Capture</div>
}
```

Create `src/pages/connections/index.tsx`:
```tsx
export default function ConnectionsPage() {
  return <div className="p-6">Connections</div>
}
```

Create `src/pages/settings/index.tsx`:
```tsx
export default function SettingsPage() {
  return <div className="p-6">Settings</div>
}
```

- [ ] **Step 4: Verify build compiles**

```bash
pnpm build
```

Expected: clean build, no TypeScript errors.

- [ ] **Step 5: Commit**

```bash
git add src/App.tsx src/main.tsx src/pages/
git commit -m "feat: set up routing with react-router-dom and QueryClientProvider"
```

---

## Task 6: Layout Components

**Files:**
- Create: `src/components/layout/NavItem.tsx`
- Create: `src/components/layout/CaptureStatusIndicator.tsx`
- Create: `src/components/layout/AppLayout.tsx`

- [ ] **Step 1: Create `src/components/layout/NavItem.tsx`**

```tsx
import { NavLink } from "react-router-dom"
import { cn } from "@/lib/utils"
import type { LucideIcon } from "lucide-react"

type NavItemProps = {
  to: string
  icon: LucideIcon
  label: string
}

export default function NavItem({ to, icon: Icon, label }: NavItemProps) {
  return (
    <NavLink
      to={to}
      end={to === "/"}
      className={({ isActive }) =>
        cn(
          "flex items-center gap-2 rounded-md px-3 py-2 text-sm font-medium transition-colors",
          isActive
            ? "bg-accent text-accent-foreground"
            : "text-muted-foreground hover:bg-accent/50 hover:text-accent-foreground"
        )
      }
    >
      <Icon className="h-4 w-4" />
      {label}
    </NavLink>
  )
}
```

- [ ] **Step 2: Create `src/components/layout/CaptureStatusIndicator.tsx`**

```tsx
import { useAppStore } from "@/lib/store"
import { useEffect, useState } from "react"

function useElapsedTime(startedAt: Date | null): string {
  const [elapsed, setElapsed] = useState("")

  useEffect(() => {
    if (!startedAt) return

    const update = () => {
      const diffMs = Date.now() - startedAt.getTime()
      const totalMinutes = Math.floor(diffMs / 60000)
      const hours = Math.floor(totalMinutes / 60)
      const minutes = totalMinutes % 60
      setElapsed(hours > 0 ? `${hours}h${minutes.toString().padStart(2, "0")}m` : `${minutes}m`)
    }

    update()
    const interval = setInterval(update, 60000)
    return () => clearInterval(interval)
  }, [startedAt])

  return elapsed
}

export default function CaptureStatusIndicator() {
  const { capture } = useAppStore()
  const elapsed = useElapsedTime(capture.startedAt)

  return (
    <div className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground">
      <span
        className={cn(
          "h-2 w-2 rounded-full",
          capture.isRunning ? "bg-green-500" : "bg-gray-400"
        )}
      />
      {capture.isRunning ? `捕获中 · ${elapsed}` : "未运行"}
    </div>
  )
}

function cn(...classes: (string | boolean | undefined)[]) {
  return classes.filter(Boolean).join(" ")
}
```

- [ ] **Step 3: Create `src/components/layout/AppLayout.tsx`**

```tsx
import { Outlet } from "react-router-dom"
import { Home, Camera, Plug, Settings } from "lucide-react"
import NavItem from "./NavItem"
import CaptureStatusIndicator from "./CaptureStatusIndicator"

export default function AppLayout() {
  return (
    <div className="flex h-screen bg-background">
      {/* Left nav */}
      <nav className="flex w-[180px] flex-col border-r border-border bg-background">
        {/* Logo */}
        <div className="px-4 py-4">
          <span className="text-base font-semibold tracking-tight">Corivo</span>
        </div>

        {/* Nav items */}
        <div className="flex flex-1 flex-col gap-1 px-2">
          <NavItem to="/" icon={Home} label="总览" />
          <NavItem to="/capture" icon={Camera} label="捕获" />
          <NavItem to="/connections" icon={Plug} label="连接" />
          <NavItem to="/settings" icon={Settings} label="设置" />
        </div>

        {/* Capture status at bottom */}
        <div className="border-t border-border py-2">
          <CaptureStatusIndicator />
        </div>
      </nav>

      {/* Main content */}
      <main className="flex-1 overflow-auto">
        <Outlet />
      </main>
    </div>
  )
}
```

- [ ] **Step 4: Verify build and manual check**

```bash
pnpm build
```

Open the app in browser (run `pnpm dev` separately in terminal). Expected: left nav visible with 4 nav items. Clicking items changes URL. Bottom shows "未运行".

- [ ] **Step 5: Commit**

```bash
git add src/components/layout/
git commit -m "feat: add app layout with nav and capture status indicator"
```

---

## Task 7: Shared Components

**Files:**
- Create: `src/components/shared/StatCard.tsx`
- Create: `src/components/shared/TimelineCard.tsx`
- Create: `src/components/shared/ConnectionCard.tsx`

- [ ] **Step 1: Create `src/components/shared/StatCard.tsx`**

```tsx
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"

type StatCardProps = {
  label: string
  value: string | number
  description?: string
}

export default function StatCard({ label, value, description }: StatCardProps) {
  return (
    <Card>
      <CardHeader className="pb-1 pt-4">
        <CardTitle className="text-xs font-medium text-muted-foreground">
          {label}
        </CardTitle>
      </CardHeader>
      <CardContent className="pb-4">
        <div className="text-2xl font-bold">{value}</div>
        {description && (
          <p className="mt-1 text-xs text-muted-foreground">{description}</p>
        )}
      </CardContent>
    </Card>
  )
}
```

- [ ] **Step 2: Create `src/components/shared/TimelineCard.tsx`**

```tsx
import { useState } from "react"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Loader2 } from "lucide-react"
import { useMutation } from "@tanstack/react-query"
import ReactMarkdown from "react-markdown"
import { generateSummary } from "@/lib/tauri"
import { formatTimeRange } from "@/lib/utils"
import type { Segment } from "@/types"

type TimelineCardProps = {
  segment: Segment
}

export default function TimelineCard({ segment }: TimelineCardProps) {
  const [open, setOpen] = useState(false)
  const [summaryText, setSummaryText] = useState(segment.summary)

  const regenerate = useMutation({
    mutationFn: () =>
      generateSummary({
        startTime: segment.startTime,
        endTime: segment.endTime,
        prompt: "请总结这段时间内的工作内容。",
      }),
    onSuccess: (data) => setSummaryText(data),
  })

  return (
    <>
      <div
        className="rounded-md border border-border p-3 hover:bg-accent/50 cursor-pointer transition-colors"
        onClick={() => setOpen(true)}
      >
        <div className="text-xs font-medium text-muted-foreground mb-1">
          {formatTimeRange(segment.startTime, segment.endTime)}
        </div>
        <p className="text-sm line-clamp-2">{segment.summary}</p>
      </div>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-w-2xl max-h-[80vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle className="text-sm font-medium text-muted-foreground">
              {formatTimeRange(segment.startTime, segment.endTime)}
            </DialogTitle>
          </DialogHeader>

          <div className="prose prose-sm dark:prose-invert max-w-none">
            <ReactMarkdown>{summaryText}</ReactMarkdown>
          </div>

          {segment.screenshots.length > 0 && (
            <div className="grid grid-cols-3 gap-2 mt-4">
              {segment.screenshots.map((src, i) => (
                <img
                  key={i}
                  src={src}
                  alt={`截图 ${i + 1}`}
                  className="rounded-md border border-border object-cover aspect-video"
                />
              ))}
            </div>
          )}

          <div className="flex justify-end mt-4">
            <Button
              variant="outline"
              size="sm"
              onClick={() => regenerate.mutate()}
              disabled={regenerate.isPending}
            >
              {regenerate.isPending && (
                <Loader2 className="mr-2 h-3 w-3 animate-spin" />
              )}
              重新生成
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  )
}
```

- [ ] **Step 3: Create `src/components/shared/ConnectionCard.tsx`**

```tsx
import { Card, CardContent } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Button } from "@/components/ui/button"
import { MoreHorizontal } from "lucide-react"
import { formatLastSync } from "@/lib/utils"
import { cn } from "@/lib/utils"
import type { Connection } from "@/types"

type ConnectionCardProps = {
  connection: Connection
  onSync: (id: string) => void
  onConfigure: (id: string) => void
  onDisconnect: (id: string) => void
}

const statusConfig = {
  connected: { label: "已连接", dot: "bg-green-500" },
  expired: { label: "已过期", dot: "bg-yellow-500" },
  error: { label: "错误", dot: "bg-red-500" },
} as const

export default function ConnectionCard({
  connection,
  onSync,
  onConfigure,
  onDisconnect,
}: ConnectionCardProps) {
  const status = statusConfig[connection.status]

  return (
    <Card>
      <CardContent className="p-4">
        <div className="flex items-start justify-between">
          <div className="flex items-center gap-2">
            <span className="text-xl">{connection.icon}</span>
            <div>
              <div className="text-sm font-medium">{connection.name}</div>
              <div className="flex items-center gap-1 mt-0.5">
                <span className={cn("h-1.5 w-1.5 rounded-full", status.dot)} />
                <span className="text-xs text-muted-foreground">{status.label}</span>
              </div>
            </div>
          </div>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon" className="h-7 w-7">
                <MoreHorizontal className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onClick={() => onSync(connection.id)}>
                同步
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => onConfigure(connection.id)}>
                配置
              </DropdownMenuItem>
              <DropdownMenuItem
                onClick={() => onDisconnect(connection.id)}
                className="text-destructive"
              >
                断开
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
        <div className="mt-2 text-xs text-muted-foreground">
          上次同步：{formatLastSync(connection.lastSyncAt)}
        </div>
      </CardContent>
    </Card>
  )
}
```

- [ ] **Step 4: Commit**

```bash
git add src/components/shared/
git commit -m "feat: add StatCard, TimelineCard, and ConnectionCard shared components"
```

---

## Task 8: Overview Page

**Files:**
- Modify: `src/pages/overview/index.tsx`

- [ ] **Step 1: Replace `src/pages/overview/index.tsx` with full implementation**

```tsx
import { useQuery } from "@tanstack/react-query"
import { format } from "date-fns"
import { zhCN } from "date-fns/locale"
import { Badge } from "@/components/ui/badge"
import StatCard from "@/components/shared/StatCard"
import TimelineCard from "@/components/shared/TimelineCard"
import { listSegments, listConnections } from "@/lib/tauri"
import { useAppStore } from "@/lib/store"

export default function OverviewPage() {
  const { capture, stats } = useAppStore()
  const today = format(new Date(), "yyyy-MM-dd")

  const { data: segments = [] } = useQuery({
    queryKey: ["segments", today],
    queryFn: () => listSegments(today),
  })

  const { data: connections = [] } = useQuery({
    queryKey: ["connections"],
    queryFn: listConnections,
  })

  const connectedSources = connections.filter((c) => c.status === "connected")

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div>
        <h1 className="text-3xl font-bold">今天</h1>
        <p className="text-muted-foreground text-sm mt-1">
          {format(new Date(), "yyyy年M月d日 · EEEE", { locale: zhCN })}
        </p>
      </div>

      {/* Stats grid */}
      <div className="grid grid-cols-4 gap-3">
        <StatCard
          label="捕获状态"
          value={capture.isRunning ? "运行中" : "已停止"}
          description={capture.isRunning ? "正在捕获截图" : "点击捕获页面开始"}
        />
        <StatCard
          label="今日截图数"
          value={stats.todayScreenshots}
          description="张截图"
        />
        <StatCard
          label="Token 用量"
          value={stats.monthlyTokens.toLocaleString()}
          description="本月累计"
        />
        <StatCard
          label="本月花费"
          value={`$${stats.monthlyCost.toFixed(2)}`}
          description="Gemini API"
        />
      </div>

      {/* Timeline */}
      <div>
        <h2 className="text-sm font-medium text-muted-foreground mb-3">时间线</h2>
        {segments.length === 0 ? (
          <p className="text-sm text-muted-foreground">今天还没有记录</p>
        ) : (
          <div className="space-y-2">
            {segments.map((segment) => (
              <TimelineCard key={segment.id} segment={segment} />
            ))}
          </div>
        )}
      </div>

      {/* Connected sources */}
      {connectedSources.length > 0 && (
        <div>
          <h2 className="text-sm font-medium text-muted-foreground mb-2">
            已连接数据源
          </h2>
          <div className="flex flex-wrap gap-2">
            {connectedSources.map((c) => (
              <Badge key={c.id} variant="secondary" className="gap-1">
                <span className="h-1.5 w-1.5 rounded-full bg-green-500 inline-block" />
                {c.icon} {c.name}
              </Badge>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}
```

- [ ] **Step 2: Verify build**

```bash
pnpm build
```

Expected: clean compile.

- [ ] **Step 3: Commit**

```bash
git add src/pages/overview/index.tsx
git commit -m "feat: implement overview page"
```

---

## Task 9: Capture Page

**Files:**
- Modify: `src/pages/capture/index.tsx`

- [ ] **Step 1: Replace `src/pages/capture/index.tsx` with full implementation**

```tsx
import { useState } from "react"
import { useQuery, useMutation } from "@tanstack/react-query"
import { format, formatDistanceToNow } from "date-fns"
import { zhCN } from "date-fns/locale"
import { Loader2 } from "lucide-react"
import ReactMarkdown from "react-markdown"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "@/components/ui/accordion"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Label } from "@/components/ui/label"
import { listSessions, generateSummary } from "@/lib/tauri"
import { useAppStore } from "@/lib/store"

const DEFAULT_PROMPT =
  "请总结这段时间内的主要工作内容，包括完成的任务、遇到的问题和关键决策。"

export default function CapturePage() {
  const { capture, startCapture, stopCapture } = useAppStore()

  const [startTime, setStartTime] = useState("")
  const [endTime, setEndTime] = useState("")
  const [prompt, setPrompt] = useState(DEFAULT_PROMPT)
  const [summaryResult, setSummaryResult] = useState("")

  const { data: sessions = [] } = useQuery({
    queryKey: ["sessions"],
    queryFn: listSessions,
  })

  // Latest screenshots polling (mock — returns empty while not running)
  const { data: latestScreenshots = [] } = useQuery({
    queryKey: ["latest-screenshots"],
    queryFn: async () => [] as string[],
    refetchInterval: capture.isRunning ? 30000 : false,
  })

  const summarizeMutation = useMutation({
    mutationFn: () => generateSummary({ startTime, endTime, prompt }),
    onSuccess: (data) => setSummaryResult(data),
  })

  const elapsedText = (() => {
    if (!capture.isRunning || !capture.startedAt) return null
    const mins = Math.floor((Date.now() - capture.startedAt.getTime()) / 60000)
    const h = Math.floor(mins / 60)
    const m = mins % 60
    const elapsed = h > 0 ? `${h}h ${m.toString().padStart(2, "0")}m` : `${m}m`
    return `已运行 ${elapsed} · ${capture.screenshotCount} 张截图 · 间隔 30s`
  })()

  // Group sessions by date
  const groupedSessions: Record<string, typeof sessions> = {}
  for (const s of sessions) {
    const dateKey = format(new Date(s.date), "yyyy年M月d日", { locale: zhCN })
    if (!groupedSessions[dateKey]) groupedSessions[dateKey] = []
    groupedSessions[dateKey].push(s)
  }

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <h1 className="text-2xl font-bold">捕获</h1>

      {/* Control card */}
      <Card>
        <CardContent className="p-4 flex items-center gap-6">
          <Button
            size="lg"
            variant={capture.isRunning ? "destructive" : "default"}
            onClick={() => (capture.isRunning ? stopCapture() : startCapture())}
          >
            {capture.isRunning ? "停止捕获" : "开始捕获"}
          </Button>
          {elapsedText && (
            <span className="text-sm text-muted-foreground">{elapsedText}</span>
          )}
          {!capture.isRunning && (
            <span className="text-sm text-muted-foreground">捕获未运行</span>
          )}
        </CardContent>
      </Card>

      {/* Tabs */}
      <Tabs defaultValue="running">
        <TabsList>
          <TabsTrigger value="running">正在运行</TabsTrigger>
          <TabsTrigger value="history">历史会话</TabsTrigger>
          <TabsTrigger value="custom">自定义总结</TabsTrigger>
        </TabsList>

        {/* Tab 1: Running */}
        <TabsContent value="running" className="mt-4">
          {latestScreenshots.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {capture.isRunning ? "等待下一次截图..." : "捕获未运行"}
            </p>
          ) : (
            <div className="grid grid-cols-5 gap-2">
              {latestScreenshots.slice(0, 5).map((src, i) => (
                <img
                  key={i}
                  src={src}
                  alt={`截图 ${i + 1}`}
                  className="rounded-md border border-border object-cover aspect-video"
                />
              ))}
            </div>
          )}
        </TabsContent>

        {/* Tab 2: History */}
        <TabsContent value="history" className="mt-4">
          {Object.entries(groupedSessions).length === 0 ? (
            <p className="text-sm text-muted-foreground">暂无历史会话</p>
          ) : (
            <Accordion type="single" collapsible className="w-full">
              {Object.entries(groupedSessions).map(([date, dateSessions]) => (
                <AccordionItem key={date} value={date}>
                  <AccordionTrigger className="text-sm">{date}</AccordionTrigger>
                  <AccordionContent>
                    <div className="space-y-2 pt-1">
                      {dateSessions.map((s) => (
                        <div
                          key={s.id}
                          className="flex items-center justify-between rounded-md border border-border px-3 py-2 text-sm"
                        >
                          <span>
                            {formatDistanceToNow(new Date(s.date), {
                              addSuffix: false,
                              locale: zhCN,
                            })}前
                          </span>
                          <span className="text-muted-foreground">
                            {s.durationMinutes} 分钟 · {s.screenshotCount} 张截图
                          </span>
                        </div>
                      ))}
                    </div>
                  </AccordionContent>
                </AccordionItem>
              ))}
            </Accordion>
          )}
        </TabsContent>

        {/* Tab 3: Custom summary */}
        <TabsContent value="custom" className="mt-4 space-y-4 max-w-2xl">
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-1">
              <Label htmlFor="start-time">开始时间</Label>
              <Input
                id="start-time"
                type="datetime-local"
                value={startTime}
                onChange={(e) => setStartTime(e.target.value)}
              />
            </div>
            <div className="space-y-1">
              <Label htmlFor="end-time">结束时间</Label>
              <Input
                id="end-time"
                type="datetime-local"
                value={endTime}
                onChange={(e) => setEndTime(e.target.value)}
              />
            </div>
          </div>

          <div className="space-y-1">
            <Label htmlFor="prompt">提示词</Label>
            <Textarea
              id="prompt"
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              rows={3}
            />
          </div>

          <Button
            onClick={() => summarizeMutation.mutate()}
            disabled={summarizeMutation.isPending || !startTime || !endTime}
          >
            {summarizeMutation.isPending && (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            )}
            生成总结
          </Button>

          {summaryResult && (
            <Card>
              <CardContent className="p-4 prose prose-sm dark:prose-invert max-w-none">
                <ReactMarkdown>{summaryResult}</ReactMarkdown>
              </CardContent>
            </Card>
          )}
        </TabsContent>
      </Tabs>
    </div>
  )
}
```

- [ ] **Step 2: Verify build**

```bash
pnpm build
```

Expected: clean compile.

- [ ] **Step 3: Commit**

```bash
git add src/pages/capture/index.tsx
git commit -m "feat: implement capture page with tabs, history accordion, and custom summary"
```

---

## Task 10: Connections Page

**Files:**
- Modify: `src/pages/connections/index.tsx`

- [ ] **Step 1: Replace `src/pages/connections/index.tsx` with full implementation**

```tsx
import { useState } from "react"
import { useQuery } from "@tanstack/react-query"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import ConnectionCard from "@/components/shared/ConnectionCard"
import { listConnections } from "@/lib/tauri"

const AVAILABLE_CONNECTIONS = [
  { id: "avail-1", name: "Slack", icon: "💬" },
  { id: "avail-2", name: "Jira", icon: "🟦" },
  { id: "avail-3", name: "Google Calendar", icon: "📅" },
  { id: "avail-4", name: "Figma", icon: "🎨" },
  { id: "avail-5", name: "Obsidian", icon: "📓" },
  { id: "avail-6", name: "GitLab", icon: "🦊" },
]

export default function ConnectionsPage() {
  const [mcpDialogOpen, setMcpDialogOpen] = useState(false)
  const [mcpUrl, setMcpUrl] = useState("")

  const { data: connections = [] } = useQuery({
    queryKey: ["connections"],
    queryFn: listConnections,
  })

  return (
    <div className="p-6 space-y-8">
      {/* Header */}
      <div>
        <h1 className="text-2xl font-bold">连接</h1>
        <p className="text-sm text-muted-foreground mt-1">
          接入外部数据源和 MCP 服务
        </p>
      </div>

      {/* Connected section */}
      {connections.length > 0 && (
        <section>
          <h2 className="text-sm font-medium text-muted-foreground mb-3">已连接</h2>
          <div className="grid grid-cols-2 gap-3">
            {connections.map((c) => (
              <ConnectionCard
                key={c.id}
                connection={c}
                onSync={(id) => console.log("sync", id)}
                onConfigure={(id) => console.log("configure", id)}
                onDisconnect={(id) => console.log("disconnect", id)}
              />
            ))}
          </div>
        </section>
      )}

      {/* Available section */}
      <section>
        <h2 className="text-sm font-medium text-muted-foreground mb-3">可添加</h2>
        <div className="grid grid-cols-3 gap-3">
          {AVAILABLE_CONNECTIONS.map((a) => (
            <button
              key={a.id}
              className="flex items-center gap-2 rounded-md border-2 border-dashed border-border p-4 text-sm text-muted-foreground hover:border-foreground/30 hover:text-foreground transition-colors text-left"
              onClick={() => console.log("add", a.id)}
            >
              <span className="text-xl">{a.icon}</span>
              <span>{a.name}</span>
            </button>
          ))}
        </div>
      </section>

      {/* Custom MCP */}
      <div>
        <Button variant="outline" onClick={() => setMcpDialogOpen(true)}>
          + 自定义 MCP Server
        </Button>
      </div>

      {/* Custom MCP dialog */}
      <Dialog open={mcpDialogOpen} onOpenChange={setMcpDialogOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>添加自定义 MCP Server</DialogTitle>
          </DialogHeader>
          <div className="space-y-4 pt-2">
            <div className="space-y-1">
              <Label htmlFor="mcp-url">Server URL 或 stdio 命令</Label>
              <Input
                id="mcp-url"
                placeholder="https://your-mcp-server.com 或 node ./mcp.js"
                value={mcpUrl}
                onChange={(e) => setMcpUrl(e.target.value)}
              />
            </div>
            <div className="flex justify-end gap-2">
              <Button
                variant="outline"
                onClick={() => setMcpDialogOpen(false)}
              >
                取消
              </Button>
              <Button
                disabled={!mcpUrl.trim()}
                onClick={() => {
                  console.log("add mcp", mcpUrl)
                  setMcpDialogOpen(false)
                  setMcpUrl("")
                }}
              >
                添加
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
```

- [ ] **Step 2: Verify build**

```bash
pnpm build
```

- [ ] **Step 3: Commit**

```bash
git add src/pages/connections/index.tsx
git commit -m "feat: implement connections page with cards and MCP dialog"
```

---

## Task 11: Settings Page

**Files:**
- Modify: `src/pages/settings/index.tsx`

- [ ] **Step 1: Replace `src/pages/settings/index.tsx` with full implementation**

```tsx
import { useState } from "react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { cn } from "@/lib/utils"
import { saveConfig, testGeminiConnection, testSupabaseConnection } from "@/lib/tauri"

type Section = "general" | "apikeys" | "storage" | "privacy" | "about"

const SECTIONS: { id: Section; label: string }[] = [
  { id: "general", label: "常规" },
  { id: "apikeys", label: "API Keys" },
  { id: "storage", label: "存储" },
  { id: "privacy", label: "隐私" },
  { id: "about", label: "关于" },
]

function PlaceholderSection({ title }: { title: string }) {
  return (
    <div>
      <h2 className="text-lg font-semibold mb-4">{title}</h2>
      <p className="text-sm text-muted-foreground">该部分配置将在后续版本中开放。</p>
    </div>
  )
}

function ApiKeysSection() {
  const [geminiKey, setGeminiKey] = useState("")
  const [supabaseUrl, setSupabaseUrl] = useState("")
  const [supabaseKey, setSupabaseKey] = useState("")
  const [testing, setTesting] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  const handleTest = async (field: "gemini" | "supabase") => {
    setTesting(field)
    try {
      let ok: boolean
      if (field === "gemini") {
        ok = await testGeminiConnection(geminiKey)
      } else {
        ok = await testSupabaseConnection(supabaseUrl, supabaseKey)
      }
      toast(ok ? "连接成功" : "连接失败", {
        description: ok
          ? `${field === "gemini" ? "Gemini" : "Supabase"} API 验证通过`
          : "请检查 API Key 是否正确",
      })
    } finally {
      setTesting(null)
    }
  }

  const handleSave = async () => {
    setSaving(true)
    try {
      await saveConfig({
        geminiApiKey: geminiKey,
        supabaseUrl,
        supabaseApiKey: supabaseKey,
      })
      toast("配置已保存")
    } finally {
      setSaving(false)
    }
  }

  return (
    <div>
      <h2 className="text-lg font-semibold mb-6">API Keys</h2>
      <div className="space-y-6 max-w-md">
        {/* Gemini */}
        <div className="space-y-2">
          <Label htmlFor="gemini-key">Gemini API Key</Label>
          <div className="flex gap-2">
            <Input
              id="gemini-key"
              type="password"
              placeholder="AIza..."
              value={geminiKey}
              onChange={(e) => setGeminiKey(e.target.value)}
              className="flex-1"
            />
            <Button
              variant="outline"
              size="sm"
              disabled={!geminiKey || testing === "gemini"}
              onClick={() => handleTest("gemini")}
            >
              {testing === "gemini" ? "测试中..." : "测试"}
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">用于截图总结与信号抽取</p>
        </div>

        {/* Supabase URL */}
        <div className="space-y-2">
          <Label htmlFor="supabase-url">Supabase URL</Label>
          <div className="flex gap-2">
            <Input
              id="supabase-url"
              type="text"
              placeholder="https://xxxx.supabase.co"
              value={supabaseUrl}
              onChange={(e) => setSupabaseUrl(e.target.value)}
              className="flex-1"
            />
          </div>
        </div>

        {/* Supabase key */}
        <div className="space-y-2">
          <Label htmlFor="supabase-key">Supabase API Key</Label>
          <div className="flex gap-2">
            <Input
              id="supabase-key"
              type="password"
              placeholder="eyJ..."
              value={supabaseKey}
              onChange={(e) => setSupabaseKey(e.target.value)}
              className="flex-1"
            />
            <Button
              variant="outline"
              size="sm"
              disabled={!supabaseUrl || !supabaseKey || testing === "supabase"}
              onClick={() => handleTest("supabase")}
            >
              {testing === "supabase" ? "测试中..." : "测试"}
            </Button>
          </div>
        </div>

        <Button onClick={handleSave} disabled={saving}>
          {saving ? "保存中..." : "保存配置"}
        </Button>
      </div>
    </div>
  )
}

export default function SettingsPage() {
  const [activeSection, setActiveSection] = useState<Section>("apikeys")

  return (
    <div className="p-6">
      <h1 className="text-2xl font-bold mb-6">设置</h1>

      <div className="flex gap-6">
        {/* Left nav */}
        <nav className="w-32 shrink-0">
          <div className="space-y-1">
            {SECTIONS.map((s) => (
              <button
                key={s.id}
                className={cn(
                  "w-full rounded-md px-3 py-2 text-left text-sm transition-colors",
                  activeSection === s.id
                    ? "bg-accent text-accent-foreground font-medium"
                    : "text-muted-foreground hover:bg-accent/50 hover:text-accent-foreground"
                )}
                onClick={() => setActiveSection(s.id)}
              >
                {s.label}
              </button>
            ))}
          </div>
        </nav>

        {/* Right content */}
        <div className="flex-1">
          {activeSection === "general" && <PlaceholderSection title="常规" />}
          {activeSection === "apikeys" && <ApiKeysSection />}
          {activeSection === "storage" && <PlaceholderSection title="存储" />}
          {activeSection === "privacy" && <PlaceholderSection title="隐私" />}
          {activeSection === "about" && <PlaceholderSection title="关于" />}
        </div>
      </div>
    </div>
  )
}
```

- [ ] **Step 2: Verify final build**

```bash
pnpm build
```

Expected: clean compile across all files.

- [ ] **Step 3: Final commit**

```bash
git add src/pages/settings/index.tsx
git commit -m "feat: implement settings page with API keys section and sonner toasts"
```

---

## Self-Review Checklist

**Spec coverage:**
- [x] Section 2 (deps): Task 1
- [x] Section 3 (file structure): All tasks map to correct paths
- [x] Section 4 (types): Task 2
- [x] Section 5 (mock tauri layer): Task 3 — all 9 functions implemented
- [x] Section 6 (store): Task 4 — all store fields and actions present
- [x] Section 7 (routing): Task 5 — all 4 routes under AppLayout
- [x] Section 8 (layout): Task 6 — nav width w-[180px], NavLink active state, CaptureStatusIndicator
- [x] Section 9.1 (overview): Task 8 — header, stats grid, timeline with dialog, connected sources badges
- [x] Section 9.2 (capture): Task 9 — control card, 3 tabs, polling, accordion, generateSummary mutation
- [x] Section 9.3 (connections): Task 10 — connected grid, available dashed cards, MCP dialog
- [x] Section 9.4 (settings): Task 11 — two-column, 5 sections, API keys with test/save
- [x] Section 10 (theme): shadcn init with New York/Neutral/0.625rem radius in Task 1

**Type consistency:**
- `Segment`, `Connection`, `AppConfig`, `Session` defined in Task 2 and imported by Tasks 3–11 via `@/types`
- `formatTimeRange`, `formatLastSync`, `formatDateHeader` defined in `utils.ts` Task 2
- Store shape (`capture.isRunning`, `capture.startedAt`, `capture.screenshotCount`, `stats.*`) consistent across Tasks 4, 8, 9
- All tauri functions (`listSegments`, `listConnections`, `generateSummary`, `listSessions`, `saveConfig`, `testGeminiConnection`, `testSupabaseConnection`, `startCapture`, `stopCapture`) defined in Task 3 and imported by pages
