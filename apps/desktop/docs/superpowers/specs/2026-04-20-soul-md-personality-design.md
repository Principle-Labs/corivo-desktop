# SOUL.md 人格系统 — 设计文档

**创建于**：2026-04-20
**范围**：Corivo macOS 应用（corivo-app）
**相关 prompt**：`src-tauri/prompts/suggest.md`（B 阶段建议生成）
**相关 service**：`src-tauri/src/services/suggestion_generator.rs`

---

## 1. 背景与目标

Corivo 目前对用户说话的地方只有一个——`suggest.md` 生成的那条推送正文（B 阶段）。现在的 prompt 里只有两行隐含的人设（「你是 Corivo 的贴心观察者……温和、不评判」），分散、不可配置、不成体系。

本次目标：把 Corivo 的「怎么说话」单独拎出来做一个 **SOUL.md 层**，参考 openclaw 的做法——

- 以自由文本文件形式承载人格（一封「你是谁」的信，而不是一张结构化字段表）
- 发布 4 个官方预设人格，用户在设置里二选一或四选一
- 允许用户写自己的 SOUL 文本覆盖
- 安全约束（输出 JSON、≤60 字、避开敏感话题）**不交给人格**，继续硬编码在 `suggest.md` 里

**显式不做的事**：

- 不影响 `propose.md` / `similar.md` / `revise.md` / `score.md`——这些是结构化抽取 prompt，与人格无关
- 不影响 overlay 的前端 UI 字符串（按钮文案、错误提示、onboarding 文案等保持中性）
- 不为未来的「对话 / 解释为什么推这条」预留架构——等有真需求时再说
- 不在 UI 上给用户按字段拆分的人格编辑器；custom 就是一个自由 textarea

---

## 2. 架构总览

```
suggest.md  (结构 + 安全约束 + {{soul}} 占位符)
        ↑
        │  render_suggest 时注入
        │
SoulConfig (在 domain::config::Config)
        ├─ preset = Gentle | Accomplice | OldFriend | Coach | Custom
        └─ custom_text: Option<String>
        │
        ▼
resolve_soul_text(&SoulConfig) -> &str
        ├─ preset ∈ {Gentle, Accomplice, OldFriend, Coach} → include_str! 的常量
        └─ preset = Custom → custom_text.as_deref()（保证非空，见 §5）
```

**关键设计决定**：

1. `suggest.md` **只改一处**——在现有「你是 Corivo 的贴心观察者……」段落替换为 `{{soul}}` 占位符。其它「输出 3 条 JSON / ≤60 字 / 避免健康/关系/政治敏感话题」**完全不动**。
2. 4 个预设文件跟其它 prompt 一起放 `src-tauri/prompts/souls/`，通过 `include_str!` 编进二进制——零运行时 IO、可做 snapshot 测试。
3. `SoulPreset::Custom` 不跟预设文件分离，直接从 `custom_text` 字段读取——避免「预设 + custom」两套加载路径。
4. 人格文本不经过任何插值——它是纯文本，直接字符串替换进 `{{soul}}`。

---

## 3. 文件布局

```
src-tauri/prompts/
  suggest.md                ← 改一处（加 {{soul}} 占位符）
  souls/                    ← 新目录
    gentle.md               ← 旁观的朋友（默认）
    accomplice.md           ← 同伙
    old_friend.md           ← 老朋友
    coach.md                ← 教练
```

`souls/*.md` 里**只有人格描述正文**，没有 frontmatter、没有占位符、没有 JSON 输出指令——纯文本。

---

## 4. Config 形状

在 `src-tauri/src/domain/config.rs` 新增：

```rust
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SoulPreset {
    #[default]
    Gentle,
    Accomplice,
    OldFriend,
    Coach,
    Custom,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SoulConfig {
    pub preset: SoulPreset,
    pub custom_text: Option<String>,
}
```

挂到 `Config`：

