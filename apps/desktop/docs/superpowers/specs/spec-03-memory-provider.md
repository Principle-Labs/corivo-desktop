# spec-03-memory-provider.md

## 一、目标

实现 Corivo 的记忆层完整闭环，覆盖两部分：
- 后端：完成 `MemoryProvider` 抽象的完整定义，并提供 `SupermemoryProvider` 的 P0 实现
- 前端：完成「记忆」页的最小可用版本，支持浏览、搜索、分页/翻页、查看来源与展开详情

完成本 spec 后，Corivo 应该具备这样一条闭环：
1. `spec-02` 中已经保存好的 Supermemory API key 可被后端安全读取
2. Rust 侧通过 `MemoryService` 调用 `MemoryProvider`
3. `MemoryProvider` 当前解析为 `SupermemoryProvider`
4. 前端记忆页可通过 Tauri command 浏览与搜索记忆
5. 后续 `spec-07` / `spec-08` / `spec-09` 写入的新截图记忆，可以直接在记忆页看到

## 二、不做什么

- ❌ 不实现截图捕获或 Gemini 总结逻辑；这些由后续 spec 负责
- ❌ 不实现按来源筛选、删除记忆、批量管理；这些属于 P1
- ❌ 不实现 Claude Code / Codex / Notion / Slack 等非截图来源
- ❌ 不把 Supermemory 的 `containerTag`、profile、project 等特有字段暴露给业务层
- ❌ 不做复杂排序控制、高级搜索语法、服务端高亮字段协议
- ❌ 不做记忆页的设计打磨 beyond P0；先保证结构、状态和交互正确

## 三、成功标准

完成本 spec 后：
1. 进入 `/memory` 页面时，默认能看到一页记忆列表
2. 每条记忆至少显示：摘要/正文片段、来源类型、发生时间或创建时间
3. 点击某条记忆后，可以展开查看完整内容与来源元数据
4. 页面顶部有搜索框，输入关键词后会调用 `search_memories`
5. 搜索结果中，匹配关键词在前端做基础高亮
6. 非搜索模式下支持分页翻页；URL 中 `page` 参数变化会驱动列表刷新
7. 搜索模式下 URL 中 `q` 参数变化会驱动结果刷新，并自动把 `page` 重置为 `1`
8. Rust 侧 `MemoryProvider`、`MemoryService`、`SupermemoryProvider` 有核心单元测试
9. 前端 `use-memories` 或等价数据层逻辑有最小测试覆盖
10. `pnpm build`、`cargo test`、`pnpm tauri build` 能通过

## 四、与 story.md / base.md 对齐的边界

### 4.1 对齐的用户故事

- Story M-1（P0）：浏览 Supermemory 记忆列表
- Story M-2（P0）：搜索记忆
- Story C-2 的后半段依赖：截图总结写入后，记忆页能消费并展示这些记忆

### 4.2 必须继承的 base 约束

- `MemoryProvider` 是关键抽象层，业务层不允许依赖 Supermemory 专有模型
- Tauri command 契约沿用 base.md：
  - `list_memories(limit, offset) -> Result<MemoryPage, String>`
  - `search_memories(query, limit) -> Result<Vec<Memory>, String>`
- `/memory` 的 search params 必须继续使用：
  - `q?: string`
  - `page: number`

### 4.3 P0 范围的设计取舍

- 浏览列表使用 `list_memories`
- 搜索使用 `search_memories`
- `search_memories` 不加 offset，P0 搜索直接返回前 N 条结果
- 匹配高亮放在前端做，避免修改后端契约

## 五、Rust 后端实现

### 5.1 目录结构

在现有 `spec-02` 基础上补齐：

```text
src-tauri/src/
├── commands/
│   ├── config.rs
│   ├── demo.rs
│   └── memory.rs                本 spec 新增
├── services/
│   ├── config_service.rs
│   ├── keychain_service.rs
│   └── memory_service.rs        本 spec 新增
├── providers/
│   ├── mod.rs
│   └── memory/
│       ├── mod.rs               从 health_check-only 扩展为完整 trait
│       ├── types.rs             本 spec 新增
│       └── supermemory.rs       从 health_check-only 扩展为完整实现
└── error.rs
```

### 5.2 `providers/memory/types.rs`

