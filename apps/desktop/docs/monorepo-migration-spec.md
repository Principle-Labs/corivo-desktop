# Monorepo 改造 Spec

> Status: draft-2 · 2026-04-28
>
> 目标：把当前单仓 `corivo-app` 拆为 `apps/{desktop,web,api}` + `packages/*` 的 pnpm + Turborepo monorepo。本文聚焦**结构搬迁**与**类型自动生成接入**，Hono 内部业务（LLM 代理 / 向量化 / 登录）不在本次实现范围。

---

## 1. 目标 / 非目标

### 目标

- 让 Tauri 桌面端、Next.js Marketing 站、Hono API 端共存于同一个仓库，共享 UI 组件、Rust→TS 自动生成类型与工具配置。
- 引入 **pnpm workspace + Turborepo**，把 `dev` / `build` / `lint` / `test` / `typegen` 收敛成一组 turbo task。
- **桌面端零回归**：迁移完成后 `pnpm tauri dev` / `pnpm tauri build` 行为与今日完全一致。
- 用 **ts-rs** 替换今天 `src/lib/types.ts` 的手抄镜像，让 Rust 端 struct 改动自动同步到前端。
- 为后续 Web 端 / API 端落地做好结构地基（路径别名、tsconfig 继承、shadcn monorepo 模式、共享 Tailwind preset、共享生成类型包）。

### 非目标

- **不实现 Hono 的业务逻辑**（LLM 代理、向量化、认证）——本次仅落 hello-world 骨架，业务在后续 spec 单独立项。
- **不创建 Next.js 应用代码**——`apps/web/` 仅作为占位空目录，等 Marketing 站从其他仓库迁入。
- 不重构 Rust 端模块（`src-tauri/` 整体平移，仅为支持 ts-rs 添加 derive）。
- 不替换现有工具链（保留 Vite 7 / Tailwind 4 / TanStack Router / React Query / Vitest / Knip）。
- 不引入 Nx / Rush / Bazel；本仓体量与异构度都用不上。
- 不解决跨端数据流 / 认证设计——见 §8。

---

## 2. 已确定决策（Q1–Q6 摘要）

| # | 议题 | 决议 |
|---|------|------|
| Q1 | Web 端形态 | **对外 Marketing / Landing site**，需要 SSR；已在其他仓库存在，本次仅留**空目录**等待迁入。 |
| Q2 | Hono API 职责 | 主：**集中式 LLM 代理**（避免每台设备配 API key、统一计量 / 缓存）；次：**跨设备同步服务**；后续可能扩展数据存储、向量化、登录。 |
| Q3 | 跨端数据流 | **暂不设计**。Web 短期不直连 Hono，桌面端仍是离线优先。本次 spec 不做任何数据层假设。 |
| Q4 | 认证 | **暂不设计**。Hono 后续做 LLM 代理时再决定鉴权方案，本次只预留接入点。 |
| Q5 | 共享类型来源 | **ts-rs**：Rust struct 加 `#[derive(TS)]`，由 `cargo test` 触发导出到 `packages/shared-types/src/generated/`。前端从 `@corivo/shared-types` import。 |
| Q6 | 部署目标 | Web → **Vercel**（影响 Next 项目设置）；API → **未定**，runtime 选 Node + `@hono/node-server` 起手以保留 Bun / Workers / Fly.io 切换弹性。 |

---

## 3. 目标结构

