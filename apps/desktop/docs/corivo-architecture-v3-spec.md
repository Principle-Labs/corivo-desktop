# Corivo Architecture v3 · Day One Spec

> Status: 提案
> Supersedes: [corivo-architecture-v2-spec.md](corivo-architecture-v2-spec.md), [event-task-project-spec.md](event-task-project-spec.md)
> Related: [ax-ocr-spec.md](ax-ocr-spec.md)(并入本 spec 的提取层)
>
> 这份 spec 只描述要建什么。它假设我们今天从空仓库开始;不解释历史、不写迁移、不留兼容。

---

## 一、产品命题

Corivo 是 macOS 桌面 app:

> **"看见你屏幕上一切的眼睛,加一个能被问的大脑。"**

三个承诺(从下到上):

1. **持续观察** — 屏幕上看得见的一切,Corivo 都看得见,并以可索引、可重处理的形式留下来。
2. **忠实召回** — 你今天 / 上周 / 上个月看过的任何东西,问就答。
3. **按需组织** — 系统不主动归档;聚合只发生在用户问"X 项目最近怎么样"的那一刻,问完不留下分类负担。

**核心 wedge UX**:两个相互独立但共用底座的召回入口。

- **`/ask`**(主窗口聊天框)— 浏览历史 thread、跨时间召回、LLM 在后端持有 frames 索引 + 工具调用,流式答你。
- **Quick Ask**(全局快捷键召唤的浮窗)— 不进主窗口、不打断当前 app。按下快捷键的瞬间通过 AX 读取**当前 focus 窗口**的内容(浏览器 tab / IDE 文件 / 编辑器选区),把它作为这次对话的隐式上下文喂给 LLM,然后走和 `/ask` 同一套 tool use 循环。

> Quick Ask 解决 `/ask` 的冷启动问题(空聊天框 → 不知道问什么)。focus 内容本身就是问题的"主语"。

### 不做(明确写出来,免得偷偷重新长出来)

- ❌ 自动 Project / Task / Event 层级分类
- ❌ 主动 push 推送 / Dynamic Island overlay
- ❌ **被动 drift 评估 / 目标对照式 coach**(用户主动问 Quick Ask 来问"我现在偏离目标了吗",而不是系统每 5 分钟评一次)
- ❌ 音频录制、日历集成、AppleScript 浏览器 tab、剪贴板回填
- ❌ Windows / Linux
- ❌ 多用户 / 团队协作
- ❌ 公开 plugin / parser **市场**(per-app 适配器我们自己内部维护,见 §六)

---

## 二、设计原则

### 1. Frames 是唯一的"事实源"

所有更高级的概念(派生 session、命名实体、用户问的答案、Quick Ask 的当前 focus 上下文)都是 frames 上**派生**出来的。Frames 表写一次、不再变;任何派生失败都可以重跑。

> 推论:数据库里**没有** events / tasks / projects 表。它们如果有用,以"按需计算的视图"的形式存在 —— 系统不再持有任何用户主动标注的二级实体。

### 2. 采集与提取产出一份"信封" (SnapshotEnvelope)

采集层不直接写 DB。它产出一个版本化的 JSON envelope,经一个 `SnapshotConsumer` trait 落到本地或云端。这个 seam 是**不可妥协**的——它是未来一切扩展(云上传、Swift sidecar、第三方 ingest)的唯一入口。

### 3. 召回是 LLM + 工具调用,不是 SQL 拼接

`/ask` 后端不预先决定怎么查。它把问题给 LLM,LLM 通过 tool use 来检索(FTS / 向量 / 取截图 / 看当前屏幕)。检索策略是 prompt 行为,不是后端代码。这跟 Anthropic Messages API 的 tool use 模型对齐。

### 4. 本地优先,但每一层接口都允许整层换成云

| 层 | 本地实现 | 云实现(未来) |
|---|---|---|
| 提取(extractor) | AX + Apple Vision OCR | 上传截图,服务端 vision model |
| 索引(indexer) | SQLite FTS5 + 本地 embedding | 服务端向量库 |
| LLM 调用 | Gemini / Codex via API key in keychain | 服务端代理,客户端只持 token |
| 存储 | $APPDATA/corivo.sqlite | 服务端 PG/clickhouse + 本地缓存 |

**今天全部本地,但每个 trait 边界都能整体替换。** 不写"两套代码",写"一套 trait + 多套实现"。

### 5. 不写 Swift sidecar

Tauri 主进程是 Rust,直接 `accessibility-sys` + `objc2-vision`。Sidecar 的真实价值(系统音频 / EventKit / JavaScriptCore 跑社区 parser)在我们当前 scope 内不出现。一旦未来需要这些能力,**替换的是采集层的具体实现**,SnapshotEnvelope 形状不变,下游一行不改。

### 6. Day One 数据弃权

Pre-release。`TARGET_SCHEMA_VERSION` bump 即 purge-and-apply。任何"历史 frames"在重大 schema 变更下视作可丢。

---

## 三、架构总览

```
┌──────────────────────────────────────────────────────────────────────┐
│                  ① Capture Pipeline (Rust async)                      │
│  - Trigger: timer (15s) ∪ focus change ∪ idle wake ∪ Quick Ask invoke │
│  - Output: SnapshotEnvelope { frame, screenshot_ref, adapter, raw }   │
└──────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────┐
│                  ② SnapshotConsumer (trait)                           │
│  - LocalConsumer:  写 frames 表,触发 ③                                │
│  - CloudConsumer:  POST /snapshots (未来)                             │
└──────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────┐
│                  ③ Extractor (AX + OCR + per-app Adapters)            │
│  - Per-app adapter (Chrome / Safari / VS Code / JetBrains / ...)      │
│  - AX 通用主路径 (accessibility-sys)                                   │
│  - OCR 兜底  (objc2-vision)                                           │
│  - 写入 frames.ax_text / ocr_text / adapter_name / adapter_payload    │
└──────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────┐
│                  ④ Indexer                                            │
│  - FTS5 触发器同步 frames → frames_fts                                  │
│  - (Phase 4+) 本地 embedding → frame_embeddings                       │
└──────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────┐
│                  ⑤ Recall Surface                                     │
│  - /ask:        主窗口 LLM + tool use(search_frames / ...)           │
│  - /timeline:   按时间排序的 frames + 派生 session 聚合                 │
│  - Quick Ask:   全局快捷键浮窗,把当前 focus 内容预灌入 LLM 上下文       │
└──────────────────────────────────────────────────────────────────────┘
```

每一层只跟相邻层用 narrow interface 对接。任何一层换实现,其它层不感知。

---

## 四、SnapshotEnvelope (v1)

### Schema

```jsonc
{
  "version": 1,
  "captured_at": "2026-04-27T10:14:23.418Z",   // RFC3339 millis UTC
  "device_id": "uuid-stable-per-machine",
  "session_id": "uuid-per-capture-session",     // 进程启动一次一变,断电/休眠重启即重置

  "frame": {
    "app": {
      "bundle_id": "com.microsoft.VSCode",
      "name": "Visual Studio Code",
      "pid": 1234
    },
    "window": {
      "title": "lib.rs — corivo-app",
      "is_focused": true
    },
    "url": null,                                 // 浏览器才有
    "screenshot": {
      "path": "captures/2026/04/27/abc.jpg",     // 相对 $APPDATA;screenshot 默认开启
      "hash": "sha256:abcdef..."
    }
  },

  "extraction": {
    "strategy": "adapter",                       // "ax" | "ocr" | "ax+ocr" | "adapter" | "skipped"
    "adapter_name": "chrome",                    // 走 adapter 时填,否则 null
    "ax_text": "...",                            // 可空
    "ocr_text": null,                            // 可空
    "adapter_payload": {                          // adapter 才有的结构化字段;统一 JSON
      "tab_url": "https://...",
      "tab_title": "...",
      "selection_text": null,
      "scroll_position": 0
    },
    "duration_ms": 87,
    "fallback_reason": null                      // "ax_timeout" / "ax_too_short" / "bundle_blacklist" / "adapter_unavailable"
  },

  "trigger": "timer",                            // "timer" | "focus_change" | "quick_ask"

  "raw": {
    "ax_tree_json_path": null,                   // 可选,debug 模式才落盘
    "exclusion_match": null                      // "domain:bank" / "app:Signal" / null
  }
}
```

