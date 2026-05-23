# spec-10-onboarding.md

## 一、目标

实现 Corivo 的首次启动引导流程。完成本 spec 后：

- 用户首次启动 Corivo 时进入 onboarding 向导，而非直接进主界面。
- 向导分四步走完：欢迎 → API Keys → 屏幕权限 → 第一次捕获。
- 用户可以跳过任意步骤，但会有明确提示后果。
- 向导完成后标记状态持久化，后续启动直接进主界面。
- 用户随时可以从配置页重新触发向导（“重新跑引导”按钮）。
- 视觉设计比日常界面更友好、更有仪式感，但保持柔和友好风格一致。

## 二、不做什么

- ❌ 不做产品演示视频（第一版 onboarding 用文字 + 静态图示即可）。
- ❌ 不做交互式教程（不做“点这里点那里”的高亮引导）。
- ❌ 不做账号注册 / 邮箱订阅。
- ❌ 不做多语言（简中为主，英文 P1）。
- ❌ 不做 onboarding 指标上报（P1 做埋点再说）。
- ❌ 不做“推荐的 Supermemory 账号”等外部服务推荐。
- ❌ 不改主布局，onboarding 是独立 full-screen 视图，走完才进主 AppLayout。

## 三、成功标准

1. 用户全新安装 Corivo 并首次打开，看到 onboarding 欢迎页而不是主界面。
2. 四步引导的进度在顶部可见，每一步都能“上一步 / 下一步”。
3. 每一步都有“跳过”选项，但有合理的后果提示。
4. API Keys 步骤能现场填写并测试连接，成功后自动进入下一步。
5. 屏幕权限步骤能一键打开系统设置的权限面板，并实时检测权限变化。
6. 第一次捕获步骤能一键启动 `CaptureLoop` 并等待第一张截图出现。
7. 完成 onboarding 后跳转到概览页，而不是连接页，因为概览页才是日常主入口。
8. 配置页的「常规」子页底部有“重新跑引导”按钮，点击回到 onboarding。
9. 视觉上和日常界面有明显区分但不突兀（更大的留白、更中心的排版、更温和的文案）。

## 四、架构

### 4.1 路由结构

新增一个顶层路由 `/onboarding`，脱离 `AppLayout` 渲染（没有左侧导航），占满整个窗口。

- `/` → `AppLayout + OverviewPage`（已有）
- `/memory` → `AppLayout + MemoryPage`（已有）
- `/connections` → `AppLayout + ConnectionsPage`
- `/settings` → `AppLayout + SettingsPage`
- `/onboarding` → `OnboardingLayout + 步骤子路由`（新增，无 sidebar）
- `/onboarding/welcome`
- `/onboarding/api-keys`
- `/onboarding/permission`
- `/onboarding/first-capture`
- `/onboarding/done`

### 4.2 状态机

onboarding 的完成状态存在 `config.json` 里（不是 keychain，因为不敏感）：

```rust
// AppConfig 里新增
pub onboarding_completed: bool, // 默认 false
pub onboarding_version: u32,    // 默认 0，当前是 1
```

`onboarding_version` 的作用：未来如果要强制老用户重新跑引导（比如加了新步骤），把代码里的“当前版本”改成 `2`，检测到不一致就重新引导。P0 写好这个机制，但不触发重引导。

### 4.3 启动分发

App 启动时，`main.tsx` 判断：

```ts
if (!onboarding_completed || onboarding_version < CURRENT_VERSION) {
  navigate('/onboarding/welcome', { replace: true })
}
```

## 五、后端改动

### 5.1 AppConfig 扩展

`src-tauri/src/domain/config.rs`：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    // ... 已有字段

    #[serde(default)]
    pub onboarding_completed: bool,

    #[serde(default)]
    pub onboarding_version: u32,
}
```

不用改 `Default` 实现。`#[serde(default)]` 会让缺失字段走类型默认值（`bool` 是 `false`，`u32` 是 `0`），和想要的行为一致。

### 5.2 新增 Tauri commands

`src-tauri/src/commands/onboarding.rs`（新建）：

```rust
use crate::commands::config::AppState;
use tauri::State;

pub const CURRENT_ONBOARDING_VERSION: u32 = 1;

#[tauri::command]
pub async fn get_onboarding_state(
    state: State<'_, AppState>,
) -> Result<OnboardingState, String> {
    let cfg = state.config_service.get();
    Ok(OnboardingState {
        completed: cfg.app.onboarding_completed,
        version: cfg.app.onboarding_version,
        current_version: CURRENT_ONBOARDING_VERSION,
        needs_onboarding: !cfg.app.onboarding_completed
            || cfg.app.onboarding_version < CURRENT_ONBOARDING_VERSION,
    })
}

#[tauri::command]
pub async fn mark_onboarding_completed(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut cfg = state.config_service.get();
    cfg.app.onboarding_completed = true;
    cfg.app.onboarding_version = CURRENT_ONBOARDING_VERSION;
    state.config_service.update(cfg).map_err(String::from)
}

#[tauri::command]
pub async fn reset_onboarding(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut cfg = state.config_service.get();
    cfg.app.onboarding_completed = false;
    cfg.app.onboarding_version = 0;
    state.config_service.update(cfg).map_err(String::from)
}

#[derive(Debug, serde::Serialize)]
pub struct OnboardingState {
    pub completed: bool,
    pub version: u32,
    pub current_version: u32,
    pub needs_onboarding: bool,
}
```

