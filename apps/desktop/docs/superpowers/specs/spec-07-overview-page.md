# spec-07-overview-page.md

## 一、目标

实现 Corivo 的核心 UI 页面——**概览页时间线**。这是用户每天打开 app 最想看的页面，也是验证"截图 → 总结 → 记忆 → 展示"完整闭环用户价值的关键一环。

完成本 spec 后：

- 概览页默认展示今天的活动时间线，按时间倒序
- 每个时间段（一批截图对应的一条记忆）是一张卡片
- 卡片显示时间范围 + AI 总结 + 来源信息
- 点击卡片展开详情：完整总结 + 原始截图缩略图
- 支持切换日期查看历史（URL search param 同步）
- 数据源是 Supermemory（通过 spec-03 的 MemoryService）
- 柔和友好风格的视觉呈现

## 二、不做什么

- ❌ 不做关键指标卡（O-2 已砍）
- ❌ 不做连接状态区（O-3 已砍）
- ❌ 不做筛选（按来源、按标签）
- ❌ 不做时间线上的推送历史标记（P1）
- ❌ 不做"重新生成某个 segment"（需要额外的本地元数据，本 spec 不引入）
- ❌ 不做活动类型分组（P1 加了 activity_type 再说）
- ❌ 不做日报 / 周报生成（后续 spec）
- ❌ 不做编辑 / 删除记忆（只读视图）

## 三、成功标准

1. 启动 app，概览页默认显示今天的记忆时间线
2. 如果今天还没有任何记忆，显示一个友好的空状态引导用户去连接页启动截图
3. 有记忆时，按 occurred_at 倒序显示卡片列表，每张卡片包含时间段和总结文字
4. 点击卡片展开详情 Dialog，显示完整总结、来源 session 的截图缩略图
5. 顶部日期选择器能切换到前一天、后一天、指定日期
6. URL 的 `date` search param 随日期切换同步变化，刷新页面状态保留
7. 正在捕获中时，页面能看到 pipeline 刚产出的新记忆自动出现（不需要手动刷新）
8. 加载时有 loading 骨架屏，加载失败有明确错误提示
9. 视觉符合柔和友好风格（奶油底、暖色卡片、柔和圆角）

## 四、数据流

```
用户进入概览页
    ↓
读取 URL 里的 date search param（无则默认今天）
    ↓
计算 date 对应的 UTC 起止时间 [date_start, date_end)
    ↓
useQuery: memory_service.list({
  limit: 200,
  date_from: date_start,
  date_to: date_end,
  source_kind: Some("screenshot")
})
    ↓
按 occurred_at DESC 排序
    ↓
渲染 TimelineList
    ↓
每 30 秒自动 refetch（refetchInterval）
    ↓
Pipeline 产出新记忆后 emit Tauri event,
UI 订阅后立即 invalidate 这个 query
```

**关键点**：
- **前端每 30 秒刷一次是兜底**，pipeline 完成后主动 emit event 才是主路径
- **date_from / date_to 必须按用户本地时区计算**，否则跨时区的用户看到的"今天"会错
- **source_kind 过滤"screenshot"**：未来可能有其他来源的记忆（比如 Claude Code），概览页只展示屏幕活动

## 五、后端改动

### 5.1 MemoryService 已有 list 方法

`spec-03` 定义的 `list` 方法签名已经够用：

```rust
pub async fn list(&self, query: ListQuery) -> Result<MemoryPage>
```

`ListQuery` 包含 `date_from` / `date_to` / `source_kind`，完美契合。

### 5.2 新增 Tauri command：list_today_segments

虽然前端理论上可以直接调 `list_memories`，但做一个语义更清晰的 command 方便后续演进：

`src-tauri/src/commands/memory.rs` 追加：

