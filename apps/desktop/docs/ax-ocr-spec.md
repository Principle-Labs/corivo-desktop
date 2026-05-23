# AX + OCR 抓屏改造 · 需求文档

> 调研阶段 v0.1（决策表大部分已回填，剩余项标 TBD 留给用户决定）
> 这份是**调研用**，不是终稿 spec。决策表完整后再开实施。
>
> 已研究完成的项：Rust binding 路线（Q6）、AX 覆盖率（Q1）、Vision OCR 能力（Q4）、权限 UX（Q7）、AX → OCR 回落条件（Q5）。详见各问题下的"**结论**"段。
>
> 仍需用户决定的项：截图保留策略（Q8）、summarizer prompt 是否改（Q10）、预估实施时间、回归测试集。

---

## 目标

替换 [src-tauri/src/services/observation_ingest/transcriber.rs](../src-tauri/src/services/observation_ingest/transcriber.rs) 里"截图 → vision LLM → 文本"这一步，改为：

```
截图 ──▶ macOS AX (Accessibility API) ──▶ 文本   (主路径)
       └──▶ Apple Vision framework OCR ──▶ 文本  (AX 不可用时回落)
```

下游（summarizer / event_session / event_attributor）**暂不改**，保持兼容。

## 为什么

| 维度 | 现状（vision LLM） | 目标（AX + OCR） |
|------|---------|-----------|
| 隐私 | 原图发送到 Gemini / Codex | 原图不离开本机 |
| 成本 | 每张截图一次 LLM 调用 | 本地免费 |
| 速度 | 取决于网络 + LLM 延迟 | 预期更快 |
| 文本质量 | 压缩的语义概要 | 字面原文（待评估是否更好） |

---

## 范围

**IN（这一阶段做）**
- 抓 focused window 的文本内容
- AX 主路径，OCR 兜底
- 输出文本进现有 summarizer，保持 prompt 接口兼容
- macOS only（跟项目现状一致）

**OUT（这一阶段不做）**
- 多 monitor 全屏抓取
- 网页深层 DOM 抽取
- 视频 / 动画 / 游戏画面识别
- 下游 prompt 优化（先做到不回归，优化留下一阶段）

---

## 待研究的问题

### 技术可行性

1. **AX 覆盖率**：哪些类型的 app 通过 AX 拿到的文本是可用的？
   - 原生 macOS app（Mail, Notes, Safari, Finder）
   - Electron 类（VS Code, Slack, Discord, Figma, Notion）
   - JetBrains IDE / Xcode
   - Terminal / iTerm / Warp
   - 浏览器内的网页
   - 游戏 / canvas 类（预期完全不行）

   **产出**：每类一个 verdict（可用 / 部分可用 / 不可用）+ 一句备注

   **结论**：

   | 应用类别 | Verdict | 备注 |
   |---|---|---|
   | 原生（Mail / Notes / Safari / Finder / Messages） | 可用 | AX tree 完整；Safari 还能拿 DOM 文本 |
   | VS Code | 部分可用 | Monaco 是 canvas 渲染，AX 只暴露当前可见行；需开 `editor.accessibilitySupport: on` |
   | Slack / Discord / Notion | 部分可用 | Chromium a11y tree **lazy** ── 首次 AX 查询才生成；富文本 / 图片 caption 经常缺 |
   | Figma | 不可用 | 主画布 WebGL，AX 只能拿工具栏；画布内文本必须 OCR |
   | JetBrains IDE | 部分可用 | Swing 自绘，需开 "Screen reader support"；编辑器返回行号+片段，缩进易丢 |
   | Xcode | 可用 | 自家 a11y 实现完整 |
   | Terminal.app | 可用 | NSTextView 后端，buffer 完整可读 |
   | iTerm2 | 部分可用 | 自定义 grid 渲染，需开 "Enable accessibility support" |
   | Warp / Alacritty / Kitty | 不可用 | GPU/Metal 渲染，AX tree 几乎为空 |
   | Safari 网页 | 可用 | DOM → AX 桥接最完整 |
   | Chrome / Arc 网页 | 部分可用 | Chromium 默认禁 a11y；查询会触发 `--force-renderer-accessibility`，首次 100-500ms |
   | 游戏 / Photoshop 画布 / WebGL | 不可用 | 无 AX 节点，OCR 唯一路径 |

   **关键 caveat**：Chromium lazy a11y 是最大坑 ── AX 查询本身会"激活"它，目标 app 内存↑、首次延迟。Electron app 的 AX 反馈不稳定（AeroSpace / yabai 文档亦有警告）。

   **对回落策略的启示**：上面 4 个"不可用" + 多数 Electron 在第一次 AX 查询时都会 fall through 到 OCR；意味着 OCR 路径不是边角情况，必须当一等公民做好。

