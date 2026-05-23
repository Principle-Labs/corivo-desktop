# Corivo Connector 接入指南 — Prompt for AI 协作

> Status: Living doc · 2026-05-12
> Audience: 给 Claude Code / Codex / 任意 AI 助手当 prompt 用。让它读完就能照样子做一个新 connector。
> Companion: 完整框架设计见 [connector-framework-spec.md](./connector-framework-spec.md)。当前三种形态都已有 reference impl:
> - **oauth2** → [packages/connector-gmail/](../../../packages/connector-gmail/)
> - **cliInstall** → [packages/connector-feishu-cli/](../../../packages/connector-feishu-cli/)
> - **mcpServer** → [packages/connector-linear/](../../../packages/connector-linear/)

---

## 这份文档想干什么

让你（AI）按 Corivo 的统一模式给一个新的外部服务（Notion / Slack / Google Calendar / Discord / Linear / Asana / …）写一个**连接器（connector）**。

读完这份文档后，你应该能：

1. 知道有几种 connector **形态**、怎么挑（§零）
2. 知道一个 connector 的**目录结构**长什么样（§一）
3. 知道每个文件要写什么、互相之间的契约是什么（§二）
4. 知道怎么把它**接入框架**（每种形态对应的登记位置不一样，§三）
5. 知道有哪些**禁令**（不要打包重 SDK、不要绕过 `ctx.fetch` 等，§四）
6. 不知道目标服务的 API 长什么样时，**去哪里翻参考**——Raycast 的几千个开源扩展是首选（§五）

---

## 零、先挑形态：三种 connector 形态

manifest 的 `auth.type` 字段在 Rust 端是三 variant 的 tagged enum（[manifest.rs::ConnectorAuthConfig](../src-tauri/src/services/connector/manifest.rs)）。三种形态对作者的工作量差好几个数量级，**动手前必须先想清楚选哪种**。

| 形态 | 一句话 | 你要写多少 TS | OAuth 谁管 | 工具来源 | 典型例子 |
|---|---|---|---|---|---|
| `oauth2` | 全自营：Corivo 直连 vendor API，你手写每个 `AgentTool` | 一个完整 npm 包 + 每个 tool 一个文件 | Rust 端 `oauth_loopback`（你要在 `env.rs` / `client_credentials_for` 加一行 vendor client） | 你写 | Gmail |
| `cliInstall` | 外包给本地 CLI：agent shell 出去调 vendor 的官方 CLI | 0 行 TS | 用户在 CLI 里自己登录（`lark-cli auth login` 之类） | CLI 的 `--help`，由 agent + 用户写的 skill markdown 调度 | 飞书 |
| `mcpServer` | 外包给 vendor 的 MCP server：sidecar mcporter 直接连他们的服务 | 0 行 TS | mcporter 走 MCP spec 的 dynamic client registration，**vendor 自己管** | vendor 通过 MCP 暴露的 tools，sidecar `mcp/runtime.ts` 自动包成 `AgentTool` | Linear |

**决策树**：

```
官方提供 MCP server (走 HTTP/SSE，OAuth 内建)？
  └ 是 → mcpServer
  └ 否 → 官方有成熟 CLI、能力覆盖你要的所有场景？
          └ 是 → cliInstall
          └ 否 → oauth2（最大工作量，但也最可控）
```

**怎么取舍**（详见 [connector-framework-spec.md §0](./connector-framework-spec.md)）：

- 偏好 **`mcpServer`** when: vendor 有官方 MCP（Linear / Notion / GitHub 都在路上），你不想维护 wrapper 跟着 vendor API 更新，能接受 30+ 个 tool 灌进 prompt。
- 偏好 **`cliInstall`** when: 巨型多域套件（飞书全家桶有消息/文档/日历/Base/OKR …），vendor 自己已经发了成熟 CLI；你不想手写也不想要 vendor MCP server 那套 30+ tool。
- 偏好 **`oauth2`** when: 服务的表面很小（3–5 个 endpoint），你想精确控制 agent 看到哪些 tool、错误语义跟 Corivo 的 error 体系对齐、Sandbox 内 `ctx.fetch` 100% 可审计。

