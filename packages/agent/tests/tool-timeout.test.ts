// Unit tests for `wrapToolsWithTimeout`. The wrapper's job is to bound every
// tool call's runtime so a hung tool can never freeze a turn — see the source
// header in tool-timeout.ts for the contract.

import { describe, test, expect } from "bun:test";
import { Type } from "typebox";
import type { AgentTool, AgentToolResult } from "@mariozechner/pi-agent-core";
import { wrapToolsWithTimeout } from "../src/tool-timeout.js";

const EmptyParams = Type.Object({});
type EmptyParams = typeof EmptyParams;

function ok(text: string): AgentToolResult<{ text: string }> {
  return { content: [{ type: "text", text }], details: { text } };
}

function makeTool(
  name: string,
  execute: AgentTool<EmptyParams, { text: string }>["execute"],
): AgentTool<EmptyParams, { text: string }> {
  return {
    name,
    label: name,
    description: name,
    parameters: EmptyParams,
    execute,
  };
}

describe("wrapToolsWithTimeout", () => {
  test("fast tool returns its result unchanged", async () => {
    const inner = makeTool("fast", async () => ok("done"));
    const [wrapped] = wrapToolsWithTimeout([inner], 1_000);
    const result = await wrapped.execute("call_1", {});
    expect(result.details.text).toBe("done");
  });

  test("hung tool rejects with a timeout error after the deadline", async () => {
    const inner = makeTool(
      "hung",
      () => new Promise(() => {}), // never resolves
    );
    const [wrapped] = wrapToolsWithTimeout([inner], 50);
    const start = Date.now();
    await expect(wrapped.execute("call_2", {})).rejects.toThrow(
      /timed out after 0s|timed out after 1s/,
    );
    const elapsed = Date.now() - start;
    expect(elapsed).toBeLessThan(500); // well under the 30s default
  });

  test("timeout aborts the composed signal so the inner can clean up", async () => {
    let observedAbort = false;
    const inner = makeTool("aborts", async (_id, _p, signal) => {
      await new Promise<void>((resolve) => {
        if (signal?.aborted) {
          observedAbort = true;
          resolve();
          return;
        }
        signal?.addEventListener(
          "abort",
          () => {
            observedAbort = true;
            resolve();
          },
          { once: true },
        );
      });
      throw new Error("aborted-inner");
    });
    const [wrapped] = wrapToolsWithTimeout([inner], 30);
    await expect(wrapped.execute("call_3", {})).rejects.toThrow(/timed out/);
    expect(observedAbort).toBe(true);
  });

  test("user cancellation propagates the original error (not a timeout error)", async () => {
    const inner = makeTool("user-cancel", async (_id, _p, signal) => {
      await new Promise<void>((resolve) => {
        signal?.addEventListener("abort", () => resolve(), { once: true });
      });
      throw new Error("USER_CANCEL");
    });
    // Plenty of headroom so the timeout never fires.
    const [wrapped] = wrapToolsWithTimeout([inner], 60_000);
    const userController = new AbortController();
    const pending = wrapped.execute("call_4", {}, userController.signal);
    setTimeout(() => userController.abort(), 10);
    await expect(pending).rejects.toThrow(/USER_CANCEL/);
  });

  test("exempt tools (bash, ask_permission) are not wrapped", async () => {
    const bash = makeTool("bash", async () => ok("bash-result"));
    const askPermission = makeTool("ask_permission", async () =>
      ok("permission-result"),
    );
    const [wrappedBash, wrappedAsk] = wrapToolsWithTimeout(
      [bash, askPermission],
      10,
    );
    // Identity check: exempt tools come back unchanged (same `execute` ref).
    expect(wrappedBash.execute).toBe(bash.execute);
    expect(wrappedAsk.execute).toBe(askPermission.execute);
  });

  test("does not wrap exempt even when they hang past the cap", async () => {
    let started = false;
    const bash = makeTool("bash", async (_id, _p, signal) => {
      started = true;
      await new Promise<void>((resolve) => {
        signal?.addEventListener("abort", () => resolve(), { once: true });
      });
      return ok("late");
    });
    const [wrapped] = wrapToolsWithTimeout([bash], 20);
    const ctrl = new AbortController();
    const pending = wrapped.execute("call_5", {}, ctrl.signal);
    // Wait beyond what would be a 20ms timeout if bash WERE wrapped.
    await new Promise((r) => setTimeout(r, 80));
    expect(started).toBe(true);
    ctrl.abort();
    await pending;
  });
});