2. **AX 文本质量**：拿到的是干净可读的文本，还是要大量清洗？比如有没有大量重复、隐藏 element、ARIA label 噪音？

   **结论（部分）**：原生 NSTextView 类应用通常干净。Chromium / Electron app 的 a11y tree 含大量 ARIA label 噪音 + role 节点（"button"、"link"、"navigation"）需要过滤。Swing (JetBrains) 文本经常带行号前缀。**精确的清洗规则需要在真机 dogfood 时一类 app 一条规则积累**，本调研只能定原则。

3. **AX 性能**：复杂界面（VS Code 全屏 + 多面板）一次 AX 遍历的耗时？会不会阻塞主线程？

   **结论（不完整）**：调研中**未拿到权威 benchmark**，唯一确定数据点是 Chromium 首次 AX 查询触发 `--force-renderer-accessibility` 有 100-500ms 延迟。建议：
   - AX 调用必须 spawn off main thread（在 Tokio blocking task 里跑）
   - 加 timeout（建议 1500ms），超时直接走 OCR
   - 真机 benchmark 留作 implementation 阶段第一件事

4. **Apple Vision OCR 质量**：
   - 代码识别（等宽字体、缩进保留）
   - UI 文字（按钮、菜单、tab）
   - 中英混排
   - 速度（一张 5K 截图大概多少 ms）

   **结论**：
   - **延迟**：5K 截图，`.accurate` 在 Apple Silicon 上约 **600-1200ms**；`.fast` **150-300ms**。代码 / UI 文本场景必须 `.accurate`
   - **中英混排**：macOS 13+ 支持简繁中文。**必须显式设置** `recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US"]`，否则默认仅英文。`usesLanguageCorrection` 在代码场景**建议关**（否则会"纠正"标识符）
   - **代码识别**：括号 `{} [] <>` 大体 OK；`|` `l` `1` `I` 易混；**缩进会丢** ── Vision 输出文本块不保留前导空格，要恢复缩进必须按 bounding box X 坐标自己重建
   - **UI 文字**：反而比正文更稳（字号大、对比强、背景干净）；< 11pt 小字准确率明显下降
   - **输出结构**：返回 `[VNRecognizedTextObservation]`，每个带 `boundingBox`（归一化坐标）+ top-N 候选。**不是 plain string**。阅读顺序拼接：按 `boundingBox.midY` 降序（Vision 坐标系 Y 朝上）再按 `minX` 升序；多列布局（IDE 侧栏+编辑器）需要先做列分割，否则会"串行"

5. **AX → OCR 回落判断**：触发条件是什么？
   - AX 完全拿不到？
   - AX 拿到但是空？
   - AX 拿到但只是 placeholder（如 Electron 的"Web Content"）？
   - 还是固定某些 bundle id 直接走 OCR？

   **结论 ── 推荐三条 OR 触发**：
   1. AX 调用本身失败 / timeout（包含权限被拒）
   2. AX 返回的文本节点总字符数 < 阈值（建议 30 字符）── 覆盖"AX 拿到但空" + "只有 placeholder"
   3. bundle id 在硬编码 OCR-only 列表里（首版至少包含 `com.figma.Desktop`、`dev.warp.Warp-Stable`、`org.alacritty`、`net.kovidgoyal.kitty` 以及 PHOTOSHOP / 游戏类）

   **不做的方案**：检测 placeholder 字符串内容（如 "Web Content"）── 太脆，每个 Electron app 字串不一样。改用规则 2 的字符数阈值兜底。

### 工程实现