---

## 一、目录结构

每个 connector 都活在 `packages/connector-<id>/`。`<id>` 是小写短串，**全框架"主键"**——它必须等于 `manifest.id` / Rust catalog lookup key / 前端 ConnectorCard key。

### 1.1 `oauth2` 形态（完整 pnpm package）

```
packages/connector-<id>/
├── package.json              # workspace dep + scripts
├── tsconfig.json             # extends @repo/tsconfig/node.json
├── manifest.json             # 元数据 + OAuth 配置（hand-written，Rust 端 include_str!）
├── icon.svg                  # SVG 图标（占位也行，前端用首字母 fallback）
└── src/
    ├── index.ts              # export `createTools(ctx)`：connector 的唯一入口
    └── tools/
        ├── <action_a>.ts     # 单个工具实现（每个 AgentTool 一个文件）
        ├── <action_b>.ts
        └── ...
```

### 1.2 `cliInstall` / `mcpServer` 形态（manifest-only）

这两种形态**不写一行 TS**——所有工具来自外部进程（本地 CLI 或 vendor MCP server）。所以包结构最小：

```
packages/connector-<id>/
└── manifest.json             # 唯一必需文件
   (icon.svg 推荐补一个；其他都不要)
```

不要建 `package.json` / `tsconfig.json` / `src/`。建了反而麻烦——pnpm 会把空包加进 workspace 图，agent loader 会以为有 `createTools` 可以调。

### 1.3 命名约定（所有形态共用）

| 元素 | 例子 | 规则 |
|---|---|---|
| 目录名 | `packages/connector-gmail/` | 与 `manifest.id` 后缀一致 |
| `manifest.id` | `"gmail"` / `"feishu-cli"` / `"linear"` | 小写短串，全框架"主键" |
| package name (oauth2 only) | `@corivo/connector-gmail` | 始终 `@corivo/connector-<id>` |
| tool name (oauth2 only) | `gmail_send` | `<id>_<verb>` snake_case，给 LLM 看的 |
| tool 文件 (oauth2 only) | `src/tools/send.ts` | tool 后缀不用 connector id（已经在父目录里了）|

mcpServer tool 命名由 sidecar 自动加 `<serverName>__` 前缀（见 [packages/agent/src/mcp/runtime.ts](../../../packages/agent/src/mcp/runtime.ts)），形如 `linear__list_issues`——你不用命名工具。

---

## 二、每个文件的样板

> 提示：`cliInstall` / `mcpServer` 形态**只需要 §2.3 一种 manifest**（外加可选的 icon），其余小节都是 oauth2 专属。直接跳到 §2.3.2 / §2.3.3 看 manifest 模板。

### 2.1 `package.json`

```jsonc
{
  "name": "@corivo/connector-<id>",
  "version": "0.0.1",
  "private": true,
  "type": "module",
  "description": "Corivo connector for <Service> — Phase 1 <send-only|read-only|...>.",
  "main": "src/index.ts",
  "exports": {
    ".": "./src/index.ts",
    "./manifest.json": "./manifest.json"
  },
  "scripts": {
    "type-check": "tsc --noEmit"
  },
  "dependencies": {
    // typebox is usually needed for tool parameter schemas — match the agent's version.
    "typebox": "^1.1.24"
  },
  "devDependencies": {
    // Why these and only these — see §四 "禁令".
    "@corivo/shared-types": "workspace:*",
    "@mariozechner/pi-agent-core": "0.73.0",
    "@repo/tsconfig": "workspace:*",
    "@types/bun": "^1.3.0",
    "typescript": "~5.8.3"
  }
}
```

**绝对禁止** 在 dependencies 里加 `@corivo/agent` —— agent 已经依赖了你（通过 BUILTIN map），反过来再依赖会产生循环依赖、pnpm 拒绝 resolve。所有需要的类型从 `@corivo/shared-types` 取。

