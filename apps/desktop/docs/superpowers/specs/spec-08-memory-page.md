# spec-08-memory-page.md

## 一、目标

实现 Corivo 的**记忆页**。这是用户查找历史内容、回溯过去任何时刻工作状态的入口。完成本 spec 后：

- 记忆页显示所有来源的记忆，按时间倒序
- 顶部有搜索框，支持关键词语义搜索
- 无限滚动加载更多
- 搜索结果与列表视图切换流畅
- 点击任意记忆条目查看完整内容和原始截图（复用概览页的 Dialog）
- URL search params 同步搜索关键词和分页状态
- 柔和友好风格

## 二、不做什么

- ❌ 不做按来源筛选（Story M-3，P1）
- ❌ 不做删除单条记忆（Story M-4，P1 —— 一旦做删除就要做确认 Dialog、错误恢复、列表刷新等一堆逻辑）
- ❌ 不做编辑记忆
- ❌ 不做批量操作
- ❌ 不做记忆导出
- ❌ 不做标签管理
- ❌ 不做聚类 / 分组视图（按日期分组、按主题聚类等）
- ❌ 不做"相关记忆"推荐

## 三、成功标准

1. 打开记忆页，看到所有记忆按时间倒序排列的列表
2. 滚动到列表底部时自动加载下一页，loading 指示明显
3. 顶部搜索框输入关键词（300ms 防抖）后，列表变成搜索结果视图
4. 清空搜索框或点"返回列表"回到全量列表视图
5. 搜索关键词同步到 URL `?q=xxx`，刷新页面状态保留
6. 点击任意记忆条目弹出详情 Dialog（复用概览页的 TimelineDetailDialog）
7. 空状态友好：全新用户看到引导去启动捕获；搜索无结果看到"没找到"
8. 搜索框高亮匹配的关键词（简单实现，不做语义高亮）
9. 视觉和概览页协调但有区分（列表密度稍高、有时间分组感）

## 四、数据流

```
用户打开记忆页
    ↓
读取 URL 的 q / page search params
    ↓
┌──────────────────┬──────────────────────┐
│ 有 q 参数(搜索)  │ 无 q 参数(列表视图)  │
├──────────────────┼──────────────────────┤
│ useInfiniteQuery │ useInfiniteQuery     │
│ search_memories  │ list_memories        │
│ 参数: query, page│ 参数: limit, offset  │
└──────────────────┴──────────────────────┘
    ↓
渲染 MemoryList（共用组件）
    ↓
滚动到底部 → fetchNextPage
    ↓
点击条目 → 打开 Dialog（复用 TimelineDetailDialog）
    ↓
memory-added event → invalidate 列表 query（不打扰搜索结果）
```

**关键设计**：
- **搜索和列表是两个独立的 query key**：切换时不会互相污染缓存
- **搜索用 infinite query 是为了未来扩展**：Supermemory 的 search API 目前返回 `Vec<Memory>` 不分页，但接口先按无限滚动设计，后续 SearchQuery 加 offset 字段就能接
- **memory-added event 只刷新列表不刷新搜索**：用户正在搜索"支付"时，新产生的截图记忆不应该打断他

## 五、后端改动

### 5.1 扩展 search_memories 支持分页（预留）

Supermemory 的 search API 当前不支持 offset 分页，但我们让接口先具备分页能力。前端传 offset，后端目前忽略 offset 直接调一次拿全部（Supermemory 的 search 一般限制返回 20-50 条），未来 Supermemory 支持分页再接入。

`src-tauri/src/commands/memory.rs` 更新 search_memories：

```rust
#[tauri::command]
pub async fn search_memories(
    state: State<'_, MemoryAppState>,
    query: String,
    limit: usize,
    offset: usize,
    source_kind: Option<String>,
) -> Result<MemorySearchResult, String> {
    let q = SearchQuery { query, limit, source_kind };
    let results = state
        .memory_service
        .search(q)
        .await
        .map_err(|e| String::from(e))?;
    
    // 简单实现：前端传的 offset 先忽略（Supermemory 不支持）
    // 但返回结构包含 has_more 方便未来扩展
    let _ = offset;
    let has_more = false;
    
    Ok(MemorySearchResult {
        items: results,
        has_more,
    })
}

#[derive(Debug, serde::Serialize)]
pub struct MemorySearchResult {
    pub items: Vec<crate::providers::memory::Memory>,
    pub has_more: bool,
}
```

