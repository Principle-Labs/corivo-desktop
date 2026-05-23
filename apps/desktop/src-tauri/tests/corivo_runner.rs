//! End-to-end smoke for the bundled `corivo-agent` sidecar binary.
//!
//! Spawns the real `corivo-agent-{arch}-apple-darwin` from
//! `apps/desktop/src-tauri/binaries/`, hands it a §5.1 `SidecarInput`
//! that flips on the Phase A faux pi-ai provider via
//! `CORIVO_AGENT_PHASE_A_MOCK=1`, and asserts the §5.2 wire produces
//! at least one `text_delta` followed by a terminating `agent_end` —
//! i.e. the contract `protocol/corivo.rs` parses against actually
//! holds end-to-end (no real API keys involved).
//!
//! `#[ignore]`d by default because the binary is staged by
//! `prep-sidecar.sh` and a clean checkout may not have it; opt in with
//! `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml \
//!   --test corivo_runner -- --ignored`.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// Find the staged `corivo-agent-*-apple-darwin` binary. The sidecar
/// build script drops three target triples; we prefer whatever matches
/// the host (or fall back to universal). Returns `None` if nothing is
/// staged so the test can `ignore` itself with a clear message rather
/// than failing hard on a fresh checkout.
fn locate_staged_corivo_agent() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let binaries = manifest_dir.join("binaries");
    let host_arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    let candidates = [
        format!("corivo-agent-{host_arch}-apple-darwin"),
        "corivo-agent-universal-apple-darwin".to_string(),
    ];
    for name in candidates {
        let p = binaries.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

#[tokio::test]
#[ignore = "requires staged corivo-agent binary; run with --ignored once \
            `pnpm --filter @corivo/agent build` has produced one"]
async fn faux_provider_emits_text_delta_then_agent_end() {
    let Some(bin) = locate_staged_corivo_agent() else {
        panic!(
            "corivo-agent binary not staged under apps/desktop/src-tauri/binaries/ — \
             run `pnpm --filter @corivo/agent build`"
        );
    };

    let socket_path = std::env::temp_dir().join(format!(
        "corivo-agent-rpc-test-{}-{}.sock",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));

    let input = json!({
        "session_id": "01TEST",
        "user_message": { "role": "user", "content": "hello" },
        "model": {
            "id": "faux-test-model",
            "api_shape": "anthropic",
            "thinking_level": "off",
        },
        "auth": {
            "mode": "byok",
            "base_url": null,
            "token": "faux-token",
        },
        "tools": {
            "native": [],
            "mcp_servers": [],
        },
        "rpc_socket": socket_path.to_string_lossy(),
    });

    let mut child = Command::new(&bin)
        .env("CORIVO_AGENT_PHASE_A_MOCK", "1")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .env_remove("ANTHROPIC_BASE_URL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn corivo-agent");

    let mut stdin = child.stdin.take().expect("stdin pipe");
    let stdout = child.stdout.take().expect("stdout pipe");
    let stderr = child.stderr.take().expect("stderr pipe");

    // Drain stderr in the background so a verbose sidecar can't fill its
    // pipe buffer and deadlock the parent.
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            eprintln!("corivo-agent stderr: {line}");
        }
    });

    let payload = format!("{input}\n");
    stdin
        .write_all(payload.as_bytes())
        .await
        .expect("write stdin");
    stdin.flush().await.expect("flush stdin");
    // Don't drop stdin — spec §5.1 keeps it open for control messages.
    let _stdin_keepalive = stdin;

    let mut lines = BufReader::new(stdout).lines();
    let mut saw_text_delta = false;
    let mut saw_agent_end = false;
    let mut finish_reason: Option<String> = None;

    loop {
        let next = timeout(READ_TIMEOUT, lines.next_line()).await;
        let line_opt = next
            .expect("corivo-agent NDJSON read timed out")
            .expect("read error");
        let Some(line) = line_opt else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let env: Value = serde_json::from_str(trimmed)
            .unwrap_or_else(|_| panic!("invalid NDJSON from sidecar: {trimmed}"));
        match env["type"].as_str() {
            Some("text_delta") => saw_text_delta = true,
            Some("agent_end") => {
                saw_agent_end = true;
                finish_reason = env["data"]["finish_reason"].as_str().map(String::from);
                break;
            }
            Some("error") => panic!("sidecar emitted error event: {env:#}"),
            _ => {}
        }
    }

    let _ = child.wait().await;

    assert!(
        saw_text_delta,
        "expected at least one text_delta event before agent_end"
    );
    assert!(saw_agent_end, "expected an agent_end terminator event");
    assert!(
        matches!(finish_reason.as_deref(), Some("EndTurn") | Some("ToolUse")),
        "unexpected finish_reason: {finish_reason:?}"
    );
}