6. **Rust binding 路线**（三选一或混搭）：
   - 路线 A：`objc2` 生态直接 FFI 到 AppKit / AX / Vision framework
   - 路线 B：写一个 Swift helper binary，主进程通过 stdio / unix socket 通信
   - 路线 C：现成的 crate（如 `accessibility` / `accessibility-sys`）

   **决策维度**：稳定性、维护成本、构建复杂度、跟现有 Tauri 架构的契合度

   **结论 ── 混搭 A+C：`objc2-vision` for OCR + `accessibility-sys` for AX**：
   - **Vision 走 `objc2-vision` (0.3.2)**：API 完整覆盖 `VNRecognizeTextRequest` / `VNRecognizedTextObservation` / `VNImageRequestHandler`；本仓库已经在用 `objc2-app-kit` (1.71.0) 做 NSPanel + 菜单栏，自然延续；Vision 可以同步 `performRequests:` 跑，不需要 block2 异步胶水
   - **AX 走 `accessibility-sys` (0.2.0)**：`objc2` 生态**没有** AX binding 且短期不会有（madsmtm/objc2 issue #624 确认）。`accessibility-sys` 是薄 C-FFI 层，C API 全覆盖（`AXUIElementCreateApplication` / `AXUIElementCopyAttributeValue` 等）；递归 `kAXFocusedApplication → kAXFocusedWindow → kAXChildren` 大约 ~150 行 unsafe FFI，写一次就稳定
   - **跟 objc2 共存 OK**：accessibility-sys 基于 `core-foundation` / `core-graphics`，objc2 显式支持跟 core-foundation-rs 互操作（issue #719）
   - **不走 Swift sidecar**：调研里看到 Tauri 应用确实有这么干（Hiyoko PDF Vault），但要多带一条 swiftc 工具链 + 单独 codesign + JSON marshalling + 进程 spawn 延迟，全 Rust 路径都通的情况下不值得。留作 escape hatch

7. **权限 UX**：AX 权限是独立于 Screen Recording 的系统授权。
   - 第一次启动如何引导
   - 权限被拒时的降级（只用 OCR？提示用户？）
   - 在 onboarding 流程里加一步

   **结论**：
   - **AX 和 Screen Recording 是两套独立授权**（System Settings → Privacy & Security 下两个独立面板）；互不依赖
   - **`AXIsProcessTrustedWithOptions(kAXTrustedCheckOptionPrompt: true)` 行为**：第一次调用弹一次系统对话框；用户拒绝后**后续调用不会再弹**，只能引导用户手动去 System Settings。重置只能 `tccutil reset Accessibility <bundle-id>`。**app 加入列表后通常需要重启进程**才能拿到权限
   - **业界 onboarding 标准（Raycast / Rectangle / AeroSpace / Bartender）**：
     1. 专门一屏说明"为什么需要"+ 截图示意
     2. "Open System Settings" 按钮 → `x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility` deep link 直跳面板
     3. 后台 1-2s 轮询 `AXIsProcessTrusted()` 侦测授权 → 自动跳下一步
     4. 提示"You may need to restart Corivo"，提供一键重启按钮
   - **降级路径**：**Screen Recording 单独够跑 OCR**。`CGWindowListCreateImage` / `ScreenCaptureKit` 仅依赖 Screen Recording；Vision OCR 不依赖 AX。**结论：把 AX 当 enhancement，Screen Recording 才是 hard requirement**。AX 拒绝时静默走 OCR-only 模式，UI 上做不打扰的小提示

8. **截图存还是不存**？
   - **选项 A**：继续存（用于 /now 截图回看 / 调试）
   - **选项 B**：只存文本，不存图（更彻底的隐私故事）
   - **选项 C**：默认存，setting 里可关

   **结论：TBD ── 留给用户决定**。这是产品/隐私权衡，不是技术问题。**调研建议**：默认走选项 C（默认存，setting 可关），理由：dogfood 阶段截图回看价值大；正式发布前再讨论是否切默认值。

9. **输出格式**（给下游 summarizer）：
   - 纯文本拼接（最简单）
   - 结构化 JSON（区分 heading / button / text）
   - 带角色标签的 plaintext（如 `[BUTTON] Save`）

   **结论 ── 推荐"按 reading order 拼接的 plaintext + 简单角色标签"**：
   - AX 路径输出：保留 role 信息但只标注少数 high-signal 类型（`[TITLE]` / `[BUTTON]` / `[TAB]` / `[CODE]`），其余直接拼文本，不要每个节点都加标签噪音
   - OCR 路径输出：纯 plaintext，按 §4 推荐的 reading order 拼接（按 midY 降序 → minX 升序，多列先列分割）
   - 两条路径**输出格式一致**，summarizer 不感知来源
   - JSON 结构化方案弃用 ── 体积膨胀 ~3x，token 成本不划算；少数 role 标签已够 summarizer 区分

   **可能要改 prompt 的项见 Q10**。

