# Connector 框架 — Bun `--compile` Dynamic Import PoC

> Verified: 2026-05-11 · Bun 1.3.11 · macOS arm64

[connector-framework-spec.md §5.4 / §14.1](../connector-framework-spec.md) 的技术前提验证。

## 验证目标

证明 `bun build --compile` 产出的独立 binary 能在**运行时** `import()` 一个**编译时不存在的**外部 `.js` 文件，并跟它做双向函数调用——这是 connector 框架"agent sidecar 内 lazy import connector bundle"能成立的前提。

## 5 项验证

| # | 项目 | 状态 |
|---|---|---|
| V1 | binary 能 `await import(absolutePath)` 外部 .js | ✅ |
| V2 | 拿到 default export，调用上面的方法 | ✅ |
| V3 | connector 通过参数接收 host API，能调回 host 函数（双向通信） | ✅ |
| V4 | import 不存在的路径抛错，host `try/catch` 能拦住 | ✅ |
| V5 | import 语法错误的 .js 抛错，host `try/catch` 能拦住 | ✅ |

## 运行

```bash
cd apps/desktop/docs/connector-framework-poc
./run.sh
```

末尾打印 `==> ALL PoC CHECKS PASSED` 即通过。

## 关键发现

1. **Bun `--compile` binary 的 `import()` 是真正的运行时解析**。错误信息里 `from '/$bunfs/root/host-bin'` 显示 binary 内部的虚拟文件系统跟外部 .js 路径是分开的——确认了 connector bundle 不会被打进 binary。

2. **Bun 在 runtime import 时仍会对 .js 做 esbuild 处理**。验证里 V5 抛的错是 `3 errors building "broken.js"` 而不是普通的 `SyntaxError`——意味着 connector bundle 可以是 .ts 源码、未经 minify 的 .js、或者预编译过的 .js，Bun 运行时都接受。spec 里我们仍然约定 connector ship 预 bundle 后的 .js，以求可观测的体积 + 一致的 sourcemap。

3. **冷启动 import 延迟可接受**。0.55 KB 的 connector bundle 在 macOS arm64 实测 import 耗时 < 5ms（包含 esbuild 重处理），符合 spec §5.2 "lazy import 不影响 agent turn 启动开销"的设计前提。

## 限制 / 后续验证

PoC 验证的是**机制本身**。生产化前还需要：

- 较大 bundle（10-50 KB）+ 含 npm 依赖时的 import 延迟
- 多 connector 并发 lazy import 时的内存峰值
- agent 实际嵌入到 Tauri 桌面 app 后，bundle 路径从 `$APPDATA/connectors/...` 读取的端到端流程

这些在 [Phase 1](../connector-framework-spec.md#13-实施分阶段) "Rust 端 `services/connector/` 落地" 阶段一并验证。

## 目录结构

```
connector-framework-poc/
├── README.md            # 本文件
├── run.sh               # 一键验证脚本
├── host/
│   └── main.ts          # 模拟 corivo-agent —— 被 bun --compile 成独立 binary
└── connector/
    ├── send.ts          # 模拟 connector bundle —— bun build 成 .js 后被 host 动态加载
    └── broken.js        # 故意写错的 .js —— V5 验证 host 能 catch
```
