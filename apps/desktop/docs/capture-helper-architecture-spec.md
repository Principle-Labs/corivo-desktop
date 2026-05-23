# Capture Helper 跨平台架构 · spec

> 状态：v1 设计稿，完成 review 后启动 Spec 2 (macOS impl) / Spec 3 (Windows impl) 的实施。
> 这是**契约 spec** —— 锁定 Rust 主进程 ↔ Helper 之间的所有边界、IPC 协议、生命周期。两个平台 helper 都必须实现这份契约。

---

## 1. 目标

把所有"和 OS 谈话"的 platform-divergent 代码从 Rust 主进程剥离到一个独立的、按平台实现的 sidecar 进程，让 Rust 主进程**完全 platform-independent**，同时为后续会议录制 + 实时转录、跨平台扩展、长会话崩溃隔离打好基础。

## 2. 非目标（v1）

- 转录、LLM、UI、业务编排 —— 全留 Rust 主进程
- per-app text 后处理（lark / vscode / jetbrains adapter）—— 输入是字符串，platform-independent，留 Rust
- NSPanel overlay、全局热键、NSScreen 几何 —— UI 层就近放主进程
- per-app system audio 隔离（Core Audio Tap）—— v1 不做，留 v2 用 capability flag 平滑加
- **视频录制** —— v1 完全不做（会议产品只录音）
- 多 helper / helper pool —— 单进程足够
- 转录引擎内嵌 helper —— 转录是计算不是 I/O，留 Rust

## 3. Helper 边界（the contract）

| 功能 | Helper | Rust 主 | 边界依据 |
|---|---|---|---|
| 系统音频采集 | ✅ | | macOS Core Audio vs Windows WASAPI 完全异构 |
| 麦克风采集 | ✅ | | 同上 |
| 多轨 AAC 编码 | ✅ | | macOS AVAssetWriter / Windows MF Sink Writer |
| Segment 切分（10s） | ✅ | | 紧贴编码循环 |
| 单帧截图 | ✅ | | macOS SCK / Windows WGC，xcap 退役 |
| Accessibility 文本抽取（focused window walk） | ✅ | | AXUIElement vs IUIAutomation |
| Accessibility 选区探测 | ✅ | | AXTextMarker vs UIA TextPattern |
| Accessibility 事件订阅 | ✅ | | AXObserver+CFRunLoop vs UIA event handler |
| 前台 app 监听 | ✅ | | NSWorkspace observer vs SetWinEventHook |
| 本地 OCR | ✅ | | Vision vs Windows.Media.Ocr |
| 会议 app 白名单匹配 | | ✅ | 字符串比较，platform-independent |
| 录制 session 编排（start/stop/路径登记） | | ✅ | 业务逻辑 |
| 转录（whisper.cpp / 云 API） | | ✅ | platform-independent 计算 |
| per-app text adapter | | ✅ | 字符串处理 |
| NSPanel overlay 渲染 | | ✅ | 与 tauri-nspanel 紧耦合 |
| 全局热键（NSEvent / RegisterHotKey） | | ✅ | 触发主进程 UI |
| dock icon / activationPolicy | | ✅ | 主进程自身状态 |

**判定标准**（Day 1 复盘后归纳出的 4 条）：
1. API 是否 platform-divergent
2. 是否需要 OS-native delegate / callback / runloop
3. 是否值得崩溃隔离（长跑、未知输入、外来异常）
4. 权限是否能 / 应该 scoped 到独立 binary

任意 ≥ 3 条命中 → 进 helper；UI 强耦合的例外（NSPanel、热键）就近留主进程。

## 4. 进程模型