```
corivo-app/
├── apps/
│   ├── desktop/              # 现 Tauri 应用全部内容
│   │   ├── index.html
│   │   ├── notification-overlay.html
│   │   ├── quick-ask.html
│   │   ├── src/               # 原仓库 src/ 整体平移
│   │   ├── src-tauri/         # 原仓库 src-tauri/ 整体平移
│   │   ├── public/
│   │   ├── components.json    # 指向 ../../packages/ui
│   │   ├── tsconfig.json      # extends @repo/tsconfig/vite.json
│   │   ├── tailwind.config.ts # extends @repo/tailwind-config
│   │   ├── vite.config.ts
│   │   └── package.json       # name: "@corivo/desktop"
│   ├── web/                   # 空目录占位 — Marketing site 从其他仓库迁入
│   │   └── README.md          # 简述"待迁入，部署目标 Vercel"
│   └── api/                   # Hono — 仅 hello-world
│       ├── src/index.ts       # /health 路由 + export type AppType
│       ├── tsconfig.json      # extends @repo/tsconfig/node.json
│       └── package.json       # name: "@corivo/api"
├── packages/
│   ├── ui/                    # shadcn/ui 组件 + 共享 React 组件
│   │   ├── src/components/
│   │   ├── src/lib/utils.ts   # cn() 等
│   │   ├── src/styles/globals.css
│   │   ├── package.json       # name: "@repo/ui", exports map
│   │   └── tsconfig.json
│   ├── shared-types/          # ts-rs 自动生成的 Rust→TS 类型镜像
│   │   ├── src/generated/     # 由 cargo test 写入，纳入 git
│   │   ├── src/index.ts       # re-export generated/*
│   │   ├── package.json       # name: "@corivo/shared-types"
│   │   └── tsconfig.json
│   ├── tailwind-config/       # 共享 Tailwind 4 preset + theme tokens
│   │   ├── shared.ts
│   │   └── package.json       # name: "@repo/tailwind-config"
│   ├── tsconfig/              # 共享 tsconfig 基座
│   │   ├── base.json
│   │   ├── vite.json          # 桌面端 / Vite app
│   │   ├── nextjs.json        # Next.js app（供 Web 迁入时使用）
│   │   ├── node.json          # Hono / 纯 Node 库
│   │   ├── react-library.json # packages/ui 等组件库
│   │   └── package.json       # name: "@repo/tsconfig"
│   ├── eslint-config/         # 共享 ESLint flat config（如未来引入）
│   │   └── package.json       # name: "@repo/eslint-config"
│   └── api-client/            # 占位：Hono `hc<AppType>()` typed client
│       └── package.json       # name: "@repo/api-client"
├── docs/                      # 保持在仓库根
├── scripts/                   # 保持在仓库根
├── pnpm-workspace.yaml
├── turbo.json
├── package.json               # 根 workspace package
└── tsconfig.json              # 根 solution-style tsconfig（references 各 package）
```

### 命名约定

- **跨产品的内部库**前缀 `@repo/`：`@repo/ui`、`@repo/tsconfig`、`@repo/tailwind-config`、`@repo/api-client`。
- **属于 Corivo 产品**的 app 与生成物前缀 `@corivo/`：`@corivo/desktop`、`@corivo/web`、`@corivo/api`、`@corivo/shared-types`。

> `shared-types` 用 `@corivo/` 而不是 `@repo/`，是因为它的内容来自 Corivo Rust 代码，跟产品强绑定，不是通用基础设施。

---

## 4. 工具链选择

| 关注点 | 选择 | 理由 |
|--------|------|------|
| 包管理 | **pnpm workspace** | 已经在用；硬链接节省磁盘；workspace protocol 直接 `workspace:*`。 |
| 任务编排 | **Turborepo** | 配置心智轻；语言无关；本仓全 TS，`turbo.json` 几十行就够。 |
| 类型共享 | **TS Project References + path mapping** | 跨 app 的类型补全 + 增量编译；不需要构建 packages 即可被 import。 |
| Rust→TS 类型 | **ts-rs** | 替换 `src/lib/types.ts` 的手抄镜像；产出干净 TS 接口；不动 IPC wrapper（保留 `lib/tauri.ts` 的边界层职责）。 |
| UI 共享 | **shadcn monorepo 模式** | 官方原生支持；`packages/ui` 放源码，apps 各自 `components.json` 指过去。 |
| Tailwind | **Tailwind 4 preset 包** | 当前已用 v4 + `@tailwindcss/vite`；preset 放 `@repo/tailwind-config`，每个 app 在自己的 `tailwind.config.ts` 里 `presets: [shared]`。 |
| Hono runtime（起手） | **Node + `@hono/node-server`** | 最大兼容性；Bun / Workers / Fly.io 后续可换，业务代码不变。 |
| Hono 类型链 | **`hc<AppType>()` + `@repo/api-client`** | API 端 `export type AppType = typeof app`，前端 import 即得端到端类型，无需 codegen。 |

