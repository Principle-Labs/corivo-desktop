//! End-to-end smoke for the `corivo-mcp` sidecar binary.
//!
//! Spawns the real binary (cargo wires `CARGO_BIN_EXE_corivo-mcp` for us),
//! pretends to be claude on its stdio, and pretends to be the Corivo
//! main process on a Unix socket. Verifies:
//!
//!   1. `initialize` returns the protocol version + server info
//!   2. `tools/list` advertises both `ask_permission` and
//!      `recall_screen_history`
//!   3. `tools/call` round-trips through the bridge — i.e. the sidecar
//!      forwards a JSON request over UDS, our fake bridge answers, and
//!      the response surfaces back as MCP `content`
//!
//! If this passes but the actual app still says
//! "Available MCP tools: none", the bug is upstream of the sidecar
//! (Tauri-side bridge bind failure, wrong socket path in --mcp-config,
//! or claude-side mcp-config parsing).

use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::time::timeout;

const READ_TIMEOUT: Duration = Duration::from_secs(5);

struct Harness {
    child: tokio::process::Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    socket_path: std::path::PathBuf,
}

impl Harness {
    /// Boot fake bridge + sidecar. `bridge_handler` runs once per
    /// accepted connection: it gets the UDS stream and is expected to
    /// read `BridgeRequest` lines and write `BridgeResponse` lines.
    async fn boot<F, Fut>(bridge_handler: F) -> Self
    where
        F: Fn(UnixStream) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let socket_path = std::env::temp_dir().join(format!(
            "corivo-mcp-test-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path).expect("bind UDS");
        let handler = std::sync::Arc::new(bridge_handler);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let h = handler.clone();
                tokio::spawn(async move {
                    h(stream).await;
                });
            }
        });

        // Give the listener a tick to actually bind before sidecar dials.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let sidecar = env!("CARGO_BIN_EXE_corivo-mcp");
        let mut child = Command::new(sidecar)
            .env("CORIVO_MCP_BRIDGE_SOCK", &socket_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn sidecar");

        let stdin = child.stdin.take().expect("sidecar stdin");
        let stdout = BufReader::new(child.stdout.take().expect("sidecar stdout"));

        Self {
            child,
            stdin,
            stdout,
            socket_path,
        }
    }

    async fn send(&mut self, line: &str) {
        self.stdin.write_all(line.as_bytes()).await.expect("write");
        self.stdin.write_all(b"\n").await.expect("newline");
        self.stdin.flush().await.expect("flush");
    }

    async fn next_message(&mut self) -> Value {
        let mut line = String::new();
        let n = timeout(READ_TIMEOUT, self.stdout.read_line(&mut line))
            .await
            .expect("read timed out — sidecar likely crashed; check stderr")
            .expect("read error");
        assert!(n > 0, "sidecar closed stdout unexpectedly");
        serde_json::from_str(line.trim()).expect("invalid JSON from sidecar")
    }

    async fn shutdown(mut self) {
        // Closing stdin lets the sidecar's stdin loop exit cleanly.
        drop(self.stdin);
        let mut stderr = self.child.stderr.take();
        let _ = self.child.wait().await;
        // If the test panics we'll see stderr in the failure message.
        if let Some(mut s) = stderr.take() {
            let mut buf = String::new();
            let _ = s.read_to_string(&mut buf).await;
            if !buf.is_empty() {
                eprintln!("sidecar stderr:\n{buf}");
            }
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

#[tokio::test]
async fn initialize_and_tools_list_succeed_without_bridge_traffic() {
    // Bridge accepts the connection but never replies — initialize and
    // tools/list don't go through the bridge, so this is enough.
    let mut h = Harness::boot(|stream| async move {
        // Hold the connection so sidecar doesn't see EOF.
        let _stream = stream;
        std::future::pending::<()>().await;
    })
    .await;

    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
        .await;
    let init = h.next_message().await;
    assert_eq!(init["jsonrpc"], "2.0", "{init:#}");
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(init["result"]["serverInfo"]["name"], "corivo");
    assert!(init["result"]["capabilities"]["tools"].is_object());

    h.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
        .await;
    let list = h.next_message().await;
    assert_eq!(list["id"], 2);
    let tools = list["result"]["tools"].as_array().expect("tools is array");
    let names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().unwrap_or(""))
        .collect();
    assert!(
        names.contains(&"ask_permission"),
        "tools/list missing ask_permission, got {names:?}"
    );
    assert!(
        names.contains(&"recall_screen_history"),
        "tools/list missing recall_screen_history, got {names:?}"
    );

    h.shutdown().await;
}

#[tokio::test]
async fn tools_call_round_trips_through_bridge() {
    // Bridge replies to every request with a fake "allow" verdict.
    let mut h = Harness::boot(|stream| async move {
        let (read_half, write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half).lines();
        let writer = std::sync::Arc::new(tokio::sync::Mutex::new(write_half));

        while let Ok(Some(line)) = reader.next_line().await {
            let req: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let id = req["id"].as_str().unwrap_or("?").to_string();
            let method = req["method"].as_str().unwrap_or("").to_string();

            let resp = serde_json::json!({
                "id": id,
                "result": {
                    "echoed_method": method,
                    "behavior": "allow",
                }
            });
            let mut payload = serde_json::to_vec(&resp).unwrap();
            payload.push(b'\n');
            let mut w = writer.lock().await;
            let _ = w.write_all(&payload).await;
        }
    })
    .await;

    // Drive the call.
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "ask_permission",
            "arguments": { "tool_name": "Bash", "input": { "command": "ls" } }
        }
    });
    h.send(&req.to_string()).await;

    let resp = h.next_message().await;
    assert_eq!(resp["id"], 7, "{resp:#}");
    assert_eq!(resp["result"]["isError"], false, "{resp:#}");
    let content = resp["result"]["content"]
        .as_array()
        .expect("content array")
        .first()
        .expect("content[0]");
    assert_eq!(content["type"], "text");
    let inner_text = content["text"].as_str().expect("text is string");
    let inner: Value =
        serde_json::from_str(inner_text).expect("inner text should be JSON-encoded bridge result");
    assert_eq!(inner["echoed_method"], "ask_permission");
    assert_eq!(inner["behavior"], "allow");

    h.shutdown().await;
}

