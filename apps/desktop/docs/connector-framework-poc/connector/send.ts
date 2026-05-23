// PoC connector —— 模拟一个 connector bundle。
// 用 `bun build send.ts --outfile bundle.js` 打包，然后被 host 在运行时 import。

type HostApi = {
  log(level: "info" | "warn" | "error", msg: string): void;
  echo(input: string): Promise<string>;
};

export default {
  manifestId: "poc-send",
  tools: [
    {
      name: "send",
      async execute(
        params: { to?: string; body?: string },
        host: HostApi,
      ) {
        host.log("info", `connector received params: ${JSON.stringify(params)}`);
        const reply = await host.echo(params.body ?? "(no body)");
        host.log("info", `connector got reply from host.echo: ${reply}`);
        return {
          ok: true,
          messageId: `msg-${Date.now()}`,
          echoed: reply,
        };
      },
    },
  ],
};