list_memories 已经支持 offset，不改。

### 5.2 新增：list_memories_paginated

现有的 list_memories 签名有 limit 和 offset，够用。但为了前端语义清晰，新增一个包装：

```rust
#[tauri::command]
pub async fn list_all_memories(
    state: State<'_, MemoryAppState>,
    limit: usize,
    offset: usize,
    source_kind: Option<String>,
) -> Result<MemoryPage, String> {
    let query = ListQuery {
        limit,
        offset,
        date_from: None,
        date_to: None,
        source_kind,
    };
    state.memory_service.list(query).await.map_err(Into::into)
}
```

注册到 invoke_handler：

```rust
commands::memory::list_all_memories,
```

原有的 `list_memories` 保留（spec-03 已注册），`list_all_memories` 只是语义更清晰的别名。前端只用 `list_all_memories`。

## 六、前端实现

### 6.1 类型与 API 封装

`src/lib/types.ts` 追加：

```ts
export interface MemorySearchResult {
  items: Memory[]
  has_more: boolean
}
```

`src/lib/tauri.ts` 追加：

```ts
import type { MemoryPage, MemorySearchResult } from './types'

export async function listAllMemories(
  limit: number,
  offset: number,
  sourceKind?: string
): Promise<MemoryPage> {
  return invoke<MemoryPage>('list_all_memories', {
    limit,
    offset,
    sourceKind: sourceKind ?? null,
  })
}

export async function searchMemoriesPaginated(
  query: string,
  limit: number,
  offset: number,
  sourceKind?: string
): Promise<MemorySearchResult> {
  return invoke<MemorySearchResult>('search_memories', {
    query,
    limit,
    offset,
    sourceKind: sourceKind ?? null,
  })
}
```

### 6.2 路由 search params 扩展

`src/routes/memory.tsx` 更新 schema：

```ts
import { createRoute } from '@tanstack/react-router'
import { z } from 'zod'
import { Route as RootRoute } from './__root'
import { MemoryPage } from '@/pages/memory/memory-page'

const memorySearchSchema = z.object({
  q: z.string().optional(),
})

export const Route = createRoute({
  getParentRoute: () => RootRoute,
  path: '/memory',
  validateSearch: memorySearchSchema,
  component: MemoryPage,
})
```

去掉了 `page`——分页用无限滚动管理状态在组件里，不需要放 URL。

### 6.3 useDebouncedValue hook

`src/hooks/use-debounced-value.ts`（新建）：

```ts
import { useEffect, useState } from 'react'

export function useDebouncedValue<T>(value: T, delay: number): T {
  const [debounced, setDebounced] = useState(value)

  useEffect(() => {
    const t = setTimeout(() => setDebounced(value), delay)
    return () => clearTimeout(t)
  }, [value, delay])

  return debounced
}
```

### 6.4 useMemories hooks

`src/hooks/use-memory-list.ts`（新建）：

