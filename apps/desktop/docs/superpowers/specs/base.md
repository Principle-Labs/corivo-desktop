一、文档定位
这份文档是 Corivo 的顶层技术设计，描述整个系统的模块划分、数据流、技术栈锁定、目录结构和关键接口定义。所有后续的模块 spec（app-shell、memory-provider、screenshot-connector 等）都基于这份文档展开。
本文档不描述：具体 UI 布局细节、具体函数实现、prompt 工程细节——这些在模块 spec 里展开。
本文档确定即锁定：技术栈、目录结构、核心 interface 签名、SQLite schema、设计 token。这些一旦确定，后续模块 spec 不再讨论，直接引用。
二、系统边界
2.1 Corivo 做什么
- 在用户本地后台运行，定期截图并调用 Gemini 生成活动总结
- 把总结写入 Supermemory（通过 MemoryProvider 抽象层）
- 提供四个功能模块的 UI：概览、记忆、连接、配置
- 管理本地原始数据（截图文件、元数据、配置）
  2.2 Corivo 不做什么（P0）
- 不做云端存储（Supermemory 除外，那是用户自己的账号）
- 不做账号系统、不做多用户
- 不做主动通知、不做评分引擎
- 不集成 Claude Code / Codex 等任何外部 Agent
- 不接入 Notion / Slack 等任何其他数据源
  三、技术栈（已锁定）
  暂时无法在飞书文档外展示此内容
  明确不用：Electron、Redux、Material UI、Ant Design、Prisma、sqlx、ORM 抽象层。
  四、系统模块划分
  4.1 前端模块（React 侧）
  src/
  ├── app/                    路由和布局
  │   ├── layout.tsx          AppLayout 根布局
  │   └── router.tsx          路由树定义（createRouter + 所有 Route）
  │
  ├── routes/                 每个路由一个文件
  │   ├── __root.tsx          根路由（套 AppLayout）
  │   ├── overview.tsx        /  概览
  │   ├── memory.tsx          /memory  记忆
  │   ├── connections.index.tsx   /connections  连接列表
  │   ├── connections.screenshot.tsx  /connections/screenshot  截图详情
  │   └── settings.tsx        /settings  配置
  │
  ├── pages/                  四个主页面（UI 实现）
  │   ├── overview/           概览（时间线）
  │   ├── memory/             记忆列表
  │   ├── connections/        连接器管理
  │   │   └── screenshot/     截图连接器详情页
  │   └── settings/           配置
  │
  ├── components/             通用组件
  │   ├── ui/                 shadcn 组件（生成到这里）
  │   ├── layout/             Sidebar、NavItem、StatusIndicator
  │   └── timeline/           TimelineCard、TimelineList
  │
  ├── lib/                    工具和接口
  │   ├── tauri.ts            所有 invoke 调用的封装
  │   ├── types.ts            TS 类型定义（与 Rust 侧对齐）
  │   ├── utils.ts            shadcn 默认 utils
  │   └── format.ts           日期、数字、token 格式化
  │
  ├── stores/                 Zustand stores
  │   ├── capture-store.ts    捕获状态
  │   ├── config-store.ts     配置状态
  │   └── ui-store.ts         UI 状态（当前选中日期等）
  │
  ├── hooks/                  自定义 hooks
  │   ├── use-segments.ts     拉取 segment 列表（react-query）
  │   ├── use-memories.ts     拉取记忆列表
  │   └── use-tauri-event.ts  订阅 Tauri event
  │
  └── styles/
  └── globals.css         Tailwind + 暖化覆盖变量
  4.2 后端模块（Rust 侧）
  src-tauri/src/
  ├── main.rs                 Tauri app 入口，注册 commands 和 events
  ├── commands/               所有暴露给前端的 Tauri command
  │   ├── mod.rs
  │   ├── capture.rs          start_capture, stop_capture, list_sessions
  │   ├── memory.rs           list_memories, search_memories
  │   ├── config.rs           get_config, set_config, test_gemini, test_supermemory
  │   └── segment.rs          list_segments, get_segment, regenerate_segment
  │
  ├── services/               核心业务逻辑
  │   ├── mod.rs
  │   ├── capture_service.rs  截图采集后台 task
  │   ├── segment_service.rs  切片和调度
  │   ├── summary_service.rs  调 Gemini 生成总结
  │   ├── memory_service.rs   记忆读写（通过 MemoryProvider）
  │   └── config_service.rs   配置读写
  │
  ├── providers/              外部依赖的抽象层
  │   ├── mod.rs
  │   ├── memory/             记忆层抽象
  │   │   ├── mod.rs          MemoryProvider trait
  │   │   ├── supermemory.rs  Supermemory 实现
  │   │   └── types.rs        Memory、SearchQuery 等数据类型
  │   └── llm/                LLM 抽象
  │       ├── mod.rs          LlmProvider trait
  │       └── gemini.rs       Gemini 实现
  │
  ├── db/                     数据库
  │   ├── mod.rs              连接池、初始化
  │   ├── schema.sql          建表 SQL
  │   ├── migrations.rs       schema 迁移
  │   └── repos/              每个表一个 repo
  │       ├── screenshots.rs
  │       ├── sessions.rs
  │       └── segments.rs
  │
  ├── events/                 Tauri event 定义
  │   └── mod.rs              emit_segment_updated 等
  │
  └── utils/
  ├── paths.rs            数据目录路径管理
  ├── image.rs            图片压缩、哈希
  └── error.rs            错误类型
  五、数据流
  5.1 P0 主数据流（截图 → 记忆）
  ┌─────────────┐
  │ CaptureLoop │  每 30s 触发
  │ tokio task  │
  └──────┬──────┘
  │ xcap 截图
  ▼
  ┌─────────────┐
  │ 本地文件系统 │  JPEG 写入 ~/Library/.../captures/<session>/
  └──────┬──────┘
  │ 元数据写入
  ▼
  ┌─────────────┐
  │   SQLite    │  screenshots 表
  │ screenshots │
  └──────┬──────┘
  │
  │ 每 15 分钟扫描
  ▼
  ┌──────────────┐
  │SegmentService│  把 screenshots 切成 segment
  └──────┬───────┘
  │ 创建 segment(status=pending)
  ▼
  ┌─────────────┐
  │   SQLite    │  segments 表
  └──────┬──────┘
  │ SummaryService 消费 pending segment
  ▼
  ┌──────────────┐
  │SummaryService│  加载 segment 的截图
  └──────┬───────┘
  │ 调用 LlmProvider.summarize(images, prompt)
  ▼
  ┌─────────────┐
  │   Gemini    │  返回文本总结
  └──────┬──────┘
  │
  ▼
  ┌──────────────┐
  │SummaryService│  把总结写回 segments 表 + 调用 MemoryProvider.add
  └──────┬───────┘
  │
  ▼
  ┌─────────────┐      ┌─────────────┐
  │MemoryProvider│ ──→ │ Supermemory │
  └──────┬──────┘      └─────────────┘
  │ emit Tauri event
  ▼
  ┌─────────────┐
  │  前端 UI    │  概览页时间线自动更新
  └─────────────┘
  5.2 查询数据流（记忆页）
  用户在记忆页输入关键词
  ↓
  前端调用 invoke('search_memories', { query })
  ↓
  Rust commands/memory.rs
  ↓
  MemoryService.search(query)
  ↓
  MemoryProvider::search → Supermemory API
  ↓
  返回 Memory[] 给前端
  ↓
  前端用 react-query 缓存并渲染
  六、核心 Interface 定义
  6.x 路由 search params 约定
  所有需要 URL 同步的状态用 TanStack Router 的 validateSearch 配合 Zod schema 定义。禁止直接读 window.location.search，禁止手动 stringify URL。
  P0 阶段需要 search params 的路由：
  // /  概览页：按日期切换
  const overviewSearchSchema = z.object({
  date: z.string().optional(),  // ISO date "2026-04-10"，缺省为今天
  });