### 触发条件

| 触发 | 何时 |
|---|---|
| **Timer** | 默认每 15s,可配 5–60s |
| **Focus change** | 前台 app / 窗口 / URL 变化时立即触发(去抖 500ms) |
| **Quick Ask invoke** | 用户按下全局快捷键时**立即**触发一次高优先级提取(优先走 per-app adapter,跳过 dedup,跳过 idle skip)。结果既存 frame 也即时返回给 Quick Ask 浮窗作为 LLM 上下文。 |
| **Idle skip** | `CGEventSource.secondsSinceLastEventOfType` > idle 阈值时跳过(Quick Ask invoke 不受此约束) |
| **Dedup skip** | 当前帧 content_hash 与上一帧相同 → 不存 frame,只更新前一帧的 `still_present_until`(Quick Ask invoke 不参与 dedup) |
| **Exclusion skip** | 前台 app 在排除列表 / 当前 URL 在排除域名分类 → 不存帧,产 envelope 时 `extraction.strategy = "skipped"` 并记录 `exclusion_match`。Quick Ask 命中排除时浮窗显示"该应用已被排除,无法读取内容",**不强行突破**。 |

### Consumer trait

```rust
// src-tauri/src/services/snapshot_consumer/mod.rs
#[async_trait]
pub trait SnapshotConsumer: Send + Sync {
    async fn ingest(&self, env: SnapshotEnvelope) -> Result<()>;
}

pub struct LocalConsumer { /* db pool, extractor, indexer */ }
impl SnapshotConsumer for LocalConsumer { /* ... */ }

// 未来:
// pub struct CloudConsumer { /* http client, endpoint, token */ }
```

`AppState` 持有 `Arc<dyn SnapshotConsumer>`。Capture pipeline 只调 `consumer.ingest(env).await`。

---

## 五、数据模型

### 核心表(全部新建,schema_version = 300)

#### `frames` — 唯一的事实源

```sql
CREATE TABLE frames (
  id                       TEXT PRIMARY KEY,            -- ULID
  captured_at              TEXT NOT NULL,
  device_id                TEXT NOT NULL,
  capture_session_id       TEXT NOT NULL,

  app_bundle_id            TEXT,
  app_name                 TEXT,
  window_title             TEXT,
  url                      TEXT,

  screenshot_path          TEXT,                         -- 相对 $APPDATA 路径,可空
  screenshot_hash          TEXT,
  screenshot_size_bytes    INTEGER,

  ax_text                  TEXT,
  ocr_text                 TEXT,
  adapter_name             TEXT,                         -- 'chrome' | 'safari' | 'vscode' | ... | NULL
  adapter_payload          TEXT,                         -- JSON,adapter 特有的结构化字段
  extraction_strategy      TEXT NOT NULL,                -- 'ax' | 'ocr' | 'ax+ocr' | 'adapter' | 'skipped'
  extraction_duration_ms   INTEGER,
  fallback_reason          TEXT,

  trigger                  TEXT NOT NULL,                -- 'timer' | 'focus_change' | 'quick_ask'

  content_hash             TEXT,                         -- SHA256(ax_text || '|' || ocr_text || '|' || adapter_payload)
  derived_from_frame_id    TEXT REFERENCES frames(id),   -- dedup 指向上一帧
  still_present_until      TEXT,                         -- 同 content 的最后一次见到时间

  exclusion_match          TEXT,                         -- 命中排除时填,内容为空

  created_at               TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),

  CHECK (captured_at GLOB '????-??-??T??:??:??.???Z'),
  CHECK (created_at  GLOB '????-??-??T??:??:??.???Z')
);

CREATE INDEX idx_frames_captured_at ON frames(captured_at DESC);
CREATE INDEX idx_frames_app         ON frames(app_bundle_id, captured_at DESC);
CREATE INDEX idx_frames_url         ON frames(url) WHERE url IS NOT NULL;
CREATE INDEX idx_frames_content     ON frames(content_hash);
CREATE INDEX idx_frames_trigger     ON frames(trigger, captured_at DESC);  -- Quick Ask invoke 历史可独立筛
```

#### `frames_fts` — FTS5 (external content)

```sql
CREATE VIRTUAL TABLE frames_fts USING fts5(
  app_name, window_title, url, ax_text, ocr_text,
  content='frames',
  content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2'   -- 第二阶段挂上 jieba
);

-- triggers: insert/update/delete 同步
```

> 分词:第一阶段用 unicode61(够用,中英都能切但中文较粗);Phase 4 切换到现有 [tokenize.rs](../src-tauri/src/services/tokenize.rs) 提供的 jieba tokenizer。

#### `frame_embeddings` — Phase 4+ 才建

```sql
CREATE TABLE frame_embeddings (
  frame_id     TEXT PRIMARY KEY REFERENCES frames(id) ON DELETE CASCADE,
  model        TEXT NOT NULL,                            -- 'voyage-3' / 'bge-m3' / ...
  vector       BLOB NOT NULL,                            -- f32 平铺
  created_at   TEXT NOT NULL
);
-- ANN 索引方案 Phase 4 决定(sqlite-vec / hnswlib-rs / 直接全表 cosine)
```

#### `chat_threads` + `chat_messages` — `/ask` 对话历史

```sql
CREATE TABLE chat_threads (
  id           TEXT PRIMARY KEY,
  title        TEXT,                          -- LLM 自动生成或用户改
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL
);

CREATE TABLE chat_messages (
  id              TEXT PRIMARY KEY,
  thread_id       TEXT NOT NULL REFERENCES chat_threads(id) ON DELETE CASCADE,
  role            TEXT NOT NULL,              -- 'user'|'assistant'|'tool'
  content         TEXT NOT NULL,              -- markdown / tool result JSON
  tool_calls      TEXT,                        -- JSON array of tool_use blocks
  cited_frame_ids TEXT,                        -- JSON array,LLM 答案引用了哪些 frames
  created_at      TEXT NOT NULL
);

CREATE INDEX idx_chat_messages_thread ON chat_messages(thread_id, created_at);
```

### 不存在的表

`events`、`tasks`、`projects`、`facts`、`anchors`、`event_screenshots`、`reassignment_log`、`about_user`、`task_tags`、`tag_anchors`、`coach_notes`、`user_model`、`time_segments`、`observations`、`goals`、`goal_evaluations`、`saved_clips`、`entities`、`event_attributor` 相关、`retrospective_reviewer` 相关。

**全部不在 v3 schema 里。** 如果未来要回来,作为派生视图或独立功能模块再讨论。

> 关于 `goals`:被动 drift 评估这一类已被 Quick Ask 取代。如果用户想"对照目标看看自己今天怎么样",做法是按快捷键、问 "我今天偏离 X 目标了吗" —— LLM 走 tool use 自查 frames 即可。系统不再持有"目标"实体。

> 关于 `saved_clips`:原本作为 Hummingbird 风格的"用户主动标注"层。撤回的理由是 v3 已经把 `chat_threads` + `quick_ask_pin_thread` 当作主动标注的入口 —— 把对话钉到 /ask 既能保留"这一段重要"的语义,又不需要再引入一个独立的实体表。clip 概念整体下线。

> 关于 `entities`:原计划作为 LLM 抽取的结构化原子(person / email / url / doc_title / repo / code_symbol / command / ticket),用于 recall 层精确召回。撤回的理由是 frames + FTS 的语义检索已经能覆盖 Quick Ask / `/ask` 的所有召回场景,entity 表与后台抽取任务带来的复杂度与成本并不划算。entity 抽取整层下线。

---

## 六、采集与提取层

### Capture Pipeline

`src-tauri/src/services/capture_pipeline/`

职责:

1. 用 Tokio 跑两个 task:
   - `timer_task`:每 N 秒醒一次
   - `focus_watcher_task`:订阅 macOS workspace 通知(`NSWorkspaceDidActivateApplicationNotification` 等),前台变化触发 capture