```ts
import {
  useInfiniteQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { useEffect } from 'react'
import { listen } from '@tauri-apps/api/event'
import { listAllMemories, searchMemoriesPaginated } from '@/lib/tauri'
import type { Memory } from '@/lib/types'

const PAGE_SIZE = 30

export function useAllMemories() {
  const queryClient = useQueryClient()

  const query = useInfiniteQuery({
    queryKey: ['memories', 'all'],
    queryFn: ({ pageParam }) =>
      listAllMemories(PAGE_SIZE, pageParam, 'screenshot'),
    initialPageParam: 0,
    getNextPageParam: (lastPage, allPages) => {
      if (!lastPage.has_more) return undefined
      return allPages.length * PAGE_SIZE
    },
    staleTime: 30_000,
  })

  useEffect(() => {
    const unlistenPromise = listen('memory-added', () => {
      queryClient.invalidateQueries({ queryKey: ['memories', 'all'] })
    })
    return () => {
      unlistenPromise.then((un) => un())
    }
  }, [queryClient])

  return {
    ...query,
    items: query.data?.pages.flatMap((p) => p.items) ?? ([] as Memory[]),
  }
}

export function useMemorySearch(query: string) {
  const enabled = query.trim().length > 0

  const result = useInfiniteQuery({
    queryKey: ['memories', 'search', query],
    queryFn: ({ pageParam }) =>
      searchMemoriesPaginated(query, PAGE_SIZE, pageParam, 'screenshot'),
    initialPageParam: 0,
    getNextPageParam: (lastPage, allPages) => {
      if (!lastPage.has_more) return undefined
      return allPages.length * PAGE_SIZE
    },
    staleTime: 60_000,
    enabled,
  })

  return {
    ...result,
    items: result.data?.pages.flatMap((p) => p.items) ?? ([] as Memory[]),
    enabled,
  }
}
```

### 6.5 useInfiniteScroll hook

用 IntersectionObserver 检测"到达底部"。

`src/hooks/use-infinite-scroll.ts`（新建）：

```ts
import { useEffect, useRef } from 'react'

interface Options {
  hasNextPage: boolean
  isFetchingNextPage: boolean
  onLoadMore: () => void
  rootMargin?: string
}

export function useInfiniteScroll({
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
  rootMargin = '200px',
}: Options) {
  const sentinelRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const el = sentinelRef.current
    if (!el || !hasNextPage || isFetchingNextPage) return

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting) {
          onLoadMore()
        }
      },
      { rootMargin }
    )

    observer.observe(el)
    return () => observer.disconnect()
  }, [hasNextPage, isFetchingNextPage, onLoadMore, rootMargin])

  return sentinelRef
}
```

### 6.6 记忆页主组件

`src/pages/memory/memory-page.tsx`（完全重写）：

```tsx
import { useState, useEffect } from 'react'
import { useSearch, useNavigate } from '@tanstack/react-router'
import { Search, X } from 'lucide-react'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import { Route as MemoryRoute } from '@/routes/memory'
import { useDebouncedValue } from '@/hooks/use-debounced-value'
import { MemoryListView } from './components/memory-list-view'
import { MemorySearchView } from './components/memory-search-view'

export function MemoryPage() {
  const search = useSearch({ from: MemoryRoute.id })
  const navigate = useNavigate()

  const [inputValue, setInputValue] = useState(search.q ?? '')
  const debouncedQuery = useDebouncedValue(inputValue, 300)

  // 把 debounced 值同步回 URL
  useEffect(() => {
    const trimmed = debouncedQuery.trim()
    if (trimmed === (search.q ?? '')) return
    navigate({
      to: '/memory',
      search: trimmed ? { q: trimmed } : {},
      replace: true,
    })
  }, [debouncedQuery, navigate, search.q])

  const handleClear = () => {
    setInputValue('')
  }

  const activeQuery = debouncedQuery.trim()
  const isSearching = activeQuery.length > 0

  return (
    <div className="space-y-6">
      <header>
        <h1 className="text-xl font-semibold mb-1">记忆</h1>
        <p className="text-xs text-muted-foreground">
          Corivo 帮你记住的所有工作上下文
        </p>
      </header>

      <div className="relative">
        <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground pointer-events-none" />
        <Input
          value={inputValue}
          onChange={(e) => setInputValue(e.target.value)}
          placeholder="搜索记忆…比如"auth 重构"、"王总提案""
          className="pl-9 pr-9 h-10"
          autoFocus
        />
        {inputValue && (
          <Button
            variant="ghost"
            size="icon"
            onClick={handleClear}
            className="absolute right-1 top-1/2 -translate-y-1/2 h-7 w-7"
          >
            <X className="w-3 h-3" />
          </Button>
        )}
      </div>

      {isSearching ? (
        <MemorySearchView query={activeQuery} />
      ) : (
        <MemoryListView />
      )}
    </div>
  )
}
```

### 6.7 MemoryListView