// /memory  记忆页：搜索 + 分页
const memorySearchSchema = z.object({
q: z.string().optional(),
page: z.number().int().min(1).default(1),
});
其他路由（connections、settings）P0 阶段不使用 search params。
6.1 MemoryProvider trait（关键抽象层）
这是整个架构里最重要的抽象，它决定了未来从 Supermemory 切换到 Mem0/自建的成本。
rust
// src-tauri/src/providers/memory/mod.rs

use async_trait::async_trait;
use serde::{Serialize, Deserialize};

#[async_trait]
pub trait MemoryProvider: Send + Sync {
/// 写入一条记忆
async fn add(&self, memory: MemoryInput) -> Result<MemoryId, MemoryError>;

    /// 根据 ID 查询
    async fn get(&self, id: &MemoryId) -> Result<Option<Memory>, MemoryError>;
    
    /// 列出记忆（分页）
    async fn list(&self, query: ListQuery) -> Result<MemoryPage, MemoryError>;
    
    /// 搜索记忆
    async fn search(&self, query: SearchQuery) -> Result<Vec<Memory>, MemoryError>;
    
    /// 删除记忆
    async fn delete(&self, id: &MemoryId) -> Result<(), MemoryError>;
    
    /// 健康检查（用于配置页的"测试连接"按钮）
    async fn health_check(&self) -> Result<(), MemoryError>;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryInput {
pub content: String,           // 记忆正文
pub source: MemorySource,      // 来源（P0 只有 Screenshot）
pub metadata: serde_json::Value, // 自由形式的元数据
pub tags: Vec<String>,
pub occurred_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum MemorySource {
Screenshot { session_id: String, segment_id: i64 },
// P1 之后扩展:
// ClaudeCode { conversation_id: String },
// Notion { page_id: String },
// ...
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Memory {
pub id: MemoryId,
pub content: String,
pub source: MemorySource,
pub metadata: serde_json::Value,
pub tags: Vec<String>,
pub occurred_at: chrono::DateTime<chrono::Utc>,
pub created_at: chrono::DateTime<chrono::Utc>,
}

pub type MemoryId = String;

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchQuery {
pub query: String,
pub limit: usize,
pub source_filter: Option<MemorySource>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListQuery {
pub limit: usize,
pub offset: usize,
pub date_from: Option<chrono::DateTime<chrono::Utc>>,
pub date_to: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryPage {
pub items: Vec<Memory>,
pub total: usize,
pub has_more: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
#[error("authentication failed")]
AuthError,
#[error("network error: {0}")]
NetworkError(String),
#[error("rate limited")]
RateLimited,
#[error("not found")]
NotFound,
#[error("internal: {0}")]
Internal(String),
}
关键约束：Supermemory 实现必须把 Supermemory 的特有概念（container_tags、user profiles 等）隐藏在 SupermemoryProvider 内部，不能泄漏到这个 trait 上。未来切换到 Mem0 时，业务代码不应该有任何改动，只替换 Box::new(SupermemoryProvider::new(...)) 为 Box::new(Mem0Provider::new(...))。
6.2 LlmProvider trait（Gemini 抽象）
同样抽象出来，未来可以切换到 Claude/GPT。
rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
async fn summarize_images(
&self,
images: Vec<ImageInput>,
prompt: String,
) -> Result<LlmResponse, LlmError>;

    async fn health_check(&self) -> Result<(), LlmError>;
}

pub struct ImageInput {
pub mime_type: String,  // "image/jpeg"
pub data: Vec<u8>,
}

pub struct LlmResponse {
pub text: String,
pub input_tokens: u32,
pub output_tokens: u32,
pub cost_usd: f64,
}
6.3 Tauri Commands 清单（前后端契约）
// Capture
start_capture(interval_secs: u64) -> Result<SessionId, String>
stop_capture() -> Result<(), String>
get_capture_status() -> Result<CaptureStatus, String>
list_sessions(date: Option<String>) -> Result<Vec<Session>, String>
get_session_screenshots(session_id: String) -> Result<Vec<Screenshot>, String>

// Segment
list_segments(date: String) -> Result<Vec<Segment>, String>
get_segment(segment_id: i64) -> Result<SegmentDetail, String>
regenerate_segment(segment_id: i64, prompt: String) -> Result<(), String>

// Memory
list_memories(limit: usize, offset: usize) -> Result<MemoryPage, String>
search_memories(query: String, limit: usize) -> Result<Vec<Memory>, String>

// Config
get_config() -> Result<Config, String>
set_config(config: Config) -> Result<(), String>
test_gemini_connection(api_key: String) -> Result<(), String>
test_supermemory_connection(api_key: String) -> Result<(), String>
open_data_directory() -> Result<(), String>
get_storage_stats() -> Result<StorageStats, String>
cleanup_old_data(days: u32) -> Result<CleanupResult, String>
6.4 Tauri Events 清单
capture-status-changed -> CaptureStatus
segment-created -> Segment
segment-updated -> Segment  (status 变化或总结生成完)
memory-added -> Memory
error-occurred -> ErrorInfo  (任何需要提示用户的错误)
七、SQLite Schema
sql
-- 截图表
CREATE TABLE screenshots (
id INTEGER PRIMARY KEY AUTOINCREMENT,
session_id TEXT NOT NULL,
captured_at TEXT NOT NULL,           -- ISO 8601 UTC
file_path TEXT NOT NULL,             -- 相对 data_dir 的路径
file_size INTEGER NOT NULL,
width INTEGER,
height INTEGER,
segment_id INTEGER,
FOREIGN KEY (segment_id) REFERENCES segments(id)
);
CREATE INDEX idx_screenshots_captured_at ON screenshots(captured_at);
CREATE INDEX idx_screenshots_session_id ON screenshots(session_id);
CREATE INDEX idx_screenshots_segment_id ON screenshots(segment_id);

-- 会话表（一次连续捕获 = 一个 session）
CREATE TABLE sessions (
id TEXT PRIMARY KEY,                 -- session-20260410-1430
started_at TEXT NOT NULL,
ended_at TEXT,                       -- NULL 表示还在运行
interval_secs INTEGER NOT NULL,
screenshot_count INTEGER DEFAULT 0
);
CREATE INDEX idx_sessions_started_at ON sessions(started_at);

-- Segment 表
CREATE TABLE segments (
id INTEGER PRIMARY KEY AUTOINCREMENT,
started_at TEXT NOT NULL,
ended_at TEXT NOT NULL,
status TEXT NOT NULL,                -- pending / processing / done / failed
prompt_used TEXT,
summary TEXT,
activity_type TEXT,                  -- 预留字段，P0 暂不填写
model TEXT,
input_tokens INTEGER,
output_tokens INTEGER,
cost_usd REAL,
error_message TEXT,
generated_at TEXT,
memory_id TEXT,                      -- 写入 Supermemory 后的 ID
screenshot_count INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_segments_started_at ON segments(started_at);
CREATE INDEX idx_segments_status ON segments(status);

-- 应用状态（单行 key-value）
CREATE TABLE app_state (
key TEXT PRIMARY KEY,
value TEXT NOT NULL
);

-- Schema 版本管理
CREATE TABLE schema_version (
version INTEGER PRIMARY KEY,
applied_at TEXT NOT NULL
);
INSERT INTO schema_version (version, applied_at) VALUES (1, datetime('now'));
SQLite PRAGMA 配置：
sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
八、配置与密钥存储
8.1 普通配置（非密钥）
用 tauri-plugin-store 存到 ~/Library/Application Support/Corivo/config.json：
ts
interface Config {
capture: {
interval_secs: number;      // 默认 30
segment_duration_mins: number; // 默认 15
max_storage_gb: number;     // 默认 5
};
summary: {
prompt_template: string;    // 用户可自定义的 prompt
model: string;              // 默认 "gemini-2.0-flash"
};
app: {
auto_start: boolean;        // 开机自启
minimize_to_tray: boolean;  // 关闭时最小化到托盘
notifications_enabled: boolean;
theme: 'light' | 'dark' | 'system';
};
memory: {
provider: 'supermemory';    // 未来扩展
};
}
8.2 密钥存储（Gemini / Supermemory API Key）
绝对不放 config.json。用系统 keychain：
- macOS：Keychain
- Windows：Credential Manager
- Linux：Secret Service (libsecret)
  通过 keyring Rust crate 统一访问：
  rust
  use keyring::Entry;

pub fn save_api_key(provider: &str, key: &str) -> Result<()> {
let entry = Entry::new("com.corivo.app", provider)?;
entry.set_password(key)?;
Ok(())
}

pub fn load_api_key(provider: &str) -> Result<Option<String>> {
let entry = Entry::new("com.corivo.app", provider)?;
match entry.get_password() {
Ok(key) => Ok(Some(key)),
Err(keyring::Error::NoEntry) => Ok(None),
Err(e) => Err(e.into()),
}
}
前端永远拿不到明文 API key，测试连接也是后端代理进行。
九、设计 Token（柔和友好风格）
基底：shadcn stone 主题 + 以下暖化覆盖，写入 src/styles/globals.css：
css
@layer base {
:root {
/* 背景层（从外到内三层） */
--background: oklch(0.985 0.008 85);     /* 奶油米白 */
--card: oklch(1 0 0);                    /* 纯白卡片 */
--popover: oklch(1 0 0);
--muted: oklch(0.96 0.01 85);            /* 略深的奶油 */

    /* 文字 */
    --foreground: oklch(0.25 0.02 60);       /* 暖深棕 */
    --muted-foreground: oklch(0.50 0.015 65); /* 暖灰 */
    --card-foreground: oklch(0.25 0.02 60);
    
    /* 边框 */
    --border: oklch(0.92 0.012 80);          /* 暖浅边框 */
    --input: oklch(0.92 0.012 80);
    
    /* 主色（操作按钮、链接） */
    --primary: oklch(0.55 0.08 55);          /* 暖棕主色 */
    --primary-foreground: oklch(0.98 0.005 85);
    
    /* 强调色（选中、激活状态） */
    --accent: oklch(0.93 0.025 75);          /* 淡奶油金 */
    --accent-foreground: oklch(0.30 0.04 55);
    
    /* 语义色 */
    --destructive: oklch(0.55 0.18 25);
    --destructive-foreground: oklch(0.98 0.005 85);
    
    /* 圆角 */
    --radius: 0.75rem;                       /* 比 shadcn 默认更圆 */
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
}
字体：
css
font-family: -apple-system, BlinkMacSystemFont, "PingFang SC",
"Hiragino Sans GB", "Microsoft YaHei", sans-serif;
系统默认，不引入自定义字体。中文字体链保证 macOS/Windows/Linux 都有合理 fallback。
字号规范：
- 页面大标题：text-xl font-semibold (20px)
- 区域标题：text-sm font-medium (14px)
- 正文：text-sm (14px)
- 辅助文字：text-xs (12px)
- 时间戳、标签：text-xs text-muted-foreground
  间距规范：
- 页面 padding：p-6
- 卡片内 padding：p-4 或 p-5
- 卡片间 gap：gap-3
- 区域间 margin：mb-6 或 mb-8
  圆角规范：
- 大卡片：rounded-xl（对应 --radius 0.75rem）
- 小元素（badge、input）：rounded-md
- 头像、圆点：rounded-full
  阴影规范：
- 不用阴影。柔和风格用颜色对比和边框表达层次，不用阴影。这一条严格执行——避免视觉噪音。
  十、目录与数据路径
  ~/Library/Application Support/Corivo/     (macOS)
  %APPDATA%\Corivo\                         (Windows)
  ~/.local/share/Corivo/                    (Linux)
  │
  ├── corivo.sqlite          主数据库
  ├── corivo.sqlite-wal      WAL 文件
  ├── config.json            非敏感配置
  ├── captures/              原始截图
  │   └── session-20260410-1430/
  │       ├── 0001.jpg
  │       ├── 0002.jpg
  │       └── ...
  └── logs/
  └── corivo.log         应用日志（滚动）
  API keys 存在系统 keychain 中，不落盘。
  十一、错误处理与日志
  错误分层：
- Rust 侧：用 thiserror 定义模块级错误类型，anyhow 用于 service 层汇总
- Tauri command：全部返回 Result<T, String>，错误消息人类可读
- 前端：用 shadcn Sonner（toast）统一展示 command 错误
  日志：
- 用 tracing + tracing-subscriber
- 输出到 logs/corivo.log，按天滚动，保留 7 天
- DEBUG 级别开关在配置里（默认 INFO）
  十二、启动流程
1. Tauri app 启动
2. 初始化 tracing
3. 创建/打开 SQLite，跑 migrations
4. 读取 config，初始化 tauri-plugin-store
5. 从 keychain 加载 API keys
6. 初始化 MemoryProvider 和 LlmProvider 实例（如果 keys 缺失，provider 为 None，功能降级）
7. 检查上次捕获状态，如果配置的"启动时恢复"开启，自动重启 CaptureLoop
8. 启动 SegmentService 后台 task
9. 启动 SummaryService 后台 task
10. 显示主窗口（或最小化到托盘）
    十三、P0 不处理但预留的扩展点
    这些地方在 P0 代码里留好 hook，但不实现，避免 P1 时重构：
1. MemorySource enum 预留其他变体（ClaudeCode / Notion / 等），但 P0 只使用 Screenshot
2. ConnectorRegistry 概念：连接页的 UI 和数据结构按"多连接器"设计，但 P0 只注册一个 ScreenshotConnector
3. activity_type 字段在 segments 表里预留，P0 不填写
4. MemoryProvider trait 已经完整抽象，P0 只实现 SupermemoryProvider
5. 通知系统的 hook：在 SummaryService 里预留 on_segment_done callback，但 P0 不挂任何 handler
   十四、模块 Spec 拆分清单
   基于这份架构总览，后续会产出以下模块 spec（按推荐开发顺序）：
   暂时无法在飞书文档外展示此内容
   推荐执行顺序：01 → 02 → 05 → 03 → 04 → 06 → 07 → 10 → 08 → 09 → 11 → 12
   关键里程碑：
- 01 完成：能跑起来一个空壳 Tauri app，四个页面能切换但都是占位
- 02-05 完成：后端基础设施就绪，可以写单元测试
- 06-07 完成：截图 + 总结 + 写 Supermemory 的完整后端链路能跑通（无 UI）
- 08-09 完成：UI 能看到真实数据，第一次端到端可用
- 10-12 完成：P0 完整交付
  每个 spec 会遵循阶段 1/2/3 spec 的格式：目标、不做什么、成功标准、技术细节、任务分解、验收清单、坑点。丢给 Claude Code 可以直接开工。
  十五、你需要确认的事
  这份架构总览 spec 写完后，我需要你确认以下几点再进入模块 spec 阶段：
1. MemoryProvider trait 的接口签名是否合理？ 这是未来切换记忆层的关键。如果你觉得 MemoryInput / SearchQuery / Memory 的字段设计有问题，现在改是 1 分钟的事，等写完 Supermemory 实现再改是 1 天的事。
1. SQLite schema 是否够用？ 特别是 segments 表的字段，和 screenshots 表的外键关系。你做过类似系统，可能会看出我没考虑到的点。
2. 目录结构是否合适？ 前端 src/ 和后端 src-tauri/src/ 的模块拆分。如果你有更熟悉的风格，现在说。
3. 模块 spec 拆分的 12 份是否合适？ 有没有哪一份该合并、哪一份该拆细？执行顺序有没有问题？
4. 暖化主题的 CSS 变量 你可以现在就在一个空的 Tailwind + shadcn 项目里试一下这段 CSS，看看真实效果是不是你想要的奶油米白感觉。如果觉得太浅/太黄/太灰，现在调比后面全套组件写完再调简单一百倍。
   回复这 5 点之后，我开始写 spec-01-app-shell.md，这是第一份可以丢给 Claude Code 真正开工的文档。