2. 每次触发:
   - 检查 idle / 排除条件
   - `xcap` 抓截图 + 落盘到 `$APPDATA/captures/YYYY/MM/DD/<ulid>.jpg`
   - 调 `Extractor::extract(frame)` 拿 `(ax_text, ocr_text, strategy, duration_ms)`
   - 拼成 `SnapshotEnvelope`
   - `consumer.ingest(env).await`
3. dedup:在调 extractor 之前,先看上一帧的 `(app_bundle_id, window_title, url)` 和当前是否相同。如果同,先抓截图算 hash,hash 同则**不调 extractor、不写新 frame**,只 UPDATE 上一帧的 `still_present_until`。

> 截图先于 extraction:截图是 ground truth,extraction 失败不能阻挡帧落盘。

### Extractor

`src-tauri/src/services/extractor/`

三层 dispatcher,**按顺序**尝试,前面成功就跳过后面:

1. **Per-app adapter**(见下文)— 当前 focus app 的 bundle_id 命中某个内置 adapter 时,优先用 adapter 做结构化抽取
2. **AX 通用主路径** — 没有 adapter 命中、或 adapter 失败时,用通用 AX traversal(`accessibility-sys`,`spawn_blocking` + 1500ms timeout)
3. **OCR 兜底**(`objc2-vision`)— AX 失败/超时 ∨ AX 文本 < 30 字 ∨ bundle id 在硬编码 OCR-only 列表

通用部分完全延续 [ax-ocr-spec.md](ax-ocr-spec.md):
- OCR 配置:`.accurate` + `recognitionLanguages = ["zh-Hans","zh-Hant","en-US"]`
- Output:`output_format::stitch(...)` 按 reading order 拼,带少量 high-signal 角色标签(`[TITLE]`/`[BUTTON]`/`[TAB]`/`[CODE]`)
- ax-ocr-spec 里所有 dispatcher 决策表 1:1 沿用,不在这里重复

### Per-app Adapter 框架

`src-tauri/src/services/extractor/adapters/`

通用 AX traversal 在 IDE 和浏览器这种**信息密度高 + 结构化强**的场景下天然吃亏(Chrome 拿到一堆 ARIA noise、VS Code 编辑器只能拿可见行)。Adapter 是"针对性更强的提取器",每一个 adapter 知道目标 app 的 AX 树长什么样,直接挑出最有价值的字段。

#### Trait

```rust
// src-tauri/src/services/extractor/adapters/mod.rs

pub trait FocusAdapter: Send + Sync {
    /// 这个 adapter 能否处理给定 bundle_id
    fn matches(&self, bundle_id: &str) -> bool;

    /// adapter 的稳定标识(写入 frames.adapter_name)
    fn name(&self) -> &'static str;

    /// 给定 focused app 的 AX 根元素 + 元数据,返回结构化抽取结果。
    /// 内部完全可以跑常规 AX traversal,只是知道哪些节点重要、怎么压成 payload。
    fn extract(&self, ctx: AdapterContext<'_>) -> Result<AdapterOutput>;
}

pub struct AdapterContext<'a> {
    pub bundle_id: &'a str,
    pub app_name: &'a str,
    pub window_title: &'a str,
    pub ax_root: &'a AxElement,         // accessibility-sys 句柄
    pub trigger: Trigger,
}

pub struct AdapterOutput {
    pub primary_text: String,            // 写入 frames.ax_text 的"主体内容"
    pub payload: serde_json::Value,      // 写入 frames.adapter_payload(URL/file path/选区/光标位置等)
    pub confidence: AdapterConfidence,   // High / Medium / Low — Low 时 dispatcher 仍会再跑通用 AX 兜底
}
```

#### 内置 adapter 列表(Phase 5 启动时的初始集合)

| Adapter | bundle_id 匹配 | 提取重点 |
|---|---|---|
| **ChromeAdapter** | `com.google.Chrome` / `com.google.Chrome.canary` | 当前 tab URL、页面标题、可见正文、用户文本选区 |
| **SafariAdapter** | `com.apple.Safari` / `com.apple.SafariTechnologyPreview` | 同上(Safari 的 AX 桥更完整,直接从 DOM 拿) |
| **VSCodeAdapter** | `com.microsoft.VSCode` / `com.microsoft.VSCodeInsiders` / Cursor 的 bundle | 当前文件路径、光标行、可见代码、文件树展开节点 |
| **JetBrainsAdapter** | `com.jetbrains.*`(IntelliJ / WebStorm / RustRover / RubyMine / GoLand 等共用) | 文件路径、可见代码、当前函数名 |
| **ITermAdapter** | `com.googlecode.iterm2` | 最近 N 行 buffer、当前 cwd(从 window title 解析)、当前 tab/session 名 |
| **LarkAdapter**(飞书) | `com.bytedance.feishu` / `com.larksuite.larkApp`(国际版)/ `com.electron.lark` | 当前会话/频道、可见消息、消息发送者、用户起草中的消息 |
| **GenericAxAdapter** | `*`(兜底,优先级最低) | 通用 AX traversal 的薄包装,产生不带 payload 的纯文本 |

> 起步 6 个具体 adapter + 兜底,覆盖 dogfood 时用得最重的 app。其它 app(Notion / Slack / Terminal.app / Warp 等)**自动落到 GenericAxAdapter / OCR**,后续按需增量加。增删 adapter **不需要**改 dispatcher 或 schema —— `adapter_name` 和 `adapter_payload` 是 open enum / open JSON。

> **实现参考**:Littlebird ContextKit-cli 解包出的 30 份 JSParser(Slack / Notion / Linear / Figma 等)是非常好的参考,看他们怎么从 Chromium AX tree 里挑信号、怎么处理 lazy a11y、怎么过滤 ARIA noise。我们用 Rust + accessibility-sys 重写,但**信号选择策略**可以直接抄。飞书是 Electron(基于 Chromium),AX 树形态跟 Slack/Notion 同源,Slack JSParser 的策略迁移成本最低。

#### 注册与查找

```rust
// src-tauri/src/services/extractor/adapters/registry.rs
pub struct AdapterRegistry {
    adapters: Vec<Arc<dyn FocusAdapter>>,
}

impl AdapterRegistry {
    pub fn builtin() -> Self {
        // 顺序敏感:先具体后通用,GenericAxAdapter 必须最后
        Self { adapters: vec![
            Arc::new(ChromeAdapter::new()),
            Arc::new(SafariAdapter::new()),
            Arc::new(VSCodeAdapter::new()),
            // ...
            Arc::new(GenericAxAdapter::new()),
        ]}
    }

    pub fn find(&self, bundle_id: &str) -> Arc<dyn FocusAdapter> {
        self.adapters.iter().find(|a| a.matches(bundle_id))
            .cloned()
            .expect("GenericAxAdapter must always match")
    }
}
```

#### 与 Quick Ask 的关系

Quick Ask invoke 触发的 frame **永远走 adapter 路径**(skip OCR、skip dedup)。如果命中应用没有专属 adapter 而走到 GenericAxAdapter,Quick Ask 浮窗的 "current focus" 区域就用 AX 文本兜底,质量略降但 UX 不破。

### 排除引擎

`src-tauri/src/services/exclusion/`

两层:

1. **App 级**:bundle_id 黑名单(默认含 `org.whispersystems.signal-desktop`、密码管理器、银行 app);用户在 Settings 可加。
2. **域名级**:URL 落到分类(adult / banking / health / shopping)就跳过。本地内置基础分类列表(JSON 资源);未来支持远程更新。

命中即跳过 extractor 调用、跳过 ax_text/ocr_text 落库,frame 行仍然写但 `extraction.strategy = "skipped"`,`exclusion_match` 注明原因。**用户能在 /timeline 看到"这段时间在某 app,但内容不存"。**

### 权限

- **Screen Recording**: hard requirement,首次启动 onboarding 一屏说明 + deep link 跳系统设置 + 1-2s 轮询授权状态。
- **Accessibility**: enhancement(AX 走不通就降级 OCR-only),onboarding 单独一屏,**可跳过**。

