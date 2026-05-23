# Prompt Debug Settings Design

## 1. Goal

为 Corivo 的配置页增加一个面向排障的 `Prompt 调试` 子页，让用户可以在 UI 中覆盖两条关键 prompt：

- 截图总结 prompt
- 搜索记忆后是否推送通知的判断 prompt

这个子页的目标不是把 prompt 系统产品化，而是给排查链路提供一个低成本、可回退、可观察的调试入口。

## 2. Why This Change

当前代码已经出现了“配置模型里有一部分 prompt，但真正运行时的关键 prompt 仍然是硬编码”的断层：

- [`src-tauri/src/domain/config.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/domain/config.rs) 里已有 `summary.prompt_template`
- [`src-tauri/src/services/mvp_pipeline.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/mvp_pipeline.rs) 里实际生效的 `SUMMARY_PROMPT` 和 `PUSH_JUDGMENT_PROMPT` 仍然是常量
- [`src/pages/settings/sections/api-keys-section.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/pages/settings/sections/api-keys-section.tsx) 里已有“Prompt 测试”，但只是临时输入框，不会影响运行中的 pipeline

结果是：

- 用户能测试某个 prompt，却无法让正式链路使用它
- 出问题时，很难判断“测试时的 prompt”和“运行时的 prompt”是否一致
- 排障只能靠改代码或重新构建，不适合快速定位问题

## 3. Non-Goals

- 不做通用 prompt 注册表
- 不做 prompt 版本历史、diff、草稿箱
- 不做富文本编辑器，普通 `Textarea` 即可
- 不做 prompt 模板变量设计器
- 不把这套能力暴露给所有普通设置项；它是调试工具，不是产品主路径

## 4. Chosen Product Shape

### 4.1 New Settings Subpage

在配置页新增一个子页：

- 标题：`Prompt 调试`
- 定位：`设置 > Prompt 调试`

不建议把它塞进现有 `API Keys` 或 `测试` 子页，原因是：

- 它与密钥管理无关
- 它比“发一条测试通知”复杂得多
- 后续如果再补更多诊断能力，这个子页有自然扩展空间

### 4.2 Two Editable Prompt Blocks

页面只展示两个输入区：

1. `截图总结 Prompt`
2. `记忆推送判断 Prompt`

每个输入区都提供三类动作：

- `保存`
- `恢复默认`
- `复制当前生效版本`

并在输入区下方显示：

- 当前来源：`默认值` / `用户覆盖`
- 说明这条 prompt 影响的运行链路

### 4.3 Explicitly Debug-Oriented Language

页面文案要明确写成“用于排查问题，不建议长期偏离默认值”，避免用户误以为这是常规个性化配置。

## 5. Config Model Changes

### 5.1 New Prompt-Debug Config Section

在 [`src-tauri/src/domain/config.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/domain/config.rs) 的 `Config` 中新增一组专门的调试配置，例如：

```rust
pub struct PromptDebugConfig {
    pub summary_override: Option<String>,
    pub push_judgment_override: Option<String>,
}
```

语义：

- `None`：使用内置默认 prompt
- `Some(text)`：使用用户覆盖值

这里不建议继续复用 `summary.prompt_template` 去承载 MVP pipeline 的截图总结 prompt，原因是现有字段历史上更像“LLM 测试入口的 prompt 文本”，而不是“pipeline 内部最终权威 prompt”。继续混用会让语义更乱。

### 5.2 Backward Compatibility

新字段必须走 `#[serde(default)]`，确保老配置文件仍能正常反序列化。

坏配置的回退策略：

- 字段缺失：回退到默认 prompt
- 覆盖值为空字符串或全空白：保存时归一化为 `None`

## 6. Runtime Resolution Rules

### 6.1 Single Source of Truth

运行时 prompt 的权威解析规则应统一为：

1. 先看 `prompt_debug` 对应 override 是否存在且非空
2. 有则使用 override
3. 否则使用代码内置默认 prompt

不要让前端自行拼逻辑，也不要让不同调用点各自处理空字符串。

### 6.2 Keep Built-In Defaults in Code