```rust
use chrono::{DateTime, Duration, Utc};

#[tauri::command]
pub async fn list_memories_by_date(
    state: State<'_, MemoryAppState>,
    date_iso: String,           // "2026-04-10"
    timezone_offset_mins: i32,  // 本地时区相对 UTC 的分钟偏移 (北京 = 480)
    source_kind: Option<String>,
) -> Result<MemoryPage, String> {
    let date_from = parse_local_day_to_utc(&date_iso, timezone_offset_mins, false)
        .map_err(|e| format!("解析日期失败: {}", e))?;
    let date_to = date_from + Duration::days(1);
    
    let query = ListQuery {
        limit: 200,
        offset: 0,
        date_from: Some(date_from),
        date_to: Some(date_to),
        source_kind,
    };
    
    state.memory_service.list(query).await.map_err(Into::into)
}

fn parse_local_day_to_utc(
    date_iso: &str,
    tz_offset_mins: i32,
    end_of_day: bool,
) -> Result<DateTime<Utc>, String> {
    use chrono::NaiveDate;
    let naive = NaiveDate::parse_from_str(date_iso, "%Y-%m-%d")
        .map_err(|e| format!("bad date: {}", e))?;
    let local_midnight_secs = naive.and_hms_opt(0, 0, 0)
        .ok_or("bad date")?
        .and_utc()
        .timestamp();
    // local midnight → UTC: 减去时区偏移
    let utc_secs = local_midnight_secs - (tz_offset_mins as i64) * 60;
    let dt = DateTime::from_timestamp(utc_secs, 0).ok_or("bad timestamp")?;
    if end_of_day {
        Ok(dt + Duration::days(1))
    } else {
        Ok(dt)
    }
}
```

**为什么前端传时区偏移而不是后端读系统时区**：Tauri 后端读系统时区跨平台不稳，前端 `new Date().getTimezoneOffset()` 稳定可靠。

注册 command：

```rust
// lib.rs invoke_handler 增加
commands::memory::list_memories_by_date,
```

### 5.3 新增 Tauri event：memory-added

让 pipeline 处理完一批后主动通知前端刷新：

`src-tauri/src/events/mod.rs`（新建）：

```rust
use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Clone, Debug, Serialize)]
pub struct MemoryAddedEvent {
    pub memory_id: String,
    pub session_id: String,
    pub occurred_at: String,  // RFC3339
}

pub fn emit_memory_added(app: &AppHandle, event: MemoryAddedEvent) {
    if let Err(e) = app.emit("memory-added", event) {
        tracing::warn!("failed to emit memory-added: {:?}", e);
    }
}
```

在 `lib.rs` 里声明 mod：

```rust
mod events;
```

修改 `MvpPipeline::process_batch` 在成功写入 Supermemory 后触发 event：

```rust
// services/mvp_pipeline.rs

use crate::events::{emit_memory_added, MemoryAddedEvent};

// 在 memory.add 成功后
match self.memory.add(memory_input).await {
    Ok(id) => {
        tracing::info!("写入 Supermemory 成功: {}", id);
        emit_memory_added(&self.app_handle, MemoryAddedEvent {
            memory_id: id,
            session_id: batch.session_id.clone(),
            occurred_at: Utc::now().to_rfc3339(),
        });
    }
    Err(e) => {
        tracing::error!("写入 Supermemory 失败: {:?}", e);
    }
}
```

## 六、前端实现

### 6.1 类型和工具

`src/lib/tauri.ts` 追加：

```ts
import type { MemoryPage } from './types'

export async function listMemoriesByDate(
  dateIso: string,
  sourceKind?: string
): Promise<MemoryPage> {
  const timezoneOffsetMins = -new Date().getTimezoneOffset()
  return invoke<MemoryPage>('list_memories_by_date', {
    dateIso,
    timezoneOffsetMins,
    sourceKind: sourceKind ?? null,
  })
}
```

`src/lib/format.ts`（新建）：