---

## 5. 关键风险点 / 需要小心处理的细节

### 5.1 Tauri 配置路径

`apps/desktop/src-tauri/tauri.conf.json` 中的：

- `build.frontendDist` — 当前指向 `../dist`，迁移后 Vite 输出仍然在 `apps/desktop/dist`，相对路径**保持不变**。
- `build.devUrl` — `http://localhost:1420`，无需改。
- `build.beforeDevCommand` / `beforeBuildCommand` — 可保持调用 `pnpm dev` / `pnpm build`，但要确认这些命令在 `apps/desktop/package.json` 中存在。

### 5.2 三个 HTML 入口

`vite.config.ts` 里 `rollupOptions.input` 引用了 `index.html`、`notification-overlay.html`、`quick-ask.html`。迁移后这三个文件随 desktop app 一起平移到 `apps/desktop/`，`vite.config.ts` 中的 `path.resolve(__dirname, ...)` 自动适配新位置，无需改路径，但要确认 Tauri 的 window 创建代码（`src-tauri/src/.../window.rs` 或 lib.rs）引用 HTML 时也是相对的。

### 5.3 Path alias `@/`

当前 `tsconfig.json` 与 `vite.config.ts` 都把 `@/*` 映射到 `./src/*`。迁移后：

- `apps/desktop` 的 `@/*` → `./src/*`（不变）。
- `apps/web`（Marketing）从外部仓库迁入时，其内部别名约定按原仓库继承，不与 desktop 互通。
- 跨 app 共享代码**不要**走 `@/`，统一走 `@repo/ui`、`@repo/api-client`、`@corivo/shared-types` 等显式包名。这是硬约束，避免阴险的双向 import。

### 5.4 Tailwind 4 + monorepo

Tailwind 4 不再依赖 `tailwind.config.ts` 做内容扫描——`@tailwindcss/vite` 插件直接吃 Vite resolve graph，意味着 `packages/ui/src/**` 里的 utility class 默认能被 desktop app 扫到（因为 import 链可达）。但：

- Web 端用 Next，需要走 `@tailwindcss/postcss`（Next 还没正式支持 Vite plugin），content scan 行为略不同——届时 Marketing 站迁入时再确认 `packages/ui` 的 class 能被发现。
- 当前仓库同时存在 `tailwind.config.js` 和 `tailwind.config.ts`，迁移时只保留 `.ts`，删除 `.js`。

### 5.5 shadcn monorepo 模式

shadcn CLI 的 monorepo 模板要求：

- 根 `components.json` 设 `"$schema": "..."` 与 workspace 信息（CLI 用来定位）。
- 每个 app 各自有 `components.json`，`aliases.components` 指向 `@repo/ui/components`，`aliases.ui` 同样。
- 新组件 `npx shadcn@latest add button` 在 app 目录下执行，CLI 会自动写到 `packages/ui/src/components/`。

迁移时需要：把当前 `src/components/ui/` 的内容平移到 `packages/ui/src/components/`，根 `components.json` 改成 monorepo 配置，每个 app 写自己的 `components.json`。

### 5.6 测试与 Knip

- `vitest` 当前在根；迁移后建议**每个有 TS 代码的 package/app 各自一份 `vitest.config.ts`**，根 `turbo.json` 用 `test` task 串起来。`environment: "node"` 的现状可保留为默认。
- `knip.json` 当前在根；Knip 5+ 支持 monorepo（`workspaces` 字段），改造时把每个 app/package 列进去。

### 5.7 Cargo / Rust 端

`src-tauri/Cargo.toml` 是一个独立 Rust crate，平移后位置变成 `apps/desktop/src-tauri/`。注意：

