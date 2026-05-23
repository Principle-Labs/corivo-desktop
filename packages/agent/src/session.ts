// SessionManager 集成 (spec §8.1).
//
// pi-coding-agent 自带 jsonl-backed `SessionManager`:append-only entry tree,
// 支持 compaction entries(下次 `buildSessionContext()` 自动合并 summary)。
// 我们用 thread_id 当文件名(spec §8.1),目录由 Rust 通过 `SidecarInput`
// 下传(默认 `$APPDATA/corivo-agent-sessions/`)。
//
// `SessionManager.open(path, sessionsDir, cwdOverride)` 对不存在的 path
// 也安全:内部 `loadEntriesFromFile` 不存在直接返回 `[]`,然后会按 `cwd`
// 起一个新 session 但保留我们传入的显式 path。所以 open 既是「打开」也是
// 「按指定路径创建」。
//
// `cwdOverride` 我们用 `corivo://chat` 这个 sentinel — pi 的 cwd 概念
// 是给 coding agent 自己用(默认 sessionDir 是 `~/.pi/agent/sessions/<encoded-cwd>/`),
// 跟 corivo 的会话语义无关。我们显式传 sessionsDir,cwd 只是写进 header 自描述。

import { SessionManager } from "@mariozechner/pi-coding-agent";
import path from "node:path";

/** Sentinel cwd written into session header — corivo 会话与工作目录无关。 */
export const CORIVO_SESSION_CWD = "corivo://chat";

/**
 * Open or create the jsonl session file for `threadId` under `sessionsDir`.
 *
 * - `sessionsDir` 必须是一个绝对路径;Rust 端在 spawn 前 ensure 该目录存在。
 * - 文件命名固定为 `{threadId}.jsonl`(spec §8.1)。
 * - 如果文件不存在,SessionManager 会用 `CORIVO_SESSION_CWD` 写一个新 session header
 *   并保留我们传入的显式路径。
 */
export function loadOrCreateSession(
  sessionsDir: string,
  threadId: string,
): SessionManager {
  const file = path.join(sessionsDir, `${threadId}.jsonl`);
  return SessionManager.open(file, sessionsDir, CORIVO_SESSION_CWD);
}

/** Mock-mode / test-mode session — no disk persistence. */
export function inMemorySession(): SessionManager {
  return SessionManager.inMemory(CORIVO_SESSION_CWD);
}