```ts
import { format, formatDistanceToNow, isToday, isYesterday } from 'date-fns'
import { zhCN } from 'date-fns/locale'

export function formatTimeRange(occurredAt: string, batchMinutes = 2): string {
  const end = new Date(occurredAt)
  const start = new Date(end.getTime() - batchMinutes * 60 * 1000)
  return `${format(start, 'HH:mm')} — ${format(end, 'HH:mm')}`
}

export function formatDateHeader(date: Date): string {
  if (isToday(date)) return '今天'
  if (isYesterday(date)) return '昨天'
  return format(date, 'M月d日 EEEE', { locale: zhCN })
}

export function formatRelativeTime(occurredAt: string): string {
  return formatDistanceToNow(new Date(occurredAt), {
    addSuffix: true,
    locale: zhCN,
  })
}

export function toISODate(date: Date): string {
  const y = date.getFullYear()
  const m = String(date.getMonth() + 1).padStart(2, '0')
  const d = String(date.getDate()).padStart(2, '0')
  return `${y}-${m}-${d}`
}

export function fromISODate(iso: string): Date {
  const [y, m, d] = iso.split('-').map(Number)
  return new Date(y, m - 1, d)
}
```

### 6.2 hook：useTodayMemories

`src/hooks/use-memories.ts`（新建）：

```ts
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { listen } from '@tauri-apps/api/event'
import { listMemoriesByDate } from '@/lib/tauri'

export function useMemoriesByDate(dateIso: string) {
  const queryClient = useQueryClient()

  const query = useQuery({
    queryKey: ['memories', 'by-date', dateIso],
    queryFn: () => listMemoriesByDate(dateIso, 'screenshot'),
    refetchInterval: 30_000,
    staleTime: 10_000,
  })

  useEffect(() => {
    const unlistenPromise = listen('memory-added', () => {
      queryClient.invalidateQueries({
        queryKey: ['memories', 'by-date', dateIso],
      })
    })
    return () => {
      unlistenPromise.then((un) => un())
    }
  }, [dateIso, queryClient])

  return query
}
```

### 6.3 概览页主组件

`src/pages/overview/overview-page.tsx`（完全重写，替换 spec-01 的占位符）：

```tsx
import { useSearch, useNavigate } from '@tanstack/react-router'
import { Route as OverviewRoute } from '@/routes/overview'
import { toISODate, fromISODate, formatDateHeader } from '@/lib/format'
import { useMemoriesByDate } from '@/hooks/use-memories'
import { DateSwitcher } from './components/date-switcher'
import { TimelineList } from './components/timeline-list'
import { TimelineEmpty } from './components/timeline-empty'
import { TimelineSkeleton } from './components/timeline-skeleton'

export function OverviewPage() {
  const search = useSearch({ from: OverviewRoute.id })
  const navigate = useNavigate()

  const currentDate = search.date ? fromISODate(search.date) : new Date()
  const dateIso = toISODate(currentDate)

  const { data, isLoading, error } = useMemoriesByDate(dateIso)

  const onDateChange = (newDate: Date) => {
    navigate({
      to: '/',
      search: { date: toISODate(newDate) },
    })
  }

  return (
    <div className="space-y-6">
      <header>
        <div className="flex items-baseline gap-3 mb-1">
          <h1 className="text-xl font-semibold">
            {formatDateHeader(currentDate)}
          </h1>
          <span className="text-xs text-muted-foreground">{dateIso}</span>
        </div>
        <DateSwitcher value={currentDate} onChange={onDateChange} />
      </header>

      {isLoading && <TimelineSkeleton />}

      {error && (
        <div className="rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive">
          加载失败：{String(error)}
        </div>
      )}

      {data && data.items.length === 0 && !isLoading && (
        <TimelineEmpty date={currentDate} />
      )}

      {data && data.items.length > 0 && (
        <TimelineList memories={data.items} />
      )}
    </div>
  )
}
```

### 6.4 子组件：DateSwitcher

`src/pages/overview/components/date-switcher.tsx`：