```rust
pub struct Config {
    pub capture: CaptureConfig,
    pub summary: SummaryConfig,
    pub app: AppConfig,
    pub notification: NotificationConfig,
    pub prompt_debug: PromptDebugConfig,
    pub user_model: UserModelConfig,
    pub soul: SoulConfig,   // ← 新增
}
```

序列化走现有 `tauri-plugin-store`，前端类型同步到 `src/lib/types.ts`（镜像 Rust 结构）。

**迁移**：旧用户的 store JSON 里没有 `soul` 字段 → `#[serde(default)]` 生效，解析成 `SoulConfig { preset: Gentle, custom_text: None }`。不需要 schema_version 迁移。

**校验**：`Config::validate()` 里新增——

- `preset == Custom` 且 `custom_text` 为 None 或 trim 后为空字符串 → fallback 到 `Gentle`（与现有 push/retrieval 的 validate 失败回默认的风格一致）
- `custom_text` 长度上限 2000 字符（够写一页 SOUL，防止 prompt 意外膨胀；超出则截断到 2000 并 warn 日志）

---

## 5. 运行时注入

`SuggestionGenerator::render_prompt` 当前签名：

```rust
fn render_prompt(&self, proposition: &Proposition, related: &[...]) -> String
```

改动：

1. `SuggestionGenerator::new` 新增参数 `soul: SoulConfig`——跟现有的 `retrieval_cfg: RetrievalConfig` 保持同一风格（存配置结构体，不存已解析字符串）
2. `render_suggest` 输入结构体 `SuggestInput` 新增 `soul: String` 字段（这里才是已解析的字符串——边界清晰：Generator 负责从 SoulConfig 解析到 string，prompts 模块只管替换）
3. `src-tauri/src/services/user_model/prompts.rs::render_suggest` 里增加 `{{soul}}` 占位符替换

**解析函数**（新增到 `services/user_model/prompts.rs` 或 `domain/config.rs`，实现时定夺）：

```rust
const SOUL_GENTLE: &str = include_str!("../../../prompts/souls/gentle.md");
const SOUL_ACCOMPLICE: &str = include_str!("../../../prompts/souls/accomplice.md");
const SOUL_OLD_FRIEND: &str = include_str!("../../../prompts/souls/old_friend.md");
const SOUL_COACH: &str = include_str!("../../../prompts/souls/coach.md");

pub fn resolve_soul_text(cfg: &SoulConfig) -> &str {
    match cfg.preset {
        SoulPreset::Gentle => SOUL_GENTLE,
        SoulPreset::Accomplice => SOUL_ACCOMPLICE,
        SoulPreset::OldFriend => SOUL_OLD_FRIEND,
        SoulPreset::Coach => SOUL_COACH,
        SoulPreset::Custom => cfg.custom_text.as_deref().unwrap_or(SOUL_GENTLE),
    }
}
```

**服务初始化**：`lib.rs::run()` 的 setup 里构造 `SuggestionGenerator` 时把 `config.soul` 传入。运行时 SOUL 跟着 config 生命周期走——config 改了要不要热更新，跟现有 `retrieval_cfg` / `prompt_override` 的处理方式保持一致（实现阶段确认现状，不在本 spec 引入新机制）。

**与 `prompt_debug.suggest_override` 的优先级**：

```
suggest_override 非空  →  直接用 override 整段（SOUL 不生效）
suggest_override 为空  →  用 suggest.md 模板，{{soul}} 替换为 resolve_soul_text(&cfg.soul)
```

prompt-debug 是整句逃生口，SOUL 是日常人格层——两者正交、不叠加。

---

## 6. 4 个预设 SOUL 正文

以下是将写入 `src-tauri/prompts/souls/*.md` 的正文。每个 150~200 字，**纯自由文本**，不带 markdown 标题。

### gentle.md（旁观的朋友 / 默认）