### 2.2 `tsconfig.json`

```jsonc
{
  "$schema": "https://json.schemastore.org/tsconfig",
  "extends": "@repo/tsconfig/node.json",
  "compilerOptions": {
    "lib": ["ES2023"],
    "types": ["bun"],
    "strict": true,
    "moduleResolution": "bundler",
    "noEmit": true,
    "verbatimModuleSyntax": false
  },
  "include": ["src/**/*"]
}
```

照搬 `packages/connector-gmail/tsconfig.json`，不要改。

### 2.3 `manifest.json`

这是 connector 的"身份证"——Rust 端在 catalog 里 `include_str!` 它解析为 `ConnectorManifest`，前端 ConnectorCard 直接渲染它的 `name` / `description` / `auth.*`。schema 是 [`ConnectorManifest` + `ConnectorAuthConfig`](../src-tauri/src/services/connector/manifest.rs) 的三 variant tagged enum。

**JSON 字段必须用 camelCase**（`authUrl` 而不是 `auth_url`）——Rust serde 用了 `rename_all = "camelCase"`。写错 Rust 在 `OnceLock::get_or_init` 里 panic。

#### 2.3.1 `oauth2` 形态

```jsonc
{
  "id": "<id>",
  "name":        { "zh": "服务中文名", "en": "Service English Name" },
  "description": {
    "zh": "一句话说明 connector 干什么。",
    "en": "One-line summary of what this connector lets the user do."
  },
  "version": "1.0.0",
  "icon": "icon.svg",
  "auth": {
    "type": "oauth2",
    "provider": "<google|slack|notion|...>",       // 必须 match Rust 端 client_credentials_for 的 arm
    "authUrl":  "https://accounts.<service>.com/o/oauth2/v2/auth",
    "tokenUrl": "https://oauth2.<service>.com/token",
    "scopes": [
      {
        "value": "https://www.googleapis.com/auth/<scope-url>",
        "label": { "zh": "做某事的权限", "en": "Permission to do X" },
        "required": true
      }
      // 可以加多个；optional 的把 required 设为 false
    ],
    "extraAuthParams": {
      // 跟 IdP 相关。Google 常用：
      // "access_type": "offline",    // 让 Google 返回 refresh_token
      // "prompt": "consent",         // 强制重新 consent，确保拿到 refresh_token
      // "include_granted_scopes": "true"
    }
  }
}
```

参考实现：[packages/connector-gmail/manifest.json](../../../packages/connector-gmail/manifest.json)。

#### 2.3.2 `cliInstall` 形态

```jsonc
{
  "id": "<id>",
  "name":        { "zh": "服务中文名 CLI", "en": "Service CLI" },
  "description": { "zh": "通过本地 <name> CLI ...", "en": "Via the local <name> CLI ..." },
  "version": "1.0.0",
  "icon": "icon.svg",
  "auth": {
    "type": "cliInstall",
    "binary": "<binary-name>",                // 用来探测 PATH 是否已装；前端「已安装」状态据此判
    "installPrompt": {
      // 用户点「安装」时塞进新 /ask 线程的 preset prompt。
      // 让 agent 抓官方安装文档、检测系统、确认命令再装，禁止盲推 `curl|bash`。
      "zh": "请帮我安装 ...\n\n官方文档：https://...\n\n请先 WebFetch 抓文档，按步骤执行；不要凭记忆猜命令。",
      "en": "Please install ...\n\nOfficial docs: https://...\n\nFetch the docs first; don't guess commands."
    }
  }
}
```

`cliInstall` connector 不需要登录 token——用户自己 `<binary> auth login`，agent 直接 shell 出去用。安装的"已装/未装"两态由 Rust 端 `binary_on_path()` 探测。参考实现：[packages/connector-feishu-cli/manifest.json](../../../packages/connector-feishu-cli/manifest.json)。

#### 2.3.3 `mcpServer` 形态