注册到 `invoke_handler`：

```rust
commands::onboarding::get_onboarding_state,
commands::onboarding::mark_onboarding_completed,
commands::onboarding::reset_onboarding,
```

## 六、前端路由重构

### 6.1 新增 onboarding 路由树

`src/routes/onboarding.tsx`（新建，父路由）：

```tsx
import { createRoute, Outlet } from '@tanstack/react-router'
import { Route as RootRoute } from './__root'
import { OnboardingLayout } from '@/pages/onboarding/onboarding-layout'

export const Route = createRoute({
  getParentRoute: () => RootRoute,
  path: '/onboarding',
  component: () => (
    <OnboardingLayout>
      <Outlet />
    </OnboardingLayout>
  ),
})
```

关键：这个父路由的 `component` 用了自己的 `OnboardingLayout`，不是 `AppLayout`。这样 onboarding 期间没有 sidebar。

但是 `__root.tsx` 默认用了 `AppLayout`。需要改造根路由，让它基于路径决定用哪个 layout。

`src/routes/__root.tsx` 更新：

```tsx
import { createRootRoute, Outlet, useRouterState } from '@tanstack/react-router'
import { TanStackRouterDevtools } from '@tanstack/router-devtools'
import { AppLayout } from '@/app/layout'

export const Route = createRootRoute({
  component: RootComponent,
})

function RootComponent() {
  const state = useRouterState()
  const isOnboarding = state.location.pathname.startsWith('/onboarding')

  return (
    <>
      {isOnboarding ? (
        <Outlet />
      ) : (
        <AppLayout>
          <Outlet />
        </AppLayout>
      )}
      {import.meta.env.DEV && <TanStackRouterDevtools />}
    </>
  )
}
```

### 6.2 子路由定义

`src/routes/onboarding.welcome.tsx`：

```tsx
import { createRoute } from '@tanstack/react-router'
import { Route as OnboardingRoute } from './onboarding'
import { WelcomeStep } from '@/pages/onboarding/steps/welcome-step'

export const Route = createRoute({
  getParentRoute: () => OnboardingRoute,
  path: '/welcome',
  component: WelcomeStep,
})
```

类似地给其他四步各建一个文件：

- `onboarding.api-keys.tsx` → `ApiKeysStep`
- `onboarding.permission.tsx` → `PermissionStep`
- `onboarding.first-capture.tsx` → `FirstCaptureStep`
- `onboarding.done.tsx` → `DoneStep`

在 `src/app/router.tsx` 里把它们加入路由树：

```tsx
import { Route as OnboardingRoute } from '@/routes/onboarding'
import { Route as OnboardingWelcomeRoute } from '@/routes/onboarding.welcome'
import { Route as OnboardingApiKeysRoute } from '@/routes/onboarding.api-keys'
import { Route as OnboardingPermissionRoute } from '@/routes/onboarding.permission'
import { Route as OnboardingFirstCaptureRoute } from '@/routes/onboarding.first-capture'
import { Route as OnboardingDoneRoute } from '@/routes/onboarding.done'

const routeTree = RootRoute.addChildren([
  OverviewRoute,
  MemoryRoute,
  ConnectionsRoute,
  ScreenshotDetailRoute,
  SettingsRoute,
  OnboardingRoute.addChildren([
    OnboardingWelcomeRoute,
    OnboardingApiKeysRoute,
    OnboardingPermissionRoute,
    OnboardingFirstCaptureRoute,
    OnboardingDoneRoute,
  ]),
])
```

### 6.3 启动分发逻辑

`src/main.tsx` 更新，增加“判断是否需要 onboarding”的前置逻辑。不能直接在 `main.tsx` 里加 async 逻辑，用 React 组件包一层。

`src/app/app-boot.tsx`（新建）：

