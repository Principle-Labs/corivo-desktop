# Corivo Connector 框架 Spec

> Status: Final v2 · 2026-05-11
> Owner: Desktop + Agent · Surface: `apps/desktop/` + `packages/connector-*/`
> Supersedes: `gmail-integration-spec.md`（建议删除）

把 Corivo 跟外部服务（Gmail / Notion / Slack / Discord / Google Calendar / Docs / …）的对接做成一个统一模式：每个集成是一个独立 pnpm package，走相同的 OAuth 流程、相同的 token 存储、相同的 UI 卡片、相同的 Tauri commands。**没有 SDK 包，没有 MCP 协议，没有自定义协议**——每个 connector 就是 export 一个返回 `AgentTool[]` 的函数。

## 0. 关键决策摘要

| 维度 | 选择 | 否决了什么 |
|---|---|---|
| 数据流 | 桌面进程客户端直连第三方 API；不经过 Corivo 后端 | 服务端代理 |
| OAuth | Rust 端 `oauth_loopback`（loopback PKCE）；Google 全家桶共用一个 Corivo OAuth client | 用户自配 client_id（"Path A"）/ 服务端 OAuth |
| Connector 包 | 每个集成一个 pnpm package：`packages/connector-<service>/` | 单一大 service crate / 散落的 native tool |
| 集成代码形状 | export `createTools(ctx) → AgentTool[]` + `manifest.json` | 独立 SDK 包 / MCP server / 自定义协议 |
| Agent 加载 | 静态 import + 按 enabled 列表筛选注册 | 第一版不做 dynamic import / CDN / Worker |
| Token 流转 | Rust 端持有 token + 跨 stdio 提供给 agent；access_token 永不落盘 | broker HTTP server / token 写 input JSON |
| Token 存储 | macOS Keychain（`keyring` crate） | plaintext config.json |
| UI | Settings → 集成 section + 通用 `ConnectorCard`（manifest 驱动） | Gmail-only 卡片 |
| Tauri commands | 通用接口接 id 参数（`connector_enable` / `_connect` / `_disconnect`） | 每个 service 一套命令 |
| 第一版生态 | 只有 Corivo 写的内置 connector，全部打包进 app | CDN 按需下载 / 社区 marketplace / 用户自配外部 MCP server |

未来扩展路径（不在本 spec 范围）：
- **CDN 按需下载**：dynamic import 能力已 PoC 验证（[connector-framework-poc/](./connector-framework-poc/)），Phase 2 启用
- **Worker 隔离**：marketplace 开放前再做，connector 代码不变只换 transport
- **mcporter 通道**：agent 已经预留 `McpServerSpec` 接入点，未来给"用户自配外部 stdio MCP server"用，跟 Corivo connector 框架并存互不交叉

---

## 1. 范围

### In scope

- Rust 端 `oauth_loopback`（从现有 `google_oauth.rs` 抽离的通用 loopback OAuth flow）
- Rust 端 `services/connector/`（registry + secrets/Keychain + token bridge）
- 通用 Tauri commands + ts-rs 类型
- Agent 端 connector types + loader + `ctx.fetch` helper（token 自动注入）
- Settings → 集成 section + 通用 `ConnectorCard`
- **首个 connector：`packages/connector-gmail`**（Phase 1 send-only）

### Out of scope

- 独立 SDK 包、MCP 协议、自定义协议
- CDN 下载 / 远程 connector / 签名机制
- Worker Thread 隔离 / 进程级 sandbox
- 后台任务（poll / watch / cron）
- 第二个 connector 的实现（框架就位后再说）
- 多账号 / 账号切换
- Windows / Linux

---

## 2. Connector 包形状

每个 connector 是一个独立的 pnpm workspace package。结构：

```
packages/connector-gmail/
├── package.json                   # name: "@corivo/connector-gmail"
├── manifest.json                  # 元数据（§2.1）
├── icon.svg
├── src/
│   ├── index.ts                   # export { createTools }
│   └── tools/
│       └── send.ts                # 实现 gmail_send
└── tsconfig.json                  # extends @repo/tsconfig/base.json
```

### 2.1 manifest.json schema