```jsonc
{
  "id": "<id>",
  "name":        { "zh": "服务中文名", "en": "Service English Name" },
  "description": {
    "zh": "通过 <Service> 官方 MCP 服务器让 Agent 直接操作 ...。点击安装会跳浏览器走 OAuth 授权。",
    "en": "Operate <Service> via its official MCP server. Clicking install opens the browser for OAuth consent."
  },
  "version": "1.0.0",
  "icon": "icon.svg",
  "auth": {
    "type": "mcpServer",
    "transport": "http",                       // v0 只支持 http (SSE)；stdio 在 schema 里但 bootstrap 暂未接线
    "url": "https://mcp.<service>.com/sse"     // vendor 官方 endpoint
  }
}
```

`mcpServer` 完全不写任何 OAuth 字段——MCP spec 的 dynamic client registration 让 vendor MCP server 自己当 OAuth 服务器，sidecar 的 `mcporter` 通过它注册一个临时 client，开浏览器，loopback 收回调，拿到 token 后写进 Corivo 指定的 `tokenCacheDir`（`$APPDATA/corivo-mcp-tokens/<id>/`）。参考实现：[packages/connector-linear/manifest.json](../../../packages/connector-linear/manifest.json)。

**MCP 安装/断开生命周期**（自动处理，你不用 wire）：
- 用户点「安装」→ Rust 调 `connector_install_mcp` → spawn `corivo-agent --bootstrap-mcp-oauth <input>` 一次性子进程 → mcporter 走 OAuth → 写 token cache → 退出 → Rust 写一份 `ConnectorAccountMeta` 进 Config。
- 用户点「断开」→ Rust 调 `corivo-agent --clear-mcp-oauth <input>` 一次性子进程清掉 mcporter 那边的状态 → 删 `$APPDATA/corivo-mcp-tokens/<id>/` → 移除 Config 里的 account row。

### 2.4 `src/index.ts`

唯一约定：default 不用，导出一个 named `createTools` 函数。框架的 agent loader 通过这个名字调用。

```ts
import type { AgentTool } from "@mariozechner/pi-agent-core";
import type { ConnectorCtx } from "@corivo/shared-types";

import { sendTool } from "./tools/send.js";
// import { listTool } from "./tools/list.js";
// import { ... } from "./tools/...";

export function createTools(ctx: ConnectorCtx): AgentTool<any>[] {
  return [
    sendTool(ctx),
    // listTool(ctx),
  ];
}
```

### 2.5 `src/tools/<action>.ts`

每个工具一个文件。工具是一个**工厂函数** —— 接 `ctx`，返回 `AgentTool`。这样工具实现内部可以闭包捕获 ctx，调 `ctx.fetch` / `ctx.log` 时直接用。

```ts
import type { AgentTool } from "@mariozechner/pi-agent-core";
import type { ConnectorCtx } from "@corivo/shared-types";
import { Type } from "typebox";

// 参数 schema — typebox。会被 pi-agent-core 转成 JSON Schema 暴露给 LLM。
const Parameters = Type.Object({
  to: Type.String({ description: "Recipient address (single)." }),
  body: Type.String({ description: "Plain text body." }),
});

export function sendTool(ctx: ConnectorCtx): AgentTool<typeof Parameters> {
  return {
    name: "<id>_<verb>",            // 例：gmail_send；snake_case；LLM 直接调用
    label: "发送邮件",                // 中文短标签，UI 用
    description:
      "Send an email through the user's connected <Service> account. " +
      "Plain text only; HTML/attachments unsupported.",
    parameters: Parameters,
    execute: async (_toolCallId, params) => {
      // 一律走 ctx.fetch —— host 会自动注入 Authorization: Bearer <token>，
      // 401 时透明 refresh+重试。**不要**自己拼 Authorization 头、
      // 不要假定有 access_token 可拿。
      const response = await ctx.fetch(
        "https://<service>.example.com/v1/<endpoint>",
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ /* ... */ }),
        },
      );

      if (!response.ok) {
        const text = await response.text().catch(() => "(no body)");
        throw new Error(
          `<Service> <verb> failed (HTTP ${response.status}): ${text.slice(0, 500)}`,
        );
      }

      const result = await response.json();
      ctx.log("info", "<id>_<verb>.ok", { /* useful structured fields */ });

      return {
        // pi-agent-core 的 AgentTool 期望 content blocks。文本块给模型读，
        // details 字段给 UI 或 downstream code 看（可选）。
        content: [
          { type: "text", text: `<Service>: <verb> succeeded. ...` },
        ],
        details: result,
      };
    },
  };
}
```

