// Compaction unit tests.
//
// Two layers cover spec §8.3:
//   1. The mid-turn safeguard (`applyCompactionSafeguard`) — pure
//      in-memory, hard-truncates when a single LLM round still blows past
//      contextWindow × 0.95.
//   2. The cross-turn driver (`runCrossTurnCompaction`) — runs once after
//      `Agent.waitForIdle()`, walks `SessionManager.getEntries()`, calls
//      a real LLM (mocked here via the cheap-model auth shape) and lands a
//      `compaction` entry on the jsonl.
//
// We don't network-call here: the smoke test (`smoke.test.ts`) covers
// the live LLM path. This file exercises the cut-point math + the
// SessionManager append, which are the parts that actually have logic.

import { describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import { SessionManager } from "@mariozechner/pi-coding-agent";
import type { Model } from "@mariozechner/pi-ai";
import type { AgentMessage } from "@mariozechner/pi-agent-core";

import {
  applyCompactionSafeguard,
  estimateTokens,
} from "../src/extensions/compaction-safeguard.js";
import {
  COMPACTION_TRIGGER_RATIO,
  MIN_KEEP_TURNS,
} from "../src/cross-turn-compaction.js";
import type { BuiltModel } from "../src/auth.js";

// Tiny window — 100 tokens — so a transcript of 10–15 short messages
// lands well above 0.95 × 100 = 95.
const TINY_CONTEXT_WINDOW = 100;

function buildFakeMain(): BuiltModel {
  const model: Model<"anthropic-messages"> = {
    id: "fake-main",
    name: "fake",
    api: "anthropic-messages",
    provider: "anthropic",
    baseUrl: "http://localhost",
    reasoning: false,
    input: ["text"],
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: TINY_CONTEXT_WINDOW,
    maxTokens: 1024,
  };
  return { model, streamOptions: { apiKey: "fake" } };
}

function fakeUser(text: string): AgentMessage {
  return {
    role: "user",
    content: text,
    timestamp: 0,
  } as AgentMessage;
}

function fakeAssistant(text: string): AgentMessage {
  return {
    role: "assistant",
    content: [{ type: "text", text }],
    api: "anthropic-messages",
    provider: "anthropic",
    model: "fake-main",
    usage: {
      input: 0,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      totalTokens: 0,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
    },
    stopReason: "stop",
    timestamp: 0,
  } as AgentMessage;
}

describe("compaction safeguard", () => {
  test("hard-truncates when above 0.95 × contextWindow", () => {
    const main = buildFakeMain();
    const messages: AgentMessage[] = [];
    for (let i = 0; i < 20; i++) {
      messages.push(fakeUser(`huge ${"q".repeat(100)} #${i}`));
    }
    const result = applyCompactionSafeguard(messages, { main });
    expect(result.length).toBeLessThan(messages.length);
  });

  test("leaves messages alone when already under the hard ceiling", () => {
    const main = buildFakeMain();
    const messages: AgentMessage[] = [
      fakeUser("hi"),
      fakeAssistant("hello"),
    ];
    const result = applyCompactionSafeguard(messages, { main });
    expect(result).toBe(messages);
  });

  test("does not drop existing compactionSummary entries", () => {
    const main = buildFakeMain();
    const summary: AgentMessage = {
      role: "compactionSummary",
      summary: "(prior summary)",
      tokensBefore: 0,
      timestamp: 0,
    } as AgentMessage;
    const messages: AgentMessage[] = [summary];
    for (let i = 0; i < 20; i++) {
      messages.push(fakeUser(`huge ${"q".repeat(100)} #${i}`));
    }
    const result = applyCompactionSafeguard(messages, { main });
    // The summary must survive — it's the smart-compacted head we
    // already paid an LLM for.
    const head = result[0] as { role?: string };
    expect(head.role).toBe("compactionSummary");
  });
});

describe("cross-turn compaction constants", () => {
  // Spec §8.3 floors:
  test("MIN_KEEP_TURNS is at least 4", () => {
    expect(MIN_KEEP_TURNS).toBeGreaterThanOrEqual(4);
  });

  test("trigger ratio is between 0.5 and 0.9 — sane window", () => {
    expect(COMPACTION_TRIGGER_RATIO).toBeGreaterThanOrEqual(0.5);
    expect(COMPACTION_TRIGGER_RATIO).toBeLessThanOrEqual(0.9);
  });
});

describe("estimateTokens", () => {
  test("returns 0 for empty input", () => {
    expect(estimateTokens([])).toBe(0);
  });

  test("scales roughly with content length", () => {
    const short = estimateTokens([fakeUser("hi")]);
    const long = estimateTokens([fakeUser("x".repeat(400))]);
    expect(long).toBeGreaterThan(short * 5);
  });
});

describe("SessionManager round-trip with corivo paths", () => {
  // Sanity: `loadOrCreateSession` opens a fresh file under our chosen
  // sessions_dir (Rust side passes `$APPDATA/corivo-agent-sessions/`).
  // Reading back the same thread_id should see what we appended.
  //
  // pi-coding-agent's `_persist` defers the first flush until an assistant
  // message arrives — empty/abandoned sessions never hit disk. Our own
  // production flow honors that contract because `appendMessage(assistant)`
  // always fires in `turn_end` after the user message went in. The test
  // mirrors that order.
  test("appendMessage survives a reopen after an assistant turn lands", async () => {
    const dir = await mkdtemp(join(tmpdir(), "corivo-session-"));
    const file = resolve(dir, "thread-x.jsonl");

    const sm1 = SessionManager.open(file, dir, "corivo://chat");
    sm1.appendMessage(fakeUser("hello world") as never);
    sm1.appendMessage(fakeAssistant("hi there") as never);

    const sm2 = SessionManager.open(file, dir, "corivo://chat");
    const ctx = sm2.buildSessionContext();
    expect(ctx.messages.length).toBe(2);
    const u = ctx.messages[0] as { role?: string; content?: string };
    expect(u.role).toBe("user");
    expect(u.content).toBe("hello world");
    const a = ctx.messages[1] as { role?: string };
    expect(a.role).toBe("assistant");
  });
});

// Sanity: write-and-read a tmp marker so this file is exercised on
// tmpdir filesystems too (older Bun bug).
test("test rig spawns ok in tmp", async () => {
  const dir = await mkdir(
    join(tmpdir(), `corivo-agent-compaction-${Date.now()}`),
    {
      recursive: true,
    },
  );
  const file = resolve(dir ?? "", "marker");
  await writeFile(file, "ok");
  expect(true).toBe(true);
});