```jsonc
{
  "id": "gmail",
  "name": { "zh": "Gmail", "en": "Gmail" },
  "description": {
    "zh": "通过你的 Gmail 账户收发邮件",
    "en": "Send and receive emails through your Gmail account"
  },
  "version": "1.0.0",
  "icon": "icon.svg",
  "auth": {
    "type": "oauth2",
    "provider": "google",                              // OAuth client 提供者，§3.3
    "authUrl": "https://accounts.google.com/o/oauth2/v2/auth",
    "tokenUrl": "https://oauth2.googleapis.com/token",
    "scopes": [
      {
        "value": "https://www.googleapis.com/auth/gmail.send",
        "label": { "zh": "发送邮件", "en": "Send emails" },
        "required": true
      }
    ],
    "extraAuthParams": {
      "access_type": "offline",
      "prompt": "consent",
      "include_granted_scopes": "true"
    }
  }
}
```

**为什么是单独的 JSON 文件**：Rust 端在 boot 时枚举所有内置 connector 的 manifest 来填 Settings UI 的 connector 列表——不需要 load JS 就能拿到元数据。前端通过 Tauri command 拿 manifest，渲染 ConnectorCard。

### 2.2 index.ts 的 export 形状

```ts
// packages/connector-gmail/src/index.ts
import type { AgentTool } from "@mariozechner/pi-agent-core"
import type { ConnectorCtx } from "@corivo/agent/connector"  // type-only, monorepo path

import { sendTool } from "./tools/send"

export function createTools(ctx: ConnectorCtx): AgentTool[] {
  return [sendTool(ctx)]
}
```

`createTools` 是唯一约定。Agent loader 在 boot 时按 enabled 列表对每个 connector 调一次，把返回的 `AgentTool[]` 全部 push 到 pi-agent 的 tool registry。

### 2.3 工具实现示例

```ts
// packages/connector-gmail/src/tools/send.ts
import type { AgentTool } from "@mariozechner/pi-agent-core"
import type { ConnectorCtx } from "@corivo/agent/connector"

type SendParams = { to: string; subject: string; body: string }

export function sendTool(ctx: ConnectorCtx): AgentTool<SendParams> {
  return {
    name: "gmail_send",
    description: "Send an email through the user's Gmail account.",
    inputSchema: {
      type: "object",
      required: ["to", "subject", "body"],
      properties: {
        to: { type: "string", description: "Recipient email address" },
        subject: { type: "string" },
        body: { type: "string", description: "Plain text body" },
      },
    },
    execute: async (params) => {
      const raw = buildRfc2822({ to: params.to, subject: params.subject, body: params.body })
      const res = await ctx.fetch(
        "https://gmail.googleapis.com/gmail/v1/users/me/messages/send",
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ raw: toBase64Url(raw) }),
        },
      )
      if (!res.ok) {
        throw new Error(`gmail_send failed: ${res.status} ${await res.text()}`)
      }
      return await res.json()
    },
  }
}

function buildRfc2822(_: { to: string; subject: string; body: string }): string { /* ... */ return "" }
function toBase64Url(_: string): string { /* ... */ return "" }
```

**关键**：connector 用 `ctx.fetch` 而不是全局 `fetch`——这一行差别让 host 能：
- 自动注入 `Authorization: Bearer <access_token>`
- 401 时透明 refresh + 重试一次
- 把 access_token 字符串本体留在 host 边界内（不暴露给 connector 代码）

### 2.4 禁令

每个 connector 包**禁止**：
- Native 依赖（任何 `*.node`、`node-gyp`、`@google-cloud/*` 一类官方 SDK）
- Provider 官方 SDK（动辄 1-10 MB，且经常 bundle native code）—— 直接用 `ctx.fetch` 调 raw HTTP
- Import 其他 connector 包
- Import `@corivo/*` 桌面端代码

允许：
- `@corivo/agent` 的 type-only import（拿 `ConnectorCtx` 类型）
- `@mariozechner/pi-agent-core` 的 type-only import（拿 `AgentTool` 类型）
- Bun / Node 标准库
- 小型 npm 包（`zod`、`date-fns` 一类，<100 KB tarball）

---

## 3. OAuth 框架

### 3.1 抽离 `oauth_loopback` 模块

[apps/desktop/src-tauri/src/services/corivo_session/google_oauth.rs](../src-tauri/src/services/corivo_session/google_oauth.rs) 已经把 loopback + PKCE + state + 系统浏览器 + 302 重定向写好。抽离成通用模块：

