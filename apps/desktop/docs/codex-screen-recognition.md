# Codex 截图识别集成分析

本文对 Corivo 当前 Codex provider 的 HTTP Responses API 路线与 Dayflow 项目的 Codex CLI 路线做一次完整对比，并诊断你遇到的 `502 Bad Gateway` 报错根因。

> 源参考：[JerryZLiu/Dayflow](https://github.com/JerryZLiu/Dayflow)
> 关键文件：`Dayflow/Core/AI/ChatCLIProvider.swift`、`Dayflow/Core/AI/ChatCLIRunner.swift`、`Dayflow/Core/AI/LLMService.swift`

---

## 1. 报错根因

日志片段：

```
ERROR Gemini 总结失败: Llm("Codex HTTP 502 Bad Gateway: Upstream request failed")
base_url = http://54.178.71.193/
```

两件事：

1. **错误文案里的「Gemini 总结失败」是误导**。那是 `mvp_pipeline` 的统一 error 模板，不是实际 provider 名——你在 Codex 上也会触发同一条日志。
2. **真正的失败来自 `POST /responses` 被上游代理拒绝**。`54.178.71.193` 是一个自建的 OpenAI 兼容代理，常见情况：
   - 只转发了 `/v1/chat/completions`（社区里绝大多数转发站都只实现到这里）。
   - 没有实现 `/v1/responses`（OpenAI 2024-03 之后的新接口）。
   - 更罕见的情况是代理实现了 `/responses` 但对 `input_image` + base64 data URL 这种超过 MB 级的 body 直接截断。
   
   上游代理把 body 丢给真正的 OpenAI 时失败，自己回了一个 `502 Upstream request failed`。

快速排查办法（在终端执行）：

```bash
# 1. 看看这个代理到底接哪些路径
curl -i -X POST "http://54.178.71.193/v1/responses" \
  -H "Authorization: Bearer sk-xxx" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-5.4","input":"hi"}'

# 2. 对比 chat/completions
curl -i -X POST "http://54.178.71.193/v1/chat/completions" \
  -H "Authorization: Bearer sk-xxx" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}'
```

如果 (1) 返回 404 / 502 而 (2) 正常，就能实锤：**这个 host 不支持 Responses API**。

---

## 2. Corivo 当前实现（HTTP Responses API）

位置：`src-tauri/src/providers/llm/codex.rs`

**方法**

- `POST {base_url}/responses`
- `Authorization: Bearer {api_key}`
- 超时 60s，429 指数退避，最多 2 次重试。

**请求体结构**

```json
{
  "model": "gpt-5.4",
  "input": [
    {
      "role": "user",
      "content": [
        { "type": "input_text", "text": "请分析这些截图..." },
        { "type": "input_image", "image_url": "data:image/jpeg;base64,/9j/4AA..." },
        { "type": "input_image", "image_url": "data:image/jpeg;base64,/9j/4AA..." }
      ]
    }
  ],
  "max_output_tokens": 2048,
  "temperature": 0.2,
  "text": {
    "format": {
      "type": "json_schema",
      "schema": { ... },
      "strict": true
    }
  }
}
```

**调用频率**

`MvpPipeline::run_once` 每批截图调用一次（默认 `capture.interval_secs = 30s`，`batch_size = 5`，大致 2.5 分钟一个批次）。每次调用打包该批所有截图作为 base64 data URL，单请求 payload 通常 2–8 MB。

**图像预处理**：仅在 `capture` 时以 JPEG 75 质量存盘，pipeline 阶段**不做降采样**。这意味着原图分辨率（Retina 下可能 2560×1600）直接入 body。

---

## 3. Dayflow 的 Codex 截图识别路线（CLI 子进程）

Dayflow 不走 HTTP Responses API，而是**直接启一个 shell 子进程执行 `codex exec`**，用户需要先本地装好 [Codex CLI](https://github.com/openai/codex-cli)。

### 3.1 触发入口

文件 `ChatCLIProvider.swift`，`transcribeScreenshots(_:batchStartTime:batchId:)`。这是对应 Corivo「summarize 截图批次」的同义功能。

```swift
let model  = "gpt-5.4-mini"   // 专门用一个 mini 模型跑视觉总结
let effort = "low"            // reasoning_effort=low
```

**批次采样**：Dayflow 一个 batch 的截图数量可能几十上百张，它会**等距抽样 15 张**再送进去（`strideAmount = count / 15`），避免单次调用 token 爆掉。Corivo 的 `batch_size = 5` 则是全送。

### 3.2 图像预处理（关键）

`ChatCLIProvider.swift` 的 `resize(_ src:into:)`：

- 每次调用**在独立临时目录**里产出 720p 以内的 JPEG 副本。
- **最大高度 720px**，等比缩放，宽度随动。
- JPEG 质量 **0.85**。
- 原图已经 ≤720p 的话直接 copy（不重新编码）。
- 调用结束后整个 temp dir 删除。

Corivo 没做这一步；压榨一下 payload 是最低成本的优化点。

### 3.3 子进程命令

`ChatCLIRunner.swift::run(tool:prompt:workingDirectory:imagePaths:model:reasoningEffort:)`，针对 `.codex` 分支，命令行拼成这样：

```sh
codex exec \
  --skip-git-repo-check \
  -m gpt-5.4-mini \
  -c model_reasoning_effort=low \
  -c mcp_servers.<each>.enabled=false \
  -c rmcp_client=false \
  -c web_search=disabled \
  --image /tmp/xxx/001.jpg \
  --image /tmp/xxx/002.jpg \
  ... \
  -- '<prompt 文本，shell-escaped>'
```

整条再包一层 login shell：

```sh
/bin/zsh -l -i -c "cd <workdir> && exec codex exec ..."
```

- `--skip-git-repo-check`：绕开 Codex CLI 对"必须在 git 仓库里执行"的校验。
- `-c ... =false`：关闭 MCP、网页搜索、RMCP 客户端，避免 Codex 跑偏去调工具。
- `--image <path>`：Codex CLI 原生支持的多图参数，图片走**本地文件路径**而不是 base64 data URL，这是跟 Corivo 最本质的差异。
- `--`：分隔命令参数与 prompt。

子进程用 `Process` 启，stdout 走 `Pipe`，Claude 分支才需要 PTY；Codex 用普通管道即可。

### 3.4 Prompt（核心）

`buildScreenshotTranscriptionPrompt`：

```
Analyze these {N} screenshots from a {duration} screen recording
({start} to {end}). They are 1 min apart and in order.

Create an activity log detailed enough that someone could reconstruct what
the user did.

...
Capture from screenshots:
- Exact app/site names visible
- Exact file names, URLs, page titles
- Exact usernames, search queries, messages
- Exact numbers, stats, prices shown

Bad: "Checked email"
Good: "Gmail: Read email from boss@company.com 'RE: Budget approval' ..."

3-8 segments total.
Group by GOAL not app (debugging across IDE+Terminal+Browser = 1 segment).

Timestamps must start at {start} and end at {end}. No gaps.

Return JSON only:
{"segments":[{"start":"HH:MM:SS","end":"HH:MM:SS","description":"..."}]}
```

**要点**

- 要求 Codex 在 stdout 吐**纯 JSON**（没有 `text.format.json_schema` 强约束）。
- 明确告诉模型帧数、时长、相邻帧间隔，帮它在没有绝对时间戳的情况下打出正确的 `HH:MM:SS`。
- 给正反面例子抑制模型输出模糊描述（"Checked email" 这种）。

### 3.5 校验与重试

`transcribeScreenshots` 外层循环 `maxTranscribeAttempts = 3`：

1. 先尝试 4 种 JSON 解析策略（直接 decode → 数组 decode → 提取 `{...}` → 去 OSC 转义再提取），任何一个成功就继续。
2. 再跑 `validateSegments`：检查相邻 segment 无 gap/overlap、首段从 `00:00:00` 起、末段覆盖到结束。
3. 验证失败时把错误原因**注入下一轮 prompt**：
   ```
   <原 prompt>
   
   PREVIOUS ATTEMPT FAILED - FIX THE FOLLOWING:
   <validationError>
   
   Return JSON only.
   ```
4. 每次失败按 `2 × 2^(attempt-1)` 秒 sleep（2s、4s、8s）。

这是很值得借鉴的模式：**让模型自己看到上一轮为什么被拒**，而不是盲目重试。

### 3.6 调用频率 & 上下文控制

| 维度 | Dayflow | Corivo |
|------|---------|--------|
| 触发时机 | 每个 observation batch | 每个 `MvpPipeline` 循环 |
| batch 截图数 | 自适应，目标 15 张等距采样 | 固定 `batch_size = 5`（全送） |
| 批次间隔 | 依赖 batch 完成时间，非固定 | `capture.interval_secs × batch_size ≈ 2.5min` |
| 图像尺寸 | 720p JPEG Q=0.85，isolated tempdir | 原图 JPEG Q=75，不重采样 |
| 并发 | 每次调用独立 temp dir，本质可并发 | MvpPipeline 串行 |

---

## 4. 两种路线对比

| 维度 | HTTP Responses API（Corivo 现状） | CLI 子进程（Dayflow） |
|------|--------------------------------------|------------------------|
| 网络路径 | 应用 → 用户自建代理 → OpenAI | 应用 → 本机 `codex` → Codex 后端 |
| 对代理兼容性 | ❌ 必须支持 `/v1/responses` | ✅ 不经过用户自建代理 |
| 身份 | API key（Bearer） | Codex CLI 登录态（Plus/Pro 订阅或 API key 任选） |
| 部署门槛 | 零，拿到 key 即用 | 用户需先 `brew install codex` 或等价步骤 |
| 图像载荷 | base64 data URL，payload 常 2–8 MB | 本地文件路径，子进程读磁盘 |
| 强 JSON Schema | ✅ `text.format.json_schema` 可强约束 | ❌ 靠 prompt 要求 + 解析兜底 |
| 流式 | 可选 `stream: true` | 原生 `exec` 即流式 stdout |
| 超时/重试 | HTTP 层超时、429 重试 | 进程层超时、kill + 重跑 |
| 成本归属 | 计入 API key 的 OpenAI 账单 | 计入用户的 Codex 订阅/账单 |
| 用户能换模型/host | ✅ Host 和 model 都在 UI 里 | 部分——host 由 Codex CLI 决定 |

**一句话总结**：HTTP 路线"服务化、可嵌入任何代理"，CLI 路线"靠本地 CLI 绕开代理问题、对订阅制用户更友好"。

---

## 5. 对 Corivo 的建议

按改动量从小到大给三档：

### 方案 A：最小修复，只解 502

把错误文案修正 + 给出"这个 host 可能不支持 /responses"的明确提示。

- `src-tauri/src/services/mvp_pipeline.rs`：日志从 `Gemini 总结失败` 改成 `LLM 总结失败 (provider={})`。
- `src-tauri/src/providers/llm/codex.rs`：502/404 时在 `CorivoError::Llm` 里带上路径提示，比如 `"该 host 可能未实现 /responses，请改用官方地址或兼容 Responses API 的代理"`。
- Settings → 模型页面：Base URL 输入框下方加一行说明「Host 必须支持 POST /responses，不是 /chat/completions」。

**适用场景**：用户本来就该用官方 host，只是误填了旧代理。

### 方案 B：加一条 Chat Completions 兼容回退

保留 Responses API 作为主路径，但在首次遇到 404/`unknown endpoint` 时自动切到 `/v1/chat/completions`（大部分代理都实现了这条）。

- 需要新增 `codex_chat_compat.rs`，转换 request body（`input` → `messages`、`input_image` → `image_url`、去掉 `text.format`）。
- JSON schema 降级成 `response_format: { type: "json_object" }` 或纯 prompt 约束。
- 在 `LlmProvider` 实现里做 transparent fallback，记录一个 `base_url` 级别的"这个 host 只认 chat/completions"缓存，下次直接走兼容路径。

**适用场景**：用户想继续用自建代理站，同时你愿意吃下一点接口差异带来的功能损失（最主要的是没 strict json_schema）。

### 方案 C：照搬 Dayflow 的 CLI 路线

新增 `codex_cli.rs`，作为第三种 provider kind（或者给 `codex` 加一个 `mode: "http" | "cli"` 开关）：

- `std::process::Command::new("codex")`，按 Dayflow 的 arg list 构造。
- 启动前把 `ImageInput.data` 写到 `$TMPDIR/corivo-codex-{uuid}/` 下的 `.jpg`，并**实现 720p 缩放**（用 `image` crate 的 `imageops::resize`）。
- `wait_with_output()` 抓 stdout，按 Dayflow 的 4-策略解析 JSON。
- Settings 里检测 `which codex` 是否存在，未装就禁用这条路径并给出安装提示。

**适用场景**：你想完全规避自建代理的不确定性、走用户本机已有的 Codex 订阅。

---

## 6. 建议优先级

1. **立刻做**：方案 A 里的日志修正 + 错误提示（5 行改动），把"误导用户以为是 Gemini 挂了"这件事先解掉。
2. **短期**：给 Corivo 加 720p JPEG 预处理（跟 Dayflow 一样的预处理成本极低，payload 能压 5–10 倍，对 HTTP Responses 调用尤其友好）。
3. **中期评估**：如果用户频繁反馈"自建 host 不好使"——做方案 B 的 Chat Completions 兼容回退；如果用户本来就是 Codex CLI 重度用户——做方案 C。

两条路不冲突，`LlmProviderKind` 完全可以扩成 `Gemini | CodexHttp | CodexCli`。