```tsx
import { useEffect, useState } from 'react'
import { RouterProvider } from '@tanstack/react-router'
import { getOnboardingState } from '@/lib/tauri'
import { router } from '@/app/router'

export function AppBoot() {
  const [ready, setReady] = useState(false)

  useEffect(() => {
    const init = async () => {
      try {
        const state = await getOnboardingState()
        if (state.needs_onboarding) {
          // 延迟到 router 初始化后 navigate
          router.navigate({ to: '/onboarding/welcome', replace: true })
        }
      } catch (e) {
        console.error('failed to load onboarding state', e)
      } finally {
        setReady(true)
      }
    }
    init()
  }, [])

  if (!ready) {
    return <BootSplash />
  }

  return <RouterProvider router={router} />
}

function BootSplash() {
  return (
    <div className="h-screen flex items-center justify-center bg-background">
      <div className="text-center space-y-3">
        <div className="mx-auto w-10 h-10 rounded-xl bg-primary/10 flex items-center justify-center">
          <span className="text-primary font-semibold">C</span>
        </div>
        <div className="text-xs text-muted-foreground">Corivo 启动中…</div>
      </div>
    </div>
  )
}
```

`src/main.tsx` 替换为：

```tsx
import React from 'react'
import ReactDOM from 'react-dom/client'
import { QueryClientProvider } from '@tanstack/react-query'
import { ReactQueryDevtools } from '@tanstack/react-query-devtools'
import { queryClient } from '@/lib/query-client'
import { AppBoot } from '@/app/app-boot'
import '@/styles/globals.css'

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <AppBoot />
      {import.meta.env.DEV && <ReactQueryDevtools />}
    </QueryClientProvider>
  </React.StrictMode>
)
```

### 6.4 Tauri API 封装

`src/lib/tauri.ts` 追加：

```ts
export interface OnboardingState {
  completed: boolean
  version: number
  current_version: number
  needs_onboarding: boolean
}

export async function getOnboardingState(): Promise<OnboardingState> {
  return invoke<OnboardingState>('get_onboarding_state')
}

export async function markOnboardingCompleted(): Promise<void> {
  return invoke<void>('mark_onboarding_completed')
}

export async function resetOnboarding(): Promise<void> {
  return invoke<void>('reset_onboarding')
}
```

## 七、OnboardingLayout 和共享组件

### 7.1 Layout

`src/pages/onboarding/onboarding-layout.tsx`（新建）：

```tsx
import { ReactNode } from 'react'
import { useRouterState } from '@tanstack/react-router'
import { StepIndicator } from './components/step-indicator'
import { Toaster } from '@/components/ui/sonner'

const STEP_ORDER = [
  '/onboarding/welcome',
  '/onboarding/api-keys',
  '/onboarding/permission',
  '/onboarding/first-capture',
  '/onboarding/done',
]

const STEP_LABELS = ['欢迎', 'API Keys', '权限', '捕获', '完成']

export function OnboardingLayout({ children }: { children: ReactNode }) {
  const { location } = useRouterState()
  const currentIndex = STEP_ORDER.indexOf(location.pathname)
  const activeIdx = currentIndex >= 0 ? currentIndex : 0

  const showIndicator = location.pathname !== '/onboarding/welcome'

  return (
    <div className="h-screen flex flex-col bg-background text-foreground antialiased">
      {showIndicator && (
        <header className="shrink-0 border-b border-border/60 bg-card/40">
          <div className="max-w-2xl mx-auto px-8 py-4">
            <StepIndicator
              labels={STEP_LABELS}
              activeIndex={activeIdx}
            />
          </div>
        </header>
      )}
      <main className="flex-1 overflow-auto">
        <div className="max-w-2xl mx-auto px-8 py-10 md:py-16 min-h-full">
          {children}
        </div>
      </main>
      <Toaster />
    </div>
  )
}
```

### 7.2 StepIndicator

`src/pages/onboarding/components/step-indicator.tsx`（新建）：

```tsx
import { Check } from 'lucide-react'
import { cn } from '@/lib/utils'

interface Props {
  labels: string[]
  activeIndex: number
}

export function StepIndicator({ labels, activeIndex }: Props) {
  return (
    <div className="flex items-center gap-2">
      {labels.map((label, i) => {
        const isActive = i === activeIndex
        const isDone = i < activeIndex
        const isLast = i === labels.length - 1

        return (
          <div key={i} className="flex items-center flex-1 gap-2 min-w-0">
            <div
              className={cn(
                'shrink-0 w-6 h-6 rounded-full flex items-center justify-center text-xs tabular-nums transition-colors',
                isActive &&
                  'bg-primary text-primary-foreground font-medium ring-2 ring-primary/30',
                isDone && 'bg-primary/80 text-primary-foreground',
                !isActive &&
                  !isDone &&
                  'bg-muted text-muted-foreground'
              )}
            >
              {isDone ? <Check className="w-3 h-3" /> : i + 1}
            </div>
            <span
              className={cn(
                'text-xs truncate',
                isActive && 'font-medium text-foreground',
                !isActive && 'text-muted-foreground'
              )}
            >
              {label}
            </span>
            {!isLast && (
              <div
                className={cn(
                  'flex-1 h-px min-w-[20px]',
                  i < activeIndex ? 'bg-primary/50' : 'bg-border'
                )}
              />
            )}
          </div>
        )
      })}
    </div>
  )
}
```