```
apps/desktop/src-tauri/src/services/oauth_loopback/
├── mod.rs        # LoopbackFlow + TokenSet（公开 API）
├── server.rs     # loopback HTTP server（accept callback + redirect）
└── token.rs      # token exchange POST + refresh
```

`google_oauth.rs` 重构成对它的薄包装。Connector OAuth 是第二个 caller。

### 3.2 LoopbackFlow 公开接口

```rust
pub struct LoopbackFlow {
    pub client_id: String,
    pub client_secret: Option<String>,        // Desktop client 可省，"伪秘密"
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    pub extra_auth_params: Vec<(String, String)>,
    pub success_url: String,                  // 302 终点（成功）
    pub error_url_prefix: String,             // 302 终点（失败，会被加 ?reason=）
}

impl LoopbackFlow {
    pub async fn run<F>(&self, open_browser: F) -> Result<TokenSet>
    where F: FnOnce(String) -> Result<()> + Send;

    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenSet>;
}

pub struct TokenSet {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,             // 仅 openid scope 在场
    pub token_type: String,
    pub expires_at: Option<DateTime<Utc>>,    // 绝对时间，不是 expires_in 相对值
    pub scope: Vec<String>,                   // 实际授予的 scope（granular permissions）
}
```

### 3.3 OAuth Client 提供者

第一版只支持 `provider: "google"`——Corivo 持有一个 Google OAuth client，所有 Google 系 connector 共用。其他 provider 落地时再加。

OAuth client 凭据从环境变量取，命名跟现有 `CORIVO_GOOGLE_CLIENT_ID` 同模式：
- `CORIVO_GOOGLE_CLIENT_ID` —— **复用**现有变量
- `CORIVO_GOOGLE_CLIENT_SECRET` —— 复用

> ⚠️ 注意：现有 `google_client_id()` / `google_client_secret()` 函数在 [env.rs](../src-tauri/src/env.rs) 已经定义，**我们直接复用，不创建新的环境变量**。Corivo session login 和 Gmail connector 共用同一个 Google OAuth client 是合理的：scope 不同、verification 路径不同（Corivo login 只要 `openid email profile`，是 non-sensitive；Gmail 要 `gmail.send`，是 sensitive），但 client 本身可以共用。

### 3.4 Consent screen 必须 In Production

OAuth consent screen 在 "Testing" 状态下颁发的 refresh_token **7 天过期**。Phase 1 落地前必须把 Corivo 项目的 consent screen publish 到 "In production"——这是外部前置任务，不在代码 spec 范围。

### 3.5 Token 存储

- **access_token / refresh_token 存系统 Keychain**（`keyring` crate 3.x）
- service=`ai.corivo.desktop`，account=`connector:<connector_id>:<google_sub>:{access,refresh}_token`
- 元数据（email / display_name / granted_scopes / expires_at）存 `Config.connectors.accounts[id]`，配合 ts-rs 让前端能拿账号列表无 round trip

### 3.6 Token 刷新

- `access_token` TTL = 1 小时
- 调用 Gmail API 前看 `expires_at - now < 60s` 就主动 refresh
- 并发 refresh 用 `tokio::sync::Mutex` single-flight
- refresh 失败（`invalid_grant`）→ 清 Keychain → 标 `needs_reauth` → UI 红色提示 → 不重试

---

## 4. Token 跨 stdio 注入 agent

Agent sidecar 是 per-turn 进程，由 Rust 端 spawn。Token 怎么从 Rust 流到 connector 代码：

### 4.1 启动时注入 snapshot

Rust 把 enabled connector + 各自的 access_token 写入 input JSON：

```jsonc
// agent input JSON（追加在现有 schema 上）
{
  "tools": { ... },
  "connectors": {
    "enabled": [
      {
        "id": "gmail",
        "access_token": "ya29....",
        "expires_at": "2026-05-11T13:00:00Z"
      }
    ]
  }
}
```

**refresh_token 不写入 input JSON**——只在 Rust + Keychain 持有。

### 4.2 Turn 中刷新（stdio NDJSON）

如果 turn 持续超过 1 小时（少见但可能），agent 的 `ctx.fetch` 检测到 `expires_at - now < 60s` 或收到 401：

