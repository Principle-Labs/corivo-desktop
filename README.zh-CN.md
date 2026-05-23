# Corivo Desktop 社区版

[English](./README.md) · **中文**

Corivo 桌面客户端的开源发行版 —— 一个基于 Tauri + React 的 macOS 应用，按定时器截屏，通过 AX / OCR / 各应用适配器提取文本，让你在本地搜索、对话、置顶这段历史。

本仓库包含 Corivo 中无需任何托管后端就能运行的部分。闭源 Corivo 构建（Google 登录、托管模型网关、Stripe 充值、托管集成、Sentry 遥测、签名更新策略）**不**包含在内；`apps/desktop/src-tauri/src/services/cloud/` 中基于 trait 的能力层让这些都是可替换的，fork 项目可以接入自己的实现。

---

## 状态

**预发布。** 能力拆分是最近完成的，`@corivo/agent` / `@corivo/mcp` 边车源码现已包含。闭源构建的 SaaS 集成已迁移到由后端中转的连接器网关之后，在 OSS 构建中默认关闭。目前你可以构建并迭代本地桌面体验；托管账号、计费、托管模型目录、托管集成、遥测和签名更新策略均有意缺席。

---

## 目录结构

```
apps/
  desktop/              @corivo/desktop       — Tauri 2 + React 19 macOS 应用
packages/
  ui/                   @repo/ui              — shadcn/ui 原子组件 + cn()
  desktop-helpers/      @corivo/desktop-helpers — 跨平台截屏 helper（macOS 用 Swift、Windows 用 C++）
  agent/                @corivo/agent           — 由 Bun 编译的 chat / Quick Ask 边车
  mcp/                  @corivo/mcp             — Rust MCP 桥接边车
  shared-types/         @corivo/shared-types  — ts-rs 从 Rust 生成的 TS 类型
  tailwind-config/      @repo/tailwind-config — Tailwind v4 预设
  tsconfig/             @repo/tsconfig        — TS 配置
```

有意**不**放进来的内容：

- `apps/api/` 与 `apps/web/` —— Corivo 的托管后端和官网。
- `apps/desktop/src-tauri/src/services/cloud/corivo/` —— 闭源云能力 trait 实现（Google 登录、Stripe 结账、托管模型目录、托管连接器、Sentry 遥测、corivo-policy 更新器）。
- `apps/desktop/src-tauri/prompts/corivo/` —— 为 persona-distill 和 session-memory-learning 后台 agent 调优过的 prompts。本仓库中的 `prompts/*.md` 是简化的开源默认版。
- Corivo 的托管连接器网关与账号池。OSS 构建默认关闭 connectors 能力；fork 可以自建后端网关、OAuth 代理，或走纯本地的 MCP / CLI 集成路线。

---

## 架构（当前可读的部分）

云能力层位于：

- `apps/desktop/src-tauri/src/services/cloud/` —— trait 定义（`AuthService`、`BillingService`、`ModelsService`、`ConnectorsService`、`TelemetryService`、`UpdaterPolicyService`）以及全 noop 默认实现。
- `apps/desktop/src-tauri/src/commands/cloud.rs` —— 前端启动时探测的单一 `get_capabilities` Tauri 命令。
- `apps/desktop/src/hooks/use-capabilities.ts` —— 在会话内缓存结果的 React Query hook。

`apps/desktop/src/` 中每一处与云耦合的 UI 都会读 `useCapabilities()`，要么渲染、要么短路：

- `app/app-boot.tsx` 中的 `/login` 路由跳转
- `app/layout.tsx` 中的 `BillingDialog`
- `components/settings/settings-dialog.tsx` 中的 Settings → Integrations 标签页
- `components/layout/user-card.tsx` 中的侧栏账号下拉与余额 chip

如果 fork 想接入云后端，模式如下：

1. 在 `apps/desktop/src-tauri/src/services/cloud/yourcloud/` 添加同级模块，每个 trait 一个实现。
2. 在 `lib.rs::run()` 中构建完 `CloudServices::noop()` 后、包装进 `Arc::new` 前，将字段替换成你的实现。
3. 每个 impl 的 `is_available()` 返回 `true`，对应的 UI 就会被点亮。

---

## 开发

先决条件：pnpm 11+、Rust stable + Tauri 平台依赖（macOS 需 Xcode CLT 等）。

```sh
pnpm install
pnpm app:dev               # 完整 Tauri 应用（Rust + Vite，端口 1420）
pnpm --filter @corivo/desktop dev:vite   # 仅前端
pnpm --filter @corivo/desktop test       # vitest
```

**注意：** chat / Quick Ask 路径需要 `apps/desktop/src-tauri/binaries/` 下的 `corivo-agent` 和 `corivo-mcp` 边车，以及在 Settings 中配置好的 BYOK 模型 / API key。边车从 `packages/agent/` 和 `packages/mcp/` 构建；编译 agent 需要 Bun。

---

## 协议

Apache 2.0，详见 [LICENSE](./LICENSE)。

发布二进制的 fork 必须修改 `tauri.conf.json#identifier` 与 `productName`，让用户可以与任何官方 Corivo 构建共存（或替代）安装。

---

## 贡献

欢迎 PR。非琐碎改动请先开 issue 描述方向。