### 7.3 StepShell

统一每步的骨架（标题 + 主体 + 底部按钮行）：

`src/pages/onboarding/components/step-shell.tsx`（新建）：

```tsx
import { ReactNode } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { ChevronLeft } from 'lucide-react'
import { Button } from '@/components/ui/button'

interface Props {
  icon?: ReactNode
  title: string
  description?: string
  children: ReactNode
  prevTo?: string
  nextLabel?: string
  nextDisabled?: boolean
  onNext?: () => void | Promise<void>
  skipLabel?: string
  onSkip?: () => void
  showBack?: boolean
}

export function StepShell({
  icon,
  title,
  description,
  children,
  prevTo,
  nextLabel = '下一步',
  nextDisabled,
  onNext,
  skipLabel,
  onSkip,
  showBack = true,
}: Props) {
  const navigate = useNavigate()

  return (
    <div className="flex flex-col min-h-full">
      <div className="flex-1 space-y-8">
        <div className="space-y-4">
          {icon && (
            <div className="w-14 h-14 rounded-2xl bg-primary/10 flex items-center justify-center">
              <div className="text-primary">{icon}</div>
            </div>
          )}
          <div className="space-y-2">
            <h1 className="text-2xl font-semibold leading-tight">{title}</h1>
            {description && (
              <p className="text-sm text-muted-foreground leading-relaxed">
                {description}
              </p>
            )}
          </div>
        </div>

        <div>{children}</div>
      </div>

      <div className="pt-8 mt-auto flex items-center gap-3">
        {showBack && prevTo && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => navigate({ to: prevTo })}
            className="gap-1"
          >
            <ChevronLeft className="w-3 h-3" />
            上一步
          </Button>
        )}
        {skipLabel && onSkip && (
          <Button variant="ghost" size="sm" onClick={onSkip}>
            {skipLabel}
          </Button>
        )}
        <div className="flex-1" />
        {onNext && (
          <Button
            size="default"
            onClick={onNext}
            disabled={nextDisabled}
            className="min-w-[96px]"
          >
            {nextLabel}
          </Button>
        )}
      </div>
    </div>
  )
}
```

## 八、四个步骤的实现

### 8.1 WelcomeStep

`src/pages/onboarding/steps/welcome-step.tsx`：

```tsx
import { useNavigate } from '@tanstack/react-router'
import { Sparkles, Eye, Lock } from 'lucide-react'
import { Button } from '@/components/ui/button'

export function WelcomeStep() {
  const navigate = useNavigate()

  return (
    <div className="flex flex-col min-h-full text-center">
      <div className="flex-1 space-y-10 py-8">
        <div className="space-y-5">
          <div className="mx-auto w-16 h-16 rounded-2xl bg-primary/10 flex items-center justify-center">
            <span className="text-primary text-2xl font-semibold">C</span>
          </div>
          <div className="space-y-3">
            <h1 className="text-3xl font-semibold">欢迎使用 Corivo</h1>
            <p className="text-base text-muted-foreground leading-relaxed max-w-md mx-auto">
              一个安静陪着你工作的记忆伙伴。
              <br />
              它记得你做过什么，在你需要时提醒你。
            </p>
          </div>
        </div>

        <div className="grid grid-cols-1 md:grid-cols-3 gap-4 max-w-xl mx-auto pt-4">
          <Feature
            icon={<Eye />}
            title="安静地观察"
            text="每 15 秒截屏，用 AI 总结你在做什么"
          />
          <Feature
            icon={<Sparkles />}
            title="适时地提醒"
            text="发现有用的上下文时主动通知你"
          />
          <Feature
            icon={<Lock />}
            title="本地优先"
            text="截图存在你自己的电脑，你完全控制"
          />
        </div>

        <div className="text-xs text-muted-foreground/80 max-w-md mx-auto leading-relaxed pt-4">
          接下来大约 3 分钟，我们一起完成初始配置。
          <br />
          你需要准备：Gemini API Key 和 Supermemory API Key。
        </div>
      </div>

      <div className="pt-8">
        <Button
          size="lg"
          onClick={() => navigate({ to: '/onboarding/api-keys' })}
          className="min-w-[140px]"
        >
          开始配置
        </Button>
      </div>
    </div>
  )
}

function Feature({
  icon,
  title,
  text,
}: {
  icon: React.ReactNode
  title: string
  text: string
}) {
  return (
    <div className="rounded-xl border border-border/60 bg-card p-4 text-left">
      <div className="w-8 h-8 rounded-lg bg-primary/10 flex items-center justify-center mb-3 text-primary [&>svg]:w-4 [&>svg]:h-4">
        {icon}
      </div>
      <div className="text-sm font-medium mb-1">{title}</div>
      <div className="text-xs text-muted-foreground leading-relaxed">
        {text}
      </div>
    </div>
  )
}
```

