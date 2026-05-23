// PoC host —— 模拟 `corivo-agent` 的角色：
// 通过 `bun build --compile` 编译成一个独立 binary，
// 在运行时 dynamic import 一个外部 .js bundle（不在 compile 时打进 binary）。
//
// 验证项：
//   1. import 成功，能拿到 default export
//   2. 能调用 default export 上的方法
//   3. connector 能调回 host 提供的 API（双向通信）
//   4. import 不存在的路径 → 能 try/catch
//   5. import 语法错误的 .js → 能 try/catch

import { resolve as resolvePath } from "node:path";

type HostApi = {
  log(level: "info" | "warn" | "error", msg: string): void;
  echo(input: string): Promise<string>;
};

type ConnectorDefinition = {
  manifestId: string;
  tools: Array<{
    name: string;
    execute: (params: unknown, host: HostApi) => Promise<unknown>;
  }>;
};

const host: HostApi = {
  log(level, msg) {
    console.log(`[host.log:${level}] ${msg}`);
  },
  async echo(input) {
    return `echo:${input}`;
  },
};

async function loadConnector(bundlePath: string): Promise<ConnectorDefinition> {
  const absolute = resolvePath(bundlePath);
  const mod = await import(absolute);
  const def = (mod.default ?? mod) as ConnectorDefinition;
  if (!def?.manifestId || !Array.isArray(def.tools)) {
    throw new Error(`bundle at ${absolute} did not export a ConnectorDefinition`);
  }
  return def;
}

async function main() {
  const [, , bundlePath, toolName, paramsJson] = process.argv;
  if (!bundlePath || !toolName) {
    console.error("usage: host-bin <bundle.js> <toolName> [params.json]");
    process.exit(2);
  }

  // Verification 1+2: load + call default export.
  let connector: ConnectorDefinition;
  try {
    connector = await loadConnector(bundlePath);
  } catch (err) {
    console.log(`[host] import failed (caught): ${(err as Error).message}`);
    process.exit(3);
  }

  console.log(`[host] loaded connector id=${connector.manifestId}`);
  console.log(`[host] tools: ${connector.tools.map((t) => t.name).join(",")}`);

  const tool = connector.tools.find((t) => t.name === toolName);
  if (!tool) {
    console.error(`[host] tool '${toolName}' not found in bundle`);
    process.exit(4);
  }

  // Verification 3: connector calls back into host.
  const params = paramsJson ? JSON.parse(paramsJson) : {};
  const result = await tool.execute(params, host);
  console.log(`[host] tool result: ${JSON.stringify(result)}`);
}

main().catch((err) => {
  console.error("[host] uncaught:", err);
  process.exit(1);
});