---

## 七、索引层

`src-tauri/src/services/frame_indexer/`

### Phase 1-3: FTS only

frames 表 INSERT/UPDATE/DELETE 触发器自动同步 frames_fts。无独立 indexer 服务。

### Phase 4+: Embedding

后台定时任务:扫 `frames` 里 `frame_embeddings` 不存在的行,批量 embed,写表。

Embedding 模型选型(决策延迟到 Phase 4 dogfood 时):
- 本地候选:bge-m3-small via candle / fastembed-rs(轻、离线)
- 云候选:Voyage-3 / OpenAI text-embedding-3-small(走我们已有的 LlmProvider trait 扩展)

---

## 八、召回层

### `/ask` 后端

`src-tauri/src/services/recall/`

每次用户发问:

1. 取该 thread 的历史消息
2. 拼 system prompt = `recall_orchestrator.md` + 当前时间 + 设备信息
3. 调 LLM(`LlmProvider::complete_with_tools`,需新增到 trait)传入工具定义
4. LLM 走 tool use 循环;每次 tool_use 命中时,后端跑对应 Rust 实现,把结果 stringify 成 tool_result 喂回 LLM
5. 把 LLM 流 + 工具执行结果转成 **Vercel AI SDK Data Stream Protocol** 文本块(详见下文「聊天栈选型」),通过 Tauri `Channel<String>` 推给前端。`cited_frame_ids` 用 `data-cited-frames` part 增量推送,前端直接渲染到右侧栏

### 工具集合

```rust
// src-tauri/src/services/recall/tools.rs

// 后端工具:LLM 调,后端在本机执行
pub async fn search_frames(
    query: String,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    app_bundle_id: Option<String>,
    top_k: Option<u32>,                    // 默认 20
) -> Result<Vec<FrameSearchHit>>;

pub async fn get_frame_text(frame_id: String) -> Result<FrameDetail>;

pub async fn get_frame_screenshot_b64(frame_id: String) -> Result<String>;

pub async fn list_apps_used(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<AppUsageRow>>;

pub async fn get_current_screen() -> Result<FrameDetail>;
// 触发即时 capture + extract,产一帧并返回。这是 Hummingbird "client tool" 的本地实现。
```

`search_frames` 内部:
- Phase 1-3: FTS only → BM25 排序 → top_k
- Phase 4+: hybrid(FTS top_50 ∪ vector top_50 → reciprocal rank fusion → top_k)

### 客户端 UI

`src/pages/ask/`

- 三栏布局:左 thread 列表,中聊天流,右"引用的 frames"侧栏(点击 thumbnail 跳 /timeline)
- LLM 思考链可见(tool calls 折叠展示)
- 流式 token 渲染 + tool_use 状态条("正在搜索 2026-04 的 frames...")
- 用户可以拖一帧到聊天框引用("这一帧具体是什么")

### 聊天栈选型