```
┌──────────────────────────────────────────────────────────────────┐
│ Rust 主进程 (Tauri)                                               │
│                                                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ services/capture_client (新)                              │   │
│  │  ├─ HelperProcess (spawn + lifecycle)                    │   │
│  │  ├─ Codec (NDJSON encode/decode)                         │   │
│  │  ├─ RpcRouter (id → response promise)                    │   │
│  │  ├─ EventBus (async stream of helper events)             │   │
│  │  └─ HealthMonitor (heartbeat + restart)                  │   │
│  └──────────────────────────────────────────────────────────┘   │
└────────────┬─────────────────────────────────────────────────────┘
             │ stdin/stdout (NDJSON)        stderr → tracing
             │
┌────────────▼─────────────────────────────────────────────────────┐
│ corivo-capture-helper-{macos,windows} (sidecar binary)           │
│                                                                   │
│  Swift (macOS) / C++ cppwinrt (Windows)                          │
│                                                                   │
│  ┌──────────────┬──────────────┬──────────────┬──────────────┐  │
│  │   Audio      │   Screen     │ Accessibility│  Foreground  │  │
│  │  Recorder    │   Capture    │   + Events   │   Monitor    │  │
│  └──────────────┴──────────────┴──────────────┴──────────────┘  │
│                                                                   │
│  ┌─────────────────────── IPC core ─────────────────────────┐   │
│  │ NDJSON reader (stdin) → dispatcher → handlers            │   │
│  │ Event emitter (stdout) ← handlers + event sources        │   │
│  └──────────────────────────────────────────────────────────┘   │
└───────────────────────────────────────────────────────────────────┘
```