```tsx
import { addDays, isToday, isFuture } from 'date-fns'
import { ChevronLeft, ChevronRight, Calendar } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { Calendar as CalendarComponent } from '@/components/ui/calendar'
import { useState } from 'react'

interface Props {
  value: Date
  onChange: (date: Date) => void
}

export function DateSwitcher({ value, onChange }: Props) {
  const [open, setOpen] = useState(false)

  const goPrev = () => onChange(addDays(value, -1))
  const goNext = () => {
    const next = addDays(value, 1)
    if (!isFuture(next)) onChange(next)
  }
  const goToday = () => onChange(new Date())

  const canGoNext = !isFuture(addDays(value, 1))

  return (
    <div className="flex items-center gap-1">
      <Button variant="ghost" size="icon" onClick={goPrev} className="h-8 w-8">
        <ChevronLeft className="w-4 h-4" />
      </Button>

      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button variant="ghost" size="sm" className="h-8 px-3 gap-2">
            <Calendar className="w-3 h-3" />
            选择日期
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-auto p-0" align="start">
          <CalendarComponent
            mode="single"
            selected={value}
            onSelect={(d) => {
              if (d) {
                onChange(d)
                setOpen(false)
              }
            }}
            disabled={(d) => isFuture(d)}
          />
        </PopoverContent>
      </Popover>

      <Button
        variant="ghost"
        size="icon"
        onClick={goNext}
        disabled={!canGoNext}
        className="h-8 w-8"
      >
        <ChevronRight className="w-4 h-4" />
      </Button>

      {!isToday(value) && (
        <Button variant="ghost" size="sm" onClick={goToday} className="h-8 ml-2">
          回到今天
        </Button>
      )}
    </div>
  )
}
```

需要先安装 shadcn 组件：

```bash
pnpm dlx shadcn@latest add popover calendar
```

### 6.5 子组件：TimelineList + TimelineCard

`src/pages/overview/components/timeline-list.tsx`：

```tsx
import type { Memory } from '@/lib/types'
import { TimelineCard } from './timeline-card'

interface Props {
  memories: Memory[]
}

export function TimelineList({ memories }: Props) {
  return (
    <div className="space-y-3">
      {memories.map((m) => (
        <TimelineCard key={m.id} memory={m} />
      ))}
    </div>
  )
}
```

`src/pages/overview/components/timeline-card.tsx`：

```tsx
import { useState } from 'react'
import { format } from 'date-fns'
import { Camera } from 'lucide-react'
import { Card } from '@/components/ui/card'
import type { Memory } from '@/lib/types'
import { TimelineDetailDialog } from './timeline-detail-dialog'

interface Props {
  memory: Memory
}

export function TimelineCard({ memory }: Props) {
  const [open, setOpen] = useState(false)
  const time = new Date(memory.occurred_at)

  return (
    <>
      <Card
        className="p-4 cursor-pointer hover:bg-accent/30 transition-colors"
        onClick={() => setOpen(true)}
      >
        <div className="flex items-start gap-3">
          <div className="shrink-0 w-14 pt-0.5">
            <div className="text-xs font-medium tabular-nums">
              {format(time, 'HH:mm')}
            </div>
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-sm leading-relaxed text-foreground">
              {memory.content}
            </p>
            <div className="flex items-center gap-1.5 mt-2 text-xs text-muted-foreground">
              <Camera className="w-3 h-3" />
              <span>截图</span>
              {memory.source.type === 'screenshot' && (
                <span className="opacity-60">
                  · session {memory.source.session_id.slice(-8)}
                </span>
              )}
            </div>
          </div>
        </div>
      </Card>

      <TimelineDetailDialog
        memory={memory}
        open={open}
        onOpenChange={setOpen}
      />
    </>
  )
}
```

### 6.6 子组件：详情 Dialog

`src/pages/overview/components/timeline-detail-dialog.tsx`：

