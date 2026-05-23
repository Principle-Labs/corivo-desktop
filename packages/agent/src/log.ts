// Structured stderr logger for the corivo-agent sidecar.
//
// Wire format (mirrors packages/desktop-helpers/macos/Util/Logger.swift):
//   `[<level>] <msg> <fields_json?>\n`
// Each line is read by the Rust corivo runner's `drain_stderr` and dispatched
// to the matching `tracing` macro (target = "corivo_agent"). That means the
// sidecar's logs interleave with Rust-side logs in the daily log file
// (`~/Library/Application Support/ai.corivo.desktop.dev/logs/<date>.log`)
// and in the `pnpm app:dev` console.
//
// Usage:
//   log.info("sidecar.start", { thread_id, model_id });
//   log.error("sidecar.byok_validation_failed", { code, message });

export type LogLevel = "trace" | "debug" | "info" | "warn" | "error";

function write(level: LogLevel, msg: string, fields?: Record<string, unknown>): void {
  let line = `[${level}] ${msg}`;
  if (fields && Object.keys(fields).length > 0) {
    try {
      line += " " + JSON.stringify(fields);
    } catch {
      line += " " + JSON.stringify({ _serialize_error: true });
    }
  }
  // Synchronous write — we want logs to flush even if the sidecar is about
  // to crash. stderr is line-buffered on macOS for TTY but full-buffered
  // when piped (which is our case under Rust spawn); writing whole lines
  // each call keeps the partial-line risk low.
  try {
    process.stderr.write(line + "\n");
  } catch {
    // If stderr is closed (parent died), there's nowhere to log to. Swallow.
  }
}

export const log = {
  trace: (msg: string, fields?: Record<string, unknown>) => write("trace", msg, fields),
  debug: (msg: string, fields?: Record<string, unknown>) => write("debug", msg, fields),
  info: (msg: string, fields?: Record<string, unknown>) => write("info", msg, fields),
  warn: (msg: string, fields?: Record<string, unknown>) => write("warn", msg, fields),
  error: (msg: string, fields?: Record<string, unknown>) => write("error", msg, fields),
};

/** Mask a credential for log fields. Keeps first/last 4 chars, masks middle. */
export function redact(token: string | null | undefined): string {
  if (!token) return "(empty)";
  if (token.length < 8) return "***";
  return `${token.slice(0, 4)}...${token.slice(-4)} (${token.length}b)`;
}