**Sidecar 打包**：
- macOS: 通过 Tauri 的 `bundle.externalBin` 打入 `Corivo.app/Contents/MacOS/corivo-capture-helper-macos-aarch64-apple-darwin`
- Windows: 同机制打入 `corivo-capture-helper-windows-x86_64-pc-windows-msvc.exe`
- Tauri sidecar 命名约定使用 target triple 后缀（参考 [tauri-plugin-shell sidecar 文档](https://v2.tauri.app/develop/sidecar/)）

**生成方式**：
- macOS helper：独立 Xcode 项目，输出 universal binary，`pnpm tauri:build` 前由 `apps/desktop/scripts/build-helper-macos.sh` 调起 `xcodebuild`
- Windows helper：独立 CMake 项目，`apps/desktop/scripts/build-helper-windows.ps1` 调起 cmake + msbuild
- 两者输出落到 `apps/desktop/src-tauri/binaries/` 供 Tauri sidecar 机制拾取

**单实例约束**：
- Rust 主进程同时只能有 1 个 helper 进程在跑
- helper 启动时检测自己是不是被 Tauri 侧 spawn（通过环境变量 `CORIVO_HELPER_PARENT_PID`），不是就拒绝启动（防止用户误手动跑）

## 5. IPC 协议

### 5.1 传输

- **stdin (Rust → Helper)**：UTF-8 NDJSON，一行一个 JSON 对象，行尾 `\n`
- **stdout (Helper → Rust)**：同上
- **stderr (Helper → Rust)**：纯文本日志行，Rust 端转发到 `tracing::info!(target = "capture_helper", ...)`，不参与协议
- 单行最大 1 MB（防止异常 helper 把 Rust OOM）

### 5.2 消息分类

每条 JSON 必须含 `type` 字段。三类消息：

| 类型 | 方向 | 必含字段 | 用途 |
|---|---|---|---|
| **Request** | Rust → Helper | `type, id, payload` | RPC 调用，期望对应 response |
| **Response** | Helper → Rust | `type, id, ok, result?, error?` | 对 request 的回复，echo 同一个 `id` |
| **Event** | Helper → Rust | `type, ts, payload` | 异步推送，无 id |

**id 规则**：UUIDv4 字符串。Rust 客户端为每个 in-flight request 维护 `id → oneshot::Sender<Response>` 映射。

### 5.3 版本协商（核心机制）

**Helper 启动后必须发的第一条消息**（在任何其他消息之前）：

```json
{
  "type": "hello",
  "helper_version": "1.2.3",
  "supported_protocols": ["v1"],
  "capabilities": {
    "audio_record": true,
    "audio_per_app": false,
    "screen_capture": true,
    "ax_query": true,
    "ax_events": true,
    "foreground_monitor": true,
    "ocr_local": true
  },
  "platform": {
    "os": "macos",
    "os_version": "14.5.0",
    "arch": "arm64"
  }
}
```

**Rust 主回复**（必须在 5s 内）：

```json
{
  "type": "hello_ack",
  "selected_protocol": "v1",
  "client_version": "0.1.0"
}
```

握手规则：
- 协议号 `v1`、`v2` 等是离散字符串。Rust 从 helper 的 `supported_protocols` 里挑**自己也支持的最高版本**
- `selected_protocol` 之后所有消息必须按该协议的 schema
- 不兼容（无交集）→ Rust kill helper + 进入 degraded 模式 + emit `degraded` event 到主程序
- helper_version 是 SemVer 字符串，Rust 用它做"功能可用性"诊断而非协议判断

**capabilities 是 Rust 决策的唯一依据**：Rust 不应通过 `helper_version` 或 `os_version` 推断功能。
- `audio_per_app: true` → 才显示"按 app 录"选项
- `ocr_local: false` → 截图 + AX 抽不到字时 fallback 到云 OCR 或不做 OCR
- 任何 capability 缺失 → Rust 优雅降级，不报错

### 5.4 v1 消息列表

#### 控制类

| Request | 说明 | result schema |
|---|---|---|
| `ping` | 健康检查（除 heartbeat 外的兜底） | `{ "pong": true }` |
| `shutdown` | 优雅停机（等 in-flight 完成、关 stdout、退出 0） | `{ "ack": true }` |

#### 音频录制

| Request | payload | result |
|---|---|---|
| `recording.start` | `{ session_id, output_dir, segment_seconds, capture_system_audio: bool, capture_microphone: bool, microphone_device_id?: string }` | `{ session_id, started_at }` |
| `recording.stop` | `{ session_id }` | `{ session_id, stopped_at, total_segments }` |
| `recording.list_microphones` | `{}` | `{ devices: [{ id, name, is_default }] }` |

| Event | payload |
|---|---|
| `recording.segment_closed` | `{ session_id, segment_index, path, duration_ms, started_at, ended_at, sys_track_present, mic_track_present }` |
| `recording.error` | `{ session_id, code, message, fatal: bool }` |

**输出文件结构**（helper 直接写盘，Rust 拿路径）：
```
<output_dir>/
  segment-0000.m4a   (sys + mic 多轨 AAC)
  segment-0001.m4a
  ...
  manifest.json      (session 元数据，stop 时写)
```

#### 单帧截图

| Request | payload | result |
|---|---|---|
| `screen.capture` | `{ display_id?: string, output_path?: string, format: "jpeg" \| "png", quality?: 0..100 }` | `{ path, width, height, captured_at }` |
| `screen.list_displays` | `{}` | `{ displays: [{ id, name, width, height, is_main }] }` |

`output_path` 省略 → helper 写到 `$TEMP/corivo-helper/screenshot-<ulid>.jpg`，Rust 读完负责删。

#### Accessibility

| Request | payload | result |
|---|---|---|
| `ax.query` | `{ pid, max_depth?: 64, max_chars?: 64000, deadline_ms?: 1500, skip_predicate?: SkipPredicate }` | `{ text, role_tags: [{ offset, len, tag }], elapsed_ms, truncated: bool }` |
| `ax.probe_selection` | `{ pid, deadline_ms?: 250 }` | `{ selection?: string }` |

`SkipPredicate`：v1 仅支持 `{ skip_roles?: [string], skip_subroles?: [string], skip_descriptions_substr?: [string] }`，等价于现有 [adapters/lark.rs](apps/desktop/src-tauri/src/services/extractor/adapters/lark.rs) 等的需求。复杂闭包逻辑保留给 Rust 主侧（拿到原始 tree 的 outline 后处理）—— v2 协议可加 `ax.query_tree` 直接返回结构化树。

| Event | payload |
|---|---|
| `ax.focused_window_changed` | `{ pid, bundle_id?, window_title? }` |
| `ax.title_changed` | `{ pid, bundle_id?, window_title? }` |

订阅控制：

| Request | payload | result |
|---|---|---|
| `ax.subscribe` | `{ pid, notifications: ["focused_window", "title", ...] }` | `{ subscribed: [...] }` |
| `ax.unsubscribe` | `{ pid }` | `{ ack: true }` |

helper 内部维持"一次只订一个 pid"，新订阅自动替换旧订阅（对应 [ax_observer.rs](apps/desktop/src-tauri/src/services/capture_pipeline/event_sources/ax_observer.rs) 当前行为）。

#### 前台监听

| Request | payload | result |
|---|---|---|
| `foreground.subscribe` | `{}` | `{ subscribed: true }` |
| `foreground.unsubscribe` | `{}` | `{ ack: true }` |
| `foreground.current` | `{}` | `{ pid?, bundle_id?, app_name?, window_title? }` |

| Event | payload |
|---|---|
| `foreground.app_activated` | `{ pid?, bundle_id?, app_name?, window_title?, ts }` |

helper 启动后默认**不**订阅；Rust 主进程显式 subscribe（保留主进程对事件流量的控制权）。

#### OCR

| Request | payload | result |
|---|---|---|
| `ocr.run` | `{ image_path, languages?: ["zh-Hans","zh-Hant","en-US"], use_language_correction?: false }` | `{ text, elapsed_ms }` |

#### 心跳与诊断

| Event | payload |
|---|---|
| `heartbeat` | `{ ts, in_flight_requests, recording_sessions: [{session_id, segments_written}], ax_subscriptions: [pid] }` |
| `log` | `{ level, message, target?, fields? }` —— 用于 stderr 之外的结构化日志（少用，除非确实需要走主通道） |

heartbeat 每 5s 一发。Rust 端监听 `last_heartbeat_at`，> 15s 触发重启。

### 5.5 错误码（Response 的 `error.code`）

| code | 说明 |
|---|---|
| `INVALID_REQUEST` | 协议错（缺字段、id 重复、enum 越界） |
| `UNSUPPORTED` | 当前 capability 不支持 |
| `PERMISSION_DENIED` | 需要的系统权限未授予 |
| `RESOURCE_BUSY` | 录音 session 已在跑 / 同 pid 已订阅 |
| `NOT_FOUND` | pid / display_id / session_id 不存在 |
| `TIMEOUT` | 内部 deadline 命中 |
| `INTERNAL` | helper 内部异常（带堆栈到 `error.detail`） |
| `OS_ERROR` | OS API 返回非零（带 OSStatus / HRESULT 到 `error.detail.os_code`） |

Rust 端把所有 helper 错误映射到 `services::capture_client::error::CaptureError`（强类型枚举）。

### 5.6 协议变更政策

- `v1` 一旦发布**只能加 optional 字段**，不能改语义、不能删字段
- 任何破坏性变更走新协议号 `v2`
- helper 必须**同时支持** `[v1, v2]` 直到 Rust 主进程的最低 client_version 不再请求 v1
- Rust 主进程在 hello_ack 选择 `v1`，helper 就只用 v1 schema —— v1 的 handler 路径不能被 v2 的逻辑污染
- 协议 schema JSON Schema 文件放 `apps/desktop/src-tauri/schemas/capture-helper-v1.json`，Rust + 两个 helper 共享，CI 验证

## 6. 生命周期

### 6.1 启动序列

```
T0    Rust spawn helper(env: CORIVO_HELPER_PARENT_PID=<rust pid>)
T0+δ  Helper start → write hello to stdout
T0+δ' Rust read hello → write hello_ack
T1    Rust 标记 helper Ready，开始接受调用
```

启动失败处理：
- helper 在 5s 内没发 hello → Rust kill + retry（最多 3 次，指数 backoff 1s/2s/4s）
- 3 次都失败 → Rust emit `capture_helper:degraded` Tauri event，主程序进入"无 helper"模式：录音/截图/AX 全部 fail-fast，前台监听用 Rust 内置的最小化 fallback（仅在 macOS：Tauri 自带的 active-app 查询；Windows：完全没有）

### 6.2 心跳 & 健康监测

Helper 每 5s emit `heartbeat`。Rust 端规则：
- 连续 3 次 miss（即 ≥ 15s 无心跳）→ helper 视为僵死
- 视为僵死后：kill (-9) → 触发 6.3 重启
- in-flight requests 全部 fail with `CaptureError::HelperCrashed`

### 6.3 崩溃恢复

helper 进程意外退出（exit code != 0 / signal / 心跳超时）：

1. Rust capture_client 捕获 stdout EOF
2. 标记 helper Dead，所有 in-flight request 拒绝
3. 触发 restart 策略：
   - 录音 session 进行中 → 立即 spawn 新 helper，新 helper hello 后**自动恢复**未完成的录音 session（接着写下一个 segment-NNNN，segment_index 不重置；这意味着 helper 启动时必须扫 `output_dir` 推断 next index）
   - 无录音 session → spawn 但不重新订阅；让 Rust 主侧的逻辑按需重新调用 `foreground.subscribe` / `ax.subscribe`
4. emit `capture_helper:restarted` Tauri event 给前端（让 UI 知道有过中断）

**恢复上限**：1 分钟内 ≥ 3 次崩溃 → 停止重启，进入 degraded 模式（同 6.1）。

### 6.4 优雅关闭

Rust 主程序退出时：
1. send `shutdown` request
2. helper 完成 in-flight + 关闭 stdout
3. Rust 等 helper 自然退出，超时 5s → 发 SIGTERM → 再超时 2s → SIGKILL

helper 收到 SIGTERM / Ctrl-C 也走同样的优雅路径，确保录音 session 落盘 + 写 manifest。

## 7. Rust 客户端（services/capture_client/）

### 7.1 模块结构

```
services/capture_client/
  mod.rs              pub use 入口，启动函数 install()
  process.rs          HelperProcess: spawn / wait / kill
  codec.rs            NDJSON 行编/解码
  protocol.rs         v1 schema 类型（用 serde derive；与 schemas/ 对应）
  router.rs           RpcRouter: id ↔ oneshot::Sender 映射
  events.rs           EventBus: tokio::broadcast::Sender<HelperEvent>
  health.rs           HeartbeatMonitor + 重启策略
  client.rs           CaptureClient（公共 API，await-friendly）
  error.rs            CaptureError 枚举
```

### 7.2 公共 API（client.rs）

```rust
pub struct CaptureClient { /* ... */ }

impl CaptureClient {
    /// 录音
    pub async fn start_recording(&self, opts: StartRecordingOpts) -> Result<RecordingHandle>;
    pub async fn list_microphones(&self) -> Result<Vec<MicrophoneDevice>>;

    /// 截图
    pub async fn capture_screen(&self, opts: CaptureScreenOpts) -> Result<Screenshot>;
    pub async fn list_displays(&self) -> Result<Vec<Display>>;

    /// AX
    pub async fn ax_query(&self, pid: i32, opts: AxQueryOpts) -> Result<AxText>;
    pub async fn ax_probe_selection(&self, pid: i32) -> Result<Option<String>>;
    pub fn subscribe_ax_events(&self, pid: i32) -> Result<broadcast::Receiver<AxEvent>>;

    /// 前台监听
    pub async fn current_foreground(&self) -> Result<ForegroundApp>;
    pub fn subscribe_foreground(&self) -> Result<broadcast::Receiver<ForegroundEvent>>;

    /// OCR
    pub async fn run_ocr(&self, opts: OcrOpts) -> Result<String>;

    /// 健康
    pub fn health(&self) -> HelperHealth;
    pub fn capabilities(&self) -> &HelperCapabilities;
}

pub struct RecordingHandle { /* session_id + Drop 自动 stop */ }
```

`RecordingHandle` 的 Drop 必须 best-effort 调 `recording.stop` —— 用户拖 Quick Ask 把主进程切出去时也不能让录音继续。

### 7.3 启动接入

```rust
// lib.rs::run() 里
let capture_client = capture_client::install(&app_handle).await?;
state.set_capture_client(capture_client);
```

`install()` 内部 spawn helper、跑握手、启动 reader / writer / health task，返回 ready 的 client。失败返回带原因的 error，主程序根据是否 fatal 决定 abort 还是 degraded 启动。

### 7.4 测试支持

```rust
#[cfg(any(test, feature = "test-support"))]
pub mod mock {
    pub struct MockCaptureClient { /* 编程式响应 */ }
    pub fn mock_client() -> (MockCaptureClient, MockHelperHandle) { ... }
}
```

集成测试不 spawn 真 helper，用 MockCaptureClient 注入。

helper 自身也提供 `--echo-mode`：忽略所有平台调用、对每个 request 返回固定 stub —— 用于 CI 验证 IPC 协议本身（不需要 macOS / Windows runner 的功能性能力）。

## 8. 文件系统布局

```
$APPDATA/captures/
  recordings/
    <session-ulid>/
      segment-0000.m4a
      segment-0001.m4a
      ...
      manifest.json     {created_at, ended_at, segments: [{index,path,duration_ms,...}], system_audio: bool, microphone: { device_id, device_name } | null}
  screenshots/
    <frame-ulid>.jpg    (helper 写，Rust 在 frames 入库后保留；不入库的写到 $TEMP)
$TEMP/corivo-helper/
  screenshot-<ulid>.jpg (no output_path 时的临时文件；helper 不负责清理，Rust 用完即删)
```

跨平台路径：
- macOS: `~/Library/Application Support/com.corivo.app/captures/`
- Windows: `%APPDATA%\Corivo\captures\`
- Tauri 的 `app_data_dir()` 已经处理；helper 在 `recording.start` 的 `output_dir` 由 Rust 算好后传给它，helper 不自己解析平台路径

## 9. 错误模型

```rust
#[derive(thiserror::Error, Debug)]
pub enum CaptureError {
    #[error("helper not started or has crashed")]
    HelperUnavailable,

    #[error("helper crashed during request")]
    HelperCrashed,

    #[error("transport error: {0}")]
    Transport(String),

    #[error("helper rejected request: {code} - {message}")]
    HelperError { code: String, message: String, detail: serde_json::Value },

    #[error("permission denied for {what}")]
    PermissionDenied { what: PermissionKind },  // Microphone | ScreenRecording | Accessibility

    #[error("capability {0} not supported by current helper")]
    Unsupported(String),

    #[error("timeout after {0:?}")]
    Timeout(Duration),

    #[error("internal: {0}")]
    Internal(String),
}
```

`PermissionDenied` 是单独枚举值（不混在 HelperError 里），让 UI 可以直接驱动权限引导流。

## 10. 权限模型

| 平台 | 权限 | 申请方 | UX |
|---|---|---|---|
| macOS | Microphone (`NSMicrophoneUsageDescription`) | helper bundle | 首次 `recording.start` 时系统弹窗，对话框 app 名是 helper bundle 的 `CFBundleDisplayName`（"Corivo Capture"） |
| macOS | Screen Recording | helper binary | 首次 `screen.capture` 或 `recording.start`（系统音通过 SCK） |
| macOS | Accessibility | helper binary | 首次 `ax.query` 或 `ax.subscribe` —— **关键好处**：Accessibility 是 per-bundle-id 授权，让用户授权给 "Corivo Capture Helper" 而不是 "Corivo" 主品牌 |
| Windows | Microphone (Settings > Privacy > Microphone) | helper exe | 通过 `IsAccessAllowed` 检测；未授权时 `recording.start` 返回 `PERMISSION_DENIED` |
| Windows | Screen Capture | n/a | Win10/11 不需要单独权限 |
| Windows | UI Automation | n/a | 不需要权限 |

Helper 提供独立的权限查询 RPC：

| Request | result |
|---|---|
| `permission.status` | `{ microphone: "granted"\|"denied"\|"undetermined", screen_recording: ..., accessibility: ... }` |
| `permission.request` | `{ kind: "microphone"\|"screen_recording"\|"accessibility" }` → 触发系统弹窗 / 打开系统设置面板 |

Rust 主程序的 onboarding 页面调这两个 RPC 引导用户。

## 11. 测试策略

| 层 | 工具 | 覆盖 |
|---|---|---|
| 协议 schema | JSON Schema validator | helper / Rust 双向用同一份 schema 校验 |
| Rust 客户端单元 | tokio test + MockHelperHandle | router、health、错误转换 |
| Rust 客户端集成 | helper `--echo-mode` | 全协议路径，**不需要平台能力** |
| macOS helper 单元 | XCTest | audio session lifecycle、AX walk、AVAssetWriter rotate |
| Windows helper 单元 | GoogleTest | WASAPI capture loop、SinkWriter rotate、UIA traversal |
| 端到端 | Tauri 集成测试 in CI | macOS runner 真录 30s、读 ax、截屏；Windows runner 同 |
| 1 小时 soak | 手动 + 计划 cron | 验证 segment rotation 不漏、内存不涨、崩溃自动恢复 |

## 12. 迁移计划

按依赖关系分阶段，每阶段独立可发：

**Phase 0：基建（1.5 周）**
- [ ] 定 v1 JSON Schema → 提交 `apps/desktop/src-tauri/schemas/capture-helper-v1.json`
- [ ] 实现 `services/capture_client/`（包括 echo-mode 集成测试）
- [ ] 实现 helper skeleton（macOS + Windows 同时进，包括 hello / heartbeat / shutdown / ping / `--echo-mode`）
- [ ] CI 跑通 echo-mode 集成测试

**Phase 1：迁移截图（0.5 周）**
- [ ] helper 实现 `screen.capture` + `screen.list_displays`
- [ ] Rust 把 [screen_capture.rs](apps/desktop/src-tauri/src/services/capture_pipeline/screen_capture.rs) 替换为 capture_client 调用
- [ ] 删除 `xcap` 依赖
- [ ] 验证 capture_pipeline 端到端不退化

**Phase 2：迁移 AX（1 周）**
- [ ] helper 实现 `ax.query` + `ax.probe_selection`
- [ ] Rust 把 [ax_extractor.rs](apps/desktop/src-tauri/src/services/extractor/ax_extractor.rs) + [selection_probe.rs](apps/desktop/src-tauri/src/services/extractor/selection_probe.rs) 替换为 capture_client 调用
- [ ] 验证 lark / generic_ax adapter 仍工作

**Phase 3：迁移 AX events + 前台监听（1 周）**
- [ ] helper 实现 `ax.subscribe` + `foreground.*`
- [ ] Rust 删除 [ax_observer.rs](apps/desktop/src-tauri/src/services/capture_pipeline/event_sources/ax_observer.rs) + [foreground_monitor/macos.rs](apps/desktop/src-tauri/src/services/foreground_monitor/macos.rs)
- [ ] 验证 Quick Ask focus 跟踪仍工作

**Phase 4：迁移 OCR（0.5 周）**
- [ ] helper 实现 `ocr.run`
- [ ] Rust 替换 [ocr_extractor.rs](apps/desktop/src-tauri/src/services/extractor/ocr_extractor.rs)
- [ ] 删除 `objc2-vision` 依赖

**Phase 5：录音 + 转录（2 周）**
- [ ] helper 实现 `recording.*` 全套
- [ ] Rust 实现 `services/recording_orchestrator/` + `services/transcription_service/`（whisper.cpp 本地，[Spec 1 范围之外，独立 spec]）
- [ ] 实现 `services/foreground_monitor` 之上的会议 app 检测 + Toast 引导

**Phase 6：清理（0.5 周）**
- [ ] 删除 Rust 端的 `accessibility-sys` / `objc2-app-kit` (capture 相关) / `core-foundation` / `block2` / `objc2` / `objc2-foundation` 依赖
- [ ] 保留：`objc2-app-kit`（NSPanel / NSEvent / NSScreen 仍用）、`tauri-nspanel`
- [ ] 文档更新

合计 **~6.5 周**，可由 macOS + Windows 工程师并行（Windows helper 从 Phase 0 起独立推进）。

## 13. 风险与决策

| 风险 | 缓解 |
|---|---|
| AX 高频调用 + IPC 序列化反序列化成本（100ms 一次截图 + AX）超过现状 | benchmark 阶段做：单次 AX query JSON 序列化预估 < 5ms（典型 8KB 文本），总开销 < 10%；不可接受时考虑 binary protocol（CBOR）作为 v2 |
| helper 启动耗时拖慢 app cold start | Rust 主进程异步 spawn，不阻塞 boot；degraded 模式让 UI 先起来 |
| Windows AAC 编码许可（MS 的 AAC encoder 在某些 SKU 受限） | Phase 5 启动前验证 Win10/11 Home/Pro 都可用；不行则改用 MFAac wrap libfdk-aac (license 需评估) |
| Core Audio Tap 推迟到 v2 后用户问"为啥 Spotify 也录进去了" | v1 默认开"录制时尽量静音其他 app"提示；v2 加 Tap 后用 capability flag 平滑切换 |
| 协议 schema 漂移导致 helper / Rust 不兼容 | schema 文件 SSOT + CI 双向验证 + capability negotiation 兜底 |

## 14. 附录：决策摘要

| # | 决策 | 依据 |
|---|---|---|
| 1 | helper 边界覆盖 audio + screen + AX + foreground + OCR，**不**含 NSPanel / 热键 / 业务逻辑 | Day 1 4 条标准；NSPanel 等与 Tauri 紧耦合就近 |
| 2 | sidecar 进程而非 dylib | 崩溃隔离对长会话录制不可让步 |
| 3 | NDJSON over stdio，不用 XPC / 命名管道 | 与 [exec_agent](apps/desktop/src-tauri/src/services/exec_agent/runner.rs) 同模式，跨平台一致，可观测性最佳 |
| 4 | 协议显式版本号 + capability flags | 让 helper 和 Rust 主进程能独立演进 |
| 5 | v1 不做 Core Audio Tap、不做 per-app 音频 | 全系统混合音对会议场景够用，复杂度大幅降低 |
| 6 | v1 不录视频 | 产品决策（会议产品只录音） |
| 7 | macOS 13+，Windows 10 1903+ | SCK 系统音 + WGC 单帧的最低要求 |
| 8 | 转录留 Rust，**不**进 helper | 转录是计算不是 I/O；保持 helper 极薄 |
| 9 | helper binary 名带 target triple | 跟随 Tauri sidecar 命名约定 |

---

## 15. 后续 spec

- **Spec 2** [capture-helper-macos-spec.md](capture-helper-macos-spec.md) —— Swift 实现细节
- **Spec 3** [capture-helper-windows-spec.md](capture-helper-windows-spec.md) —— C++/cppwinrt 实现细节
- **Spec 4** (待) —— 录制 + 转录 orchestrator（消费本 spec 的 helper API，加 whisper.cpp / 云 API、会议 app 检测、转录入库）
- **Spec 5** (待) —— Rust 主进程的 capture_pipeline 重构（消费 helper events 替代当前的 NSWorkspace + AXObserver 直连）