### 兼容性

10. **summarizer prompt 会不会需要调？**
    - vision LLM transcribe 给的是压缩过的语义
    - AX/OCR 给的是字面原文，体积大很多
    - token budget、风格 example 可能要重写
    - **先用现有 prompt 跑一遍，看结果再决定**

    **结论：TBD ── 留给 dogfood 验证**。现有 `transcription.md` 输入是图片+提示，输出是结构化语义；改 AX/OCR 后，transcribe 这一步实际上**消失**了 ── transcriber 直接产文本，跳过 LLM 一次调用。下游 `summary.md` 输入由"多个 transcript"变成"多个 AX/OCR plaintext block"，体积可能 5-20x。
    **建议**：实施时先把 `summary.md` 的 token budget 上限调高（4k → 16k），跑一周看 summarizer 是否还稳；不稳就开第二轮专门优化。

11. **回归怎么测？**
    - 选 5-10 个真实工作日的观察数据，分别用旧流程和新流程跑
    - 对比 event 边界、归属、summary 质量
    - 量化的指标：归属准确率不能掉

    **结论 ── 推荐方法论**：
    - 录制阶段：实施前在生产路径加一个 `transcriber_dual_run` 影子模式，AX/OCR 路径产出独立写入 `observations_shadow` 表（同 schema），不进入下游
    - 比对维度：
      - 文本 token 体积比（diagnostic）
      - 同一截图的 fact 抽取重叠率（用 `event_boundary` LLM 跑两次比 facts）
      - 端到端：旧 vs 新各自跑完整 pipeline，看 event 标题/边界/归属差异
    - 验收门槛沿用现有 `## 后续切换的退出条件` 表
    - 测试集：5-10 个真实工作日 ── **TBD：需要用户挑代表性日期**

---

## 调研产出

| 决策点 | 结论 | 来源 |
|-------|------|------|
| AX binding 路线 | `accessibility-sys` (0.2.0) 直接走 C-FFI；薄包装写 ~150 行 unsafe 包住递归遍历 | Q6 |
| OCR binding 路线 | `objc2-vision` (0.3.2) 同步 `performRequests:`，跟现有 `objc2-app-kit` 一脉相承 | Q6 |
| 截图保留策略 | **C：默认存 + setting 可关**（dogfood 阶段截图回看价值大；正式发布前再讨论是否切默认） | Q8 |
| 输出格式 | 按 reading order 拼接的 plaintext + 少数 high-signal 角色标签（`[TITLE]` `[BUTTON]` `[TAB]` `[CODE]`）；AX 和 OCR 两条路径输出格式一致 | Q9 |
| AX → OCR fallback 触发条件 | OR 三条：① AX 调用失败 / timeout（含权限被拒）② AX 文本总字符数 < 30 ③ bundle id 在硬编码 OCR-only 列表（Figma / Warp / Alacritty / Kitty / Photoshop / 游戏类） | Q5 |
| summarizer prompt 是否需要改 | **不专项改 prompt** ── 实施时先把 `summary.md` token 上限 4k → 16k，留给 dogfood 验证；跑一周稳就保持现状，不稳再开第二轮优化 | Q10 |
| 预估实施时间 | **TBD ── 用户估** | ── |
| 预期回归风险 | **中** ── 主要风险：① OCR 体积膨胀冲击 token budget（缓解：summary.md 上限调高）② Electron / Chromium app 首次 AX 100-500ms 延迟（缓解：off-main-thread + 1500ms timeout 走 OCR）③ Vision OCR 缩进丢失影响代码语义（缓解：reading-order 拼接 + 角色标签弥补） | Q1+Q3+Q4 |
| AX 性能保护 | AX 调用必须 spawn 到 Tokio blocking task，加 1500ms timeout，超时走 OCR | Q3 |
| 权限 onboarding | Onboarding 加一屏：deep link `x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility` + 1-2s 轮询 `AXIsProcessTrusted()` + 重启提示。**Screen Recording 是 hard requirement，AX 是 enhancement** | Q7 |