这里直接承接 base.md 的锁定接口，不再另起一套：

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryInput {
    pub content: String,
    pub source: MemorySource,
    pub metadata: serde_json::Value,
    pub tags: Vec<String>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemorySource {
    Screenshot { session_id: String, segment_id: i64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: MemoryId,
    pub content: String,
    pub source: MemorySource,
    pub metadata: serde_json::Value,
    pub tags: Vec<String>,
    pub occurred_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

pub type MemoryId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub limit: usize,
    pub source_filter: Option<MemorySource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListQuery {
    pub limit: usize,
    pub offset: usize,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPage {
    pub items: Vec<Memory>,
    pub total: usize,
    pub has_more: bool,
}
```

`MemoryError` 继续用 base.md 约定的结构；项目内部可以映射到 `CorivoError::Provider` / `CorivoError::Network`，但 trait 层语义不要丢。

### 5.3 `MemoryProvider` trait

`src-tauri/src/providers/memory/mod.rs` 从 `health_check` 最小版升级为完整版：

```rust
#[async_trait]
pub trait MemoryProvider: Send + Sync {
    async fn add(&self, memory: MemoryInput) -> Result<MemoryId, MemoryError>;
    async fn get(&self, id: &MemoryId) -> Result<Option<Memory>, MemoryError>;
    async fn list(&self, query: ListQuery) -> Result<MemoryPage, MemoryError>;
    async fn search(&self, query: SearchQuery) -> Result<Vec<Memory>, MemoryError>;
    async fn delete(&self, id: &MemoryId) -> Result<(), MemoryError>;
    async fn health_check(&self) -> Result<(), MemoryError>;
}
```

关键约束：
- `MemorySource`、`MemoryInput`、`Memory`、`MemoryPage` 这些类型是 Corivo 自己的领域模型
- `SupermemoryProvider` 里可以使用任何内部 helper、DTO、containerTag、raw response
- 但 command、service、UI 永远只看到上面这套抽象

### 5.4 `MemoryService`

新增 `src-tauri/src/services/memory_service.rs`，职责如下：
- 从 `ConfigService` 读取当前 `memory.provider`
- 从 `KeychainService` 读取对应 provider 的 API key
- 解析出当前 provider 实例
- 对外提供：
  - `list(query: ListQuery) -> Result<MemoryPage>`
  - `search(query: SearchQuery) -> Result<Vec<Memory>>`
  - `add(memory: MemoryInput) -> Result<MemoryId>`
  - `get(id: &MemoryId) -> Result<Option<Memory>>`
  - `delete(id: &MemoryId) -> Result<()>`

建议接口：

```rust
pub struct MemoryService {
    config_service: Arc<ConfigService>,
    keychain_service: Arc<KeychainService>,
}
```

provider 解析逻辑：
- 当前只支持 `MemoryProviderKind::Supermemory`
- 如果 key 不存在，返回用户可读错误：`Supermemory API key 未配置`
- provider 解析逻辑放在 service 内部，前端和 command 都不直接 new provider

### 5.5 `SupermemoryProvider` 实现策略

`src-tauri/src/providers/memory/supermemory.rs` 承担 3 件事：
- 调 Supermemory HTTP API
- 把 Corivo 的领域模型映射为 Supermemory payload
- 把 Supermemory 的原始响应映射回 Corivo `Memory`

实现原则：
1. 写代码前先 `curl` 确认官方 endpoint；文档变化时以实际跑通结果为准
2. 优先使用最轻量、最稳定的官方 endpoint
3. 即使官方 response 很复杂，也要在 provider 内部压平

推荐映射策略：
- `add(memory)`：
  - `memory.content` 作为主文本
  - `memory.tags` 直接映射为 tags
  - `memory.metadata` 保留为 provider payload 的 metadata/json blob
  - `memory.source` 同时写入 metadata，便于之后恢复 `MemorySource`
- `list(query)`：
  - 优先使用官方的 recent/list endpoint
  - 如果官方没有稳定 list endpoint，可在 provider 内部用“空搜索 + limit + offset”或官方建议的 fallback
  - 返回值统一映射为 `MemoryPage`
- `search(query)`：
  - 走官方搜索 endpoint
  - 只返回 Corivo `Memory[]`
- `get(id)` / `delete(id)`：
  - 如果官方有 detail/delete endpoint，直接使用
  - 如果没有，允许 `get` 在 P0 内作为“best effort”实现，但必须把这种限制关在 provider 内部，不暴露给调用方

### 5.6 Supermemory 内部 metadata 约定

为了让截图来源在未来可回溯、可展示，provider 写入时统一附带以下 metadata：

```json
{
  "corivo": {
    "source_type": "screenshot",
    "session_id": "session-20260410-1430",
    "segment_id": 12,
    "occurred_at": "2026-04-10T07:15:00Z"
  }
}
```

设计原则：
- 这些字段只属于 provider 内部映射策略
- 业务层永远只消费 `MemorySource::Screenshot { ... }`
- 如果未来 P1 新增 ClaudeCode / Notion，只扩展 `MemorySource` 和内部 metadata 映射，不动上层页面协议

### 5.7 `commands/memory.rs`

新增 Tauri commands：

```rust
#[tauri::command]
pub async fn list_memories(
    state: State<'_, AppState>,
    limit: usize,
    offset: usize,
) -> Result<MemoryPage, String>

#[tauri::command]
pub async fn search_memories(
    state: State<'_, AppState>,
    query: String,
    limit: usize,
) -> Result<Vec<Memory>, String>
```

实现规则：
- command 只做参数接收、service 调用、错误转字符串
- command 不直接拼 HTTP 请求
- `limit` 要做上限保护，例如 `min(limit, 100)`，避免前端误传极大值

### 5.8 `AppState` 装配

在 `lib.rs` 的 setup 中新增 `MemoryService` 并挂到 `AppState`：

```rust
pub struct AppState {
    pub config_service: Arc<ConfigService>,
    pub keychain_service: Arc<KeychainService>,
    pub memory_service: Arc<MemoryService>,
}
```

`invoke_handler` 里追加：
- `commands::memory::list_memories`
- `commands::memory::search_memories`

### 5.9 错误处理

P0 需要把这些错误转成清晰的人类可读文案：
- 未配置 key：`Supermemory API key 未配置`
- 认证失败：`Supermemory 认证失败，请检查 API key`
- 网络失败：`连接 Supermemory 失败：...`
- rate limit：`Supermemory 请求过于频繁，请稍后重试`
- 上游返回结构不符合预期：`Supermemory 返回格式异常`

注意：
- 前端 toast/页面错误态直接消费 command 返回的字符串
- 不把原始 bearer token、payload 打进日志

### 5.10 测试策略

Rust 测试至少覆盖：
- `SupermemoryProvider` 的 response -> `Memory` 映射
- `SupermemoryProvider` 的 `MemoryInput` -> request payload 映射
- `MemoryService` 在 key 缺失时返回正确错误
- `MemoryService` 能把 list/search 请求转发给 provider
- `Config` 序列化后不包含任何 api key 字段（如果 `spec-02` 已有，可复用）

建议做法：
- provider 单元测试用静态 JSON fixture，不依赖真实网络
- service 测试用 fake provider / fake keychain backend
- 真正打 Supermemory 的联调不放进自动测试

## 六、前端实现

### 6.1 TypeScript 类型

在 `src/lib/types.ts` 补齐 memory 相关类型，与 Rust 对齐：

```ts
export type MemorySource =
  | { type: "screenshot"; session_id: string; segment_id: number }

export interface Memory {
  id: string
  content: string
  source: MemorySource
  metadata: Record<string, unknown>
  tags: string[]
  occurred_at: string
  created_at: string
}

export interface MemoryPage {
  items: Memory[]
  total: number
  has_more: boolean
}
```

说明：
- Rust enum 到 TS 时，建议转成更易消费的 discriminated union
- 如果当前 Tauri 序列化出来是别的形状，也要在前端适配层统一成这个形状后再进入 UI

### 6.2 `src/lib/tauri.ts`

新增两个封装：

```ts
export async function listMemories(limit: number, offset: number): Promise<MemoryPage>
export async function searchMemories(query: string, limit: number): Promise<Memory[]>
```

规则：
- 组件和 hook 不直接 `invoke`
- 所有记忆接口只通过这里访问

### 6.3 hooks

新增 `src/hooks/use-memories.ts`，统一处理 `/memory` 页面数据获取逻辑：

行为约定：
- 当 `q` 为空：调用 `listMemories(PAGE_SIZE, offset)`
- 当 `q` 非空：调用 `searchMemories(q, SEARCH_LIMIT)`
- 搜索模式下不再额外分页；直接展示前 N 条
- queryKey：
  - 列表模式：`["memories", "list", page]`
  - 搜索模式：`["memories", "search", q]`

建议常量：
- `PAGE_SIZE = 20`
- `SEARCH_LIMIT = 50`

### 6.4 页面结构

`src/pages/memory/` 建议拆分：

```text
src/pages/memory/
├── memory-page.tsx
└── components/
    ├── memory-search-bar.tsx
    ├── memory-list.tsx
    ├── memory-list-item.tsx
    └── memory-detail-panel.tsx
```

`memory-page.tsx` 负责：
- 读取 route search params
- 绑定搜索框输入到 `q`
- 绑定上一页/下一页到 `page`
- 渲染 loading / empty / error / content

### 6.5 页面交互

页面顶部：
- 标题：`记忆`
- 副标题：`浏览和搜索 Supermemory 中的记忆`
- 搜索框：placeholder 如 `搜索你做过的事、看过的内容、讨论过的话题`

列表项最少展示：
- 主文本：取 `content` 的前 2-3 行
- 来源 badge：`截图`
- 时间：优先 `occurred_at`，fallback `created_at`

点击列表项后：
- 在当前页面内展开详情，不开新路由
- 展示完整文本、tags、来源元数据
- 对截图来源，显示：
  - `session_id`
  - `segment_id`
- 不展示原始截图；那是连接器详情页的职责

### 6.6 搜索行为

搜索输入规则：
- 输入后 250-300ms 防抖再触发查询
- 改变 `q` 时把 `page` 重置为 `1`
- 清空 `q` 时回到 browse 模式

高亮策略：
- 前端根据 `q` 对 `content` 做基础 substring 高亮
- 只高亮展示片段，不修改后端返回
- 如果找不到匹配片段，就展示正文前 140-180 字符

### 6.7 分页行为

列表模式：
- `page=1` 时禁用“上一页”
- `has_more=false` 时禁用“下一页”
- 翻页后滚到列表顶部

搜索模式：
- 不显示分页按钮
- 顶部显示 `找到 N 条相关记忆`

### 6.8 状态处理

必须覆盖 4 种状态：
- loading：骨架屏或占位文字
- empty（无任何记忆）
- empty-search（有搜索词但无匹配）
- error（命令失败）

推荐文案：
- 无记忆：`还没有任何记忆。等截图总结开始入库后，这里会出现内容。`
- 搜索无结果：`没有找到与“xxx”相关的记忆。`
- 错误：`加载记忆失败：...`

### 6.9 与后续 spec 的边界

本 spec 只把记忆页做成“可浏览、可搜索”的最小可用页。
明确留给后续：
- 来源筛选：P1
- 删除记忆：P1
- 多来源 badge 和来源详情：P1/P2
- 更强的高亮/摘要生成：后续优化

## 七、推荐实施顺序

1. 先补 `providers/memory/types.rs` 和 `MemoryProvider` trait
2. 再补 `MemoryService`
3. 再实现 `SupermemoryProvider` 的 list/search/add 映射
4. 再加 `commands/memory.rs`
5. Rust 单元测试先跑通
6. 前端补 type、tauri wrapper、hook
7. 最后做 `/memory` 页面 UI 和搜索/分页交互

## 八、验收清单

- `cargo test` 通过
- `pnpm build` 通过
- `pnpm tauri build` 通过
- 启动 app 进入 `/memory`，能看到基础列表
- 搜索框输入关键词后，触发搜索并刷新结果
- 搜索结果中的关键词有基础高亮
- 清空搜索后返回 browse 模式
- browse 模式下分页按钮可用
- 列表项显示来源与时间
- 点击列表项能展开完整内容与来源元数据
- Supermemory API key 未配置时，页面显示清晰错误而不是空白
- 后端没有把 Supermemory 专有字段暴露给前端类型

## 九、坑点预警

1. Supermemory 的 endpoint / payload 可能迭代很快：实现前必须先 `curl` 验证，不能凭印象写死。
2. 如果官方没有稳定的 list endpoint，不要把 workaround 写到 `MemoryService`；必须封装在 `SupermemoryProvider` 内部。
3. `search_memories` 不带 offset 是 base.md 已锁定契约，P0 不要擅自改 command 签名。
4. Rust enum 到 TS 的序列化形状可能不好用；必要时在前端 adapter 层统一，别把 UI 写死在 serde 的默认输出上。
5. 搜索高亮不要在后端做 string mutate，否则会污染原始 `content` 语义。
6. 记忆正文可能很长，列表项要做截断；详情展开才展示全文。
7. 所有错误必须是人类可读字符串，不能把 reqwest debug dump 或原始 token 打到前端。
8. `MemoryProvider` 后续还要服务截图总结写入，所以 `add()` 不能为了记忆页偷懒不实现。

## 十、产出物

完成本 spec 后应得到：
- 一套完整的 `MemoryProvider` 抽象与 `SupermemoryProvider` 实现
- 一个可复用的 `MemoryService`
- 两个可用的 Tauri commands：`list_memories`、`search_memories`
- 一个最小可用的记忆页，支持浏览、搜索、分页、展开详情
- 与 Story M-1 / M-2 对齐的 P0 记忆闭环

## 十一、下一份 spec

推荐下一份写 `spec-04-llm-provider-and-summary.md`，完成 Gemini 总结的完整 provider、summary service 和 segment 总结链路。这样 `spec3 + spec4 + 后续截图链路` 才能拼出“截图 -> 总结 -> 写 Supermemory -> 记忆页可见”的完整闭环。