```tsx
import { useQuery } from '@tanstack/react-query'
import { format } from 'date-fns'
import { convertFileSrc } from '@tauri-apps/api/core'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { ScrollArea } from '@/components/ui/scroll-area'
import { listSessionScreenshots } from '@/lib/tauri'
import type { Memory } from '@/lib/types'

interface Props {
  memory: Memory
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function TimelineDetailDialog({ memory, open, onOpenChange }: Props) {
  const sessionId =
    memory.source.type === 'screenshot' ? memory.source.session_id : null

  const { data: screenshots } = useQuery({
    queryKey: ['session-screenshots', sessionId],
    queryFn: () => listSessionScreenshots(sessionId!),
    enabled: open && !!sessionId,
  })

  const occurredTime = new Date(memory.occurred_at)

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl max-h-[85vh] overflow-hidden flex flex-col">
        <DialogHeader>
          <DialogTitle className="font-medium">
            {format(occurredTime, 'M月d日 HH:mm')} 的记录
          </DialogTitle>
        </DialogHeader>

        <ScrollArea className="flex-1 pr-4 -mr-4">
          <div className="space-y-5">
            <section>
              <div className="text-xs text-muted-foreground mb-2">总结</div>
              <p className="text-sm leading-relaxed whitespace-pre-wrap">
                {memory.content}
              </p>
            </section>

            {memory.tags.length > 0 && (
              <section>
                <div className="text-xs text-muted-foreground mb-2">标签</div>
                <div className="flex flex-wrap gap-1.5">
                  {memory.tags.map((t) => (
                    <span
                      key={t}
                      className="px-2 py-0.5 text-xs rounded-md bg-accent/60 text-accent-foreground"
                    >
                      {t}
                    </span>
                  ))}
                </div>
              </section>
            )}

            {sessionId && (
              <section>
                <div className="text-xs text-muted-foreground mb-2">
                  原始截图（共 {screenshots?.length ?? 0} 张）
                </div>
                {screenshots && screenshots.length > 0 ? (
                  <div className="grid grid-cols-3 gap-2">
                    {screenshots.map((s) => (
                      <div
                        key={s.filename}
                        className="rounded-md border border-border overflow-hidden bg-muted/30"
                      >
                        <img
                          src={convertFileSrc(s.absolute_path)}
                          alt={s.filename}
                          loading="lazy"
                          className="w-full aspect-video object-cover"
                        />
                      </div>
                    ))}
                  </div>
                ) : (
                  <div className="text-xs text-muted-foreground py-4">
                    该 session 的原始截图已被清理
                  </div>
                )}
              </section>
            )}

            <section className="pt-2 border-t border-border">
              <div className="text-xs text-muted-foreground">
                记忆 ID: <span className="font-mono">{memory.id}</span>
              </div>
              {memory.source.type === 'screenshot' && (
                <div className="text-xs text-muted-foreground mt-1">
                  来源 session:{' '}
                  <span className="font-mono">{memory.source.session_id}</span>
                </div>
              )}
            </section>
          </div>
        </ScrollArea>
      </DialogContent>
    </Dialog>
  )
}
```

需要安装的 shadcn 组件：

```bash
pnpm dlx shadcn@latest add dialog scroll-area
```

### 6.7 子组件：空状态 + 骨架屏

`src/pages/overview/components/timeline-empty.tsx`：

```tsx
import { Link } from '@tanstack/react-router'
import { CameraOff, ArrowRight } from 'lucide-react'
import { Card } from '@/components/ui/card'
import { isToday } from 'date-fns'

interface Props {
  date: Date
}

export function TimelineEmpty({ date }: Props) {
  const today = isToday(date)

  return (
    <Card className="p-8 text-center bg-muted/30 border-dashed">
      <div className="mx-auto w-10 h-10 rounded-full bg-background flex items-center justify-center mb-3">
        <CameraOff className="w-5 h-5 text-muted-foreground" />
      </div>
      <div className="text-sm font-medium mb-1">
        {today ? '今天还没有记录' : '这一天没有记录'}
      </div>
      <p className="text-xs text-muted-foreground mb-4 max-w-sm mx-auto">
        {today
          ? '去连接页启动屏幕捕获，Corivo 会自动整理你接下来的活动。'
          : '这一天你没有开启捕获，或记录已被清理。'}
      </p>
      {today && (
        <Link
          to="/connections"
          className="inline-flex items-center gap-1 text-xs font-medium text-primary hover:underline"
        >
          去连接页 <ArrowRight className="w-3 h-3" />
        </Link>
      )}
    </Card>
  )
}
```