`src/pages/memory/components/memory-list-view.tsx`：

```tsx
import { Loader2 } from 'lucide-react'
import { useAllMemories } from '@/hooks/use-memory-list'
import { useInfiniteScroll } from '@/hooks/use-infinite-scroll'
import { MemoryList } from './memory-list'
import { MemoryEmpty } from './memory-empty'
import { MemorySkeleton } from './memory-skeleton'

export function MemoryListView() {
  const {
    items,
    isLoading,
    isFetchingNextPage,
    hasNextPage,
    fetchNextPage,
    error,
  } = useAllMemories()

  const sentinelRef = useInfiniteScroll({
    hasNextPage: hasNextPage ?? false,
    isFetchingNextPage,
    onLoadMore: () => fetchNextPage(),
  })

  if (isLoading) return <MemorySkeleton count={6} />

  if (error) {
    return (
      <div className="rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive">
        加载失败：{String(error)}
      </div>
    )
  }

  if (items.length === 0) {
    return <MemoryEmpty variant="no-data" />
  }

  return (
    <div className="space-y-3">
      <MemoryList memories={items} />

      <div ref={sentinelRef} className="flex items-center justify-center py-4">
        {isFetchingNextPage && (
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <Loader2 className="w-3 h-3 animate-spin" />
            加载更多…
          </div>
        )}
        {!hasNextPage && items.length > 10 && (
          <div className="text-xs text-muted-foreground">没有更多了</div>
        )}
      </div>
    </div>
  )
}
```

### 6.8 MemorySearchView

`src/pages/memory/components/memory-search-view.tsx`：

```tsx
import { Loader2 } from 'lucide-react'
import { useMemorySearch } from '@/hooks/use-memory-list'
import { useInfiniteScroll } from '@/hooks/use-infinite-scroll'
import { MemoryList } from './memory-list'
import { MemoryEmpty } from './memory-empty'
import { MemorySkeleton } from './memory-skeleton'

interface Props {
  query: string
}

export function MemorySearchView({ query }: Props) {
  const {
    items,
    isLoading,
    isFetchingNextPage,
    hasNextPage,
    fetchNextPage,
    error,
  } = useMemorySearch(query)

  const sentinelRef = useInfiniteScroll({
    hasNextPage: hasNextPage ?? false,
    isFetchingNextPage,
    onLoadMore: () => fetchNextPage(),
  })

  if (isLoading) return <MemorySkeleton count={4} />

  if (error) {
    return (
      <div className="rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive">
        搜索失败：{String(error)}
      </div>
    )
  }

  if (items.length === 0) {
    return <MemoryEmpty variant="no-results" query={query} />
  }

  return (
    <div className="space-y-3">
      <div className="text-xs text-muted-foreground">
        找到 {items.length} 条与「{query}」相关的记忆
      </div>

      <MemoryList memories={items} highlight={query} />

      <div ref={sentinelRef} className="flex items-center justify-center py-4">
        {isFetchingNextPage && (
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <Loader2 className="w-3 h-3 animate-spin" />
            加载更多…
          </div>
        )}
      </div>
    </div>
  )
}
```

### 6.9 MemoryList 和 MemoryItem

`src/pages/memory/components/memory-list.tsx`：

```tsx
import { useState } from 'react'
import type { Memory } from '@/lib/types'
import { MemoryItem } from './memory-item'
import { TimelineDetailDialog } from '@/pages/overview/components/timeline-detail-dialog'

interface Props {
  memories: Memory[]
  highlight?: string
}

export function MemoryList({ memories, highlight }: Props) {
  const [selected, setSelected] = useState<Memory | null>(null)

  return (
    <>
      <div className="space-y-2">
        {memories.map((m) => (
          <MemoryItem
            key={m.id}
            memory={m}
            highlight={highlight}
            onClick={() => setSelected(m)}
          />
        ))}
      </div>

      {selected && (
        <TimelineDetailDialog
          memory={selected}
          open={true}
          onOpenChange={(o) => !o && setSelected(null)}
        />
      )}
    </>
  )
}
```

`src/pages/memory/components/memory-item.tsx`：