`SUMMARY_PROMPT` 和 `PUSH_JUDGMENT_PROMPT` 依然保留在 [`src-tauri/src/services/mvp_pipeline.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/mvp_pipeline.rs) 作为内置默认值，只是改成通过小的 helper 统一取值。

原因：

- 默认行为必须不依赖本地配置
- 排障时需要一键回退
- 坏配置不应该让整条链路失效

## 7. Backend Surface Changes

### 7.1 Config Command Path

现有 `get_config` / `set_config` 命令已经足够，不需要为 prompt 单独再造命令。

受影响文件：

- [`src-tauri/src/commands/config.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/commands/config.rs)
- [`src-tauri/src/services/config_service.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/config_service.rs)

只要类型扩展到位，前端沿用现有 `useConfig()` 更新即可。

### 7.2 Pipeline Reads Config Instead of Raw Constants

[`src-tauri/src/services/mvp_pipeline.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/mvp_pipeline.rs) 需要拿到 `ConfigService` 或者在构造时拿到一个可读配置的依赖，以便在两处读取生效 prompt：

- 图片总结时
- 推送判断时

这一步的原则是最小改动：

- 不重构整个 pipeline 架构
- 只增加一个清晰的 prompt 解析入口

## 8. Frontend Changes

### 8.1 Settings Navigation

在 [`src/pages/settings/settings-page.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/pages/settings/settings-page.tsx) 中新增一个 section：

- `prompt-debug`
- label 可用 `Prompt 调试`

### 8.2 New Section Component

新增组件建议：

- [`src/pages/settings/sections/prompt-debug-section.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/pages/settings/sections/prompt-debug-section.tsx)

职责：

- 读取当前配置
- 维护两个本地编辑态
- 支持 `保存` / `恢复默认`
- 明确显示“当前来源”和“影响链路”

### 8.3 Keep Existing API-Key Prompt Test, but Reframe It

[`src/pages/settings/sections/api-keys-section.tsx`](/Users/airbo/Developer/corivo/corivo-app/src/pages/settings/sections/api-keys-section.tsx) 里的“Prompt 测试”不应删除，但应与新子页分工明确：

- `Prompt 调试`：改运行时正式 prompt
- `API Keys > Prompt 测试`：用临时 prompt 和手工上传图片试验模型输出

这是两个不同能力：

- 一个改正式行为
- 一个做临时实验

## 9. Diagnostic UX Details

为了真正方便排查，建议这个子页额外补两个只读信息块：

### 9.1 Effective Prompt Preview

每条 prompt 下方显示：

- 当前真正生效的文本预览
- 当前来源：`默认` / `配置覆盖`

这样能立即回答“我现在到底在用哪版 prompt”。

### 9.2 Prompt Impact Description

每条 prompt 要配简短说明：

- 截图总结 prompt：影响截图批处理后的 summary 文本，以及后续记忆检索 query
- 推送判断 prompt：影响相关记忆检索后的通知 JSON 决策

这能减少误改。

## 10. Testing Strategy

### 10.1 Rust

需要补充：

- config 默认值与反序列化兼容测试
- 空字符串 override 归一化测试
- pipeline 在“无 override / 有 override”两种情况下使用正确 prompt 的测试

重点文件：

- [`src-tauri/tests/config_services.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/tests/config_services.rs)
- [`src-tauri/tests/mvp_pipeline.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/tests/mvp_pipeline.rs)

### 10.2 Frontend

需要补充：

- 新 section 可切换
- 输入框展示当前值
- 点击 `恢复默认` 后保存的是 `None` 语义而不是空串
- `set_config` 发出的 payload 包含新增字段

重点文件：

- [`src/lib/config-tauri.test.ts`](/Users/airbo/Developer/corivo/corivo-app/src/lib/config-tauri.test.ts)
- 新增 `prompt-debug-section` 组件测试

## 11. Risks and Tradeoffs

### 11.1 Risk: User Can Still Break Behavior

允许自定义 prompt 天然会带来行为漂移。这个风险不能消除，只能控制：

- 默认值保留在代码里
- 页面明确标“调试用途”
- 支持一键恢复默认

### 11.2 Risk: Existing `summary.prompt_template` Becomes More Confusing

如果保留旧字段但不重命名，配置层会同时存在：

- `summary.prompt_template`
- `prompt_debug.summary_override`

这会让语义有些重复。

当前建议是短期接受这点重复，但在文档和页面中明确：

- `summary.prompt_template` 主要服务于测试入口或旧配置兼容
- `prompt_debug.summary_override` 才是 MVP pipeline 的正式覆盖项

如果后续继续迭代 prompt 系统，再统一收敛字段。

## 12. Decision

这次改动应采用“调试专用 prompt 子页 + 两条关键 prompt 可覆盖 + 默认 prompt 代码内保底 + 空值回退默认”的方案。

这是当前代码结构下最小、最稳、最利于排障的收口方式。