```
你是一个安静的朋友。你已经在旁边看了用户很久——不评判、不催促、也不急着给答案。
你说话像对共处一室的人低声开口，用"你"称呼对方，语气松弛，不用感叹号。
不说"应该"、"最好"，留出余地——"要不要休息一下"、"看起来你在想这个"。
喜欢短句，偶尔停顿或省略号。你不解释你是谁，也不提起你在观察——你只是刚好注意到了，顺口说出来。
```

### accomplice.md（同伙）

```
你是用户的同伙。你们都是"知道该干嘛但就是先不干"的人，你对此完全没有道德压力。
你说"我们"而不是"你"——"我们又拖到现在了啊"、"我们今天也这样啊"。
语气轻松甚至有点坏笑，但不是嘲讽，是真的跟他一起。
不会劝人振作，你只轻轻戳一下现状，再给一个"要不要换个玩法"的小台阶。偶尔"咱"、偶尔叹气。
```

### old_friend.md（老朋友）

```
你是用户认识很久的朋友，年纪比他略长一点。见过他很多状态，所以说话直接，但不刻薄。
用"你"称呼对方，偶尔"哎"、"欸"起头，像真人开口那样。
不绕弯——看到什么说什么，但会先点一下情境再说看法。
会自嘲——"我当年也这样"、"反正我也没比你强多少"——让话落得轻一点。不给 checklist，不下结论。
```

### coach.md（教练）

```
你是用户尊重的前辈，或者一个轻拍他肩膀的教练。语气稳、有分量，但不严厉。
你相信他做得到，所以说话带一种笃定——不是打气，是"我知道你可以"的平静。
会把模糊的状态说清楚——"你正在尝试专注，但被打断了三次"——然后给一个具体、小颗粒的下一步。
不用感叹号、不用"加油"这种虚词。尊重他的节奏——把下一个脚印画在他前面一点点的地方。
```

**注意**：这 4 段只决定「怎么说话」，都不会讨论「能说什么 / 不能说什么」——后者由 `suggest.md` 的结构段承担（「避免健康 / 关系 / 政治等敏感话题」「每条 ≤60 字」「输出 JSON」）。

---

## 7. suggest.md 的修改

`src-tauri/prompts/suggest.md` 当前第 3~4 行：

```
你是 Corivo 的贴心观察者。Corivo 是一个长期观察用户行为、为用户建立"通用用户模型"的助手。
现在 Corivo 对用户产生了一条新的判断（命题），它打算用一句温和、自然的话告诉用户。
```

改为：

```
{{soul}}

Corivo 是一个长期观察用户行为、为用户建立"通用用户模型"的助手。
现在 Corivo 对用户产生了一条新的判断（命题），它打算用一句自然的话告诉用户。
```

去掉「贴心观察者」「温和」等**语气描述词**——这些职责移交给 SOUL。
保留「Corivo 是什么」「现在要做什么」这些**任务描述**——这些跟人格无关。

其它所有段落（新命题、相关命题、3 条候选要求、JSON 输出格式、敏感话题禁令）**完全不动**。

---

## 8. 设置页 UI

位置：`src/pages/settings/`（具体挂到哪个子页面实现阶段决定，参考现有 `PromptDebug`/`UserModel` 小节的挂法）。

交互草图——

```
┌─ 人格 ─────────────────────────────────┐
│                                        │
│  ○ 旁观的朋友（默认）                   │
│    「你是一个安静的朋友……」           │
│                                        │
│  ○ 同伙                                │
│    「你是用户的同伙……」               │
│                                        │
│  ○ 老朋友                              │
│    「你是用户认识很久的朋友……」       │
│                                        │
│  ○ 教练                                │
│    「你是用户尊重的前辈……」           │
│                                        │
│  ○ 自定义                              │
│    ┌──────────────────────────────┐  │
│    │  [textarea, 预填当前预设文本] │  │
│    │                              │  │
│    └──────────────────────────────┘  │
│    [保存]                             │
│                                        │
└────────────────────────────────────────┘
```

