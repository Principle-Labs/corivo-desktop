# Privacy Filter Spec

**Status**: Draft v1 — 等待 `openai-q4f16` eval 验证通过后转 Approved
**Owner**: TBD
**Last updated**: 2026-05-20

---

## 0. TL;DR

为 Corivo 加一层本地 PII 检测，目标是**捕获到屏幕文本时识别敏感信息、在数据离开本设备时执行 redact**。识别用 OpenAI privacy-filter q4f16 ONNX，跑在主 Tauri 进程里的 `ort` crate，CoreML EP + CPU fallback。

核心架构原则：

> **Classify once at capture, enforce at egress.**
> 模型在捕获时跑一次，结果（PII spans）作为元数据持久化；原文不改。任何离开本进程的数据出口都按 spans 做字符串替换。

唯一例外：`secret` 类目（密码/API key/token）在捕获时直接物理 redact，不留原文。

---

## 1. Goals / Non-goals

### Goals

- 阻止用户的 PII 在用户主动发往云端 LLM 时离开本设备
- 阻止 `secret` 类目数据落地到本地 SQLite
- 不破坏 Corivo 的核心能力（屏幕记录、本地搜索、回看历史）——本地数据原文保留
- 为未来的 egress 路径（同步、导出、分享、备份）提供统一的 enforce 模板
- Apple Silicon + Intel Mac 都能跑（ONNX 双架构）

### Non-goals

- **不**做合规认证（GDPR/HIPAA 担保）——这只是 privacy-by-design 的一层
- **不**做实时安全过滤（不是 prompt injection / jailbreak 防御）
- **不**支持运行时改类目策略——OpenAI 模型固定 8 类，加类目要重训
- **不**做服务端 redact / cloud sync——纯本地
- **不**做模型微调（产品未发布，无真实数据）

---

## 2. Threat Model

| 威胁 | 是否覆盖 | 覆盖方式 |
|---|---|---|
| PII 通过 LLM API 流向云端（OpenAI/Anthropic 等） | ✅ 主目标 | Egress redact |
| 高危凭据（密码、token）落地到本地 SQLite | ✅ | Capture-time hard redact |
| 普通 PII 在本地 SQLite 中驻留 | ⚠️ 接受 | 用户安装即接受 |
| 模型误判（FP/FN） | ⚠️ 缓解 | UI 预览 + 用户 override |
| 设备物理失窃后磁盘取证 | ❌ 不覆盖 | 依赖 macOS FileVault |
| 进程内存被木马读取 | ❌ 不覆盖 | OS sandbox 之外的范围 |

---

## 3. Architecture Overview

### 3.1 数据流（标注 vs 执行解耦）

```
┌──────────────────────────────────────────────────────────────────┐
│                  CLASSIFY PHASE (one-shot per frame)              │
│                                                                   │
│   AX text ──► hash check ──┬─ cache hit ──► spans (memoized)     │
│      │                      │                                     │
│      │                      └─ cache miss ──► ort::Session       │
│      │                                          │                 │
│      │                                          ▼                 │
│      └──────────────────────────► frames table (ax_text +        │
│                                                pii_spans JSON)    │
└──────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────┐
│                  ENFORCE PHASE (every egress)                     │
│                                                                   │
│   read frame (ax_text + spans) ──► apply_redact_template          │
│                                          │                        │
│   destinations:                          │                        │
│     • exec_agent → cloud LLM ◄──────────┤                        │
│     • future: export / share / sync ◄────┘                        │
└──────────────────────────────────────────────────────────────────┘
```

### 3.2 `secret` 类目特例

`secret` 命中的 span**不走上图**：在 capture phase 内部直接对 `ax_text` 做物理替换（写入 frames 之前），spans 表里这条记录的 type 标记为 `secret`，但原文位置已经是 `[REDACTED:secret]`。

理由：
- 用户不会想"搜索我上次的 API key"
- 这类数据本地驻留比传输风险更大（被木马、备份、误同步偷走）
- FP 损失低：把一个 UUID 当 secret 误判后，用户基本无感

---

## 4. Model & Runtime

### 4.1 模型选型