### 8.2 ApiKeysStep

`src/pages/onboarding/steps/api-keys-step.tsx`：

```tsx
import { useEffect, useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { Key, Loader2, CheckCircle2 } from 'lucide-react'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Button } from '@/components/ui/button'
import { StepShell } from '../components/step-shell'
import {
  getApiKeyStatus,
  saveGeminiApiKey,
  saveSupermemoryApiKey,
  testGeminiConnection,
  testSupermemoryConnection,
} from '@/lib/tauri'

export function ApiKeysStep() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const { data: status } = useQuery({
    queryKey: ['api-key-status'],
    queryFn: getApiKeyStatus,
  })

  const [geminiKey, setGeminiKey] = useState('')
  const [smKey, setSmKey] = useState('')
  const [geminiTested, setGeminiTested] = useState(
    status?.gemini_configured ?? false
  )
  const [smTested, setSmTested] = useState(
    status?.supermemory_configured ?? false
  )
  const [testing, setTesting] = useState<'gemini' | 'sm' | null>(null)

  useEffect(() => {
    if (status?.gemini_configured) setGeminiTested(true)
    if (status?.supermemory_configured) setSmTested(true)
  }, [status])

  const handleTestAndSaveGemini = async () => {
    if (!geminiKey) {
      toast.error('请先输入 Gemini API Key')
      return
    }
    setTesting('gemini')
    try {
      await testGeminiConnection(geminiKey)
      await saveGeminiApiKey(geminiKey)
      queryClient.invalidateQueries({ queryKey: ['api-key-status'] })
      setGeminiTested(true)
      toast.success('Gemini 已连接并保存')
    } catch (e) {
      toast.error(`Gemini: ${e}`)
      setGeminiTested(false)
    } finally {
      setTesting(null)
    }
  }

  const handleTestAndSaveSm = async () => {
    if (!smKey) {
      toast.error('请先输入 Supermemory API Key')
      return
    }
    setTesting('sm')
    try {
      await testSupermemoryConnection(smKey)
      await saveSupermemoryApiKey(smKey)
      queryClient.invalidateQueries({ queryKey: ['api-key-status'] })
      setSmTested(true)
      toast.success('Supermemory 已连接并保存')
    } catch (e) {
      toast.error(`Supermemory: ${e}`)
      setSmTested(false)
    } finally {
      setTesting(null)
    }
  }

  const canGoNext = geminiTested && smTested

  return (
    <StepShell
      icon={<Key className="w-7 h-7" />}
      title="连接你的 API"
      description="Corivo 需要两把钥匙才能工作：Gemini 用来看懂你的屏幕，Supermemory 用来记住。两把钥匙都只存在你的电脑 keychain 里，不会上传到任何地方。"
      prevTo="/onboarding/welcome"
      nextLabel="继续"
      nextDisabled={!canGoNext}
      onNext={() => navigate({ to: '/onboarding/permission' })}
      skipLabel="稍后配置（不推荐）"
      onSkip={() => {
        toast.warning('未配置 API Key，大部分功能无法使用，可以随时在配置页补充')
        navigate({ to: '/onboarding/permission' })
      }}
    >
      <div className="space-y-6">
        <KeyField
          label="Gemini API Key"
          description="用于把你的屏幕截图总结成文字"
          placeholder="AIza..."
          value={geminiKey}
          onChange={setGeminiKey}
          tested={geminiTested}
          testing={testing === 'gemini'}
          onTestAndSave={handleTestAndSaveGemini}
          docsHref="https://ai.google.dev/gemini-api/docs/api-key"
          docsLabel="获取 Gemini API Key"
        />
        <KeyField
          label="Supermemory API Key"
          description="你的记忆总结都存在你自己的 Supermemory 账号里"
          placeholder="sm_..."
          value={smKey}
          onChange={setSmKey}
          tested={smTested}
          testing={testing === 'sm'}
          onTestAndSave={handleTestAndSaveSm}
          docsHref="https://supermemory.ai/dashboard"
          docsLabel="获取 Supermemory API Key"
        />
      </div>
    </StepShell>
  )
}

function KeyField({
  label,
  description,
  placeholder,
  value,
  onChange,
  tested,
  testing,
  onTestAndSave,
  docsHref,
  docsLabel,
}: {
  label: string
  description: string
  placeholder: string
  value: string
  onChange: (v: string) => void
  tested: boolean
  testing: boolean
  onTestAndSave: () => void
  docsHref: string
  docsLabel: string
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-baseline justify-between">
        <Label className="text-sm font-medium">{label}</Label>
        {tested && (
          <span className="inline-flex items-center gap-1 text-xs text-green-700">
            <CheckCircle2 className="w-3 h-3" />
            已连接
          </span>
        )}
      </div>
      <p className="text-xs text-muted-foreground">{description}</p>
      <div className="flex gap-2">
        <Input
          type="password"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={tested ? '••••••••••••••••' : placeholder}
          className="flex-1"
        />
        <Button
          variant={tested ? 'outline' : 'default'}
          onClick={onTestAndSave}
          disabled={testing}
          className="min-w-[100px]"
        >
          {testing && <Loader2 className="w-3 h-3 mr-1.5 animate-spin" />}
          {tested ? '重新测试' : '测试并保存'}
        </Button>
      </div>
      <a
        href={docsHref}
        target="_blank"
        rel="noreferrer"
        className="inline-block text-xs text-muted-foreground hover:text-foreground underline underline-offset-2"
      >
        {docsLabel} ↗
      </a>
    </div>
  )
}
```