### 2.6 `icon.svg`

任何 SVG 都行，2 KB 内。第一版前端 ConnectorCard 用首字母 fallback，所以这只是给将来准备的 asset slot。不要拿带版权的官方 logo，简单几何图形即可（参见 [packages/connector-gmail/icon.svg](../../../packages/connector-gmail/icon.svg)）。

---

## 三、框架接入清单（按形态分）

新建 package 文件搞好之后，要在框架里"登记"它，agent 才能实际加载到。**登记多少个地方因形态而异**——挑你的形态走对应清单。

### 3.1 `oauth2` 形态（最多 7 步）

| # | 文件 | 改什么 | 不改的后果 |
|---|---|---|---|
| 1 | [`packages/agent/package.json`](../../../packages/agent/package.json) `dependencies` | 加 `"@corivo/connector-<id>": "workspace:*"` | bun 跑 agent 时 `import` 找不到包 |
| 2 | [`packages/agent/src/connector/loader.ts`](../../../packages/agent/src/connector/loader.ts) `BUILTIN` | 加 `<id>: createTools` 一行 + 顶部 `import` | snapshot 进来了但工具不会注册（日志：`connector.loader.unknown_id`）|
| 3 | [`apps/desktop/src-tauri/src/services/connector/catalog.rs`](../src-tauri/src/services/connector/catalog.rs) | `include_str!` 新 manifest + push 到 `parse_builtin()` 的 vec | Settings UI 列表里看不到这个 connector |
| 4 | [`packages/agent/src/system-prompt.ts`](../../../packages/agent/src/system-prompt.ts) `CONNECTOR_HINTS` | 加一行 hint，告诉模型这个 connector 干什么、**不要** fallback 到 bash | 工具注册成功但模型不用它，去 `bash` 找 `which sendmail` 凑合（真踩过的坑） |

如果服务是**新 OAuth provider**（不是已有的 `google`），还要再加：

| # | 文件 | 改什么 |
|---|---|---|
| 5 | [`apps/desktop/src-tauri/src/env.rs`](../src-tauri/src/env.rs) | 加 `CORIVO_<PROVIDER>_CLIENT_ID` + `_SECRET` 的 `option_env!` 包装函数 |
| 6 | [`apps/desktop/src-tauri/src/services/connector/auth.rs`](../src-tauri/src/services/connector/auth.rs) `client_credentials_for` | 加一个 match arm 返回新 provider 的 client 凭据 |
| 7 | [`apps/desktop/src-tauri/src/services/connector/auth.rs`](../src-tauri/src/services/connector/auth.rs) `identity_scopes_for` | 如果新 provider 不是 OIDC（拿不到 `sub`），还要额外想办法拿 `account_id`（可能要在 connect 后调一次 `/me` API）|

### 3.2 `cliInstall` 形态（1 步）

| # | 文件 | 改什么 |
|---|---|---|
| 1 | [`apps/desktop/src-tauri/src/services/connector/catalog.rs`](../src-tauri/src/services/connector/catalog.rs) | `include_str!` 新 manifest + push 到 `parse_builtin()` |

无需改 agent loader / system prompt / OAuth provider。所有工具都来自用户机器上的 CLI；agent 通过 `bash` + 用户写的 skill 调度。

### 3.3 `mcpServer` 形态（1 步）

