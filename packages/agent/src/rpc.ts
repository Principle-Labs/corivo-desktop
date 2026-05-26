// JSON-RPC client over a Unix domain socket.
//
// Phase A: this client is implemented but only invoked lazily — the first
// time a native AgentTool calls `rustRpc()` we connect to the UDS at
// `input.rpc_socket`. With `CORIVO_AGENT_PHASE_A_MOCK=1`, native tools
// short-circuit before reaching here, so the socket is never opened in
// smoke tests.
//
// Phase B will exercise this against the real Rust `rpc_server.rs` listener.
//
// Wire format (spec §5.4):
//   request:  { id: "rpc-{ulid}", method: string, params: object }
//   response: { id, result?, error?: { code, message } }
//
// Each request gets one-shot stream framing: one JSON object per line.

import { connect, type Socket } from "node:net";
import { randomBytes } from "node:crypto";

interface RpcRequest {
  id: string;
  method: string;
  params: unknown;
}

interface RpcSuccess {
  id: string;
  result: unknown;
}

interface RpcFailure {
  id: string;
  error: { code: string | number; message: string };
}

type RpcResponse = RpcSuccess | RpcFailure;

let sharedClient: RpcClient | null = null;

function makeRpcId(): string {
  return `rpc-${Date.now().toString(36)}-${randomBytes(6).toString("hex")}`;
}

class RpcClient {
  private socket: Socket | null = null;
  private buffer = "";
  private connectPromise: Promise<void> | null = null;
  private readonly pending = new Map<
    string,
    { resolve: (v: unknown) => void; reject: (err: Error) => void }
  >();

  constructor(private readonly socketPath: string) {}

  private async ensureConnected(): Promise<void> {
    if (this.socket) return;
    if (this.connectPromise) return this.connectPromise;

    this.connectPromise = new Promise<void>((resolve, reject) => {
      const sock = connect(this.socketPath);
      const onError = (err: Error) => {
        sock.removeListener("connect", onConnect);
        this.connectPromise = null;
        reject(err);
      };
      const onConnect = () => {
        sock.removeListener("error", onError);
        this.socket = sock;
        sock.setEncoding("utf8");
        sock.on("data", (chunk: string) => this.onData(chunk));
        sock.on("close", () => this.onClose());
        sock.on("error", (err) => this.onSocketError(err));
        resolve();
      };
      sock.once("error", onError);
      sock.once("connect", onConnect);
    });
    return this.connectPromise;
  }

  private onData(chunk: string): void {
    this.buffer += chunk;
    let idx: number;
    while ((idx = this.buffer.indexOf("\n")) >= 0) {
      const line = this.buffer.slice(0, idx).trim();
      this.buffer = this.buffer.slice(idx + 1);
      if (!line) continue;
      let resp: RpcResponse;
      try {
        resp = JSON.parse(line) as RpcResponse;
      } catch (err) {
        // unframed garbage; nothing reasonable to do besides surface to logs
        process.stderr.write(
          `rpc: failed to parse response line: ${String(err)}\n`,
        );
        continue;
      }
      const waiter = this.pending.get(resp.id);
      if (!waiter) continue;
      this.pending.delete(resp.id);
      if ("error" in resp) {
        waiter.reject(
          new Error(`rpc error ${String(resp.error.code)}: ${resp.error.message}`),
        );
      } else {
        waiter.resolve(resp.result);
      }
    }
  }

  private onClose(): void {
    this.socket = null;
    const err = new Error("rpc: socket closed");
    for (const waiter of this.pending.values()) waiter.reject(err);
    this.pending.clear();
  }

  private onSocketError(err: Error): void {
    process.stderr.write(`rpc: socket error: ${err.message}\n`);
  }

  async call(
    method: string,
    params: unknown,
    signal?: AbortSignal,
    timeoutMs?: number,
  ): Promise<unknown> {
    await this.ensureConnected();
    const id = makeRpcId();
    const req: RpcRequest = { id, method, params };
    const line = `${JSON.stringify(req)}\n`;

    return new Promise<unknown>((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout> | null = null;
      let abortListener: (() => void) | null = null;

      const cleanup = () => {
        if (timer !== null) {
          clearTimeout(timer);
          timer = null;
        }
        if (signal && abortListener) {
          signal.removeEventListener("abort", abortListener);
          abortListener = null;
        }
      };

      const settle = {
        resolve: (v: unknown) => {
          cleanup();
          resolve(v);
        },
        reject: (err: Error) => {
          cleanup();
          reject(err);
        },
      };

      this.pending.set(id, settle);

      const onAbort = () => {
        this.pending.delete(id);
        settle.reject(new DOMException("aborted", "AbortError"));
      };
      if (signal) {
        if (signal.aborted) {
          onAbort();
          return;
        }
        abortListener = onAbort;
        signal.addEventListener("abort", onAbort, { once: true });
      }

      // Defense-in-depth: if the Rust side hangs (e.g. an IO deadlock or a
      // crashed handler that never writes a response), we'd otherwise sit
      // here forever waiting for `onData`. Callers can pass a finite
      // `timeoutMs` to bound this; `Infinity` opts out (used by
      // `ask_permission`, which legitimately waits for the user).
      if (
        typeof timeoutMs === "number" &&
        Number.isFinite(timeoutMs) &&
        timeoutMs > 0
      ) {
        timer = setTimeout(() => {
          this.pending.delete(id);
          settle.reject(
            new Error(
              `rpc: timeout after ${timeoutMs}ms (method=${method}, id=${id})`,
            ),
          );
        }, timeoutMs);
      }

      this.socket?.write(line, (err) => {
        if (err) {
          this.pending.delete(id);
          settle.reject(err);
        }
      });
    });
  }
}

export function configureRpc(socketPath: string): void {
  sharedClient = new RpcClient(socketPath);
}

/// Default timeout for native tool RPCs. Bigger than any reasonable
/// in-process handler latency (sqlite scans, FTS lookups), small enough
/// that a real hang gets surfaced as an error within ~90s instead of
/// blocking the agent's main loop until the user manually cancels.
/// `ask_permission` overrides this to `Infinity` because it legitimately
/// blocks on user input.
const DEFAULT_RPC_TIMEOUT_MS = 90_000;

export async function rustRpc(
  method: string,
  params: unknown,
  signal?: AbortSignal,
  timeoutMs: number = DEFAULT_RPC_TIMEOUT_MS,
): Promise<unknown> {
  if (!sharedClient) {
    throw new Error(
      "rpc: client not configured (configureRpc was never called)",
    );
  }
  return sharedClient.call(method, params, signal, timeoutMs);
}