### 8.3 PermissionStep

`src/pages/onboarding/steps/permission-step.tsx`：

```tsx
import { useNavigate } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import {
  MonitorSmartphone,
  CheckCircle2,
  XCircle,
  ExternalLink,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { StepShell } from '../components/step-shell'
import {
  checkScreenRecordingPermission,
  openSystemSettingsPrivacy,
} from '@/lib/tauri'

export function PermissionStep() {
  const navigate = useNavigate()

  const { data: hasPermission, refetch } = useQuery({
    queryKey: ['screen-permission'],
    queryFn: checkScreenRecordingPermission,
    refetchInterval: 2000,
  })

  return (
    <StepShell
      icon={<MonitorSmartphone className="w-7 h-7" />}
      title="授权屏幕录制"
      description="Corivo 需要屏幕录制权限才能定时截图。授权后所有截图都只保存在你的本地。"
      prevTo="/onboarding/api-keys"
      nextLabel="继续"
      nextDisabled={!hasPermission}
      onNext={() => navigate({ to: '/onboarding/first-capture' })}
      skipLabel="稍后授权"
      onSkip={() => navigate({ to: '/onboarding/first-capture' })}
    >
      <div className="space-y-5">
        <div
          className={
            'rounded-xl border p-4 ' +
            (hasPermission
              ? 'border-green-200 bg-green-50/50'
              : 'border-border bg-card')
          }
        >
          <div className="flex items-start gap-3">
            {hasPermission ? (
              <CheckCircle2 className="w-5 h-5 text-green-600 shrink-0 mt-0.5" />
            ) : (
              <XCircle className="w-5 h-5 text-muted-foreground shrink-0 mt-0.5" />
            )}
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium">
                {hasPermission ? '权限已授予' : '权限未授予'}
              </div>
              <p className="text-xs text-muted-foreground mt-1 leading-relaxed">
                {hasPermission
                  ? 'Corivo 可以正常截图了。点下一步继续。'
                  : '打开系统设置 → 隐私与安全性 → 屏幕录制，勾选 Corivo。'}
              </p>
            </div>
            {!hasPermission && (
              <Button
                size="sm"
                onClick={() => openSystemSettingsPrivacy()}
                className="gap-1.5 shrink-0"
              >
                <ExternalLink className="w-3 h-3" />
                打开系统设置
              </Button>
            )}
          </div>
        </div>

        {!hasPermission && (
          <div className="rounded-xl border border-amber-200 bg-amber-50/50 p-4 text-xs text-amber-900 leading-relaxed">
            <div className="font-medium mb-1">注意</div>
            <p>
              在 macOS 上首次授权后需要重启 Corivo 才能生效。
              重启后这个引导会自动恢复到这一步。
            </p>
            <Button
              variant="outline"
              size="sm"
              className="mt-3"
              onClick={() => refetch()}
            >
              我已授权，重新检测
            </Button>
          </div>
        )}

        <div className="text-xs text-muted-foreground">
          Corivo 只在你点“开始捕获”时才会真实截图。
          即使权限在这里授予了，只要没有主动启动捕获，不会产生任何截图。
        </div>
      </div>
    </StepShell>
  )
}
```

### 8.4 FirstCaptureStep

`src/pages/onboarding/steps/first-capture-step.tsx`：