```tsx
import { format, isToday, isYesterday, isSameYear } from 'date-fns'
import { zhCN } from 'date-fns/locale'
import { Camera } from 'lucide-react'
import { Card } from '@/components/ui/card'
import type { Memory } from '@/lib/types'

interface Props {
  memory: Memory
  highlight?: string
  onClick: () => void
}

export function MemoryItem({ memory, highlight, onClick }: Props) {
  const time = new Date(memory.occurred_at)
  const label = formatSmartDate(time)

  return (
    <Card
      className="p-3.5 cursor-pointer hover:bg-accent/30 transition-colors"
      onClick={onClick}
    >
      <div className="flex items-start gap-3">
        <div className="shrink-0 w-20 pt-0.5">
          <div className="text-xs text-muted-foreground tabular-nums">
            {label.top}
          </div>
          <div className="text-xs font-medium tabular-nums mt-0.5">
            {label.bottom}
          </div>
        </div>
        <div className="flex-1 min-w-0">
          <p className="text-sm leading-relaxed text-foreground line-clamp-3">
            {highlight ? (
              <HighlightedText text={memory.content} keyword={highlight} />
            ) : (
              memory.content
            )}
          </p>
          <div className="flex items-center gap-1.5 mt-1.5 text-xs text-muted-foreground">
            <Camera className="w-3 h-3" />
            <span>截图</span>
            {memory.tags.length > 0 && (
              <>
                <span className="opacity-40">·</span>
                <span className="opacity-80">{memory.tags.slice(0, 2).join(' · ')}</span>
              </>
            )}
          </div>
        </div>
      </div>
    </Card>
  )
}

function formatSmartDate(d: Date): { top: string; bottom: string } {
  const now = new Date()
  if (isToday(d)) return { top: '今天', bottom: format(d, 'HH:mm') }
  if (isYesterday(d)) return { top: '昨天', bottom: format(d, 'HH:mm') }
  if (isSameYear(d, now)) {
    return { top: format(d, 'M月d日', { locale: zhCN }), bottom: format(d, 'HH:mm') }
  }
  return { top: format(d, 'yyyy.M.d', { locale: zhCN }), bottom: format(d, 'HH:mm') }
}

function HighlightedText({ text, keyword }: { text: string; keyword: string }) {
  if (!keyword.trim()) return <>{text}</>

  // 不区分大小写匹配
  const escaped = keyword.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const regex = new RegExp(`(${escaped})`, 'gi')
  const parts = text.split(regex)

  return (
    <>
      {parts.map((part, i) =>
        regex.test(part) ? (
          <mark
            key={i}
            className="bg-primary/15 text-foreground px-0.5 rounded-sm"
          >
            {part}
          </mark>
        ) : (
          <span key={i}>{part}</span>
        )
      )}
    </>
  )
}
```

### 6.10 空状态

`src/pages/memory/components/memory-empty.tsx`：

```tsx
import { Link } from '@tanstack/react-router'
import { Archive, SearchX, ArrowRight } from 'lucide-react'
import { Card } from '@/components/ui/card'

interface Props {
  variant: 'no-data' | 'no-results'
  query?: string
}

export function MemoryEmpty({ variant, query }: Props) {
  if (variant === 'no-results') {
    return (
      <Card className="p-8 text-center bg-muted/30 border-dashed">
        <div className="mx-auto w-10 h-10 rounded-full bg-background flex items-center justify-center mb-3">
          <SearchX className="w-5 h-5 text-muted-foreground" />
        </div>
        <div className="text-sm font-medium mb-1">没有找到相关记忆</div>
        <p className="text-xs text-muted-foreground max-w-sm mx-auto">
          「{query}」没有匹配的记忆，换一个关键词试试看。
        </p>
      </Card>
    )
  }

  return (
    <Card className="p-8 text-center bg-muted/30 border-dashed">
      <div className="mx-auto w-10 h-10 rounded-full bg-background flex items-center justify-center mb-3">
        <Archive className="w-5 h-5 text-muted-foreground" />
      </div>
      <div className="text-sm font-medium mb-1">还没有记忆</div>
      <p className="text-xs text-muted-foreground mb-4 max-w-sm mx-auto">
        去连接页启动屏幕捕获，Corivo 会自动记录并整理你的工作内容。
      </p>
      <Link
        to="/connections"
        className="inline-flex items-center gap-1 text-xs font-medium text-primary hover:underline"
      >
        去连接页 <ArrowRight className="w-3 h-3" />
      </Link>
    </Card>
  )
}
```