- `src-tauri/tests/time_discipline.rs` 里 `grep` 扫描的根路径如果是写死的相对路径，确认其相对的是 crate 根（即 `src-tauri/`），不是仓库根——否则需要改。
- `db/schema.sql`、`prompts/**.md` 等用 `include_str!` 读取的资源，`include_str!` 是基于源码文件位置解析的，**平移后相对路径不变**，不需要改。
- 不在本次引入 Rust workspace（`Cargo.toml [workspace]`）；只有一个 crate，没必要。

### 5.8 ts-rs 导出路径

ts-rs 的 `#[ts(export_to = "...")]` 是相对于 **crate manifest 目录**（即 `apps/desktop/src-tauri/`）解析的。指向 `packages/shared-types/src/generated/` 需要写：

```rust
#[ts(export_to = "../../../packages/shared-types/src/generated/")]
```

平移前后这个相对路径不同（迁移前是 `../packages/shared-types/...`，迁移后多两级）。**先平移到 `apps/desktop/`，再接入 ts-rs**，避免路径反复改。

### 5.9 ts-rs 与 git

生成的 `.ts` 文件**纳入 git**（不进 `.gitignore`）。理由：

- 让 PR review 能看到 Rust struct 变化对前端的具体影响（diff 直观）。
- 让 CI 可以校验"生成产物已最新"——CI 跑一次 `cargo test`，再 `git diff --exit-code packages/shared-types/src/generated/`，若有 diff 即代表开发者忘记重新生成。
- pnpm install / Vite dev 时不需要 cargo 工具链就能拿到类型。

### 5.10 GitHub Actions / CI

当前未审阅 `.github/workflows/`（如有），改造后所有 CI script 里的 `pnpm <cmd>` 要切到 `pnpm --filter @corivo/desktop <cmd>` 或 `pnpm turbo <task>`。同时新增一个 `typegen-check` job 跑 ts-rs 生成校验。这一步留到 Phase 2 收尾时统一处理。

### 5.11 `latest.json` / Tauri Updater

仓库根有 `latest.json`（updater 元数据）。这个文件不属于源码，留在仓库根或挪到 `apps/desktop/` 都行，但要看 Tauri updater 配置和发布脚本的引用路径——确认后再决定。

---

## 6. 阶段划分

每个 Phase 结束都要满足"`pnpm tauri dev` 能跑且行为不变"。

### Phase 0 — Workspace 基座（不动业务代码）

1. 重写根 `pnpm-workspace.yaml`，加入 `apps/*` 和 `packages/*`。
2. 创建 `packages/{tsconfig,tailwind-config,ui,shared-types,api-client}` 的空骨架（含 `package.json` + `index.ts`）。
3. 加根 `turbo.json`：定义 `build`、`dev`（persistent, no cache）、`lint`、`test`、`type-check`、`typegen` 六个 task。
4. 引入 root `package.json` 的 `devDependencies`：`turbo`、`typescript`（保留）。
5. 确认 `pnpm install` 能跑通，`pnpm turbo run --help` 正常。

**验收**：仓库结构变了，但所有现有命令依然在仓库根工作；`pnpm tauri dev` 仍正常。

### Phase 1 — 抽离共享 packages

1. 把 `src/components/ui/*`（shadcn 组件）平移到 `packages/ui/src/components/`，`src/lib/utils.ts` 平移到 `packages/ui/src/lib/utils.ts`。
2. `packages/ui/package.json` 配置 `exports`：`"./components/*"`、`"./lib/utils"`、`"./styles/globals.css"`。
3. 当前 `tsconfig.json` 的通用部分抽到 `packages/tsconfig/base.json`，桌面端特定部分留在 `packages/tsconfig/vite.json`。
4. 当前 `tailwind.config.ts` 的 theme / plugins 抽到 `packages/tailwind-config/shared.ts`。
5. **此时**仍未移动 desktop：保持 src/ + src-tauri/ 在仓库根，只把 import 路径改成 `@repo/ui/...`，验证桌面端依然能跑。

**验收**：`pnpm tauri dev` 行为不变；`src/components/ui/` 已被删除，import 全走 `@repo/ui`。