```tsx
import { useEffect, useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { useMutation, useQuery } from '@tanstack/react-query'
import { listen } from '@tauri-apps/api/event'
import { Camera, Loader2, CheckCircle2, Play } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { StepShell } from '../components/step-shell'
import {
  startCapture,
  stopCapture,
  getCaptureStatus,
  listSessions,
} from '@/lib/tauri'

type Phase = 'idle' | 'capturing' | 'got-first' | 'got-summary'

export function FirstCaptureStep() {
  const navigate = useNavigate()
  const [phase, setPhase] = useState<Phase>('idle')

  const { data: status } = useQuery({
    queryKey: ['capture-status'],
    queryFn: getCaptureStatus,
    refetchInterval: 2000,
  })

  const { data: sessions, refetch: refetchSessions } = useQuery({
    queryKey: ['sessions'],
    queryFn: listSessions,
    refetchInterval: 3000,
  })

  const currentSession = sessions?.find(
    (s) => s.id === status?.current_session_id
  )
  const shotCount = currentSession?.screenshot_count ?? 0

  // 订阅 memory-added event，作为“完成第一批处理”的信号
  useEffect(() => {
    const unlisten = listen('memory-added', () => {
      setPhase('got-summary')
    })
    return () => {
      unlisten.then((un) => un())
    }
  }, [])

  useEffect(() => {
    if (phase === 'capturing' && shotCount > 0) {
      setPhase('got-first')
    }
  }, [shotCount, phase])

  const startMut = useMutation({
    mutationFn: startCapture,
    onSuccess: () => {
      setPhase('capturing')
      refetchSessions()
    },
  })

  const handleStart = () => {
    startMut.mutate()
  }

  return (
    <StepShell
      icon={<Camera className="w-7 h-7" />}
      title="试一次捕获"
      description="最后一步。点下方按钮，Corivo 会开始截屏、总结、存入记忆。等看到第一条记忆产生再结束。"
      prevTo="/onboarding/permission"
      nextLabel={phase === 'got-summary' ? '完成引导' : '跳过'}
      onNext={() => navigate({ to: '/onboarding/done' })}
    >
      <div className="space-y-5">
        {phase === 'idle' && (
          <div className="rounded-xl border border-border bg-card p-6 text-center">
            <div className="mb-4">
              <div className="text-sm mb-1">现在 Corivo 会：</div>
              <ul className="text-xs text-muted-foreground space-y-1 inline-block text-left">
                <li>1. 每 15 秒截一张屏幕</li>
                <li>2. 攒够 5 张后用 Gemini 总结</li>
                <li>3. 把总结存到你的 Supermemory</li>
                <li>4. 搜索相关记忆，判断是否推送</li>
              </ul>
            </div>
            <Button
              size="lg"
              onClick={handleStart}
              disabled={startMut.isPending}
              className="gap-2 min-w-[160px]"
            >
              {startMut.isPending ? (
                <Loader2 className="w-4 h-4 animate-spin" />
              ) : (
                <Play className="w-4 h-4" />
              )}
              开始第一次捕获
            </Button>
            <p className="text-xs text-muted-foreground mt-3 leading-relaxed">
              第一条记忆大约需要 75 秒。你可以在等待时正常使用电脑。
            </p>
          </div>
        )}

        {phase === 'capturing' && (
          <ProgressCard
            title="正在捕获..."
            subtitle={`已截取 ${shotCount}/5 张`}
            done={false}
          />
        )}

        {phase === 'got-first' && (
          <ProgressCard
            title="截图进行中"
            subtitle={`已截取 ${shotCount}/5 张，马上会触发总结`}
            done={false}
          />
        )}

        {phase === 'got-summary' && (
          <div className="rounded-xl border border-green-200 bg-green-50/60 p-6 text-center">
            <CheckCircle2 className="w-10 h-10 text-green-600 mx-auto mb-3" />
            <div className="text-sm font-medium mb-1">第一条记忆已产生！</div>
            <p className="text-xs text-muted-foreground leading-relaxed">
              Corivo 已经帮你记下了过去一分钟的工作。
              <br />
              你可以继续让它在后台运行，也可以现在结束。
            </p>
            <Button
              variant="outline"
              size="sm"
              onClick={() => stopCapture()}
              className="mt-4"
            >
              先停止捕获
            </Button>
          </div>
        )}
      </div>
    </StepShell>
  )
}

function ProgressCard({
  title,
  subtitle,
  done,
}: {
  title: string
  subtitle: string
  done: boolean
}) {
  return (
    <div className="rounded-xl border border-border bg-card p-6 text-center">
      <div className="mx-auto w-10 h-10 mb-3">
        {done ? (
          <CheckCircle2 className="w-10 h-10 text-green-600" />
        ) : (
          <Loader2 className="w-10 h-10 text-primary animate-spin" />
        )}
      </div>
      <div className="text-sm font-medium mb-1">{title}</div>
      <p className="text-xs text-muted-foreground">{subtitle}</p>
    </div>
  )
}
```

### 8.5 DoneStep

`src/pages/onboarding/steps/done-step.tsx`：