- **Model**: `openai/privacy-filter`
- **Variant**: `model_q4f16.onnx` (~810 MB)
- **Tokenizer**: HF tokenizer.json
- **类目**: `account_number` / `private_address` / `private_email` / `private_person` / `private_phone` / `private_url` / `private_date` / `secret`
- **输出**: BIOES tagging, 33 类 logits per token → 约束 Viterbi 解码成 spans

### 4.2 选型论证（基于 `pii-shootout` 评测）

| 维度 | 数据 |
|---|---|
| 总 F1 | 0.758（仅次于 gliner-large 的 0.775，但 large 公众人物 FP 8/8）|
| 中文 F1 | **0.763**（所有候选最高）|
| 公众人物 pass-through | **7/8 通过**（仅 Naomi Klein 误判，其他 7 个全过）|
| Benign FP | 17 条（最低）|
| q4f16 vs fp16 质量 | 待 eval，社区数据指 ±0.003 F1 噪声内 |

**Ship gate**（q4f16 必须通过才能上线，否则降级到 fp16）：

1. 公众人物 pass-through 不能比 fp16 退化（fp16 是 7/8）
2. 中文 F1 ≥ 0.74（vs fp16 的 0.763 允许 0.02 退化）
3. Benign FP 数 ≤ 22（vs fp16 的 17 允许少量退化）
4. Warm p50 (1KB) 不慢于 fp16 的 1.5×（即 ≤ 290ms）

### 4.3 Runtime

- **Crate**: `ort` v2.x（onnxruntime Rust binding）
- **EP order**: `[CoreMLExecutionProvider, CPUExecutionProvider]`
- **进程形态**: **内联**进主 Tauri 进程，不起新 sidecar（q4f16 RAM ~1.5GB 可接受）
- **Tokenizer**: `tokenizers` crate (HF 官方 Rust 实现)

### 4.4 已知 caveats

1. **CoreML EP 覆盖率低**：fp16 实测只盖 24/365 节点（6.5%），q4f16 估计更低。Apple Silicon 上不会有 10× 提速预期，主要是 CPU 跑
2. **transformers 还没合 `openai_privacy_filter` 架构**：原始 PyTorch 路径用不了；只能走 ONNX。这影响不到我们但要知道
3. **MLX 社区移植存在**（`OpenMed/privacy-filter-mlx-8bit`），但需要 Python 运行时——不进 Phase 1，留作 Phase 4 候选

---

## 5. Data Model

### 5.1 frames 表 schema 变更