| # | 文件 | 改什么 |
|---|---|---|
| 1 | [`apps/desktop/src-tauri/src/services/connector/catalog.rs`](../src-tauri/src/services/connector/catalog.rs) | `include_str!` 新 manifest + push 到 `parse_builtin()` |

只有一行 Rust 改动。`bootstrap_install` / `enabled_mcp_specs` / `wipe_token_cache`（[services/connector/mcp.rs](../src-tauri/src/services/connector/mcp.rs)）跟 sidecar 的 `mcp/runtime.ts` + `mcp/bootstrap.ts` 都是 connector-agnostic 的——它们读 manifest 自动认你这个新 connector。**不要碰 `BUILTIN` map**（mcpServer 不走 `createTools`），**不要碰 `client_credentials_for`**（OAuth 由 vendor MCP 自己管）。

### 3.4 完成后跑一遍

```bash
pnpm install                          # 让 pnpm 看到新 workspace 包（oauth2 必跑；其他形态可省）
pnpm --filter @corivo/desktop typegen # 刷新 ts-rs（schema 没变也跑，确认 Rust 编译过）
cd apps/desktop/src-tauri && cargo check
pnpm type-check                       # 工作区全检（包括 agent + connector 包 + desktop 前端）
```

每一步都必须通过。

---

## 四、禁令

| 禁令 | 原因 |
|---|---|
| 在 dependencies 里加 `@corivo/agent` | 产生循环依赖（`agent → connector-gmail → agent`），pnpm 直接拒绝 install |
| 打包**重 native 依赖**（`@google-cloud/*`、`@notionhq/client`、任何带 `*.node` 的包）| 连接器 bundle 体积爆炸；很多 SDK bundle 不进 Bun 的 single-file build |
| 自己拿 access_token 字符串 | host 故意不暴露——通过 `ctx.fetch` 才能做透明 refresh + 401 重试 + 未来 Worker 隔离时的边界 |
| 覆盖 `Authorization` header | `ctx.fetch` 会强制 set 它；你 set 的会被丢弃，行为意外 |
| Import 其他 `packages/connector-*/` | connector 应该**互相不可见**；如果有跨服务需求，把通用 helper 提到 `@corivo/shared-types` 或 agent 内部 |
| Import `@corivo/*` 桌面端代码 | 桌面端代码不会进 bun bundle，会运行时 import 失败 |
| 用 Node 专属 API (`fs`、`process.exit`、`child_process`) | Bun 兼容大部分但不是全部；用前查一下 |
| Bundle 超过 200 KB | 第一版只内置，Phase 2 上 CDN 时这是硬约束。raw HTTP + 自写 helper 几乎一定能压到 100 KB 以下 |

---

## 五、不知道怎么写时 → 翻 Raycast

> 这一节只对 **`oauth2` 形态**有意义。`cliInstall` / `mcpServer` 不写任何 TS 代码，没什么可"翻参考"的——它们的 manifest 模板就是全部，照 §2.3.2 / §2.3.3 抄就完了。

Raycast 的扩展生态是当前最好的参考——同样 TypeScript + 同样 OAuth + 同样"少量代码包装 SaaS API"的形态。**几千个开源扩展**覆盖几乎所有主流 SaaS。

### 5.1 推荐参考顺序

1. **目标服务的官方 API 文档**——OAuth scope / endpoint / 请求体结构看官方，最准
2. **Raycast 对应扩展的 source**（https://github.com/raycast/extensions）——OAuth flow / endpoint URL / 字段约定的"事实标准"
3. **本仓库的 [connector-gmail](../../../packages/connector-gmail/)**——架构 / 风格参照

### 5.2 在 Raycast 仓库里查什么

```bash
# 想加个 Notion connector？看人家怎么调 Notion API：
git clone --depth=1 https://github.com/raycast/extensions /tmp/raycast-extensions
ls /tmp/raycast-extensions/extensions/notion/src/

# OAuth utils 实现可以学：
cat /tmp/raycast-extensions/extensions/notion/src/utils/oauth.ts
# 或者搜 OAuth 配置示例：
grep -r "authorizationEndpoint\|tokenEndpoint" /tmp/raycast-extensions/extensions/<id>/
```