### 6.11 骨架屏

`src/pages/memory/components/memory-skeleton.tsx`：

```tsx
import { Card } from '@/components/ui/card'

interface Props {
  count?: number
}

export function MemorySkeleton({ count = 5 }: Props) {
  return (
    <div className="space-y-2">
      {Array.from({ length: count }).map((_, i) => (
        <Card key={i} className="p-3.5">
          <div className="flex items-start gap-3">
            <div className="shrink-0 w-20 space-y-1.5">
              <div className="h-3 w-10 bg-muted rounded animate-pulse" />
              <div className="h-3 w-12 bg-muted rounded animate-pulse" />
            </div>
            <div className="flex-1 space-y-2">
              <div className="h-3 bg-muted rounded animate-pulse w-full" />
              <div className="h-3 bg-muted rounded animate-pulse w-5/6" />
              <div className="h-3 bg-muted/60 rounded animate-pulse w-20 mt-2" />
            </div>
          </div>
        </Card>
      ))}
    </div>
  )
}
```

## 七、视觉和交互细节

**和概览页的差别**：

| 维度 | 概览页 TimelineCard | 记忆页 MemoryItem |
|---|---|---|
| 时间区域 | 单行 HH:mm | 双行（日期 + 时间）|
| 宽度 | w-14（约 56px） | w-20（约 80px） |
| 正文行数 | 不截断 | `line-clamp-3` 最多 3 行 |
| padding | p-4 | p-3.5 |
| 信息密度 | 宽松 | 紧凑 |

**原因**：概览页单日浏览不用太紧凑；记忆页跨越很多天，紧凑能让用户一屏看到更多条目。

**`line-clamp-3` 需要 Tailwind plugin**：确认 tailwind 配置里有 `@tailwindcss/line-clamp`（Tailwind 3.3+ 已经内置，不用额外配）。

**搜索防抖 300ms**：输入时视觉无感知，停顿时立刻响应。超过 300ms 再停顿就能感觉到"跟不上"。

**highlight 实现选简单路径**：正则分段 + `<mark>` 包裹，不做复杂的多关键词高亮、模糊匹配高亮。对技术用户够用。

## 八、任务分解

| 会话 | 范围 | 完成判定 |
|---|---|---|
| 1 | 后端 `list_all_memories` command 注册 | cargo build 通过 |
| 2 | 前端 types/tauri.ts 更新 + 路由 schema 更新 | 编译通过 |
| 3 | useDebouncedValue / useInfiniteScroll / useMemoryList 三个 hook | 在 demo 里能跑通 |
| 4 | MemoryPage 主组件 + 搜索输入框 + URL 同步 | 输入关键词 URL 变化 |
| 5 | MemoryListView + MemoryItem + MemorySkeleton + MemoryEmpty | 列表视图完整 |
| 6 | MemorySearchView + HighlightedText | 搜索视图完整 |
| 7 | 复用 TimelineDetailDialog（确保 import 路径正确） | 点击条目弹详情 |
| 8 | 视觉微调 + 边界 case（空列表、长 content、特殊字符搜索） | 验收清单全过 |

## 九、验收清单

