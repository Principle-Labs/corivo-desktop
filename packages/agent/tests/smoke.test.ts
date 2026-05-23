// Phase A smoke test: spawn the bun sidecar with CORIVO_AGENT_PHASE_A_MOCK=1,
// feed a SidecarInput on stdin, and assert we see agent_start →
// some text_delta → turn_end → agent_end on stdout.

import { describe, test, expect } from "bun:test";
import { spawn } from "bun";
import { resolve } from "node:path";
import type { SidecarInput } from "../src/types.js";

const MAIN = resolve(import.meta.dir, "..", "src", "main.ts");

function buildInput(overrides: Partial<SidecarInput> = {}): SidecarInput {
  return {
    session_id: "01J000000000000000000000",
    user_message: { role: "user", content: "hello" },
    model: {
      id: "corivo:claude-sonnet-4-6",
      api_shape: "anthropic",
      thinking_level: "off",
    },
    auth: {
      mode: "corivo_proxy",
      base_url: "https://api.corivo.example",
      token: "phase-a-mock-token",
    },
    tools: { native: ["recall_screen_history", "ask_permission"] },
    rpc_socket: "/tmp/corivo-agent-rpc-phase-a-mock.sock",
    ...overrides,
  };
}

async function runSidecar(input: SidecarInput): Promise<{
  stdout: string;
  stderr: string;
  exitCode: number;
}> {
  const proc = spawn({
    cmd: ["bun", "run", MAIN],
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
    env: { ...process.env, CORIVO_AGENT_PHASE_A_MOCK: "1" },
  });

  proc.stdin.write(`${JSON.stringify(input)}\n`);
  await proc.stdin.end();

  const exitCode = await proc.exited;
  const stdout = await new Response(proc.stdout).text();
  const stderr = await new Response(proc.stderr).text();
  return { stdout, stderr, exitCode };
}

function parseEnvelopes(stdout: string): Array<{
  type: string;
  data: any;
  v: number;
  ts: number;
}> {
  return stdout
    .split("\n")
    .filter((l) => l.trim().length > 0)
    .map((l) => JSON.parse(l));
}

describe("@corivo/agent Phase A sidecar", () => {
  test("emits agent_start, text_delta, turn_end, agent_end with faux provider", async () => {
    const { stdout, stderr, exitCode } = await runSidecar(buildInput());
    if (exitCode !== 0) {
      console.error("sidecar stderr:\n", stderr);
      console.error("sidecar stdout:\n", stdout);
    }
    expect(exitCode).toBe(0);

    const events = parseEnvelopes(stdout);
    const types = events.map((e) => e.type);

    expect(types).toContain("agent_start");
    expect(types).toContain("text_delta");
    expect(types).toContain("turn_end");
    expect(types).toContain("agent_end");

    // Order sanity
    const startIdx = types.indexOf("agent_start");
    const endIdx = types.lastIndexOf("agent_end");
    expect(startIdx).toBeLessThan(endIdx);

    // agent_end should report a non-error finish reason for the happy path
    const last = events[events.length - 1];
    expect(last.type).toBe("agent_end");
    expect(["EndTurn", "ToolUse"]).toContain(last.data.finish_reason);
  });

  test("rejects bad input with error envelope and non-zero exit", async () => {
    const proc = spawn({
      cmd: ["bun", "run", MAIN],
      stdin: "pipe",
      stdout: "pipe",
      stderr: "pipe",
      env: { ...process.env, CORIVO_AGENT_PHASE_A_MOCK: "1" },
    });
    proc.stdin.write("this is not json\n");
    await proc.stdin.end();

    const exitCode = await proc.exited;
    const stdout = await new Response(proc.stdout).text();
    expect(exitCode).toBe(1);
    const events = parseEnvelopes(stdout);
    expect(events.some((e) => e.type === "error")).toBe(true);
  });
});