```
agent → Rust（stdout NDJSON）:
  { "type": "host_request", "op": "refresh_connector_token", "connector_id": "gmail" }

Rust → agent（stdin）:
  { "type": "host_response", "access_token": "...", "expires_at": "..." }
```

Rust 端 `services/connector/runtime.rs` 实现 stdin/stdout 端的请求/响应路由。复用现有 [exec_agent/runner](../src-tauri/src/services/exec_agent/runner/mod.rs) 的 NDJSON 通道，不起 HTTP broker。

### 4.3 Agent 端 ConnectorCtx 类型

```ts
// packages/agent/src/connector/types.ts
export interface ConnectorCtx {
  readonly connectorId: string
  readonly accountEmail: string | null
  readonly grantedScopes: string[]
  fetch(url: string, init?: RequestInit): Promise<Response>
  log(level: "debug" | "info" | "warn" | "error", msg: string, meta?: unknown): void
}
```

`ctx.fetch` 实现：
1. 取当前 access_token（从 in-memory token cache）
2. 加 `Authorization: Bearer <token>` header
3. 调 `fetch(url, init)`
4. 如果 401：通过 stdio 请求 refresh → 拿新 token → 重试一次
5. 不允许 caller 覆盖 `Authorization`

---

## 5. Rust 端模块结构

```
apps/desktop/src-tauri/src/
├── services/
│   ├── oauth_loopback/                # 新增，§3.1
│   │   ├── mod.rs
│   │   ├── server.rs
│   │   └── token.rs
│   ├── corivo_session/
│   │   └── google_oauth.rs            # 重构：薄包装 oauth_loopback
│   ├── connector/                     # 新增
│   │   ├── mod.rs                     # ConnectorRegistry（state on AppState）
│   │   ├── manifest.rs                # Manifest 类型 + 解析（serde）
│   │   ├── catalog.rs                 # 内置 connector 清单（编译期）
│   │   ├── secrets.rs                 # keyring 封装
│   │   ├── auth.rs                    # connector 维度的 OAuth 入口（→ oauth_loopback）
│   │   └── runtime.rs                 # agent 启动时填 input snapshot + stdio refresh
│   └── ...
├── commands/
│   └── connectors.rs                  # §6
└── domain/
    └── config.rs                      # 加 ConnectorsConfig
```

### 5.1 Cargo.toml 新增

```toml
keyring = "3"
chrono = { version = "0.4", features = ["serde"] }   # 可能已有
```

### 5.2 Config schema

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(default, rename_all = "camelCase")]
pub struct ConnectorsConfig {
    pub enabled: Vec<String>,
    pub accounts: HashMap<String, ConnectorAccountMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct ConnectorAccountMeta {
    pub account_id: String,            // Google sub 等
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub granted_scopes: Vec<String>,
    pub connected_at: String,
    pub last_refresh_at: Option<String>,
    pub needs_reauth: bool,
}
```

加到 `Config` 顶层。

---

## 6. Tauri Commands

接 connector id 参数的通用命令——加 Notion / Slack 时无需新命令。

```rust
#[tauri::command] pub async fn connectors_list(state) -> Result<Vec<ConnectorSummary>, TauriError>
#[tauri::command] pub async fn connector_enable(state, id: String) -> Result<ConnectorSummary, TauriError>
#[tauri::command] pub async fn connector_disable(state, id: String) -> Result<(), TauriError>
#[tauri::command] pub async fn connector_connect(state, id: String) -> Result<ConnectorSummary, TauriError>
#[tauri::command] pub async fn connector_disconnect(state, id: String) -> Result<ConnectorSummary, TauriError>
```

### 6.1 ts-rs 导出类型

```rust
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../../packages/shared-types/src/generated/")]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSummary {
    pub id: String,
    pub manifest: ConnectorManifest,     // 完整 manifest，前端拿来渲染卡片
    pub enabled: bool,
    pub account: Option<ConnectorAccountMeta>,
}
```

每个新增类型都要：
1. `[derive(TS)]` + `export_to`
2. 在 [lib.rs](../src-tauri/src/lib.rs) 的 `invoke_handler!` 注册
3. 在 [packages/shared-types/src/index.ts](../../../packages/shared-types/src/index.ts) 加 barrel re-export
4. 在 [apps/desktop/src/lib/tauri.ts](../src/lib/tauri.ts) 加 typed wrapper

---

## 7. Agent 端 connector loader

```
packages/agent/src/connector/
├── types.ts          # ConnectorCtx 类型
├── ctx.ts            # createCtx(snapshot, stdioBridge): ConnectorCtx
└── loader.ts         # registerConnectors(input, registry)
```

`loader.ts` 在 agent boot 时（[agent-runner.ts](../../../packages/agent/src/agent-runner.ts) 现有逻辑里）：

```ts
import { createTools as createGmailTools } from "@corivo/connector-gmail"
const BUILTIN: Record<string, (ctx) => AgentTool[]> = {
  gmail: createGmailTools,
  // notion: createNotionTools,    // 未来
}