前端使用 **[Vercel AI SDK v5](https://sdk.vercel.ai)**(`@ai-sdk/react` 的 `useChat`),不自己写聊天状态机。

**为什么选它:**

- `UIMessage` + `parts` 模型(`text` / `tool-{name}` / `data-{name}`)1:1 对应我们的三个需求 —— 流式 token、工具调用过程可见、cited frames 侧栏 —— 每个 part 类型对应一类 UI 渲染,不用自己拼状态。
- 切换 LLM provider(Gemini → Codex → 未来加 Claude)时前端零改动:它只看 stream protocol,不看 provider。
- §十四「Cloud-pivot」时,如果服务端切到 Node + AI SDK 暴露 `/api/chat`,前端代码可整段复用,只换 transport。
- `useChat` 的消息持久化、重发、停止、错误恢复都内置;省掉一周的胶水代码。

**为什么不选其他方案:**

| 方案 | 否决理由 |
|---|---|
| LangChain.js / LangGraph.js | 工具循环已经在 Rust 后端跑(因为要碰 SQLite + Keychain),前端再套一层 graph runtime 是反向架构 |
| assistant-ui | UI 组件库定制空间小,Corivo 三栏布局 + 拖帧引用 + cited frames 侧栏不是它的标准形态 |
| 自己写 | 流式 + tool_use 状态机 + 重发 + 停止,容易写错;省下来的代码重复造轮子 |

**Tauri 适配 (~100 行胶水):**

`useChat` 默认通过 `fetch` 连一个 HTTP endpoint。Tauri 没有真 HTTP server,所以我们提供自定义 `fetch`:

```ts
// src/lib/tauri-chat-fetch.ts
// 把 chat_send_message 返回的 Channel<String> 包成 Response { body: ReadableStream }
const tauriChatFetch: typeof fetch = async (_url, init) => {
  const body = JSON.parse(init!.body as string);
  const channel = new Channel<string>();
  await invoke("chat_send_message", { ...body, stream: channel });
  return new Response(channelToReadableStream(channel), {
    headers: { "Content-Type": "text/plain; charset=utf-8" },
  });
};

useChat({ fetch: tauriChatFetch, /* ... */ });
```

**流协议 —— Rust 后端 emit 的格式:**

按 [Vercel AI SDK Data Stream Protocol v1](https://sdk.vercel.ai/docs/ai-sdk-ui/stream-protocol#data-stream-protocol) 输出文本块,每行一个事件:

| 前缀 | 用途 | 我们怎么用 |
|---|---|---|
| `0:"text"` | text delta | LLM 文本 token 透传 |
| `9:{toolCallId, toolName, args}` | tool call | LLM 决定调工具时 emit |
| `a:{toolCallId, result}` | tool result | 后端工具跑完后 emit(LLM 看不到这一行,只是给前端展示) |
| `2:[{cited_frame_ids:[...]}]` | data part | 给右侧栏推 cited frames(增量) |
| `d:{finishReason, usage}` | finish | 流结束 |

**不用 Vercel AI SDK 的服务端 SDK** —— 它假设 Node.js + provider SDK(OpenAI / Anthropic / ...)。我们的 Rust 端在 `services/recall/stream.rs` 里手写 protocol emit(就是字符串拼接),provider 适配仍然走 §十的 `LlmProvider::complete_with_tools` trait —— 该 trait 输出 normalized event(`TextDelta` / `ToolCall` / `Stop`),`stream.rs` 把它转成 protocol 文本。

### `/timeline`

`src/pages/timeline/`

- 时间轴展示:横向时间,每行一个 app,色块 = 连续帧聚成的派生 session
- session 派生规则:`(app_bundle_id, window_title, url)` 不变 ∧ 相邻帧间隔 < 5min
- 点击 session → 弹层显示该段所有帧的 thumbnail + 文本预览
- **派生 session 不存表**;UI 端在 hook 内计算,SQL 用窗口函数

### Quick Ask · 全局快捷键浮窗

#### 形态

- **独立 Tauri 窗口**(NSPanel-style):
  - `decorations: false`,`always_on_top: true`,`skip_taskbar: true`,`activation_policy: Accessory`(召唤时不抢前台 app 焦点的"激活态")
  - 默认尺寸 ~640 × 480,居屏顶部偏上 25%(类似 Spotlight)
  - 用 `tauri-plugin-positioner` 居中
- **独立 Vite entry**:`quick-ask.html` → `src/overlay-quick-ask/main.tsx`(独立 React root,不进 router,跟现有 `notification-overlay.html` 同模式)
- **生命周期**:进程启动时**预创建并隐藏**;按快捷键 → show + focus input;ESC 或失焦 → hide(不 destroy,下一次召唤更快)

#### 全局快捷键

- 默认 `⌥ Space`(最少冲突)。Settings 可改,持久到 `tauri-plugin-store`
- 用 `tauri-plugin-global-shortcut` 注册;首次启动检测冲突时让用户改
- 注册失败(系统已被占)→ 在 Settings 显示红色状态卡片 + "更换快捷键" 按钮

#### 召唤流程

```
用户按 ⌥ Space
  ↓
  ① Quick Ask 窗口 show + focus input(<50ms)
  ② 后端并行触发 capture_pipeline.invoke_quick_ask()
       ↳ 拿当前前台 app 的 (bundle_id, pid, window AX root)
       ↳ 截图(异步,不阻塞 UI)
       ↳ 跑 adapter dispatcher → 拿 (primary_text, payload, screenshot_path)
       ↳ 写一行 frame(trigger='quick_ask',skip dedup)
       ↳ 通过 Tauri event 把 FocusContext { adapter_name, payload, primary_text_excerpt } 推给浮窗
  ↓
  浮窗 input 上方显示一条紧凑卡片:
    "📖 正在阅读: <tab title>  · <hostname>"   (Chrome adapter 的样子)
    "💻 正在编辑: <file_path>:<line>"           (VSCode adapter 的样子)
    "💬 在 #<channel> 看消息"                   (Slack adapter 的样子)
    点击 ⓘ 可展开看 adapter 抽到的完整文本
  ↓
  用户输入问题 → Cmd+Enter 发送
  ↓
  ③ chat_send_message(thread_id, user_text, focus_frame_id) 触发 recall 后端,
     与 /ask 用的是同一个 LlmProvider::complete_with_tools 入口,
     只是 system prompt 切到 quick_ask_orchestrator.md
     (告知 LLM:"这是用户当前在看的内容,先回答关于它的;若需要回顾历史,你有 search_frames 等工具")
  ↓
  ④ 流式回答渲染在浮窗中部;tool calls 折叠;cited frames 用小 badge 列在底部
```

#### 上下文打包

发给 LLM 的初始 message 结构(伪代码):

```
system  = quick_ask_orchestrator.md(渲染时填:当前时间、device、focus 摘要)
user    = [
  {
    type: "context",
    text: "用户在 ${app_name} 中,当前焦点:${focus_summary}\n\n--- 内容开始 ---\n${primary_text}\n--- 内容结束 ---"
  },
  {
    type: "text",
    text: "${user_question}"
  }
]
```

**截断策略**:`primary_text` > 8k tokens 时,头尾各保留 2k + 中间 marker;LLM 想看更多用 `get_frame_text(frame_id)` 工具调。

#### 持久化

- Quick Ask 的对话**默认开新 thread**,thread.title 由 LLM 在第一轮回答完后生成(基于 focus + 用户问题)
- thread 存入和 `/ask` 同一个 chat_threads 表;Settings 可设置"Quick Ask 默认匿名 thread,关闭即删"
- 用户在浮窗里可以"📌 钉到 /ask" → 把这个 thread 持久化并跳转到主窗口继续

#### 边界情况

| 情况 | 行为 |
|---|---|
| 当前 focus app 命中排除列表 | 浮窗显示"该应用已排除,Quick Ask 不读取内容";输入框仍可用,但只能纯文本对话(无 focus context) |
| 当前 focus 是 Corivo 自己 | 浮窗里 focus 区域显示"(浮窗自身,无内容)";输入框直接进无 context 模式 |
| AX 权限被拒 | adapter 全部回落 GenericAxAdapter;若 GenericAxAdapter 也拿不到内容(空树),浮窗 focus 区显示"无法读取窗口内容,可继续无 context 提问"|
| 没装任何 adapter 命中的 app | 走 GenericAxAdapter,focus 区显示"原始 AX 文本预览(N 字)" |

---

## 九、Prompt 清单

`src-tauri/prompts/` 全部清空,重建以下 4 份:

| Prompt | 用途 | 输入 | 输出 |
|---|---|---|---|
| `recall_orchestrator.md` | `/ask` 系统 prompt | 用户问题 + tool defs + 当前时间/设备 | tool use 循环 + 自然语言答案 |
| `quick_ask_orchestrator.md` | Quick Ask 系统 prompt | 用户问题 + 当前 focus 摘要 + tool defs | 优先答关于 focus 的问题,需要历史时调工具 |
| `frame_summarize.md` | 把一组 frames 压成可塞入上下文的文本 | frames[] | 简洁段落 |

> 不需要 `event_boundary` / `event_task_attribute` / `task_narrate` / `project_rollup` / `retrospective_*` / `goal_evaluator` 任何一份。

---

## 十、LlmProvider trait 扩展

现状:`complete_vision / complete_text / complete_json_value`。

新增:

```rust
pub trait LlmProvider {
    // 原有保留

    async fn complete_with_tools(
        &self,
        request: LlmToolRequest,    // messages + tool definitions + system prompt
    ) -> Result<LlmToolResponseStream>;
    // 流式;每个 chunk 是 tool_use_block / text_delta / tool_result_request / message_stop

    async fn embed(
        &self,
        texts: Vec<String>,
        model: EmbeddingModel,
    ) -> Result<Vec<Vec<f32>>>;
    // Phase 4 才用
}
```

按 Anthropic Messages API 的 tool use 流式协议建模。Gemini / Codex 各自适配它们的 function-calling 协议。

---

## 十一、Tauri 命令(前端 ↔ 后端)

`src-tauri/src/commands/` 重建为:

| Command | 用途 |
|---|---|
| **/ask 相关** | |
| `chat_threads_list` | 列出所有 thread |
| `chat_thread_create` | 新建 thread |
| `chat_thread_delete` | |
| `chat_send_message` | 发问;参数含 Tauri `Channel<String>`,后端按 AI SDK Data Stream Protocol v1 推 token / tool / data / finish 文本块 |
| `chat_messages_by_thread` | 取某 thread 全部消息 |
| **/timeline 相关** | |
| `frames_list` | 按时间 / app / url 过滤的 frames 列表(分页) |
| `frame_detail` | 单帧详情含截图 base64 |
| `frames_search` | 直接 FTS / 向量搜索(给前端搜索框,不经 LLM) |
| **Quick Ask** | |
| `quick_ask_register_hotkey` | 注册 / 替换全局快捷键(参数:accelerator string,如 `Alt+Space`) |
| `quick_ask_summon` | 显式打开浮窗(给设置/调试用,正常路径走 hotkey) |
| `quick_ask_capture_focus` | 当前前台 app 的 adapter 提取一次,写一行 `trigger='quick_ask'` 的 frame,返回 `FocusContext` 给浮窗 |
| `quick_ask_send` | 发问;复用 chat_send_message 的实现,system prompt 切到 quick_ask_orchestrator,thread 默认匿名 |
| `quick_ask_pin_thread` | 把当前 Quick Ask thread 持久化并跳转 /ask |
| **设置** | |
| `exclusion_*` | App / 域名排除增删查 |
| `permission_status` | screen recording + AX 当前状态 |
| `capture_config_*` | 频率、截图保留默认值等 |
| `hotkey_status` | 当前注册的快捷键 + 系统冲突状态 |

前端:`src/lib/tauri.ts` 全部重写;`src/hooks/use-frames.ts` + `use-recall.ts` + `use-quick-ask.ts`;旧 `use-workspace.ts` 删除。

---

## 十二、目录结构

```
src-tauri/src/
├── lib.rs                        # AppState 装配
├── commands/
│   ├── chat.rs
│   ├── frames.rs
│   ├── quick_ask.rs              # hotkey + summon + capture_focus + send + pin
│   ├── settings.rs
│   └── permissions.rs
├── services/
│   ├── capture_pipeline/         # 时间触发 + focus watcher + dedup + Quick Ask invoke 入口
│   ├── extractor/
│   │   ├── ax_extractor.rs
│   │   ├── ocr_extractor.rs
│   │   ├── output_format.rs
│   │   ├── dispatcher.rs         # 三层:adapter → AX → OCR
│   │   └── adapters/             # per-app 适配器(Phase 5 启动)
│   │       ├── mod.rs            # FocusAdapter trait + AdapterRegistry
│   │       ├── chrome.rs
│   │       ├── safari.rs
│   │       ├── vscode.rs
│   │       ├── jetbrains.rs
│   │       ├── iterm2.rs
│   │       ├── lark.rs           # 飞书 / Lark
│   │       └── generic_ax.rs     # 兜底
│   ├── snapshot_consumer/        # trait + LocalConsumer
│   ├── frame_indexer/            # FTS 同步触发器管理 + (Phase 4) embedding 后台任务
│   ├── exclusion/                # bundle / domain 黑名单引擎
│   ├── recall/                   # /ask + Quick Ask 共用后端
│   │   ├── orchestrator.rs       # /ask system prompt 渲染
│   │   ├── quick_ask.rs          # quick_ask system prompt 渲染 + focus context 打包
│   │   ├── tools.rs              # search_frames / get_frame_text / ...
│   │   └── stream.rs             # AI SDK Data Stream Protocol emit
│   ├── hotkey/                   # tauri-plugin-global-shortcut 包装 + 冲突检测
│   ├── capture_store.rs          # 截图落盘(保留)
│   ├── macos_system_surface.rs   # 保留;Quick Ask 窗口的 NSPanel 化也在此
│   └── tokenize.rs               # 保留(Phase 4 接到 FTS)
├── providers/
│   ├── llm/                      # LlmProvider trait + Gemini/Codex/mock,扩展 with_tools + embed
│   └── keychain_service.rs
├── domain/
│   ├── snapshot_envelope.rs      # SnapshotEnvelope 结构 + serde
│   ├── frame.rs
│   ├── chat.rs
│   ├── focus_context.rs          # Quick Ask 浮窗的 FocusContext payload
│   └── ipc_error.rs
├── db/
│   ├── schema.sql                # 全新,版本 300
│   ├── migrations.rs             # purge-and-apply
│   ├── pool.rs
│   ├── time.rs                   # DbInstant + Clock(保留)
│   └── repos/
│       ├── frames.rs
│       └── chat.rs
└── prompts/
    ├── recall_orchestrator.md
    ├── quick_ask_orchestrator.md
    └── frame_summarize.md

# Vite 多入口(对应 Tauri 多窗口)
index.html                          # 主窗口
quick-ask.html                      # Quick Ask 浮窗(独立 React root)

src/
├── main.tsx                        # 主窗口入口
├── app/                            # 路由 / 布局保留(主窗口用)
├── overlay-quick-ask/              # Quick Ask 浮窗独立 React root
│   ├── main.tsx
│   ├── QuickAskWindow.tsx
│   └── FocusCard.tsx
├── routes/
│   ├── ask.index.tsx
│   ├── timeline.index.tsx
│   ├── settings.*.tsx
│   └── onboarding.*.tsx
├── pages/
│   ├── ask/                        # 聊天 UI(主窗口)
│   ├── timeline/                   # 时间轴 + session 派生
│   └── settings/                   # 排除规则 / 权限 / 截图保留 / 快捷键
├── hooks/
│   ├── use-frames.ts
│   ├── use-recall.ts
│   ├── use-quick-ask.ts
│   └── use-config.ts
├── lib/
│   ├── tauri.ts
│   ├── tauri-chat-fetch.ts         # AI SDK fetch shim
│   └── types.ts
└── components/ui/                  # shadcn 保留
```

> 默认路径 `/ → /ask`(主窗口);Quick Ask 浮窗不进路由,通过快捷键召唤。

---

## 十三、实施路径

5 个阶段,每个阶段独立可运行、独立可验证。**N+1 阶段开始前 N 阶段必须 ship 完。**

每个 Phase 块里有 **启动前决策** 子节,列出该阶段开工前必须落定的事项 —— 候选方案与时机都写明,但具体值故意留空,真到 Phase 启动那一刻再敲定。这一条配合 §十五 的跨阶段纪律一起,是阻止 spec 在执行中被悄悄改动的机械防线。

### Phase 1 · 基础采集 + Frames 表

**范围**
- schema.sql v300,只含 frames + frames_fts(unicode61 分词)
- capture_pipeline:timer + focus_watcher + dedup,produce SnapshotEnvelope
- snapshot_consumer trait + LocalConsumer
- extractor 暂时只走 OCR(`objc2-vision` 已成熟,先跑通端到端)
- exclusion 引擎只做 bundle id 黑名单(域名分类延迟)
- Settings 一屏:Screen Recording 权限引导 + 截图保留开关
- 前端只有一个 `/timeline` 极简版:时间倒序列出 frames,点开看截图 + OCR 文本

**启动前决策**

启动 Phase 1 之前必须落定;每条给出候选与时机:

- **timer 间隔默认值** —— 候选 15s / 30s;Phase 1 ship 前选,dogfood 一周后可调
- **截图保留默认值** —— ON / OFF;dogfood 阶段建议 ON,Phase 1 ship 前敲定
- **content_hash 是否含 window_title** —— 含会少 dedup,不含会把"读不同章节但 AX 文本相同的页面"误判;Phase 1 dogfood 一周后定
- **多 monitor 抓哪个屏** —— 主屏 / 焦点屏 / 全部抓三选一;**Phase 1 启动前必须定**,否则多屏用户的 wedge 召回率会被静默砍 30-50%
- **截图 retention 策略** —— 30 天 / 90 天 / 永久;Phase 1 ship 前定,否则一年后 `$APPDATA/captures/` 累积 > 500GB
- **frames 文本 retention 策略** —— 同上,SQLite 占用比截图小可更宽松
- **exclusion 默认列表是否覆盖中国地区银行/支付** —— 招行/建行/工行/支付宝/微信支付等 bundle id 是否预置;Phase 1 ship 前敲定,否则中国用户开箱漏隐私
- **exclusion 命中时是否保留截图** —— 存 / 不存;**Phase 1 启动前必须明示**(§四当前未说,默认推理是存,这是隐私漏洞)
- **dedup 验收指标拆分** —— 验收门槛"`derived_from_frame_id` 不空 > 30%"必须按 app 类型分类报告(原生 vs Chromium / Electron),否则总比例失真无法定位失败原因。Chromium AX 是 lazy 的,实际 dedup 命中率可能远低于预期(见 [ax-ocr-spec.md](ax-ocr-spec.md) Q1)

**验收**
- 跑 1 小时,frames 表 ≥ 100 行
- dedup 真的省下了重复帧(`derived_from_frame_id` 不空的比例 > 30%)
- OCR 平均延迟 < 1.5s
- /timeline 渲染 1000 帧不卡

### Phase 2 · AX extractor 接入

**范围**
- `accessibility-sys` 接入,AX 主路径
- Dispatcher 三条 fallback 规则(详见 ax-ocr-spec)
- AX 权限 onboarding(可跳过)
- Settings 加 AX 状态卡片

**启动前决策**

- **AX 调用 timeout 默认值** —— 沿用 ax-ocr-spec 的 1500ms;Phase 2 实施第一件事是真机 benchmark 收 P50/P95
- **OCR-only bundle id 黑名单 v1 内容** —— Phase 2 启动前敲定;至少含 Figma / Warp / Alacritty / Kitty / Photoshop,沿用 [ax-ocr-spec.md](ax-ocr-spec.md) Q5 决策表
- **AX/OCR 路径选择是否暴露给用户** —— /timeline 单帧详情是否显示该帧走 AX 还是 OCR;Phase 2 ship 前定(影响调试体验与 UI 复杂度)
- **`accessibility-sys` 维护风险接管成本** —— 该 crate v0.2.0,长期未大版本;Phase 2 启动前评估若 macOS 大版本破坏 binding 时的接管路径(预计 ~200 行 unsafe FFI 重写)

**验收**
- AX 命中率 ≥ 50%(剩下走 OCR)
- AX 平均延迟 < 200ms
- 一周不出现 AX 引发的崩溃

### Phase 3 · `/ask` MVP(只有 FTS)

**范围**
- LlmProvider trait 扩展 `complete_with_tools`(先实现 Gemini)
- `recall_orchestrator.md` + 4 个后端工具(search_frames / get_frame_text / get_frame_screenshot_b64 / list_apps_used);**暂不做 get_current_screen**
- chat_threads + chat_messages 表 + repos
- `/ask` 前端:thread list + 聊天流 + tool call 折叠 + cited frames 侧栏
- 默认路由 `/ → /ask`

**启动前决策**

Phase 3 是 wedge,启动前决策密度最高:

- **100 题清单冻结** —— **Phase 3 启动前先写定 100 个具体问题并冻结**,启动后不允许删题或改题。否则会下意识把"答不出的题"删掉,wedge 验证失去意义
- **LLM 提供商优先级** —— 先 Gemini / 先 Claude / 双跑对比;Gemini function calling 在 5+ 轮 tool use 上稳定性弱于 Claude,但成本低 —— Phase 3 启动前必须拍板用哪个验 wedge,不要"先 Gemini 再说"
- **Provider → AI SDK protocol 适配层放哪儿** —— `LlmProvider` 自己 emit protocol 文本(每个 provider 重复实现) / trait 输出 normalized event(`TextDelta`/`ToolCall`/`Stop`)由 `recall::stream` 统一转 protocol(**推荐后者**,多 provider 时省一份重复)
- **命中率评估方法** —— 用户自评 / 外部 dogfooder(1-2 人) / LLM-as-judge;**自评不可证伪**(项目 9 个月里同一坑踩过两次),Phase 3 启动前敲定第三方判定机制
- **失败模式三分类口径** —— 每道答错的题必须归类为 (a) 检索没召回 (b) 帧里就没信息(extraction 漏 / 帧没存) (c) LLM 拿到帧没用对;Phase 3 启动前定模板,否则 60% 不达标时无法判断是 wedge 死了还是工具栈太弱(同一类失败用不同的下一步)
- **§十六 wedge 终判定数字与本 Phase 验收数字对齐** —— Phase 3 验收写"≥ 60%",§十六 终判定写"≥ 70%";Phase 3 启动前必须明示二者关系(Phase 3 是阶段 ship gate,§十六 是项目存续 gate)或统一数字
- **Phase 3 是否提前接 jieba 分词** —— 原计划推迟到 Phase 4,但中文 BM25 在 unicode61 下召回偏低,可能让 100 题命中率失真;Phase 3 启动前重新评估是否前置,否则失败原因混入分词问题分不开
- **cited_frame_ids 提取方式** —— LLM 在终态消息输出 JSON / 从 tool_use 历史反推;Phase 3 设计时定
- **`get_frame_screenshot_b64` 工具是否纳入 Phase 3** —— 此工具被 LLM 调用时会让截图进入 LLM API request,与 §十四"截图永远落本地"承诺**直接冲突**;Phase 3 启动前必须明示:要么改工具语义(本地 vision 看图 / 文本结果回 LLM),要么改隐私文案

**验收**
- 100 个真实问题(从一周 dogfood 整理出),命中率 ≥ 60%
- 答错的问题里 ≥ 80% 是因为 LLM 没找对帧,而不是帧里没有信息
- p50 首 token 延迟 < 3s

> 这一步是 wedge。如果 Phase 3 的"100 个问题命中率"达不到 60%,**全停一周**反思,而不是继续往 Phase 4。

### Phase 4 · Embedding + 语义搜索

**范围**
- LlmProvider 加 `embed`
- `frame_embeddings` 表 + 后台 indexer 任务
- search_frames 改为 hybrid(BM25 + cosine,RRF 融合)
- jieba tokenizer 接到 frames_fts

**启动前决策**

§五 的 `frame_embeddings` 表只是占位符,以下决策启动前必须逐条落定:

- **embed 单位** —— per frame / per chunk;选 chunk 必须改 schema(影响下面几条)
- **chunking 策略** —— 字符滑窗(N 字符 / M overlap) / AX reading order block 切分;影响召回粒度与冷启动成本
- **frame_embeddings schema 修正** —— PK 是否扩成 `(frame_id, chunk_id, model)`;是否加 `dim INTEGER` 列(BLOB 自身不带维度元数据,跨模型混存会让 cosine 崩)
- **多 model 并存政策** —— 互斥(切模型 = 全表 wipe + re-embed) / 双写 migration 期 / 按 model 分表
- **ANN 索引方案** —— 全表 cosine(< 10 万行) / sqlite-vec(扩展打包 + 跨平台签名) / hnswlib-rs(独立内存索引,启动重建);Phase 4 启动前定切换阈值与首选方案
- **embedding 模型选择** —— 本地(bge-m3-small via candle / fastembed-rs) / 云(Voyage-3 / OpenAI text-embedding-3-small);决策延迟到 Phase 4 dogfood 是合理的,但选哪个会反推到上面 schema 决策(尤其维度与 input_type 支持)
- **query 端 embedding** —— 与 frame 同模型 / 不同;是否使用 query/document input_type(Voyage-3 有,bge-m3 没有 —— 这反推模型选择)
- **RRF 融合参数** —— k 默认值(典型 60);两路 rank 是否各自 normalize;tie-break 规则
- **冷启动批量策略** —— 新帧 priority queue + 历史回填夜间 throttled;batch size 与 rate limit 退避;部署时已存的 N 万帧首次 backfill 估时与估钱
- **验收指标分母拆分** —— 验收"多答对 ≥ 15 题"的分母必须只算 Phase 3 失败分类中的 (a) 检索没召回,而不是总错题数 —— 否则当 (b)(c) 占比高时 embedding 即使没贡献也能被表面达标

**验收**
- Phase 3 那 100 题里,语义搜索能多答对 ≥ 15 题
- embedding 后台任务对前台 capture 无可感影响

### Phase 5 · Quick Ask + Per-app Adapters

**范围**
- 全局快捷键注册(`tauri-plugin-global-shortcut`)+ Settings 重绑 UI + 冲突检测
- `quick-ask.html` 独立窗口 + NSPanel-style 配置(NSWindow level、不抢激活态)
- `services/extractor/adapters/` 框架 + 起步 6 个 adapter:**Chrome / Safari / VSCode / JetBrains / iTerm2 / 飞书(Lark)** + GenericAxAdapter 兜底
- `quick_ask_orchestrator.md` + Quick Ask 专用 system prompt 渲染
- `capture_pipeline.invoke_quick_ask()` 入口(skip dedup / skip idle)
- `get_current_screen` 工具补到 /ask(Phase 3 推迟的部分)
- `quick_ask_pin_thread` 把浮窗对话钉到主窗口

**启动前决策**

- **快捷键默认值** —— `⌥ Space` / `⌘⇧ Space` / 不预设(强制用户绑);Phase 5 启动前定;若选 `⌥ Space` 必须在 onboarding 检 Spotlight 等系统占用并提示
- **Quick Ask 浮窗与主窗口共享 chat 持久化策略** —— 默认全部入 chat_threads(可在 /ask 找到) / 默认匿名仅当用户 pin 才入库;Phase 5 启动前定,因为决定 chat_threads 表的 churn 量
- **6 个 adapter 内部排序** —— 6 个全部 P0(都要 ship 才进 Phase 5 验收),但实施顺序需要排:建议 **Chrome → VSCode → 飞书 → JetBrains → Safari → iTerm2**(把 Chromium-Electron 类的早做完拿到通用经验,Safari 因 AX 桥最完整反而最简单留后)。Phase 5 启动前确认或重排。
- **adapter 失败回落语义** —— Adapter 返回 confidence=Low 时,dispatcher 是用 adapter 结果 + 通用 AX 拼,还是丢弃 adapter 结果走纯通用 AX;Phase 5 启动前定一条规则,后续 adapter 实现遵守
- **Littlebird parser 参考路径** —— 飞书 / Chrome / VSCode 这几个的 AX 信号挑选策略可直接对照 Littlebird ContextKit-cli 解包出的对应 JSParser;Phase 5 启动前确认你已拿到那批参考代码,并整理出每个 app "看哪些 AX role / 怎么过 ARIA noise" 的 cheatsheet
- **adapter 单测策略** —— 每个 adapter 至少给 1 份冻结的 AX dump fixture(JSON 落 `tests/fixtures/ax/`)+ 期望输出对比;Phase 5 启动前敲定 fixture 收集流程,不要边实现边凑
- **Quick Ask 召唤延迟预算** —— 从按下快捷键到浮窗 input 可输入的时间;启动前定一个具体目标(建议 < 80ms 显示窗口,focus context 异步),否则 UX 很容易劣化到"按下没反应"
- **focus_context 上限** —— LLM context 注入的 primary_text 字符上限(建议 8k tokens);Phase 5 启动前定;超时的截断策略(头尾保留 / 摘要 / 让 LLM 主动调 get_frame_text)需选一
- **AX 排除是否对 Quick Ask 同样生效** —— 用户在 Signal 里按快捷键时,adapter 是否也 skip;**默认 skip**(对齐 §四 触发条件表),Phase 5 启动前在 spec 与代码里双确认

**验收**
- 6 个 adapter 全部命中应用下,焦点提取准确率 ≥ 80%(LLM 拿到的 primary_text 是用户当时真在看的东西)
- 召唤延迟:按下快捷键 → 浮窗可输入 < 100ms p95
- 一周 dogfood:每天主动召唤 Quick Ask ≥ 5 次,且其中 ≥ 60% 的对话用户主观判断"答得有用"
- adapter 失败/没有 adapter 命中时,GenericAxAdapter 兜底不崩,且浮窗能给用户清楚的状态提示

---

## 十四、横向关切

### 隐私

- **截图永远落本地**。Phase 1-5 全程不上传截图。
- **文本走云**(LLM API)。第一次启动 onboarding 明示。
- **API key 在 macOS Keychain**,用户可见可改可清。
- **排除引擎**默认开:Signal、密码管理器、banking/adult 域名分类。
- **Hard delete**:Settings 提供"删除所有 frames + 截图 + chat 历史"按钮,一次性清空 SQLite + `$APPDATA/captures/`。

### 时间纪律

复用现有 `db/time.rs`(DbInstant + Clock + tests/time_discipline.rs)。新 schema 所有 `*_at` 列都有 GLOB 约束。`datetime('now')` 仍然禁。

### 性能预算

| 操作 | p95 |
|---|---|
| capture → frame 落库(OCR) | < 2s |
| capture → frame 落库(AX) | < 500ms |
| `/ask` 首 token | < 3s |
| `/timeline` 渲染 1000 帧 | < 200ms |

### 错误处理

`error::CorivoError`(thiserror)+ `domain::ipc_error::TauriError`(tagged union)沿用现有约定。前端 `fromInvokeError`。

### 测试

- Rust 单测:repos / dispatcher / dedup / exclusion 引擎全覆盖
- Rust 集成测:`tests/` 下复制现状的 `pool_isolation` / `time_discipline`,新增 `capture_pipeline_e2e` / `recall_tool_loop`
- 前端单测:Vitest node 环境(沿用)
- LLM 测:`mock` provider 必须能完整跑通 tool use 循环,作为 CI

### Cloud-pivot 路径(不构建,只标位置)

未来要切云:
1. 实现 `CloudConsumer: SnapshotConsumer`,捕获后整个 envelope POST 到服务端
2. `LlmProvider` 增加 `ServerProxyProvider` 实现,客户端只持 thin Bearer token
3. `LocalConsumer` 改成"本地缓存 + 异步 sync to cloud"(和 CloudConsumer 编排成 dual-write)
4. 服务端复用 frames schema(SQLite → PG/clickhouse)+ 同一份 prompt

**没有任何一项需要改采集层、提取层、UI 层**。

---

## 十五、跨阶段纪律

阶段内的具体决策已合并到 §十三 各 Phase 的 **启动前决策** 子节(原 Open Questions 已被吸收 + 多条 critique 项一并补入)。本节只列**跨整个 spec 生命周期都要遵守**的机械纪律 —— 它们不属于某个 Phase,但每个 Phase 都受其约束。

### envelope schema 版本纪律

[SnapshotEnvelope](#四snapshotenvelope-v1) 是采集层与下游解耦的唯一 seam(§二.2)。这条 seam 要真"不可妥协",必须配一条机械规矩:

- **任何 envelope 字段增删改 = 强制 bump `v` 字段**,不允许在 `v: 1` 上偷偷加字段
- **bump v 的 PR 必须同时显式更新所有 SnapshotConsumer 实现** —— LocalConsumer / TestConsumer / (未来 CloudConsumer) 都要 handle 旧 v,不写就 review 拒掉
- **envelope 字段命名 / 嵌套层次进入 spec 后视为 contract**,改名要走 deprecation 一个 Phase

没有这条机械纪律,"采集层与下游解耦"是口号不是制度。

### Day One 数据弃权与 retention 的边界

§二.6 写"pre-release 重大 schema 变更视作可丢"。但 retention(Phase 1 启动前决策中的"截图 / frames 文本 retention 策略")**不是** schema 变更触发的丢弃,是**正常运行时的数据回收策略**。两条路径不能混淆:

- schema bump → purge-and-apply,沿用 [db/migrations.rs](../src-tauri/src/db/migrations.rs)
- retention → 后台定时 hard-delete,Phase 1 实施时定算法(按 `captured_at` 老于 N 天)

### Phase gate 的执行纪律

spec 三处提到"必须停"(Phase 3 不达标全停一周 / §十三 N+1 阶段必须等 N ship / §十六 wedge 不立则停)。**写"必须停" ≠ 真的会停** —— 项目 9 个月 3 次架构都在同一句话上没停下。让 stop 真的可执行,需要两条结构性约束:

- **每个 Phase 的失败模式必须能被外部判定** —— 否则会下意识把"答不出的题"改判为"答对了"。Phase 3 / Phase 5 启动前决策已明示需要第三方判定机制
- **gate 不达标的回退路径必须在该 Phase 启动前写定** —— 不达标时回到上一 Phase 的什么状态、保留什么、丢弃什么。spec 当前没写,留给每个 Phase 启动时一并落定

### 隐私承诺与工具集合的一致性纪律

§十四 "截图永远落本地"是公开承诺。任何 LLM 工具(当前 `get_frame_screenshot_b64`,未来可能新增)只要会让截图进入 LLM API request,就**直接违背承诺**。规矩:

- 新增 `recall::tools` 工具时,**必须显式标注是否会让截图/原图离开本机**
- 任何这类工具的实现 PR 必须同时更新 §十四 隐私文案,或改工具语义(本地 vision 看图 / 只回文本)
- 不允许"反正用户大概不会注意"的隐式偏离

---

## 十六、Wedge 验证标准

整套架构的成败由两个独立的判定指标,**分别**对应 Phase 3 和 Phase 5。其中一个不立,整套召回前提倒下。

### 主 wedge · `/ask`(Phase 3 退出条件)

> 用户在一周内,至少 5 天主动打开 `/ask`,Phase 3 启动前冻结的 100 题命中率(经第三方 / LLM-as-judge 评估,不是自评) ≥ 70%。

到不了这个数:不是 prompt 问题、不是模型问题、不是 UI 问题,是**跨时间召回前提本身没立**。Phase 4 (embedding) 是这一阶段的备用救援。

### 次 wedge · Quick Ask(Phase 5 退出条件)

> 用户在一周内,每天主动召唤 Quick Ask ≥ 5 次,且其中 ≥ 60% 的对话用户主观判断"答得有用"。同时 Quick Ask 浮窗在 6 个 P0/P1 adapter 命中应用里,焦点提取准确率 ≥ 80%。

Quick Ask 验证的是另一个相关但不同的命题:**"AI 看着我手头的事 + 我的历史" 比 "AI 看我的历史" 更高频被使用**。如果用户每天 /ask 用得很爽但 Quick Ask 没用起来,说明 Corivo 是个搜索工具,不是上下文助手。

### 双 wedge 关系

| /ask 立 | Quick Ask 立 | 解读 |
|---|---|---|
| ✅ | ✅ | 完整命题成立,进 Phase 6 / 云路径 |
| ✅ | ❌ | 持续观察 + 召回有用,但用户不需要 "看我屏幕" 的浮窗;退化成"屏幕历史搜索引擎",仍可继续但产品定位重写 |
| ❌ | ✅ | 跨时间召回不立,但当下 focus + LLM 已经够用;**这就是 ChatGPT desktop 的形态**,Corivo 的"持续观察"价值被否定,大幅降范围 |
| ❌ | ❌ | 整套召回前提倒下,停。 |

不立则停 —— 不是修一周再试,是结构性反思该不该继续。
