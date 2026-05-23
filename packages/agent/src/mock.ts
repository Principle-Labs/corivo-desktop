// Phase-A-only flag, gated by `CORIVO_AGENT_PHASE_A_MOCK=1`.
//
// When set:
//   - native tools (recall_screen_history, ask_permission) skip the UDS
//     round trip and return a deterministic stub payload
//   - main.ts will register a faux pi-ai provider so a smoke test can run
//     without real API credentials
//
// Phase B removes this flag — by then the Rust UDS server is live and we
// run real LLM calls in dev.

export function isMockMode(): boolean {
  return process.env["CORIVO_AGENT_PHASE_A_MOCK"] === "1";
}