**位置**: [`apps/desktop/src-tauri/src/db/schema.sql:33-97`](../src-tauri/src/db/schema.sql#L33-L97)

在 `ocr_text` (line 50) 之后新增一列：

```sql
ax_text_pii_spans TEXT,  -- JSON: [{start, end, label, score}], NULL = 未处理, '[]' = 已处理无 PII
```

**Migration 模型**: Corivo 使用 purge-and-apply，不是 ladder（见 [`migrations.rs`](../src-tauri/src/db/migrations.rs) 文档注释）。落地步骤：

1. 直接编辑 `schema.sql` 加入新列（在 `ocr_text` 之后）
2. 把 `TARGET_SCHEMA_VERSION` 从 1420 bump 到 **1500**
3. `schema.sql` 末尾 `INSERT INTO schema_version` 同步改成 1500
4. `LEGACY_TABLES` 已包含 `frames`（既有逻辑），不需要新增条目
5. `chat_threads` + `chat_messages` 在 `preserve_chat_tables` 列表里得以保留；`frames` 历史数据**会被丢弃**，pre-release 期可弃

**v1500 实际落地于 commit XXX**（git 提交后填入），见 `migrations.rs` Bump history 注释。

### 5.2 Spans JSON schema

```json
[
  {
    "start": 12,            // char offset (UTF-8 字节 offset，对中文要注意)
    "end": 18,
    "label": "private_person",
    "score": 0.998,
    "redacted_in_storage": false  // true 仅当 label == "secret"
  }
]
```

字段说明：

- `start` / `end`: **char offset**（不是 byte offset），方便前端 highlight 不踩中文字符切半
- `label`: 8 个 OpenAI 类目之一
- `score`: 0.0–1.0
- `redacted_in_storage`: 标记原文是否已被物理替换。仅 `secret` 为 true

---

## 6. Component Design

### 6.1 新增模块

```
apps/desktop/src-tauri/
├── Cargo.toml                          # 加 ort, tokenizers, blake3, lru
└── src/services/privacy_filter/
    ├── mod.rs                          # 公共 API
    ├── session.rs                      # ort::Session 加载 + lifecycle
    ├── tokenizer.rs                    # HF tokenizer 加载
    ├── decode.rs                       # BIOES + 约束 Viterbi
    ├── cache.rs                        # blake3 hash → spans LRU
    ├── chunker.rs                      # 段落级 diff（Phase 2）
    ├── redact.rs                       # spans + text → 替换后文本
    ├── download.rs                     # 模型 artifact 首启下载
    └── settings.rs                     # 类目开关持久化
```

### 6.2 公共 API（mod.rs）

```rust
pub struct PrivacyFilter {
    session: Arc<RwLock<Option<Session>>>,
    cache: Arc<Mutex<LruCache<Hash, Vec<Span>>>>,
    settings: Arc<RwLock<PrivacySettings>>,
}

pub struct Span {
    pub start: usize,      // char offset
    pub end: usize,
    pub label: PiiLabel,
    pub score: f32,
    pub redacted_in_storage: bool,
}

#[derive(Serialize, Deserialize)]
pub enum PiiLabel {
    AccountNumber, PrivateAddress, PrivateEmail, PrivatePerson,
    PrivatePhone, PrivateUrl, PrivateDate, Secret,
}

impl PrivacyFilter {
    /// Classify-time entrypoint，从 snapshot_consumer 调用
    /// 返回 (可能被改过的 text, spans)
    pub async fn classify_and_redact_secrets(&self, text: &str)
        -> Result<(String, Vec<Span>)>;

    /// Egress-time entrypoint，从 exec_agent 调用
    /// 输入原文 + spans，按当前用户类目开关返回 redacted 文本
    pub fn enforce(&self, text: &str, spans: &[Span]) -> String;
}
```

### 6.3 依赖增量（Cargo.toml）

[`apps/desktop/src-tauri/Cargo.toml`](../src-tauri/Cargo.toml) `[dependencies]` 新增：

```toml
ort = { version = "2", features = ["coreml", "download-binaries"] }
tokenizers = "0.20"
blake3 = "1.5"
lru = "0.12"
```

`reqwest`、`tokio`、`serde`、`serde_json` 已存在，复用。

---

## 7. Hook Points

### 7.1 Capture-time classify

**位置**: [`apps/desktop/src-tauri/src/services/snapshot_consumer/local.rs:73`](../src-tauri/src/services/snapshot_consumer/local.rs#L73)

现有：

```rust
let frame = self.frames.insert(new).await?;
```

改为：

```rust
let (ax_text, spans) = self.privacy_filter
    .classify_and_redact_secrets(&new.ax_text)
    .await?;
new.ax_text = ax_text;                     // 已 secret-redact
new.ax_text_pii_spans = Some(json!(spans));
let frame = self.frames.insert(new).await?;
```

注意：classify 调用必须**容错**——模型加载失败、超时、panic 都不能阻塞捕获：

```rust
let result = tokio::time::timeout(
    Duration::from_secs(5),
    self.privacy_filter.classify_and_redact_secrets(&new.ax_text),
).await;

let (ax_text, spans) = match result {
    Ok(Ok(r)) => r,
    _ => {
        // 降级：原文入库，spans = NULL，让 egress 时 lazy classify
        sentry::capture_message("privacy_filter classify timeout", Level::Warning);
        (new.ax_text.clone(), vec![])
    }
};
```

### 7.2 Egress enforce

**位置**: [`apps/desktop/src-tauri/src/commands/exec_agent.rs:523`](../src-tauri/src/commands/exec_agent.rs#L523)

现有：

```rust
let run_result = run_turn(
    &thread_id,
    content,
    corivo_input,
    ...
).await;
```

改为：在构造 `corivo_input` 时遍历每个 frame，按 `ax_text_pii_spans` 调用 `enforce`：

```rust
let corivo_input = build_corivo_input_with_redact(
    raw_frames,
    &privacy_filter,
    &user_categories_enabled,
).await?;
```

`build_corivo_input_with_redact` 内部：
- 对每个 frame，读 `ax_text` 和 `ax_text_pii_spans`
- 如果 `spans` 为 NULL（历史数据或 capture 时降级了）→ 现场 classify 一次（同步）
- 调 `enforce` 按 `user_categories_enabled` 筛 spans，做字符串替换
- 替换模板：`"[REDACTED:{label}]"`（中文版："[已隐去:{label_zh}]"）

### 7.3 用户消息（user input）也要过

`exec_agent.rs:523` 上游的 `content`（用户在 Quick Ask 输入框打的话）同样要走 enforce——但 user input 没有预存 spans，需要现场 classify。

加 `classify_inline(text)` 同步 API，对**短文本**（< 4KB）直接同步跑，超过时间预算（500ms）回退到不 redact 并埋点。

---

## 8. Performance Budget

| 指标 | 目标 | 告警阈值 |
|---|---|---|
| 单帧 classify 平均耗时（含 cache） | < 50ms | > 200ms |
| Cache hit rate（活跃 1 小时窗口） | > 85% | < 70% |
| 模型冷启动时间 | < 8s | > 15s |
| Peak RSS（模型已加载） | < 2GB | > 3GB |
| 模型加载失败率 | < 0.1% | > 1% |
| 后台 worker 队列长度 | < 50 | > 200 |
| Egress enforce 延迟（每 frame） | < 1ms | > 10ms |

埋点（Sentry custom metric）：

- `privacy_filter.classify.duration_ms` (histogram, p50/p95/p99)
- `privacy_filter.cache.hit_rate` (gauge, hourly)
- `privacy_filter.model.load_duration_ms` (histogram)
- `privacy_filter.fallback_count` (counter, by reason: timeout/oom/load_failed)

---

## 9. Caching & Dedup

### 9.1 Phase 1: 整体 hash 缓存

```
hash = blake3(ax_text)
cache: LruCache<[u8; 32], Vec<Span>>  // 容量 10000，约 5MB
```

预期 hit rate 80%+（AX 文本重复率高：用户停留页面、typing indicator 触发的 re-capture 都是重复文本）。

### 9.2 Phase 2: 段落级 diff（如有性能压力）

```
chunks = split_by_paragraph(ax_text)
for chunk in chunks:
    if cache.contains(blake3(chunk)):
        spans += cached_spans (with offset adjustment)
    else:
        spans += model.predict(chunk) (with offset adjustment)
        cache.put(blake3(chunk), new_spans)
```

仅在 Phase 1 实测 hit rate < 70% 时启用。

### 9.3 缓存淘汰

- LRU 容量 10000 条
- 进程退出不持久化（避免缓存毒化 + 模型升级后旧缓存失效问题）
- `PrivacyFilter::clear_cache()` 暴露给 Settings"清除隐私缓存"按钮

---

## 10. Memory Management

q4f16 RAM ~1.5GB，方案选择：

| 策略 | 适用 | 决策 |
|---|---|---|
| 常驻 | RAM 充足，请求频繁 | **默认** |
| Lazy load + idle unload | RAM 紧张 | Fallback |

默认**常驻**：用户启动 Corivo 即开始加载（后台，不阻塞 UI），加载完进入 ready 状态。

**Idle unload 触发条件**（Phase 2 可选）：

- 系统 memory pressure > critical（macOS `MEMORYSTATUS_PRESSURE_CRITICAL`）
- 连续 30 分钟无 classify/enforce 请求
- 用户在 Settings 手动关闭

卸载即 `drop(session)`，下次请求触发重新加载（用户看到 toast "正在加载隐私模型..."）。

---

## 11. Model Artifact Distribution

### 11.1 不打包进 .dmg

810MB 模型 + tokenizer + config 约 ~830MB，**不进 bundle**。理由：

- .dmg 体积大 → 下载摩擦
- 模型更新需要发新 app 版本
- 首启用户可能没立即需要

### 11.2 首启下载

启动后异步下载到：

```
~/Library/Application Support/com.corivo.app/models/privacy-filter-q4f16/
├── model_q4f16.onnx
├── model_q4f16.onnx_data
├── tokenizer.json
├── config.json
└── manifest.json    # 自己加的：版本号 + sha256 + 下载时间
```

调 `app.path().app_data_dir()?`（参考 [`apps/desktop/src-tauri/src/lib.rs:998`](../src-tauri/src/lib.rs#L998)）。

下载流程：

1. 从 HF mirror (建议自托管 CDN，避免 HF rate limit) 拉 manifest
2. 比对本地 manifest，决定是否需要下载
3. 流式下载 + 实时 sha256 校验
4. 支持断点续传（HTTP Range）
5. 失败重试 3 次，指数退避
6. UI 显示进度（Settings 页 + 启动时 toast）

### 11.3 首启 UX

- 第一次启动 Corivo：privacy filter 默认 **off**，UI 提示"启用隐私过滤需要下载 830MB 模型，是否现在下载？"
- 用户同意 → 后台下载 + 显示进度
- 下载完成 → 自动加载 + 启用过滤
- 用户拒绝 → privacy filter 保持 off，Settings 里可以随时手动启用

---

## 12. Settings & UX

### 12.1 Settings 页新增段落

```
隐私过滤 (Privacy Filter)
├── [Toggle] 启用隐私过滤                         [On]
├── [Status] 模型已就绪 (v1.0, 810MB)          [重新下载]
│
├── 检测类目（命中后在发送 AI 时隐去）
│   ├── [Toggle] 人名                              [On]
│   ├── [Toggle] 邮箱                              [On]
│   ├── [Toggle] 电话                              [On]
│   ├── [Toggle] 地址                              [On]
│   ├── [Toggle] 账号                              [On]
│   ├── [Toggle] URL                              [Off]   # 默认关，URL 误报常见
│   ├── [Toggle] 日期                              [Off]
│   └── [Toggle] 密钥/Token (本地立即移除)        [On]   # 不可关
│
├── [Button] 清除隐私缓存
└── [Button] 查看检测历史 (Phase 2)
```

### 12.2 Quick Ask 发送时预览（Phase 2）

发送前显示"即将发送给 AI 的内容预览"，PII 部分高亮：

```
原文：约一下 [John Smith]，邮箱是 [john@acme.com]
                ^^^^^^^^^^         ^^^^^^^^^^^^^^^
                人名                邮箱

[ ] 不要隐去这一条
[ ] 不要隐去这一条
                                            [取消] [继续发送]
```

每条 highlight 可以单独取消。

---

## 13. Migration Plan

### DB Migration

走 Corivo 既有的 purge-and-apply 路径（详见 §5.1）：

1. 编辑 [`schema.sql`](../src-tauri/src/db/schema.sql) 在 `ocr_text` 之后加 `ax_text_pii_spans TEXT`
2. [`migrations.rs`](../src-tauri/src/db/migrations.rs) 的 `TARGET_SCHEMA_VERSION` 从 1420 → 1500
3. `chat_threads` + `chat_messages` 自动保留；`frames` 历史数据丢失（pre-release 期可弃）

历史 frame 在 v1500 之前没有 spans —— 它们要么走 backfill (Phase 2 可选)，要么在被 exec_agent 读取时按需 lazy classify 并回写。

### Settings Migration

新增 PrivacySettings 表或扩展现有 settings KV：

```sql
-- 如已有 settings 表，直接插 key
INSERT OR IGNORE INTO settings (key, value) VALUES
  ('privacy_filter.enabled', 'false'),
  ('privacy_filter.categories', '{"person":true,"email":true,...}'),
  ('privacy_filter.model_version', NULL);
```

---

## 14. Rollout Phases

### Phase 0: q4f16 eval（前置，blocking）

跑 `pii-shootout/runners/openai_q4f16.py`，验证 [§4.2 ship gate](#42-选型论证基于-pii-shootout-评测)。**不通过则降级 fp16**，本 spec 需修订：
- 改 sidecar 形态（fp16 RAM 4.9GB 不能内联）
- 改下载大小（3GB 必须显著优化下载 UX）
- 改内存管理（必须 lazy + idle unload）

### Phase 1: 后端骨架 + Egress enforce（核心隐私收益在此达成）

- DB migration
- `services/privacy_filter/` 模块完整实现
- Capture-time hook（含 secret 立即 redact）
- Egress hook in `exec_agent.rs`
- Settings 类目开关后端 API
- 模型下载器
- **暂不接 UI**，所有功能通过命令行 / dev menu 触发

Exit criteria：手动测试 10 个真实用户场景（含中文 + 公众人物 + 代码片段），FP/FN 行为符合 eval 数据。

### Phase 2: UI

- Settings 页完整 UI
- Quick Ask 发送前预览 + per-span override
- 首启模型下载向导

### Phase 3: 性能优化（按 §8 监控数据触发）

- 段落级 diff 缓存（如 hit rate 不达标）
- Idle unload（如 RAM 投诉）
- 模型加载预热策略

### Phase 4: 顺手收口

- ~~关闭 Sentry `send_default_pii: true`~~ **✅ Done 2026-05-20**（[`apps/desktop/src-tauri/src/lib.rs:946`](../src-tauri/src/lib.rs#L946)，独立改动，不依赖 Phase 1-3）
- 评估 MLX 路径（transformers 合并 `openai_privacy_filter` 架构后）
- 评估自托管 CDN 替代 HF 直连

### Phase 5（可选）: Hardened Mode

Settings 加"Hardened Mode"开关：所有类目走 capture-time hard redact（A 方案）。给监管行业用户。

---

## 15. Open Questions

1. **q4f16 在 CPU+ort+CoreML 路径上的真实延迟** — Phase 0 数据回来才知道。如果显著慢于 fp16，可能需要回到 fp16 + sidecar 方案
2. **中文 char offset vs byte offset** — Rust 端字符串 slice 是 byte 索引，前端 highlight 用 char 索引。需要在 spans 序列化时统一为 char offset 还是带两套？建议存 char offset，Rust 端用 `chars().nth()` 转换
3. **模型 artifact CDN** — HF 直接下载有 rate limit + 国内访问慢。要不要早期就上自托管？决定：Phase 1 用 HF，监控失败率，> 5% 再上自托管
4. **历史 frame backfill** — 不做主动 backfill，但 Phase 2 加一个 Settings 按钮"扫描历史数据应用隐私过滤"，给重度用户用
5. **多语言** — 模型主英文。中文 F1 0.763 已经验过。日韩等其他语言未测；产品上线后看用户分布再决定
6. **Quick Ask 输入框的 inline classify 阻塞** — 用户打完字按回车后跑 ~200ms 模型，UX 上接受吗？需 dogfood 反馈

---

## 16. Out of Scope

- **OCR text 过滤** — `frames.ocr_text` 也含 PII 风险，但 OCR 文本质量本身差，模型表现可能更糟。Phase 4 再评估
- **Screenshot 像素级 PII 检测** — 需要视觉模型，完全不同的方向
- **跨设备同步的 PII 策略** — Corivo 当前没有 sync，等 sync 设计时再补
- **数据库内已有 PII 的清理工具** — 即"我之前没启用，现在启用了，能不能把历史数据也清干净"——Phase 2 backfill 按钮覆盖
- **审计日志** — "哪些 PII 在什么时候被发送到了哪个 LLM"——Phase 5 候选

---

## 17. Appendix: 类目映射

| OpenAI label | 中文显示名 | redact 模板 | 默认开关 |
|---|---|---|---|
| `private_person` | 人名 | `[人名]` | On |
| `private_email` | 邮箱 | `[邮箱]` | On |
| `private_phone` | 电话 | `[电话]` | On |
| `private_address` | 地址 | `[地址]` | On |
| `account_number` | 账号 | `[账号]` | On |
| `private_url` | URL | `[URL]` | Off（FP 多）|
| `private_date` | 日期 | `[日期]` | Off（FP 多）|
| `secret` | 密钥 | `[REDACTED:secret]` | **On 且不可关**（强制 capture-time redact）|

---

## 18. References

- [openai/privacy-filter HF repo](https://huggingface.co/openai/privacy-filter)
- [pii-shootout eval 报告](../../../pii-shootout/results/REPORT.md)（独立项目）
- [Existing snapshot_consumer flow](../src-tauri/src/services/snapshot_consumer/local.rs)
- [Existing exec_agent flow](../src-tauri/src/commands/exec_agent.rs)
- [Monorepo migration spec](./monorepo-migration-spec.md)