export function registerConnectors(snapshot, stdioBridge) {
  const tools: AgentTool[] = []
  for (const c of snapshot.enabled) {
    const factory = BUILTIN[c.id]
    if (!factory) {
      log("warn", `unknown connector id: ${c.id}`)
      continue
    }
    const ctx = createCtx(c, stdioBridge)
    tools.push(...factory(ctx))
  }
  return tools
}
```

**第一版静态 import**——`BUILTIN` 映射写死。等 Phase 2 上 CDN/dynamic import 时把这个 map 改成 `await import()`，connector 代码零改动。

---

## 8. 前端 UI

### 8.1 Settings → 集成 section

```
apps/desktop/src/pages/settings/sections/
└── integrations-section.tsx          # 容器（拉 connectors_list）
    └── ConnectorCard.tsx              # 通用卡片（按 manifest 渲染）
```

`ConnectorCard` 完全通用——Gmail / Notion / Slack 共用同一个组件，差异由 manifest 数据驱动（图标、名字、scope 描述、连接状态）。

### 8.2 状态机

| `ConnectorSummary` 状态 | UI |
|---|---|
| `enabled=false` | 简介 + "启用"按钮（点击 → `connector_enable` + `connector_connect`） |
| `enabled=true, account=null` | "连接 {provider}"按钮 |
| `enabled=true, account != null, needs_reauth=false` | 头像 / email / scope chips / "断开"按钮 |
| `enabled=true, needs_reauth=true` | 红色"已断开，请重新授权" + "重新连接"按钮 |

### 8.3 React Query hooks

```ts
// apps/desktop/src/hooks/use-connectors.ts
export function useConnectors()                  // GET ["connectors"]
export function useEnableConnector()             // mutation
export function useConnectConnector()
export function useDisconnectConnector()
```

### 8.4 i18n 部分文案

| key | zh | en |
|---|---|---|
| `connectors.title` | 集成 | Integrations |
| `connectors.enable` | 启用 | Enable |
| `connectors.connect` | 连接 {provider} | Connect {provider} |
| `connectors.disconnect` | 断开 | Disconnect |
| `connectors.privacyNote` | Corivo 直接访问 {provider}，数据不经过 Corivo 服务器。 | Corivo accesses {provider} directly; data never touches Corivo servers. |
| `connectors.needsReauth` | 已断开，请重新授权 | Disconnected. Please reconnect. |

---

## 9. Gmail Reference Implementation

第一个落地的 connector。`packages/connector-gmail/`。

### 9.1 Phase 1：send-only

| Manifest field | 值 |
|---|---|
| id | `gmail` |
| version | `1.0.0` |
| auth.provider | `google` |
| auth.scopes | `gmail.send` (required) |
| tools | `gmail_send` |

提交 Google **Sensitive scope verification**（不是 Restricted/CASA）。审核周期 2-4 周。Verification 前 Corivo 是 closed beta，OAuth client 有 100 用户上限——刚好够 beta。

### 9.2 包大小目标

`@corivo/connector-gmail` 编译后 bundle < 100 KB。约束依据：禁止 native + 禁止官方 SDK，自己写 RFC 2822 message builder + base64url 编码 + raw `ctx.fetch`。

---

## 10. 错误处理

| 场景 | 处理 |
|---|---|
| access_token 过期未刷新 | `ctx.fetch` 透明 refresh + 重试一次 |
| refresh_token 死掉 | 清 Keychain → 标 `needs_reauth` → UI 红色提示 |
| 用户拒绝部分 scope（granular） | manifest 检查 + tool 注册时跳过未授权工具 |
| 用户拒绝授权 | OAuth callback `error=access_denied` → "用户取消"提示 |
| OAuth 超时（5 分钟未完成） | 跟 [google_oauth.rs:31](../src-tauri/src/services/corivo_session/google_oauth.rs#L31) 一致 |
| Tool 执行抛错 | agent loader catch + 通过 NDJSON 通知 Rust + UI 展示 |
| Connector unknown id | 启动时 log warning，跳过该 connector |

Sentry 上下文：参考 [auth.rs:33](../src-tauri/src/commands/auth.rs#L33) 的 `sync_sentry_user`——connector commands 把当前 connector id + account email 挂 Sentry scope。

---

## 11. 实施分阶段

### Phase 1：框架 + Gmail send-only

| # | 任务 | 文件 / 模块 | 验收 |
|---|---|---|---|
| 1 | 抽离 `oauth_loopback` | `services/oauth_loopback/` | 现有 Google login 不破；cargo test 通过 |
| 2 | Rust `services/connector/` 骨架 + `ConnectorsConfig` + Keychain | `services/connector/` + `domain/config.rs` | cargo check 通过 |
| 3 | Tauri commands + ts-rs 类型 | `commands/connectors.rs` | `pnpm typegen` 产出新类型 |
| 4 | Agent 端 connector types + loader + ctx | `packages/agent/src/connector/` | `pnpm --filter @corivo/agent build` 通过 |
| 5 | Token bridge over stdio | `services/connector/runtime.rs` + agent stdio handler | 手测 refresh path |
| 6 | `packages/connector-gmail` 实现 | 新增 package | bundle < 100KB；`gmail_send` 工具能调通 |
| 7 | Settings → 集成 section + ConnectorCard | `pages/settings/sections/integrations-section.tsx` | 4 种状态全可视化 |

外部前置（**由你做**）：
- Google Cloud Console 创建 / 复用 Desktop OAuth client
- Consent screen publish 到 In Production
- 提交 Sensitive scope verification

### Phase 2：CDN + 第二个 connector + Gmail read/modify

| 任务 |
|---|
| CDN 基础设施 + connectors-index.json |
| 桌面端：CDN 下载 + 校验 + dynamic import（PoC 已验证） |
| Gmail Phase 2 scope（`gmail.readonly` / `gmail.modify`）+ CASA Tier 2 自评估 |
| 加 connector tools：`gmail_list` / `gmail_get` / `gmail_modify` |
| 第二个 connector（Notion / Calendar / Slack 中选一个） |

### Phase 3+（候选）

- Worker Thread 隔离
- Bundle 签名（ed25519）
- 社区 marketplace
- 用户自配外部 MCP server（mcporter 通道）

---

## 12. 风险与未决事项

### 12.1 Bun `--compile` dynamic import — Phase 2 才需要

✅ **Verified 2026-05-11**：[connector-framework-poc/](./connector-framework-poc/) 已通过 5 项验证。第一版不用，留给 Phase 2。

### 12.2 OAuth client 紧急切换（cold-backup）

预案：另一 GCP 项目预先 verify 备用 client，出事时改环境变量 + 推一次桌面 app 更新（或 manifest CDN override —— Phase 2 上 CDN 时实现）。Day One 不实施，但 `LoopbackFlow` 字段全是参数化的，切换 client_id 是改 config 不改代码。

### 12.3 Per-project quota（不管）

客户端模式下所有 Corivo 用户共享 1.2M units/min Gmail quota。已确认 Day One 不监控；用户增长到几千时再评估。

### 12.4 Keychain + codesign

签名变化时 Keychain 视作新 app，会再次弹出"允许访问 Keychain"对话框。Uninstall 不清 Keychain——`connector_disconnect` 命令显式 `keyring::Entry::delete_credential()`。

---

## 13. 参考资料

- [OAuth 2.0 for Mobile & Desktop Apps — Google](https://developers.google.com/identity/protocols/oauth2/native-app)
- [Choose Gmail API scopes — Google](https://developers.google.com/workspace/gmail/api/auth/scopes)
- [keyring crate](https://crates.io/crates/keyring)
- Corivo 现存 OAuth 参考：[google_oauth.rs](../src-tauri/src/services/corivo_session/google_oauth.rs)
- Corivo agent sidecar IPC：[exec_agent/runner](../src-tauri/src/services/exec_agent/runner/mod.rs)
- PoC（Phase 2 用）：[connector-framework-poc/](./connector-framework-poc/)