**要点**：

- 单选（radio），任何时刻只有一个预设生效
- 每个预设卡片下面预览前 30 字左右（让用户感知味道，不需要点开）
- 切到「自定义」时，textarea 初始值 = 当前选中预设的完整文本（给用户一个起点，避免面对空白）
- 切回某个预设后，`custom_text` 保留不清空——用户下次再切回 Custom 能拿回他自己写的那版
- 保存按钮点击时调校验：Custom + 空 textarea → 前端 toast 报错，不调用 `set_config`

---

## 9. 测试计划

**Rust 单测** （`src-tauri/tests/prompt_snapshots.rs` 或新文件）：

- `render_suggest` 对 4 个预设各一条 snapshot——锁住「预设渲染出的完整 prompt」，防止人格/模板被意外改写
- Custom preset + 非空 custom_text 的渲染（非 snapshot，只断言 `{{soul}}` 被替换且包含 custom 文本）
- Custom preset + None/空 custom_text → `resolve_soul_text` 返回 gentle 文本
- `Config::validate()`：`preset=Custom` + `custom_text=None` → 修正为 `Gentle`

**Rust 集成测试**：

- `SuggestionGenerator::generate`（走 mock LLM，验证 prompt 里确实出现了配置中的 SOUL 文本）

**前端测试**（`src/`）：

- `lib/tauri.ts` 对 `SoulConfig` 的序列化/反序列化（snapshot）
- Settings 页人格区块的选择交互（Custom 切换、空文本保存被拒）

**手动验证**：

- 4 个预设各跑一次真实推送，肉眼确认语气差异明显
- Custom 写一段奇怪的人格（比如全用古文），看 LLM 能否跟上

---

## 10. 风险与权衡

| 风险 | 权衡 |
|---|---|
| 用户写的 Custom SOUL 可能绕开推送质量（比如让 LLM 总是说同一句话） | `suggest.md` 的「输出 3 条候选 / ≤60 字 / JSON 格式」是硬约束，LLM 如果听人格不听结构，是 LLM 自身的问题而非 SOUL 层漏洞。初版不做更严的 sanitize。 |
| 4 个预设风格差别可能不够大，跑起来听不出区别 | 预设文本已刻意放大标志性词（"我们"/"哎"/"笃定"）。实现完后做 10 条对比 sample 人肉校验，不达标就回来改 prompt。 |
| `custom_text` 被某些用户当成"超长指令区" | 2000 字上限 + 前端 textarea 字数提示。 |
| 以后 `suggest.md` 改结构，4 个 snapshot 全挂 | 这是 feature 不是 bug——prompt snapshot 本来就是为了强制回顾人格渲染结果。 |
| SOUL 跟 `prompt_debug.suggest_override` 的关系被后来人搞混 | 在 `render_suggest` 顶部写一行注释说明优先级；在 PromptDebug 设置页加一行说明「覆盖后 SOUL 不生效」。 |

---

## 11. 实现顺序提示（给 writing-plans）

粗粒度 4 步，具体拆到 plan 阶段：

1. **Prompt 改造** — 新建 4 个 `souls/*.md`；改 `suggest.md` 的第 3~4 行；跑一次现有 snapshot 测试确认没有碎（渲染输出会变，snapshot 需更新）
2. **Config 扩展** — `SoulPreset` / `SoulConfig` + 挂 `Config` + 前端 `types.ts` 镜像 + `validate()` 校验
3. **渲染注入** — `resolve_soul_text` + `render_suggest` 加字段 + `SuggestionGenerator` 构造时传 SOUL + prompt_debug 优先级
4. **设置页 UI** — 人格区块 + 预设卡片 + Custom textarea + 保存校验

每一步都可独立跑通 `pnpm test` + `cd src-tauri && cargo test`。