`src/pages/overview/components/timeline-skeleton.tsx`：

```tsx
import { Card } from '@/components/ui/card'

export function TimelineSkeleton() {
  return (
    <div className="space-y-3">
      {[1, 2, 3].map((i) => (
        <Card key={i} className="p-4">
          <div className="flex items-start gap-3">
            <div className="w-14 h-3 bg-muted rounded animate-pulse" />
            <div className="flex-1 space-y-2">
              <div className="h-3 bg-muted rounded animate-pulse w-full" />
              <div className="h-3 bg-muted rounded animate-pulse w-4/5" />
              <div className="h-3 bg-muted/60 rounded animate-pulse w-24 mt-3" />
            </div>
          </div>
        </Card>
      ))}
    </div>
  )
}
```

## 七、视觉细节

根据柔和友好风格（spec-01 已定义的暖化主题），时间线页面有几个视觉考量：

**卡片的悬浮状态**：`hover:bg-accent/30`——accent 是暖奶油金，30% 透明度让鼠标悬停时卡片"微微发光"而不抢视线。

**时间戳用 `tabular-nums`**：等宽数字让不同行的时间对齐，视觉节奏更稳。

**Camera 图标 3x3**：故意小一号（`w-3 h-3`），这是"辅助信息"不是"主内容"。

**空状态卡片用 `border-dashed`**：暗示"这里应该有内容但现在没有"的占位感。

**骨架屏动画只在首次加载出现**：react-query 的 `isLoading` 只在第一次无缓存请求时为 true，后续 refetch 不显示骨架屏，体验更顺。

## 八、任务分解

| 会话 | 范围 | 完成判定 |
|---|---|---|
| 1 | 后端 `list_memories_by_date` command + 时区转换 + event 定义 | cargo build 通过，手动 invoke 能拿到数据 |
| 2 | `emit_memory_added` 接入 pipeline | pipeline 完成后前端能收到 event |
| 3 | 前端 format.ts + useMemoriesByDate hook | 可以在 demo 页面用 hook 拉数据 |
| 4 | 安装 shadcn popover / calendar / dialog / scroll-area | 组件可用 |
| 5 | overview-page.tsx + DateSwitcher | 页面能切日期、URL 同步 |
| 6 | TimelineList + TimelineCard | 卡片能正确渲染 |
| 7 | TimelineDetailDialog + 截图缩略图 | 详情 Dialog 完整 |
| 8 | TimelineEmpty + TimelineSkeleton | 空态和 loading 态正确 |
| 9 | 端到端测试 + 视觉微调 | 跑通完整体验 |

**总计预估 2-3 天。** 前 6 步让 Claude Code 做，7-9 步你自己过一遍调视觉。

## 九、验收清单

- [ ] 启动 app 默认进入概览页，标题显示「今天」+ 当天日期
- [ ] 如果今天没有记忆，显示空状态卡片，有去连接页的链接
- [ ] 启动截图捕获后等第一批处理完，页面自动出现第一条卡片（不用手动刷新）
- [ ] 卡片按时间倒序排列，最新的在最上面
- [ ] 卡片上时间显示精确到分钟，内容是 Gemini 总结的前 80 字左右
- [ ] 点击卡片弹出详情 Dialog，显示完整总结 + 该 session 的所有截图缩略图
- [ ] 截图缩略图能正常加载（说明 capabilities 配置正确）
- [ ] 点击「←」切到昨天，能看到昨天的记忆（如果有的话）
- [ ] 点击「→」切到明天时按钮禁用（不能看未来）
- [ ] 点击日历图标打开 popover，能选任意过去日期
- [ ] 切换日期后 URL 变成 `/?date=2026-04-09`
- [ ] 直接刷新带 `?date=` 的 URL，页面能正确恢复到那一天
- [ ] 点「回到今天」按钮回到今天
- [ ] 连续开着页面 1 分钟，没有新记忆时列表不会闪烁（说明 staleTime 生效）
- [ ] 视觉上整体观感是奶油暖色、圆润、舒适，没有冷灰感