### 5.3 Raycast → Corivo 的对应翻译

| Raycast 概念 | Corivo 等价 |
|---|---|
| `package.json` 的 `commands[]` | 我们没有 commands 概念，对应**只有 `AgentTool`s** |
| `OAuthService` (raycast-utils) | 我们的 `ctx.fetch` + Rust `services/connector/auth.rs` |
| `OAuth.PKCEClient` | 我们的 `services/oauth_loopback`（共用 loopback PKCE 流程）|
| `getAccessToken()` | 不暴露——`ctx.fetch` 自动注入 |
| `useFetch` / SWR-like UI 拉数据 | 不适用——connector 不渲染 UI，只 export tools。UI 在 `apps/desktop/src/pages/settings/sections/integrations-section.tsx` 统一管 |
| Raycast 内置 OAuth providers (GitHub, Linear, Slack...) | 我们 Phase 1 只有 `google`；新 provider 需要在 `client_credentials_for` 加 arm |
| `popToRoot()` / `closeMainWindow()` | 不适用——connector 没有 UI 行为 |
| `@raycast/api` 的 `LocalStorage` | 暂未提供 host storage API；如果真需要，扩展 `ConnectorCtx` 加 `ctx.storage` |

**翻译时的两个常见陷阱**：

1. **Raycast 把 OAuth client_id 写死在扩展代码里**——他们的扩展每个独立 client。**Corivo 不这么干**：我们用统一的 Corivo OAuth client（`CORIVO_GOOGLE_CLIENT_ID` 一类），manifest 里只声明 `provider` 字符串。client_id 在 Rust 端 `client_credentials_for` 注入。
2. **Raycast 扩展常常自己 schedule 拉取 / 缓存**——connector **不要这么干**。我们是 per-turn / 事件触发：agent 调你时你才动。后台 poll / watch 是 Phase 3 才会考虑的事，不在 connector 范围。

---

## 六、写完后的自检

按形态挑对应 checklist 跑一遍，全部 ✓ 再算完成。

### 6.1 所有形态共通

- [ ] `manifest.id` === 目录后缀 === Rust catalog `include_str!` 路径里的目录名（三处一致）
- [ ] `manifest.json` 字段全用 camelCase（`authUrl` 不是 `auth_url`，`installPrompt` 不是 `install_prompt`）
- [ ] [`catalog.rs`](../src-tauri/src/services/connector/catalog.rs) 加了 `include_str!` + push 到 `parse_builtin()`
- [ ] `pnpm install` 干净
- [ ] `cargo check` (从 `apps/desktop/src-tauri/`) 通过
- [ ] `pnpm --filter @corivo/desktop typegen` 通过
- [ ] `pnpm type-check` 工作区全检通过
- [ ] 重启 `pnpm app:dev`，Settings → 集成 看到新卡片

### 6.2 仅 `oauth2`

- [ ] `packages/connector-<id>/` 含 5 个文件 + 至少一个 `src/tools/<name>.ts`
- [ ] `src/index.ts` export 了 named `createTools`（不是 default）
- [ ] 每个工具用 `ctx.fetch`，**没有**手写 `Authorization` header / 拿 token 字符串
- [ ] [`packages/agent/package.json`](../../../packages/agent/package.json) dependencies 里有 `"@corivo/connector-<id>": "workspace:*"`
- [ ] [`packages/agent/src/connector/loader.ts`](../../../packages/agent/src/connector/loader.ts) 的 BUILTIN map 加了对应 row
- [ ] [`packages/agent/src/system-prompt.ts`](../../../packages/agent/src/system-prompt.ts) `CONNECTOR_HINTS` 加了一行 hint，**明确说不要 fallback 到 bash**
- [ ] 新 OAuth provider 的话：`env.rs` + `client_credentials_for` 都改了
- [ ] 走完 OAuth，卡片显示「已连接」+ 账号信息正确
- [ ] 发一条对应业务的 chat 给 agent，日志 grep `connector.loader.registered` 看到 `connector_id` 是新 connector 的 id、`tools_count > 0`
- [ ] 模型实际调了对应工具，不是 fallback 到 bash