```tsx
import { useNavigate } from '@tanstack/react-router'
import { useMutation } from '@tanstack/react-query'
import { toast } from 'sonner'
import { Sparkles, ArrowRight } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { markOnboardingCompleted } from '@/lib/tauri'

export function DoneStep() {
  const navigate = useNavigate()

  const completeMut = useMutation({
    mutationFn: markOnboardingCompleted,
    onSuccess: () => {
      navigate({ to: '/', replace: true })
    },
    onError: (e: string) => {
      toast.error(`保存失败: ${e}`)
    },
  })

  return (
    <div className="flex flex-col min-h-full text-center">
      <div className="flex-1 flex flex-col items-center justify-center space-y-6 py-10">
        <div className="w-16 h-16 rounded-2xl bg-primary/10 flex items-center justify-center">
          <Sparkles className="w-7 h-7 text-primary" />
        </div>
        <div className="space-y-3">
          <h1 className="text-2xl font-semibold">Corivo 已就绪</h1>
          <p className="text-sm text-muted-foreground leading-relaxed max-w-md mx-auto">
            接下来 Corivo 会默默工作。你可以随时在连接页启动或暂停捕获，
            <br />
            在概览页看到每天发生了什么，在记忆页找回过去任何时刻。
          </p>
        </div>

        <div className="grid grid-cols-1 md:grid-cols-3 gap-3 max-w-xl w-full text-left pt-4">
          <HintCard title="概览" text="今天发生了什么" />
          <HintCard title="记忆" text="找回过去任何时刻" />
          <HintCard title="配置" text="随时调整行为" />
        </div>

        <div className="text-xs text-muted-foreground/80 max-w-md pt-4">
          你可以在「配置 → 常规」里重新启动这个引导，或者调整截图间隔和批次大小。
        </div>
      </div>

      <div className="pt-8">
        <Button
          size="lg"
          onClick={() => completeMut.mutate()}
          disabled={completeMut.isPending}
          className="gap-2 min-w-[160px]"
        >
          进入 Corivo
          <ArrowRight className="w-4 h-4" />
        </Button>
      </div>
    </div>
  )
}

function HintCard({ title, text }: { title: string; text: string }) {
  return (
    <div className="rounded-lg border border-border/60 bg-card p-3">
      <div className="text-sm font-medium mb-0.5">{title}</div>
      <div className="text-xs text-muted-foreground">{text}</div>
    </div>
  )
}
```

## 九、配置页入口

在 `GeneralSection`（spec-09 已定义）的底部追加一个区域：

```tsx
// 在 FieldGroup "捕获默认值" 之后追加

<FieldGroup title="引导">
  <div className="flex items-center justify-between">
    <div className="flex-1 pr-4">
      <Label className="text-sm">重新跑引导</Label>
      <p className="text-xs text-muted-foreground mt-0.5">
        重新进入欢迎向导，不会清除你已有的数据。
      </p>
    </div>
    <Button
      variant="outline"
      size="sm"
      onClick={async () => {
        await resetOnboarding()
        navigate({ to: '/onboarding/welcome' })
      }}
    >
      重新跑
    </Button>
  </div>
</FieldGroup>
```

## 十、任务分解

暂时无法在飞书文档外展示此内容。

## 十一、验收清单

- 全新安装 Corivo 首次打开，看到 `onboarding/welcome` 页。
- 手动改 `config.json` 把 `onboarding_completed` 改回 `false`，重启能再次进入。
- 点“开始配置”进入 `api-keys`，顶部出现步骤条。
- 步骤条当前步骤高亮，已完成步骤显示对勾。
- 填对 Gemini key 点“测试并保存”，显示“已连接” badge。
- 两把 key 都连接后，“继续”按钮变成可点。
- 未配置就点“稍后配置”，toast 提示，仍能进入下一步。
- 进入 permission 步骤，如果系统没授权，显示“权限未授予”。
- 点“打开系统设置”弹出系统设置的屏幕录制页。
- 授权后回到 app，2 秒内自动检测到权限已授予，UI 变成绿色。
- 进入 first-capture 步骤，点“开始第一次捕获”后状态变成“正在捕获 1/5”。
- 约 75 秒后（15s × 5）看到“第一条记忆已产生”的绿色卡片。
- 点“完成引导”跳转到 `/`（概览页），且 URL 不再是 `/onboarding/*`。
- 重启 app 直接进入概览页，不再出现 onboarding。
- 进入配置 → 常规，底部看到“重新跑引导”按钮。
- 点击后回到 `onboarding/welcome`，之前的数据仍然在。
- 所有步骤的“上一步”按钮都能正确返回。
- 所有步骤的“跳过”都能跳过（但 first-capture 的“完成引导”即使没捕获也能点）。