- [ ] 进入记忆页默认看到列表视图，按时间倒序
- [ ] 滚动到底部自动加载下一页，loading 指示清晰
- [ ] 加载完所有记忆后看到"没有更多了"提示
- [ ] 输入关键词 300ms 后列表变成搜索结果
- [ ] 搜索结果顶部显示"找到 N 条与「xxx」相关的记忆"
- [ ] 搜索结果中关键词被 `<mark>` 高亮（暖奶油金背景）
- [ ] URL 实时同步到 `?q=xxx`
- [ ] 刷新带 `?q=xxx` 的 URL，搜索框保留内容、结果直接显示
- [ ] 点击 X 清空搜索框，回到全量列表
- [ ] 搜索无结果显示空状态卡片
- [ ] 全新用户（库里无记忆）显示空状态卡片，有去连接页的链接
- [ ] 点击任意条目弹出详情 Dialog（和概览页完全一致）
- [ ] pipeline 产出新记忆时，列表视图自动加新条目；搜索视图不打扰
- [ ] 长 content 被截断到 3 行，Dialog 里显示完整
- [ ] 搜索关键词含正则特殊字符（`.*+?` 等）不崩溃
- [ ] 视觉和概览页协调但密度更高

## 十、坑点预警

1. **Supermemory 搜索的 limit**：Supermemory 的 search API 一般返回 top-K（通常 20 条）。前端 PAGE_SIZE=30 实际只会拿到 20 条然后 has_more=false。**这个不是 bug 是限制**——未来 Supermemory 支持更多就自动升级。

2. **搜索是语义的不是字符串的**：Supermemory 用向量检索，搜"auth"可能返回包含"身份认证"的记忆但不包含"auth"字样。HighlightedText 找不到匹配就不高亮，视觉上**有点违和**（找到了但没高亮）。接受这个，P1 再考虑更智能的 highlight。

3. **`useInfiniteQuery` + event invalidate 的协作**：`memory-added` event 触发 `invalidateQueries(['memories', 'all'])` 会让已加载的所有页重新请求。对 30 条/页 × 3 页来说是 3 次 API 调用，可接受。如果加载了很多页（比如 10 页），invalidate 会重拉所有。**P0 接受，P1 改用 `refetch` 只刷新第一页 + prepend 新数据**。

4. **URL 同步的循环**：`inputValue → debounced → navigate → search.q → useState initial`。我加了 `if (trimmed === (search.q ?? '')) return` 防止循环。**务必测试**：输入字符后 URL 变化、清空输入后 URL 清空、直接修改 URL 后输入框同步。

5. **`replace: true` vs 默认**：同步 URL 时用 `replace: true`，否则每个字符输入都产生一个 history 条目，按返回键会卡在搜索历史里。

6. **AutoFocus 和 Tauri 的首次打开**：`autoFocus` 在 Tauri 的 webview 里一般有效，但第一次打开窗口可能失焦。不是大问题。

7. **PAGE_SIZE=30 vs 概览页 limit=200**：概览页一天最多 200 条全量加载；记忆页分页 30 条。不同策略是因为：概览页的一天有上限，记忆页可能有数千条历史。

8. **highlight 正则的大小写**：中文没有大小写，flag 'i' 只影响英文。中文搜索"支付"和"Payment"要靠 Supermemory 的语义理解，不靠 highlight。

9. **无限滚动的 IntersectionObserver**：当 `hasNextPage` 变 false 时 observer 被卸载，不会无限触发。但要保证 `hasNextPage` 的值是稳定的——避免每次渲染都创建新的布尔值导致 observer 反复挂载。

10. **`MemoryItem` 的 `onClick` vs `<Link>`**：我用了 onClick 打开 Dialog 而不是跳路由，因为 Dialog 能直接拿到 memory 对象。如果未来想做"记忆详情页"（独立 URL），改成 `<Link>` 并把 Dialog 替换为页面。

## 十一、产出物

完成 spec-08 后你应该有：

- 一个完整的记忆浏览 + 搜索页面
- 无限滚动加载能力
- URL 同步的搜索状态
- 和概览页共享详情 Dialog 的统一体验
- 复用的 hook（`useDebouncedValue` / `useInfiniteScroll` / `useMemoryList`），未来其他列表场景能直接用
- 完整的空状态、loading 态、错误态覆盖

**产品意义**：到这一步，用户在 Corivo 里形成了两个核心闭环——
- **概览页**：看今天发生了什么（被动接收）
- **记忆页**：找过去某个事情（主动查询）

加上连接页的启停控制和推送系统，产品的"核心价值三角"完整了：**记录 → 浏览 → 召回**。