### 6.3 仅 `cliInstall`

- [ ] `packages/connector-<id>/` 只含 `manifest.json`（+ 可选 `icon.svg`）；**没有** `package.json` / `src/`
- [ ] `manifest.auth.binary` 在装好 CLI 后 `which <binary>` 能找到
- [ ] `installPrompt` 里包含官方文档链接，要求 agent 先 `WebFetch` 再执行；**禁止** `curl|bash` 这类盲推
- [ ] 点「安装」打开新 `/ask` 线程，prompt 已 prefill
- [ ] 安装完 CLI 之后卡片状态切到「已安装」

### 6.4 仅 `mcpServer`

- [ ] `packages/connector-<id>/` 只含 `manifest.json`（+ 可选 `icon.svg`）；**没有** `package.json` / `src/`
- [ ] `manifest.auth.transport` 是 `"http"`（v0 唯一支持值）
- [ ] `manifest.auth.url` 是 vendor 官方文档里写的 MCP endpoint（不是猜的）
- [ ] **没有** 改 `BUILTIN` map / `CONNECTOR_HINTS` / `client_credentials_for`
- [ ] 点「安装」弹出 vendor 浏览器，OAuth 走完后卡片切到「已连接」
- [ ] `$APPDATA/corivo-mcp-tokens/<id>/` 目录非空（mcporter 落了 token + DCR client info）
- [ ] 新开 `/ask` chat，让 agent 调 `<id>__<some_tool>`，sidecar 日志 grep `mcp.server_registered` 看到 `tools_count > 0`
- [ ] 点「断开」后 `$APPDATA/corivo-mcp-tokens/<id>/` 被清空

---

## 七、Spec 索引（深入参考）

- 框架决策、Phase 拆分、风险项 → [connector-framework-spec.md](./connector-framework-spec.md)
- Bun `--compile` dynamic import 的可行性证明 → [connector-framework-poc/](./connector-framework-poc/)

**Reference 实现**：
- `oauth2` → [packages/connector-gmail/](../../../packages/connector-gmail/)
- `cliInstall` → [packages/connector-feishu-cli/](../../../packages/connector-feishu-cli/)
- `mcpServer` → [packages/connector-linear/](../../../packages/connector-linear/)

**Rust 端**：
- 三 variant manifest schema → [services/connector/manifest.rs](../src-tauri/src/services/connector/manifest.rs)
- 内置 manifest 列表 → [services/connector/catalog.rs](../src-tauri/src/services/connector/catalog.rs)
- OAuth2 流程（`connect` / refresh / Keychain）→ [services/connector/auth.rs](../src-tauri/src/services/connector/auth.rs)
- OAuth loopback 内部 → [services/oauth_loopback/](../src-tauri/src/services/oauth_loopback/)
- MCP bootstrap / token cache / 断开清理 → [services/connector/mcp.rs](../src-tauri/src/services/connector/mcp.rs)

**Sidecar 端**：
- Agent 端 ctx 实现（OAuth2 工具拿到的 `ctx.fetch`）→ [packages/agent/src/connector/ctx.ts](../../../packages/agent/src/connector/ctx.ts)
- OAuth2 connector BUILTIN map + loader → [packages/agent/src/connector/loader.ts](../../../packages/agent/src/connector/loader.ts)
- MCP runtime（vendor MCP server → AgentTool 包装）→ [packages/agent/src/mcp/runtime.ts](../../../packages/agent/src/mcp/runtime.ts)
- MCP install/clear 一次性子命令 → [packages/agent/src/mcp/bootstrap.ts](../../../packages/agent/src/mcp/bootstrap.ts)