### Phase 2 — 平移桌面端到 `apps/desktop/`

1. `git mv` 把 `src/`、`src-tauri/`、`index.html`、`notification-overlay.html`、`quick-ask.html`、`public/`、`vite.config.ts`、`tsconfig.json`、`tailwind.config.ts`、`postcss.config.js`、`components.json`、`knip.json` 全部进 `apps/desktop/`。
2. 新建 `apps/desktop/package.json`（拷贝当前 root package.json 的依赖，name 改 `@corivo/desktop`，scripts 不变）。
3. 根 `package.json` 只保留 workspace 级别的脚本（`turbo run dev` 等）和 root devDependencies。
4. 更新 `apps/desktop/src-tauri/tauri.conf.json` 中 `beforeDevCommand` 为 `pnpm --filter @corivo/desktop dev`（或保持原样，看 turbo dev 是否能接管）。
5. 更新 `.vscode/`、`.idea/` 中固化的相对路径（如有）。
6. 全量回归：`pnpm tauri dev`、`pnpm tauri build`、`pnpm test`、`cargo test`。

**验收**：仓库根除 docs/scripts/.github 外不再有源码；桌面端通过 `pnpm --filter @corivo/desktop tauri dev` 启动一切正常。

### Phase 3 — ts-rs 接入与 `src/lib/types.ts` 替换

> 在 Phase 2 之后做，避免 `export_to` 相对路径反复改。

1. **加依赖**：`apps/desktop/src-tauri/Cargo.toml` 加 `ts-rs = "10"`（或当前最新稳定版）作为常规依赖（不是 dev-dep，因为 derive 写在产品代码里）。
2. **找出跨边界类型**：枚举所有今天前端会消费的 Rust 类型——`domain::config::Config` 系列、`domain::ipc_error::TauriError`、`events::*` payload、`db::repos::*` 的查询结果类型。建议手动在 `src/lib/types.ts` 中扫一遍，对应到 Rust 端 struct。
3. **加 derive**：每个跨边界 struct / enum 加：
   ```rust
   #[derive(TS, Serialize, Deserialize)]
   #[ts(export, export_to = "../../../packages/shared-types/src/generated/")]
   ```
4. **生成 trigger**：ts-rs 默认在 `cargo test` 时把标了 `#[ts(export)]` 的类型写出。
   - 加一个 `cargo test --test typegen` 风格的专用测试文件（即使空），确保 typegen 路径独立可触发。
   - 在 `apps/desktop/package.json` 加 `"typegen": "cargo test --manifest-path src-tauri/Cargo.toml"`。
   - turbo `typegen` task 配 `outputs: ["../../packages/shared-types/src/generated/**"]`。
5. **shared-types 包配置**：
   - `packages/shared-types/src/index.ts` 自动 re-export `./generated/*`（用一个简单脚本或手动维护）。
   - `package.json` 的 `exports` 暴露 `"."` → `./src/index.ts`。
   - `tsconfig.json` extends `@repo/tsconfig/base.json`。
6. **重写前端 `src/lib/types.ts`**：
   - 删掉所有可由 ts-rs 生成的类型定义。
   - 改成 `export type { Foo, Bar } from "@corivo/shared-types"` 的转发文件，或直接让消费者改 import 源。
   - 保留**纯前端的衍生类型**（如 React Query 的 `UseQueryOptions` wrapper）继续在 `lib/types.ts` 中手写。
7. **CI 校验**：加一个 GitHub Actions step：
   ```sh
   pnpm turbo run typegen
   git diff --exit-code packages/shared-types/src/generated/
   ```
   若有 diff 即开发者忘记跑 typegen，CI 失败。

**验收**：
- `pnpm typegen` 后 `packages/shared-types/src/generated/` 被刷新。
- `src/lib/types.ts` 体积明显缩小（手抄类型已删）。
- `pnpm tauri dev` + `pnpm test` 全绿。

### Phase 4 — Hono API 骨架

