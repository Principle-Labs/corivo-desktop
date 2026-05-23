# Corivo Agent Sidecar · spec

> 状态：v1 设计稿。完成 review 后启动实施。
> 这是**契约 spec** —— 锁定 Rust 主进程 ↔ Agent Sidecar ↔ pi-coding-agent ↔ 模型 / MCP 之间的所有边界、IPC 协议、生命周期。

---

## 1. 目标

把当前 spawn `claude` CLI 的 agent 执行层换成 Corivo 自己掌控的 sidecar，沉淀到 `packages/agent`，单独维护：

- **演进权回到自己手里**：agent loop、工具定义、上下文裁剪、auth、provider 都可改
- **多 provider day1**:Anthropic 与 OpenAI 协议形状两条路径并行支持。**架构上号池在后端**,前端只展示"模型选择",不暴露 provider/base_url/key 这些细节(详见 §7)。BYOK(用户自带 key)作为高级选项保留
- **工具系统统一在 pi `AgentTool` 接口下**:corivo 自己的 `recall_screen_history` / `ask_permission` 变成原生 AgentTool;外部 MCP 服务器通过 [mcporter](https://github.com/openclaw/mcporter) 接入(详见 §6.3)
- **会话状态、compaction、context pruning** 自治（参照 [openclaw 的 `pi-hooks/context-pruning`](https://github.com/openclaw/openclaw/tree/main/src/agents/pi-hooks)）
- **Rust 主进程改动最小**:`exec_agent::runner` 形态保留(只换 spawn 的 binary 和事件 schema);`mcp_bridge` 的 agent 调用路径**删除**,但保留作为 corivo-mcp 服务外部 MCP 客户端的桥(详见 §9.2)

## 2. 非目标 (v1)

- **Embedded 模式**：openclaw 用 `createAgentSession()` 内嵌进 Node 主进程；我们 Tauri 主进程是 Rust，不存在内嵌选项，只走 sidecar/RPC
- **Web 端复用 sidecar**：web/api 暂不接入；agent sidecar 只服务 desktop
- **工具沙箱化**：openclaw 在 Docker 里跑工具；Corivo 直接在用户机器，v1 不做沙箱，需要的限制走 `ask_permission` 工具网关
- **`SystemClaude` 逃生口**：claude CLI 走了，这条路砍掉；之后如果用户提需求再加（比如"用我自己装的 pi-coding-agent"）
- **多账号 failover / cooldown**：openclaw 有 `auth-profiles.ts`，Corivo v1 单账号单 provider 配对，多账号留 v2
- **HTTP / WebSocket 模式**：sidecar 只暴露 stdio NDJSON
- **Sub-agent / plan mode**：pi 默认就跳过这些，我们也不加
- **重新设计 `corivo-mcp`**：仅做"原地迁出 desktop crate → packages/"的位移（companion task，见 §10）

## 3. 命名与位置

| 物 | 位置 | npm 名 / crate 名 | 产物 |
|---|---|---|---|
| Agent sidecar (主体) | `packages/agent/` | `@corivo/agent` (TS) | `corivo-agent-{arch}-apple-darwin` (Bun `--compile`) |
| Corivo MCP server (迁移自 `apps/desktop/src-tauri/src/bin/corivo-mcp.rs`) | `packages/mcp/` | `corivo-mcp` (Rust crate) | `corivo-mcp-{arch}-apple-darwin` |
| pi-coding-agent 依赖 | (npm) | `@mariozechner/pi-coding-agent` `@mariozechner/pi-agent-core` `@mariozechner/pi-ai` | (workspace 透传) |

二者都是 desktop 私有 sidecar，按 root [CLAUDE.md](../../../CLAUDE.md) 命名约定使用 `@corivo/*` 前缀。

**约定**：
- sidecar 二进制最终落到 `apps/desktop/src-tauri/binaries/`，由 `tauri.conf.json` 的 `bundle.externalBin` 拾取
- 三种 target triple 都要出：`aarch64-apple-darwin` / `x86_64-apple-darwin` / `universal-apple-darwin`（参考现有 [build.mjs](../../../packages/desktop-helpers/build.mjs)）

## 4. 进程模型

```
┌──────────────────────────────────────────────────────────────────┐
│ Rust 主进程 (Tauri)                                               │
│                                                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ services/exec_agent (重构后)                              │   │
│  │  ├─ runner.rs       spawn corivo-agent + stdin/stdout    │   │
│  │  ├─ protocol.rs     parse corivo-agent NDJSON event      │   │
│  │  ├─ rpc_server.rs   ⬅ 新：响应 sidecar 的 RPC 回调       │   │
│  │  │                  (替代 mcp_bridge 的 UDS 角色)         │   │
│  │  └─ commands.rs     Tauri command 入口                   │   │
│  └──────────────────────────────────────────────────────────┘   │
└────────────┬─────────────────────────────────────────────────────┘
             │ stdin/stdout (NDJSON)        stderr → tracing
             │ + UDS for AgentTool callbacks (recall / ask_permission)
             ▼
┌──────────────────────────────────────────────────────────────────┐
│ corivo-agent (sidecar, Bun-compiled TS)                          │
│                                                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ pi-coding-agent / pi-agent-core / pi-ai                  │   │
│  │  ├─ AgentSession (createAgentSession)                    │   │
│  │  ├─ SessionManager (jsonl persistence)                   │   │
│  │  ├─ Compaction & context-pruning hooks                   │   │
│  │  └─ Provider / AuthStorage / ModelRegistry               │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                   │
│  ┌──────────────── Corivo customization layer ─────────────┐    │
│  │ - Native AgentTool: recall_screen_history,              │    │
│  │     ask_permission     (callback → Rust via UDS)        │    │
│  │ - MCP integration: spawn / connect MCP servers via      │    │
│  │     MCP gateway, register their tools as AgentTool      │    │
│  │ - Custom system prompt builder                          │    │
│  │ - Compaction safeguard + context pruning extensions     │    │
│  │ - Stdio NDJSON event emitter (corivo wire schema, §5)   │    │
│  └──────────────────────────────────────────────────────────┘   │
└──────────────────────┬───────────────────────────────────────────┘
                       │
       ┌───────────────┴───────────────┐
       │                               │
       ▼                               ▼
  ┌──────────┐                ┌──────────────────┐
  │ 模型 API │                │ mcporter runtime │
  │ Anthropic│                │ spawns/manages   │
  │ OpenAI…  │                │ MCP servers      │
  └──────────┘                │ (user-configured)│
                              └──────────────────┘
```

**单实例约束**：每个用户 turn 一个 sidecar 子进程（与现 `claude --print` 模型一致）。多轮上下文通过 `--session-id` 等价机制由 sidecar 内部 `SessionManager` 持久化到 jsonl。

**生命周期**：sidecar 通过 stdin 拿到 turn 输入 → 内部跑 agent loop → stdout 流式发事件 → loop 结束发 `done` → 进程退出。Rust 用 `tokio::process` 等 child wait。

## 5. IPC 协议（Rust ↔ Sidecar，stdio NDJSON）

> 决策来源：用户选项 (b) "新 schema 贴近 pi 原生事件"。一次改干净，不在 sidecar 里翻译成 claude stream-json。

### 5.1 Rust → Sidecar（stdin,首条写完保留 stdin）

**首条 JSON 写完后 stdin 保留打开**(不 close) —— 让 Rust 在 turn 进行中可以发控制消息(`cancel` / `steer` / `compact`,详见 §5.3)。一个 sidecar 进程 = 一个 turn。Sidecar 跑完一个 turn 后主动退出,Rust 通过 child wait 收尾(stdin 这时随子进程退出自然关闭)。

```jsonc
{
  "session_id": "01J...",            // ulid，会被映射成 sidecar 内部 session
  "user_message": {
    "role": "user",
    "content": "用户原始问题",
    "images": [                       // optional，base64 或 file path
      { "data": "...", "mime_type": "image/png" }
    ]
  },
  "focus_context": {                  // optional，Quick Ask 专用
    "frame_id": "01J...",
    "summary": "...",
    "primary_text": "...",
    "selection": "..."
  },
  "system_prompt_extra": "...",       // optional，append to base system prompt
  "model": {
    "id": "corivo:claude-sonnet-4-6",  // 后端定义的 model id（不是 Anthropic 原生 id，由号池服务下发）
    "api_shape": "anthropic",          // "anthropic" | "openai"，决定 pi-ai 用哪条 provider 路径
    "thinking_level": "medium"         // "off" | "minimal" | "low" | "medium" | "high" | "xhigh"
  },
  "compaction_model": {                // 后端按用户当前 model 配套指定的便宜 model
    "id": "corivo:claude-haiku-4-5",
    "api_shape": "anthropic"
  },
  "auth": {
    "mode": "corivo_proxy",            // "corivo_proxy" | "byok"
    "base_url": "https://api.corivo.app/v1",  // corivo_proxy 模式：号池后端入口；byok 模式：留空或用户自填
    "token": "..."                     // corivo_proxy 模式：登录会话 token；byok 模式：用户自带 key
  },
  "tools": {
    "native": ["recall_screen_history", "ask_permission"],
    "mcp_servers": [                  // optional
      { "name": "user-fs", "command": "...", "args": [...], "env": {...} }
    ]
  },
  "rpc_socket": "/tmp/corivo-agent-rpc-{pid}-{nanos}.sock"  // for native AgentTool callbacks
}
```

### 5.2 Sidecar → Rust（stdout，NDJSON 流）

事件 schema 紧贴 pi-agent-core 原生事件（参 [pi-agent-core README "Event Types"](https://github.com/badlogic/pi-mono/tree/main/packages/agent#event-types)），加一层 corivo envelope：

```jsonc
// envelope 通用结构
{
  "v": 1,                    // schema version
  "ts": 1700000000000,       // unix ms
  "type": "...",             // event type
  "data": { ... }            // type-specific payload
}
```

事件类型：

| `type` | data 关键字段 | 触发时机 |
|---|---|---|
| `agent_start` | `session_id`, `model` | sidecar 准备好 agent，turn 即将开始 |
| `turn_start` | `turn_index` | 一次 LLM 调用 + 工具执行批次开始 |
| `text_delta` | `delta` | 文本流增量（assistant text） |
| `thinking_delta` | `delta` | 推理流增量 |
| `tool_call_start` | `tool_call_id`, `name`, `arguments` (partial) | 工具调用决议中 |
| `tool_call_end` | `tool_call_id`, `name`, `arguments` (final) | 工具调用参数已定 |
| `tool_execution_start` | `tool_call_id`, `name`, `args` | 工具开始执行 |
| `tool_execution_update` | `tool_call_id`, `partial_result` | 工具流式进度（如 Bash） |
| `tool_execution_end` | `tool_call_id`, `result`, `is_error` | 工具完成 |
| `cited_frames` | `frame_ids: string[]` | corivo 专用：`recall_screen_history` 命中的 frame，对应现有 `chat_messages.cited_frame_ids` |
| `compaction_start` | `reason: "auto" \| "manual"` | 触发上下文压缩 |
| `compaction_end` | `removed_tokens`, `kept_tokens` | 压缩完成 |
| `turn_end` | `assistant_message`, `tool_results[]`, `usage` | 一个 turn 结束 |
| `agent_end` | `finish_reason`, `usage_total` | 整个会话单元结束 |
| `error` | `code`, `message`, `recoverable` | 任意阶段出错 |
| `api_retry` | `attempt`, `max_attempts`, `delay_ms`, `status` | 上游 API 重试中（来自 pi-ai） |

**finish_reason** 在现有 [`recall::stream::FinishReason`](../../src-tauri/src/services/recall/stream.rs)(`EndTurn` / `ToolUse` / `Refusal` / `Error`)基础上**新增 `Cancelled`** —— 用户主动 `cancel`(§5.3)时 emit 这个值,与 `Error` 区分,前端 UI 不显示报错样式而是中性"已取消"。这条 enum 扩展放进 Phase B(见 §10)。

### 5.3 Rust → Sidecar（控制信道,stdin 后续写入）

§5.1 已说明 stdin 在首条 JSON 后保持打开。所有控制消息一行一条 JSON:

```jsonc
// 取消当前 turn（用户在 UI 点 stop / 发送了新消息）
{ "type": "cancel" }

// 用户在工具执行间隙插话打断（v2，对应 pi `agent.steer()`）
{ "type": "steer", "content": "改成..." }

// 强制 compact（v2）
{ "type": "compact" }
```

收到 `cancel` 后, sidecar 调 `agent.abort()`, 等当前 LLM stream 中断 + 所有正在跑的工具收到 `AbortSignal` → emit `agent_end { finish_reason: "Cancelled" }` → 退出。Rust 侧 `runner.rs` 在写入 `cancel` 后等 child wait,不强 kill(除非 5s 超时未退则 SIGTERM,再 5s 未退则 SIGKILL)。

### 5.4 Sidecar → Rust（RPC，UDS）

为了让 corivo 原生 AgentTool（`recall_screen_history` / `ask_permission`）能回调 Rust 拿数据，sidecar 启动时连接 Rust 在 §5.1 `rpc_socket` 上 listen 的 UDS。这条 socket 替代当前 [`mcp_bridge`](../../src-tauri/src/services/exec_agent/mcp_bridge.rs) 的角色，但**协议层从 MCP 简化为 Corivo 自己的 JSON-RPC**（少一层 MCP 绕道）：

```jsonc
// request: sidecar → Rust
{
  "id": "rpc-{ulid}",
  "method": "recall_screen_history",
  "params": { "query": "...", "limit": 10, "...": "..." }
}

// response: Rust → sidecar
{
  "id": "rpc-{ulid}",
  "result": { "frames": [...] }   // 或 "error": { "code": ..., "message": ... }
}
```

方法白名单:
- `recall_screen_history` —— 参数 schema 沿用现有 `TOOL_RECALL_SCREEN_HISTORY` 形状
- `ask_permission` —— 同上,沿用现有 `TOOL_ASK_PERMISSION` 形状

> **注**:这两个 schema 常量目前定义在 [`apps/desktop/src-tauri/src/services/exec_agent/mcp_proto.rs`](../../src-tauri/src/services/exec_agent/mcp_proto.rs);Phase D(§10)把 `corivo-mcp` 搬到 `packages/mcp/` 后,常量定义会跟着搬到 `packages/mcp/src/proto.rs`。Sidecar 这边的参数 schema 自己单独维护一份(TS 侧 typebox),不直接依赖 Rust 常量,以保两侧独立演进。

未来如果更多 corivo 原生工具,再扩这张白名单。

### 5.5 与现有 [`stream::StreamEmitter`](../../src-tauri/src/services/recall/stream.rs) 的对接

Rust 侧 `protocol.rs` 解析 corivo wire schema 后，仍然 emit 到现有 `StreamEmitter` —— 前端 `useChatStream` 不需要改。映射表：

| corivo 事件 | StreamEmitter 调用 |
|---|---|
| `text_delta` | `text_delta(delta)` |
| `tool_call_end` | `tool_call(name, args)` |
| `tool_execution_end` | `tool_result(name, result, is_error)` |
| `cited_frames` | `cited_frames(ids)` |
| `turn_end` | (no-op, 只更新 usage 累加) |
| `agent_end` | `finish(reason)` |
| `error` | `error(message)` |
| `compaction_*` | 新增 `compaction_event(...)` 方法（前端 v1 可选忽略，v2 做 UI） |
| `api_retry` | 复用现有 `api_retry` 通道 |

## 6. Tool 模型

### 6.1 两条来源，统一接口

```
                    ┌─────────────────────────────┐
sidecar 启动时构建：│ AgentTool[]                 │ ← pi 唯一接受的形状
                    └─────────────────────────────┘
                          ▲                ▲
                          │                │
              ┌───────────┘                └───────────┐
              │                                        │
   ┌────────────────────┐                  ┌──────────────────────┐
   │ Native AgentTool   │                  │ MCP-derived AgentTool│
   │ (corivo 自家工具)  │                  │ (动态来自 mcporter)  │
   └────────────────────┘                  └──────────────────────┘
   - recall_screen_history                  - 启动时探测每个 MCP server
   - ask_permission                         - 把每个 MCP tool 映射成
   .execute = JSON-RPC over UDS               一个 AgentTool
     → Rust                                 .execute = server.call(name, args)
```

### 6.2 Native AgentTool 定义

放在 `packages/agent/src/native-tools/`，每个工具一个文件。骨架：

```ts
// packages/agent/src/native-tools/recall-screen-history.ts
import { Type } from "typebox";
import type { AgentTool } from "@mariozechner/pi-agent-core";
import { rustRpc } from "../rpc.js";

export const recallScreenHistoryTool: AgentTool = {
  name: "recall_screen_history",
  label: "查询屏幕历史",
  description: "在用户的屏幕历史里检索...",
  parameters: Type.Object({
    query: Type.String(),
    limit: Type.Optional(Type.Number({ default: 10 })),
  }),
  execute: async (toolCallId, params, signal, onUpdate) => {
    const result = await rustRpc("recall_screen_history", params, signal);
    return {
      content: [{ type: "text", text: result.summary }],
      details: { frames: result.frames },
    };
  },
};
```

### 6.3 MCP via [mcporter](https://github.com/openclaw/mcporter)

mcporter 是 TS 侧的 MCP runtime,自带 stdio/HTTP/SSE transport、OAuth 缓存、连接池。Sidecar 启动时构造一个 mcporter runtime,把发现到的所有 MCP tool 包成 pi `AgentTool`：

```ts
// packages/agent/src/mcp/runtime.ts
import { createRuntime } from "mcporter";
import type { AgentTool } from "@mariozechner/pi-agent-core";

export async function buildMcpAgentTools(
  serverSpecs: McpServerSpec[],
): Promise<{ tools: AgentTool[]; shutdown: () => Promise<void> }> {
  // mcporter 会发现 ~/.mcporter/ 配置, 我们额外注入 sidecar 输入里的 server specs
  const runtime = await createRuntime({
    inlineServers: serverSpecs,    // 来自 §5.1 input.tools.mcp_servers
    discoverHostConfigs: false,    // sidecar 模式不读用户 host 配置, 避免污染
  });

  const tools: AgentTool[] = [];
  for (const server of runtime.servers()) {
    for (const desc of await server.listTools()) {
      tools.push({
        name: `${server.name}__${desc.name}`,    // 防重名
        label: desc.title ?? desc.name,
        description: desc.description,
        parameters: desc.inputSchema,
        execute: async (toolCallId, params, signal) => {
          const result = await server.call(desc.name, params, { signal });
          return {
            content: result.content,
            details: { mcp_server: server.name, raw: result },
          };
        },
      });
    }
  }
  return { tools, shutdown: () => runtime.dispose() };
}
```

**`recall_screen_history` / `ask_permission` 不走这条** —— 它们走 §6.2 的 native 路径,直接 UDS 回 Rust,比 MCP 一层多余的协议封装更快、更易调试。

**为什么 sidecar 模式关掉 `discoverHostConfigs`**:
mcporter 默认会读取 Cursor / Claude Desktop / Codex 的 MCP 配置。Corivo 不该把用户在别处配的 MCP 服务器也拽进来 —— 只允许 corivo 自己 settings 里显式声明的。后续如果产品要"导入用户已有 MCP 配置"作为 onboarding 提速, 那是个 UI 流程, 在 Rust 侧选好后通过 §5.1 显式传入。

### 6.4 pi 内建工具的取舍

pi-coding-agent 自带 Read/Bash/Edit/Write/Grep/Glob。openclaw 的做法是替换 bash + 包装 read/edit/write 加沙箱。

Corivo v1 的取舍（**保守起步**）：

| pi 内建工具 | v1 状态 | 备注 |
|---|---|---|
| `read` | ✅ 保留 | 用户对自己 Mac 上文件读取无安全顾虑 |
| `bash` | ✅ 保留 | M3 模式当前已 allowlist `Bash`，先维持；v2 考虑通过 `ask_permission` 拦截高危命令 |
| `edit` / `write` | ✅ 保留 | 同 read |
| `grep` / `glob` | ✅ 保留 | 文件搜索 |

破坏性命令仍走现有 `ask_permission` 流程（前端弹窗确认），但拦截点从"claude 调 mcp__corivo__ask_permission"改为"sidecar 在 `beforeToolCall` hook 里识别敏感命令并 block + 调 `ask_permission` 工具"。

## 7. Auth & Provider

### 7.1 决策:号池在后端,前端只见模型

**架构原则**(产品决策):
- **后端维护账号池** —— OpenAI key、Anthropic key、其他 provider key 都在 Corivo 后端集中管理,可轮换、可计费、可限流
- **前端只看到"模型选择"** —— 用户登录 Corivo 后,后端返回一个可用模型列表;用户在 Settings 里挑一个,不需要单独配 provider / API key / base_url
- **Sidecar 通过单一会话 token + 模型 id 调用** —— 不直连 Anthropic/OpenAI,所有请求走 Corivo 后端的 API gateway,后端按 model id 路由到对应 provider 并注入真实凭据
- **Compaction 用的便宜 model 也由后端配套下发** —— sidecar 不需要决策"拿哪个 model 做压缩"

**不做**(v1):
- 用户自带 key (BYOK):**仍保留**,但作为 power user 逃生口,不是默认路径。Settings 里折叠在"高级"区。
- 单独的 OpenAI-compatible base_url 字段:不暴露给用户。如果用户想接自己的本地 Ollama/vLLM,可以走 BYOK 模式。

### 7.2 模型列表协议(Corivo 后端 ↔ Desktop)

后端新增一个端点,登录后被 desktop 拉一次并缓存:

```jsonc
// GET /v1/models
// 返回当前用户可用的所有模型
[
  {
    "id": "corivo:claude-sonnet-4-6",          // Corivo 内部 id, sidecar 用这个
    "display_name": "Claude Sonnet 4.6",       // 前端显示
    "api_shape": "anthropic",                  // sidecar 用 pi-ai 哪条 provider 路径
    "context_window": 200000,
    "supports_thinking": true,
    "supports_vision": true,
    "tier": "main",                            // "main" | "compaction"
    "compaction_partner_id": "corivo:claude-haiku-4-5"  // 配套压缩用的便宜 model
  },
  {
    "id": "corivo:gpt-4o",
    "display_name": "GPT-4o",
    "api_shape": "openai",
    "context_window": 128000,
    "supports_thinking": false,
    "supports_vision": true,
    "tier": "main",
    "compaction_partner_id": "corivo:gpt-4o-mini"
  }
  // ... haiku/4o-mini 等 tier=compaction 的也会出现, 但前端选模型 UI 只展示 tier=main
]
```

**`api_shape`** 是这个设计的关键 —— sidecar 不知道也不在乎是 Anthropic 真实账号还是 OpenAI 真实账号,只知道"这个 model 用 Anthropic 协议形状还是 OpenAI 协议形状"。后端的 gateway 同时监听两个 shape 的端点(`/v1/anthropic/messages` 和 `/v1/openai/chat/completions`),sidecar 拼请求时按 `api_shape` 决定走哪个。

### 7.3 Sidecar 内部:用 pi-ai 的 OpenAI-compatible & Anthropic-compatible 路径

```ts
// packages/agent/src/auth.ts —— 接口示意, impl 时按 pi-ai 真实 API 对齐
// pi-ai 的 getModel(provider, name) 主要负责定位 model 元数据;
// baseURL/apiKey 实际是在 stream/complete 调用或 ProviderConfig 上配置。
// 见 https://github.com/badlogic/pi-mono/tree/main/packages/ai
import { getModel } from "@mariozechner/pi-ai";

export function buildModelAndStreamConfig(input: SidecarInput) {
  const { id, api_shape } = input.model;
  const { base_url, token } = input.auth;
  const provider = api_shape === "anthropic" ? "anthropic" : "openai";
  const model = getModel(provider, id);
  const streamConfig = { baseURL: base_url, apiKey: token };  // 字段名以 pi-ai 实际为准
  return { model, streamConfig };
}
```

**两种 auth 模式**:

```rust
// services/exec_agent/runner.rs (重构后)
pub enum AuthChoice {
    /// 默认路径:Corivo 登录 → 后端号池 → API gateway
    CorivoProxy {
        gateway_url: String,        // 例 "https://api.corivo.app/v1"
        session_token: String,      // 登录后从 corivo_session 拿
        model: ResolvedModel,       // 用户在 Settings 选的, 来自 §7.2 模型列表
    },
    /// 高级:用户自带 key (折叠在 Settings 高级区)
    Byok {
        api_shape: ApiShape,        // Anthropic | OpenAI
        base_url: Option<String>,   // OpenAI 模式下可选, 默认 https://api.openai.com/v1
        key: String,
        model: String,              // 直接用 provider 原生 id, 不映射 corivo:xxx
    },
}

pub struct ResolvedModel {
    pub id: String,                 // "corivo:claude-sonnet-4-6"
    pub api_shape: ApiShape,
    pub compaction_partner_id: Option<String>,
}
```

砍掉 `SystemClaude` —— 没有 claude CLI 之后这条逃生口失去依附。

### 7.4 Config schema 变化

[`Config.exec_agent`](../../src-tauri/src/domain/config.rs) 重构:

```rust
pub struct ExecAgentConfig {
    pub auth_mode: ExecAgentAuthMode,        // CorivoProxy | Byok

    // CorivoProxy 模式:用户在 Settings 选的 model id, 引用 §7.2 列表里的某一项
    pub selected_model_id: Option<String>,   // 例 "corivo:claude-sonnet-4-6"

    // Byok 模式
    pub byok_api_shape: Option<ApiShape>,
    pub byok_base_url: Option<String>,
    pub byok_key: Option<String>,
    pub byok_model: Option<String>,          // 用户直填, 例 "claude-sonnet-4-20250514"

    // 通用
    pub thinking_level: ThinkingLevel,
}
```

`anthropic_api_key` / `openai_api_key` / `provider` 这些零散字段全删 —— BYOK 走统一的 `byok_*` 字段族,CorivoProxy 只需要一个 `selected_model_id`。

按 [CLAUDE.md "Project status"](../../../CLAUDE.md) 原则,未发布产品不做向后兼容;旧 `config.json` 缺字段时按 default 填(默认 CorivoProxy + 后端推荐 model)。

### 7.5 模型列表的本地缓存与刷新

模型列表是**瞬态缓存**,不放进 `Config.exec_agent`(避免和用户配置混在一起、避免 store 文件膨胀)。落到独立的 [`tauri-plugin-store`](https://v2.tauri.app/plugin/store/) bucket:

```
$APPDATA/.models-cache.json   # 独立文件
{
  "fetched_at": 1700000000000,
  "models": [ /* §7.2 schema */ ]
}
```

- 登录成功后立即拉一次 `/v1/models` 写入缓存
- 每次 desktop 启动 + 每次进 Settings 模型选择 UI 时刷新(后台异步,UI 先用缓存值秒开)
- Settings UI 只展示 `tier=main` 的 model;`tier=compaction` 的不展示但 sidecar 通过 `compaction_partner_id` 隐式使用
- BYOK 模式下这个缓存为空(`models: []`),Settings UI 走另一套(用户直填 model id)

### 7.6 BYOK 模式下的 compaction model

BYOK 没有"后端配套下发"。规则:
- Anthropic shape → 默认用 `claude-haiku-4-5`(写死,可在高级配置里改)
- OpenAI shape → 默认用 `gpt-4o-mini`

写死的常量列在 `packages/agent/src/auth.ts`,不进 Config(避免选项膨胀)。

### 7.7 BYOK 模式下的 model id 校验

CorivoProxy 模式下,model id 来自 §7.2 后端下发列表,后端保证合法。BYOK 模式下,用户填的 model id 是字符串,可能拼错,可能用了 provider 不支持的 model 名,可能 key 没权限调那个 model。

**校验时机**: sidecar 启动时,在跑第一个 turn 之前,调一次 provider 的标准 models 列表接口:

```ts
// packages/agent/src/auth.ts
async function validateByokModel(input: SidecarInput): Promise<void> {
  if (input.auth.mode !== "byok") return;

  const { api_shape, base_url, token } = input.auth;
  const url = api_shape === "anthropic"
    ? `${base_url ?? "https://api.anthropic.com"}/v1/models`
    : `${base_url ?? "https://api.openai.com/v1"}/models`;

  const resp = await fetch(url, {
    headers: api_shape === "anthropic"
      ? { "x-api-key": token, "anthropic-version": "2023-06-01" }
      : { authorization: `Bearer ${token}` },
  });
  if (!resp.ok) {
    throw new SidecarError("byok_validation_failed",
      `provider models endpoint returned ${resp.status}`);
  }
  const json = await resp.json();
  // 两 provider 的 /v1/models 都返回 { data: [{ id, ... }] }, 形状一致
  const ids: string[] = json.data.map((m: { id: string }) => m.id);
  if (!ids.includes(input.model.id)) {
    throw new SidecarError("byok_model_not_found",
      `model "${input.model.id}" not in provider's available list`);
  }
}
```

校验失败 → emit `{ "type": "error", "data": { "code": "byok_*", "message": "...", "recoverable": false } }` → exit 1。Rust 侧把这条 error 透到前端 settings UI("你填的 model id 不存在")。

**优化**: sidecar 进程不会反复跑(一个 turn 一个进程),所以每个 turn 都校验一次。如果觉得多余,可以让 Rust 侧在 Settings 保存 BYOK 配置时**先**起一次轻量校验 sidecar(只跑校验、不跑 turn),通过了再持久化。v1 不做,直接每 turn 校验,失败就明显报错。

## 8. Session、持久化、Compaction

### 8.1 Session 持久化

pi-coding-agent 的 `SessionManager` 把会话写成 jsonl。落盘位置：

```
$APPDATA/corivo-agent-sessions/
  └── {thread_id}.jsonl
```

`thread_id` 直接用 Corivo 的 ulid（不再像现在那样 v5 映射成 UUID 给 claude）—— jsonl 文件名只要文件系统能接就行。

**与 `chat_threads` / `chat_messages` 表的关系**：
- jsonl 是 **agent 内部记忆**（包含每轮 raw LLM messages、tool args、partial 状态）
- `chat_messages` 是 **产品视图**（用户看到的消息、引用的 frames）
- 二者通过 `thread_id` 一一对应，但 schema 不重叠

每个 turn 结束 sidecar emit `turn_end` 时,Rust 侧仍然走原 [`chat_persist_turn`](../../src-tauri/src/commands/chat.rs) 路径写 `chat_messages`。jsonl 由 sidecar 自己管。

**Retention**(jsonl 文件存多久):
- 默认 **90 天**, 与 [frames retention](../../src-tauri/src/services/retention) 对齐 —— 反正 frames 没了之后会话也很难复盘
- 复用现有 `services::retention` 的 hourly sweep, 加一条规则: 删除超过 90 天没修改的 `corivo-agent-sessions/*.jsonl`
- 用户手动删除 `chat_threads` 行时同步删对应 jsonl(在 `chat_thread_delete` command 里加一行)

### 8.2 Thread ↔ Model 绑定:切 model = 开新 thread

**决策**: 一个 chat thread 创建时绑定一个 model id, 此后不可改。用户在 Settings 切换 model 不影响已存在 threads —— 它们继续用各自创建时绑定的 model。下一次新建 thread (UI 上点"+"或 Quick Ask 触发新会话)用当前选中的 model。

**为什么这么做**:
- 同一会话里跨 model 切换,prompt cache、thinking budget、tool call 格式、context window 都会断 —— 用户体验上是"模型不连贯",对话效果会明显劣化
- 实现上跨 model 切换需要重写 messages 历史(不同 provider 的 message 格式略有差异),工程复杂度高,收益低
- "切 model 当作开新会话"和用户心智模型一致 —— 切 GPT 想得到 GPT 的回答,不应该让它继续 Claude 的话头

**Schema 变化**:按项目 [migration 约定](../CLAUDE.md)("purge-and-apply,不写 ladder"),不用 `ALTER TABLE` —— 改 [`db/schema.sql`](../../src-tauri/src/db/schema.sql) 给 `chat_threads` 加两列,bump `TARGET_SCHEMA_VERSION`,在 [`db/migrations.rs`](../../src-tauri/src/db/migrations.rs) 的 legacy-drop 列表里确认 `chat_threads` 在内(老 dev 的 `.sqlite` 会被丢掉重建)。

新增列:

```sql
-- chat_threads 在 db/schema.sql 里添加:
bound_model_id   TEXT NOT NULL,   -- "corivo:claude-sonnet-4-6" (CorivoProxy) 或 "claude-sonnet-4-20250514" (BYOK)
bound_api_shape  TEXT NOT NULL    -- "anthropic" | "openai" —— BYOK 路径下 sidecar 启动需要
```

新建 thread 时写入:
- CorivoProxy 模式:`bound_model_id = Config.exec_agent.selected_model_id`,`bound_api_shape` 从 §7.2 模型列表查
- BYOK 模式:`bound_model_id = Config.exec_agent.byok_model`,`bound_api_shape = Config.exec_agent.byok_api_shape`

**Sidecar 输入**:每次 turn 用 thread 自己绑定的 model,不读 Config 当前选中的:

```rust
// commands/exec_agent.rs (重构)
let thread = chat_repo.get_thread(thread_id)?;
let model = ResolvedModel {
    id: thread.bound_model_id,
    api_shape: thread.bound_api_shape,
    // CorivoProxy 模式下 compaction_partner_id 也要随 thread 走 ——
    // §7.2 模型列表对每个 model 都给了配套, 直接查表即可
    compaction_partner_id: lookup_compaction_partner(&thread.bound_model_id),
};
```

**前端**:thread 列表 UI 上把 `bound_model_id` 的 `display_name` 角标式展示(比如灰色小字"Claude Sonnet 4.6"),让用户清楚每个 thread 用的什么 model。

**用户路径**:
- Settings 切 model → 只改 `Config.exec_agent.selected_model_id`,已有 threads 不动
- 用户在 `/ask` 点"新会话"或 Quick Ask 起新会话 → 创建 `chat_threads` 行时把当前 `selected_model_id` 写进 `bound_model_id`
- 用户在已有 thread 里继续聊 → 用 `bound_model_id`,UI 上展示当前用的是哪个 model

### 8.3 Compaction

**触发条件**（参照 [openclaw context-pruning extension](https://github.com/openclaw/openclaw/blob/main/src/agents/pi-hooks/context-pruning/extension.ts)）：
1. **预算 token 用量**：当前 messages token 估算 ≥ model context window × 0.75 → auto compact
2. **手动触发**：Rust 侧通过 stdin 第二条 control message 发 `{"type": "compact"}`（v1 可不做，先只做 auto）
3. **cache TTL pruning**：超过 5 分钟未命中的旧消息（针对 Anthropic prompt cache）— 复用 openclaw 的 cache-ttl 思路

**Compaction 行为**：
- 用 §5.1 输入里的 `compaction_model`(CorivoProxy 模式由后端通过 `compaction_partner_id` 配套下发,BYOK 模式 sidecar 写死 haiku/4o-mini, 详见 §7.6)调一次 LLM,把超出窗口的早期 messages 总结成一条 `summary` 消息
- 保留:system prompt、最近 N 轮(N 由 token 预算反推)、`turn_end` 关联的 cited_frames 不丢
- emit `compaction_start` / `compaction_end` 给 Rust,前端 v2 可视化

**实现**：在 sidecar 注册 pi extension（参 [pi-hooks 模式](https://github.com/openclaw/openclaw/tree/main/src/agents/pi-hooks)）。Extension 文件放 `packages/agent/src/extensions/`：
- `context-pruning.ts` —— 自动总结超长上下文
- `compaction-safeguard.ts` —— 兜底：万一 compact 后还溢出就硬截断 + warn

### 8.4 Steering / Follow-up

pi-agent-core 的 `agent.steer()` / `agent.followUp()` 暂不在 v1 暴露给 Rust（现有产品没这个 UX）。预留：未来如果 Quick Ask 加"用户在 agent 跑工具时插话打断"，通过 stdin 第二条 control message 发 `{"type": "steer", "content": "..."}`，sidecar 调 `agent.steer(...)`。

## 9. 构建与打包

### 9.1 `packages/agent/`

```
packages/agent/
├── package.json          # @corivo/agent
├── tsconfig.json         # extends @repo/tsconfig/node.json
├── build.mjs             # Bun --compile per-arch
├── src/
│   ├── main.ts           # entry: 读 stdin → 跑 agent → emit stdout
│   ├── rpc.ts            # UDS client (sidecar → Rust callback)
│   ├── native-tools/
│   │   ├── recall-screen-history.ts
│   │   └── ask-permission.ts
│   ├── mcp/
│   │   └── runtime.ts    # mcporter adapter (createRuntime + AgentTool wrap)
│   ├── extensions/
│   │   ├── context-pruning.ts
│   │   └── compaction-safeguard.ts
│   ├── auth.ts           # AuthChoice → pi AuthStorage
│   ├── system-prompt.ts  # build system prompt (Quick Ask vs /ask)
│   ├── events.ts         # corivo wire envelope emitter
│   └── types.ts          # input schema (matches §5.1)
└── tests/
    └── ...
```

**Build 命令**：
```bash
pnpm --filter @corivo/agent build   # → 三 arch + universal binary 落到 src-tauri/binaries/
```

**`build.mjs` 流程**（参 [packages/desktop-helpers/build.mjs](../../../packages/desktop-helpers/build.mjs)）：
1. `bun build src/main.ts --compile --target=bun-darwin-arm64 --outfile corivo-agent-aarch64-apple-darwin`
2. `bun build … --target=bun-darwin-x64 --outfile corivo-agent-x86_64-apple-darwin`
3. `lipo -create -output corivo-agent-universal-apple-darwin {arm64} {x86_64}`
4. `cp` 三个产物到 `apps/desktop/src-tauri/binaries/`

### 9.2 `packages/mcp/` (corivo-mcp 迁移)

这是 v1 的 **companion task**，行为不动，只搬位置：

```
packages/mcp/
├── Cargo.toml            # crate = corivo-mcp
├── src/main.rs           # 从 apps/desktop/src-tauri/src/bin/corivo-mcp.rs 搬过来
├── src/proto.rs          # 从 apps/desktop/src-tauri/src/services/exec_agent/mcp_proto.rs 搬过来（去掉 desktop crate 依赖）
├── build.sh              # cargo build --release --target=...
└── README.md             # 公开使用文档（这是面向"任意 MCP 客户端"的）
```

**关键改动**：
- 砍掉对 `corivo_app_lib::services::exec_agent::mcp_proto::*` 的依赖（迁过去）
- BRIDGE_SOCKET_ENV 行为保留 —— corivo-mcp 仍然通过 UDS 回主进程拿数据（短期内）
- 主进程 [`mcp_bridge.rs`](../../src-tauri/src/services/exec_agent/mcp_bridge.rs) 在 agent sidecar 切换后**不再被自家 agent 用**，但保留服务于"外部 MCP 客户端通过 corivo-mcp 访问 corivo 数据"的场景

未来 `corivo-mcp` 是否要彻底脱离主进程独立运行（例如直读 SQLite），是 v2 议题。

### 9.3 `tauri.conf.json` 变化

```jsonc
{
  "bundle": {
    "externalBin": [
      "binaries/corivo-capture-helper",
      "binaries/corivo-mcp",
      "binaries/corivo-agent",                // ⬅ 新
      // "binaries/claude"                    // ⬅ 删除
    ]
  }
}
```

## 10. 迁移计划（从 claude CLI 到 corivo-agent）

**分阶段，最小可回滚**：

### Phase A — 搭骨架（不接前端）
1. `packages/agent/` 建好，pi-coding-agent 跑通 `bun --compile` 出 binary
2. sidecar 接受 §5.1 输入 → emit §5.2 NDJSON → 在 CLI 跑通一个 demo turn（无工具，纯 text_delta）
3. native-tools 的 `recall_screen_history` mock 实现（暂不连 Rust UDS，返回空数组）

### Phase B — 双写并行
4. Rust 侧 `runner.rs` 加一个 `agent_kind: AgentKind { Claude, Corivo }` 开关，Config 里加 `Config.exec_agent.kind`
5. `protocol.rs` 拆成 `protocol/claude.rs` + `protocol/corivo.rs`，按 kind 分支
6. `rpc_server.rs` 写好（替代 `mcp_bridge` 在 corivo 路径下的角色）
7. 手测：Settings 里切到 "Corivo Agent (beta)"，跑 /ask 和 Quick Ask 各一遍

### Phase C — 砍 claude
8. compaction extension 接入并验证(用 §7.2 后端下发的 `compaction_partner_id`)
9. 后端 `/v1/models` 端点上线 + desktop Settings 模型选择 UI 改造完(纯模型 picker, 砍掉 provider/key 字段)
10. OpenAI / Anthropic 两条 api_shape 路径都跑通端到端 turn
11. 默认值切到 corivo agent
12. `binaries/claude*` 移除, `AuthChoice::SystemClaude` 删除, Config 里 `anthropic_api_key` / `provider` 等旧字段删除
13. `mcp_bridge.rs` 的 agent 调用路径删除(保留 corivo-mcp 的外部消费路径)

### Phase D — companion task:corivo-mcp 搬家
14. `packages/mcp/` 建好,搬 source,独立 crate
15. desktop crate 移除 `bin/corivo-mcp.rs` 与 `services/exec_agent/mcp_proto.rs`(常量定义同步搬到 `packages/mcp/src/proto.rs`)
16. Tauri sidecar 路径调整、`tauri.conf.json` 的 `externalBin` 路径同步

每个 phase 各自一个 PR。**Phase A 和 B 可以并行启动**:B 在写 `protocol/corivo.rs` 时,如果 A 还没出 binary,可以先用 NDJSON fixture 做单测;真正端到端联调需要 A 的 demo binary 就位。Phase D(corivo-mcp 搬家)和前 3 个 phase **完全独立**,排期上随时插。

## 11. Open questions

| # | 问题 | 状态 |
|---|---|---|
| ~~Q1~~ | MCP 网关实现 | ✅ 定: [openclaw/mcporter](https://github.com/openclaw/mcporter), 见 §6.3 |
| ~~Q2~~ | 取消机制 | ✅ 定: stdin 控制信道 + `cancel` 控制消息, 见 §5.3 |
| ~~Q3~~ | jsonl session retention | ✅ 定: 90 天对齐 frames, 见 §8.1 |
| ~~Q4~~ | 多 provider 的 model 列表 UX | ✅ 定: 后端账号池 + 模型列表下发,前端只见模型选项, 见 §7 |
| ~~Q5~~ | Bash 工具执行点 | ✅ 定: pi 内置, ask_permission 在 `beforeToolCall` hook 拦, 见 §6.4 |
| ~~Q6~~ | Compaction model | ✅ 定: 后端通过 `compaction_partner_id` 配套下发,BYOK 写死默认 (Anthropic→haiku, OpenAI→4o-mini), 见 §7.2 / §7.6 |
| **Q7** | 后端 `/v1/models` 端点的实现 | **依赖后端落地**。spec 假定字段如 §7.2,后端实现完成后回来 cross-check |
| ~~Q8~~ | BYOK model id 合法性校验 | ✅ 定: sidecar 启动时调 `models.list` 校验 (Anthropic `/v1/models` / OpenAI `/v1/models`),失败 emit `error` event 并退出, 见 §7.7 |
| ~~Q9~~ | 切 model 时会话连续性 | ✅ 定: **切 model = 开新 thread**。`chat_threads.bound_model_id` 永久绑定创建时的 model,中途不可改, 见 §8.2 |

## 12. 验收标准

- [ ] `pnpm --filter @corivo/agent build` 输出三 arch binary 到 `apps/desktop/src-tauri/binaries/`
- [ ] `pnpm tauri:dev` 启动后 `/ask` + Quick Ask 两条路径走 corivo-agent，行为与现 claude 路径一致（流式文本、工具调用、cited_frames）
- [ ] Anthropic + OpenAI 两个 provider 都能跑通完整 turn
- [ ] `recall_screen_history` 通过 native AgentTool 路径返回正确 frames
- [ ] `ask_permission` 弹窗确认链路完整
- [ ] 用户配置一个外部 MCP server（如 `mcp-server-fs`），sidecar 自动暴露其工具，agent 可调用
- [ ] 长会话（>30 轮）触发 auto compaction，会话不崩、token 计数下降
- [ ] sidecar 子进程崩溃时 Rust 主进程优雅恢复（emit `error` event + `finish(Error)`）
- [ ] Settings 模型选择 UI 改造完成:CorivoProxy 模式纯模型 picker(从 §7.5 缓存读 `tier=main` 列表),BYOK 模式折叠在"高级"区(api_shape + base_url + key + model id 字段);旧的 `anthropic_api_key` / `provider` 等零散字段 UI 全部移除
- [ ] 用户 cancel 当前 turn 后 `agent_end.finish_reason = "Cancelled"`,前端显示中性"已取消"而不是错误样式
- [ ] 用户在 Settings 切 model 后,已有 thread 仍然用各自 `bound_model_id`,新建 thread 用新选的 model
- [ ] BYOK 模式下,填错 model id 启动 sidecar 立即报错,前端 Settings UI 提示"model id 不存在"

## 13. 参考

- [pi-coding-agent README](https://github.com/badlogic/pi-mono/tree/main/packages/coding-agent) —— 主要依赖,API 入口
- [pi-agent-core README](https://github.com/badlogic/pi-mono/tree/main/packages/agent) —— Agent loop / `AgentTool` / hook 接口
- [pi-ai README](https://github.com/badlogic/pi-mono/tree/main/packages/ai) —— 多 provider streaming 实现
- [mcporter README](https://github.com/openclaw/mcporter) —— MCP runtime, §6.3 直接依赖

> **关于 openclaw**:openclaw 走的是 **embedded** 模式(Node 主进程内嵌 pi),Corivo 走的是 **RPC sidecar** 模式 —— 二者集成路径不同。下面这些链接是**代码结构 / hook 写法 / 工具组织模式的参考样本**, 不是直接复用,实施时不要照搬其 embedded 专有代码:

- [openclaw pi 集成文档](https://github.com/openclaw/openclaw/blob/main/docs/pi.md) —— 整体架构样本
- [openclaw `pi-embedded-runner/`](https://github.com/openclaw/openclaw/tree/main/src/agents/pi-embedded-runner) —— session 生命周期组织 / 错误分类思路
- [openclaw `pi-tools.*`](https://github.com/openclaw/openclaw/tree/main/src/agents) —— 工具替换 / 包装模式 / abort 桥接
- [openclaw `pi-hooks/`](https://github.com/openclaw/openclaw/tree/main/src/agents/pi-hooks) —— 我们的 §8.3 compaction extension 直接参考此处实现

---

- 现有 [exec_agent](../../src-tauri/src/services/exec_agent) 实现 —— 迁移起点
- 现有 [capture-helper-architecture-spec.md](capture-helper-architecture-spec.md) —— IPC + sidecar 设计参考样本