## 十、坑点预警

1. **时区偏移计算**：`getTimezoneOffset()` 返回的是"UTC 相对本地时间的分钟数"，**北京返回 -480**（因为 UTC 比北京慢 8 小时）。我在 `listMemoriesByDate` 里用 `-new Date().getTimezoneOffset()` 取反，让后端拿到的是"本地时间相对 UTC 的偏移"（北京 +480）。**一定要测一下不同时区的用户看到的"今天"是否正确**。

2. **Supermemory 的 occurred_at 可能不是 pipeline 写入的值**：Supermemory 可能会覆盖或忽略用户提供的字段。spec-03 里我们把 occurred_at 存进了 `metadata.occurred_at`，反序列化时优先用它。如果发现时间线顺序错乱，检查 SupermemoryProvider 的 `from_supermemory_response` 方法。

3. **search params 的类型安全**：TanStack Router 的 `useSearch({ from: OverviewRoute.id })` 返回已经被 `validateSearch` 解析的类型化对象。`date` 是 `string | undefined`。如果你在别处写 `navigate({ search: { date: new Date() } })` 会编译失败，类型系统会救你。

4. **日历组件的本地化**：shadcn 的 calendar 基于 react-day-picker，默认英文。想要中文要传 `locale={zhCN}` 和可能的 `weekStartsOn={1}`（中国周一开始）。本 spec 略过这个细节，你上手后自己调。

5. **Dialog 里的 ScrollArea 高度**：Dialog 的 content 有 `max-h-[85vh]`，内部 ScrollArea 要 `flex-1` 才能撑开。弄错了会出现"内容能滚但滚不到底"的尴尬。

6. **截图 lazy loading**：`<img loading="lazy">` 在 Tauri 的 webview 里行为和浏览器一致，但实际上所有图片都在同一个 session 目录下本地文件，不会有明显延迟。保持 `lazy` 是好习惯。

7. **Dialog 打开时才请求截图**：`useQuery` 的 `enabled: open && !!sessionId` 避免关着 Dialog 时就发请求。注意 react-query 的 cache 会保留关闭后的数据，第二次打开同一张卡片就是缓存命中，很快。

8. **长文本换行**：Gemini 偶尔会返回带换行符的总结（尤其是做列表时）。卡片上用 `text-sm leading-relaxed` 即可自然换行；详情 Dialog 用 `whitespace-pre-wrap` 保留换行符。

9. **大量记忆的性能**：一天 200 条记忆是 `limit` 上限。如果用户开着捕获 8 小时、batch_size=5、interval=15s，一天大约 192 条，正好卡在 200 的边界。未来要改成分页或虚拟滚动（react-virtuoso）。P0 接受这个限制。

10. **Supermemory 搜索延迟**：刚 `add` 完的记忆 `list` 不一定立刻能拿到（索引构建有延迟）。所以 refetchInterval 30 秒 + memory-added event invalidate 两种机制叠加。实测如果发现"推送收到了但概览页还是空"的情况，延迟 invalidate 2-3 秒再触发。

## 十一、产出物

完成 spec-07 后你应该有：

- 一个视觉舒适、功能完整的概览页
- 完整的日期切换逻辑（前后日、日历、回到今天）
- URL 状态同步（可分享、可刷新恢复）
- 实时感知新记忆产出（event 驱动）
- 一个可复用的 `useMemoriesByDate` hook，未来记忆页也能复用
- 截图详情查看能力（通过 Dialog 打开 session 的原图）
- 空状态和 loading 态的完整覆盖

**核心验证完成**：到这一步，用户安装 Corivo → 启动截图 → 等 75 秒 → 在概览页看到第一条时间线卡片，**这是整个产品最核心的情感时刻**。这个路径丝滑，产品就立住了一半。