1. 在 `apps/api/` 落最小 Hono 应用，runtime 用 **Node + `@hono/node-server`**：
   ```ts
   // apps/api/src/index.ts
   import { Hono } from "hono"
   const app = new Hono().get("/health", (c) => c.json({ ok: true }))
   export type AppType = typeof app
   export default app
   ```
2. `package.json` 加 `dev` (`tsx watch src/index.ts`)、`build` (`tsc`)、`start`。
3. `@repo/api-client` 落 `hc<AppType>()` 工厂：
   ```ts
   // packages/api-client/src/index.ts
   import { hc } from "hono/client"
   import type { AppType } from "@corivo/api"
   export const createClient = (baseUrl: string) => hc<AppType>(baseUrl)
   ```
4. 桌面端先不消费 `@repo/api-client`，仅验证 `hc<AppType>()` 类型链通畅（写一个 `.d.ts` 测试）。
5. **不实现** LLM 代理 / 向量化 / 鉴权——这些是后续 spec 的范围。

**验收**：`pnpm --filter @corivo/api dev` 启动 Hono；`@repo/api-client` 能 import 出 typed client；`/health` 返回 200。

### Phase 5 — Web 占位

1. 创建 `apps/web/` 空目录。
2. 加一个 `apps/web/README.md`，说明：
   - 目标：Marketing / Landing site，SSR
   - 部署：Vercel
   - 状态：待从 `<原仓库地址>` 迁入
   - 迁入时建议遵守的本仓约定：tsconfig extends `@repo/tsconfig/nextjs.json`、UI 走 `@repo/ui`、Tailwind preset 用 `@repo/tailwind-config`
3. 在 `pnpm-workspace.yaml` 中**已经包含** `apps/*`，所以无需额外配置——迁入时 Next 项目放进去 `pnpm install` 就生效。

**验收**：目录存在；README 写清迁入指引。

---

## 7. 验收清单（最终态）

- [ ] `pnpm install` 在仓库根一次拉齐所有依赖。
- [ ] `pnpm turbo run build` 串行构建桌面端 + API（Web 留空跳过）。
- [ ] `pnpm --filter @corivo/desktop tauri dev` 与改造前等价。
- [ ] `pnpm --filter @corivo/desktop tauri build` 产物一致（dmg / app size 相近，updater 流程不破）。
- [ ] `cd apps/desktop/src-tauri && cargo test` 全绿。
- [ ] `pnpm typegen` 写出 `packages/shared-types/src/generated/`，再次运行 `git diff --exit-code` 干净。
- [ ] `src/lib/types.ts` 中已不存在与 Rust struct 重复的手抄定义。
- [ ] `pnpm test` 通过 turbo 跑齐所有 vitest。
- [ ] `pnpm --filter @corivo/api dev` 启动 Hono；`/health` 返回 200。
- [ ] `apps/web/README.md` 已写明迁入指引。
- [ ] Knip 无新增 false-positive。
- [ ] CI（如有）已切到 `pnpm turbo` 命令，`typegen-check` job 上线。
- [ ] `docs/`、`scripts/` 留在仓库根，未被搬动。
- [ ] 跨 app 不存在 `@/` 互引；共享走 `@repo/*` 与 `@corivo/shared-types`。

---

## 8. 待定事项（不阻塞本次改造）

以下问题在 Phase 4 完成、即将给 Hono 写真实业务时必须回答；本次 spec 不预设答案：

1. **Hono 数据流定型**：LLM 代理是否需要持久化（请求日志 / 缓存命中表）？向量化的存储选型（pgvector / Qdrant / 内嵌）？
2. **认证方案**：Hono 上的鉴权机制是 API key、JWT 还是接入第三方（Clerk / Auth.js）？桌面端 Keychain 与 API 端 token 的关系？
3. **跨设备同步触发条件**：哪些数据需要同步到 Hono、谁是事实源、冲突解决策略？
4. **Web ↔ API 契约**：当 Marketing 站需要动态内容（如博客、changelog）时是否走 Hono，还是另起 CMS？
5. **Cargo workspace 时机**：若未来 Rust 代码拆出第二个 crate（如同步 worker、CLI），届时再引入 `[workspace]`。