---

## 后续切换的退出条件

不是调研标准，是真正切换发版时的标准。

| 指标 | 目标 |
|------|------|
| 归属准确率 | ≥ 现状 |
| 单批 observation 处理延迟 | ≤ 现状 × 1.5 |
| 烟雾测试通过的常用 app 类型 | ≥ 5（IDE / browser / terminal / IM / 设计工具） |
| AX 权限被拒时的体验 | 不崩溃，自动走 OCR，用户有感知 |

---

## 不在这份文档里

- 具体的 Rust 代码草稿（binding 路线已定，进 implementation 阶段再写）
- 完整 API 列表（`accessibility-sys` + `objc2-vision` 文档自带）
- 详细 prompt 改造方案（实施阶段第二轮调优）
- 多平台扩展（Windows / Linux 不在此考虑）

---

## Implementation 计划（5 步，每步独立可验证）

> v2 architecture spec 已落地（commit `2cf70e6` / schema 200），AX/OCR 改造直接在现有 [observation_ingest](../src-tauri/src/services/observation_ingest/) 模块上动。

### Step 1 · 引入依赖 + 改 transcriber 入口为 dispatcher
- `Cargo.toml` 加 `accessibility-sys = "0.2"` + `objc2-vision = "0.3"`（按需开 feature flag）
- [transcriber.rs](../src-tauri/src/services/observation_ingest/transcriber.rs) 改成"input 是 ScreenshotRef，但内部判路 → AX 或 OCR"，先所有路径 `unimplemented!()`，确保编译通过
- 加 module 骨架：`observation_ingest/ax_extractor.rs` + `ocr_extractor.rs` + `output_format.rs`（共享的 reading-order 拼接 + 角色标签）
- **验证**：`cargo build` 通过；`cargo test --lib` 不回归

### Step 2 · OCR extractor（先做这个 ── 路径短、无权限新议题）
- `ocr_extractor.rs` 实现 `extract(jpeg_bytes) -> Result<String>`：`VNRecognizeTextRequest` `.accurate` + `recognitionLanguages=["zh-Hans","zh-Hant","en-US"]` + `usesLanguageCorrection=false`
- 调用 `output_format::stitch_observations()` 把 `[VNRecognizedTextObservation]` 按 midY/minX 拼成 plaintext
- transcriber dispatcher 暂时强制全走 OCR 路径，验证端到端
- **验证**：手动起 app，`tracing` 看 OCR 文本质量；屏幕里有中英混排时输出对比

### Step 3 · AX extractor + dispatch 逻辑
- `ax_extractor.rs` 用 `accessibility-sys` 写 `extract(pid, bundle_id) -> Result<AxText>`：递归 `kAXFocusedApplication → kAXFocusedWindow → kAXChildren`，跑在 `tokio::task::spawn_blocking` 内 + 1500ms timeout
- transcriber dispatcher 实现 §5 三条 fallback：① AX 失败/timeout ② AX 文本 < 30 字 ③ bundle id 在硬编码 OCR-only 列表
- 加 `summary.md` token budget 4k → 16k
- **验证**：在 VS Code / Slack / Figma / Terminal 各跑一遍，看 dispatch 决策日志

### Step 4 · 权限 onboarding
- Settings 加 AX 权限状态卡片（`AXIsProcessTrusted()` 状态 + deep link + 1-2s 轮询 + 重启提示）
- Onboarding 流程加一屏（可选，不阻塞首次启动）
- 截图保留策略：在 Settings/Capture 加开关（默认 ON）
- **验证**：用新 macOS 账号或 `tccutil reset Accessibility com.corivo.dev` 模拟首次

### Step 5 · 回归测试 + 切换
- 跑两周 dogfood，看 §"后续切换的退出条件"四项指标
- summary.md 是否需要专项改：dogfood 一周后回答
- 决定是否把 Step 1 引入的 vision LLM transcribe 路径完全删除（vs 留作 escape hatch）

---

## 仍开放（不阻塞 implementation）

- **回归测试集**（Q11）── dogfood 阶段在线收集 5-10 天真实数据，不需要预先挑
- **预估实施时间** ── 按 5 步节奏走，每步 1-3 天，整体 1-2 周（含 dogfood）