#[tokio::test]
async fn sidecar_starts_in_degraded_mode_when_bridge_socket_missing() {
    // Bypass Harness: spawn sidecar pointing at a socket that doesn't
    // exist. It must NOT exit(1) — that's the bug that produced
    // "Available MCP tools: none" in the host. Instead it should boot,
    // advertise tools, and report bridge errors lazily on tools/call.
    let bogus_socket = std::env::temp_dir().join(format!(
        "corivo-mcp-nonexistent-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&bogus_socket);

    let sidecar = env!("CARGO_BIN_EXE_corivo-mcp");
    let mut child = Command::new(sidecar)
        .env("CORIVO_MCP_BRIDGE_SOCK", &bogus_socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn sidecar");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // tools/list still works (no bridge needed).
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
        .await
        .unwrap();
    stdin.flush().await.unwrap();

    let mut line = String::new();
    timeout(READ_TIMEOUT, stdout.read_line(&mut line))
        .await
        .expect("tools/list timed out — sidecar likely exited")
        .unwrap();
    let resp: Value = serde_json::from_str(line.trim()).unwrap();
    let tools = resp["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 2);

    // tools/call must surface a useful isError instead of hanging.
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "ask_permission", "arguments": {"tool_name": "Bash", "input": {}}}
    });
    stdin
        .write_all(format!("{call}\n").as_bytes())
        .await
        .unwrap();
    stdin.flush().await.unwrap();

    let mut line2 = String::new();
    timeout(READ_TIMEOUT, stdout.read_line(&mut line2))
        .await
        .expect("tools/call timed out")
        .unwrap();
    let resp: Value = serde_json::from_str(line2.trim()).unwrap();
    assert_eq!(resp["id"], 2);
    assert_eq!(resp["result"]["isError"], true, "{resp:#}");
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    assert!(
        text.contains("bridge unavailable"),
        "expected human-readable error, got: {text}"
    );

    drop(stdin);
    let _ = child.wait().await;
}

#[tokio::test]
async fn unknown_tool_call_returns_method_not_found() {
    let mut h = Harness::boot(|stream| async move {
        let _ = stream;
        std::future::pending::<()>().await;
    })
    .await;

    h.send(
        r#"{"jsonrpc":"2.0","id":99,"method":"tools/call","params":{"name":"bogus","arguments":{}}}"#,
    )
    .await;
    let resp = h.next_message().await;
    assert_eq!(resp["id"], 99);
    assert_eq!(resp["error"]["code"], -32601, "{resp:#}");

    h.shutdown().await;
}
