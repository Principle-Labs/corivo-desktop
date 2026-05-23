//! Helper child-process lifecycle.
//!
//! Owns the child, the read / write / stderr-forwarder tasks, and the
//! channels into and out of the helper. After [`HelperProcess::spawn`]
//! returns the helper has completed its hello handshake and is ready for
//! RPC.
//!
//! The reader task dispatches incoming messages:
//! - `Heartbeat`  → [`HealthMonitor::mark_alive`]
//! - `Response`   → [`RpcRouter::dispatch`]
//! - `Event`      → [`EventBus::publish`]
//! - `Log`        → forwarded to `tracing` (target `"capture_helper"`)
//! - `Hello`/`HelloAck`/`Request` after handshake → logged as protocol warning
//!
//! Phase 0 keeps lifecycle minimal: no auto-restart, no in-flight retry on
//! crash. `kill_on_drop` on the child + `close_with_crash` on the router
//! makes a process exit fail every pending RPC immediately. Restart policy
//! lands in a later phase along with the production `install()` integration.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::codec::{self, CodecError};
use super::error::{CaptureError, Result};
use super::events::{EventBus, HelperEvent};
use super::health::HealthMonitor;
use super::protocol::{self, Hello, HelloAck, Message, ProtocolVersion};
use super::router::RpcRouter;

/// Spec § 6.1: hello must arrive within 5 seconds of spawn.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Capacity of the writer mpsc. Sized for typical RPC concurrency
/// (a handful of in-flight calls); backpressure here is a real signal of
/// stuck helper, not a tuning concern.
const WRITER_QUEUE_CAPACITY: usize = 64;

#[derive(Debug, Clone)]
pub struct SpawnOptions {
    pub binary_path: PathBuf,
    pub extra_args: Vec<String>,
    pub client_version: String,
    /// Override the parent-pid env passed to the helper. Tests use this so
    /// the mock helper accepts being launched by the cargo-test harness.
    pub parent_pid_override: Option<i32>,
    /// Extra environment variables to pass to the helper. Useful in tests
    /// for injecting mock hooks (`CORIVO_MOCK_*`); production callers
    /// generally leave this empty.
    pub env: Vec<(String, String)>,
}

impl SpawnOptions {
    pub fn new(binary_path: PathBuf, client_version: impl Into<String>) -> Self {
        Self {
            binary_path,
            extra_args: vec![],
            client_version: client_version.into(),
            parent_pid_override: None,
            env: vec![],
        }
    }

    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.extra_args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_parent_pid(mut self, pid: i32) -> Self {
        self.parent_pid_override = Some(pid);
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

pub struct HelperProcess {
    child: Child,
    stdin_tx: mpsc::Sender<Message>,
    /// Detached on Drop — the child dying via `kill_on_drop` causes both
    /// loops to terminate naturally on EOF / BrokenPipe.
    _reader_join: JoinHandle<()>,
    _writer_join: JoinHandle<()>,
    _stderr_join: JoinHandle<()>,
    router: RpcRouter,
    events: EventBus,
    health: HealthMonitor,
}

impl HelperProcess {
    /// Spawn helper, complete the hello handshake, and return the helper's
    /// `Hello` (capabilities + platform + version) alongside the live process.
    pub async fn spawn(opts: SpawnOptions) -> Result<(Self, Hello)> {
        let parent_pid = opts
            .parent_pid_override
            .unwrap_or_else(|| std::process::id() as i32);

        let mut command = Command::new(&opts.binary_path);
        command
            .args(&opts.extra_args)
            .env("CORIVO_HELPER_PARENT_PID", parent_pid.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (key, value) in &opts.env {
            command.env(key, value);
        }

        let mut child = command.spawn().map_err(|e| {
            CaptureError::Transport(format!("spawn helper at {:?}: {}", opts.binary_path, e))
        })?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| CaptureError::Internal("stdin not captured".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CaptureError::Internal("stdout not captured".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CaptureError::Internal("stderr not captured".into()))?;

        let mut reader = BufReader::new(stdout);
        let mut buf = Vec::with_capacity(4096);

        let hello = match timeout(
            HANDSHAKE_TIMEOUT,
            codec::read_message(&mut reader, &mut buf),
        )
        .await
        {
            Ok(Ok(Message::Hello(h))) => h,
            Ok(Ok(other)) => {
                let _ = child.start_kill();
                return Err(CaptureError::Protocol(format!(
                    "expected hello as first message, got: {other:?}"
                )));
            }
            Ok(Err(e)) => {
                let _ = child.start_kill();
                return Err(CaptureError::Transport(format!("hello read: {e}")));
            }
            Err(_) => {
                let _ = child.start_kill();
                return Err(CaptureError::Timeout(HANDSHAKE_TIMEOUT));
            }
        };

        let selected = ProtocolVersion::ALL
            .iter()
            .copied()
            .find(|v| hello.supported_protocols.contains(v))
            .ok_or_else(|| {
                CaptureError::Protocol(format!(
                    "no shared protocol; helper supports {:?}, client supports {:?}",
                    hello.supported_protocols,
                    ProtocolVersion::ALL,
                ))
            })?;

        let ack = Message::HelloAck(HelloAck {
            selected_protocol: selected,
            client_version: opts.client_version.clone(),
        });
        codec::write_message(&mut stdin, &ack)
            .await
            .map_err(|e| CaptureError::Transport(format!("hello_ack write: {e}")))?;

        let router = RpcRouter::new();
        let events = EventBus::new();
        let health = HealthMonitor::with_default_threshold();
        let (stdin_tx, stdin_rx) = mpsc::channel::<Message>(WRITER_QUEUE_CAPACITY);

        let reader_join = tokio::spawn(reader_loop(
            reader,
            buf,
            router.clone(),
            events.clone(),
            health.clone(),
        ));
        let writer_join = tokio::spawn(writer_loop(stdin, stdin_rx));
        let stderr_join = tokio::spawn(stderr_forwarder(stderr));

        Ok((
            HelperProcess {
                child,
                stdin_tx,
                _reader_join: reader_join,
                _writer_join: writer_join,
                _stderr_join: stderr_join,
                router,
                events,
                health,
            },
            hello,
        ))
    }

    pub fn router(&self) -> &RpcRouter {
        &self.router
    }
    pub fn events(&self) -> &EventBus {
        &self.events
    }
    pub fn health(&self) -> &HealthMonitor {
        &self.health
    }

    /// Send a message into the helper's stdin. Returns `HelperUnavailable`
    /// if the writer task has already exited (helper crashed).
    pub async fn send(&self, message: Message) -> Result<()> {
        self.stdin_tx
            .send(message)
            .await
            .map_err(|_| CaptureError::HelperUnavailable)
    }

    /// Wait for the child to exit. Useful in tests to confirm graceful
    /// shutdown completed.
    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }

    /// Forcibly terminate the child. Used by [`super::client::CaptureClient`]
    /// after a graceful-shutdown deadline elapses.
    pub fn force_kill(&mut self) {
        let _ = self.child.start_kill();
    }
}

async fn reader_loop(
    mut reader: BufReader<ChildStdout>,
    mut buf: Vec<u8>,
    router: RpcRouter,
    events: EventBus,
    health: HealthMonitor,
) {
    loop {
        match codec::read_message(&mut reader, &mut buf).await {
            Ok(Message::Heartbeat(h)) => {
                health.mark_alive();
                tracing::trace!(
                    target: "capture_client",
                    in_flight = h.in_flight_requests,
                    "helper.heartbeat"
                );
            }
            Ok(Message::Log(log)) => {
                forward_log(log);
            }
            Ok(Message::Response(resp)) => {
                router.dispatch(resp);
            }
            Ok(Message::Event(evt)) => {
                events.publish(HelperEvent::from_wire(evt));
            }
            Ok(Message::Hello(_)) | Ok(Message::HelloAck(_)) | Ok(Message::Request(_)) => {
                tracing::warn!(
                    target: "capture_client",
                    "helper.unexpected_message_post_handshake"
                );
            }
            Err(CodecError::Eof) => {
                tracing::info!(target: "capture_client", "helper.stdout_eof");
                break;
            }
            Err(e) => {
                tracing::error!(target: "capture_client", error = %e, "helper.read_error");
                break;
            }
        }
    }
    router.close_with_crash();
}

async fn writer_loop(mut stdin: ChildStdin, mut rx: mpsc::Receiver<Message>) {
    while let Some(msg) = rx.recv().await {
        if let Err(e) = codec::write_message(&mut stdin, &msg).await {
            tracing::error!(target: "capture_client", error = %e, "helper.write_error");
            break;
        }
    }
    let _ = stdin.shutdown().await;
}

async fn stderr_forwarder(stderr: ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if !line.is_empty() {
            tracing::info!(target: "capture_helper", message = %line);
        }
    }
}

fn forward_log(log: protocol::LogMessage) {
    use protocol::LogLevel as L;
    let module = log.target.unwrap_or_else(|| "capture_helper".to_string());
    match log.level {
        L::Trace => {
            tracing::trace!(target: "capture_helper", source = %module, message = %log.message)
        }
        L::Debug => {
            tracing::debug!(target: "capture_helper", source = %module, message = %log.message)
        }
        L::Info => {
            tracing::info!(target: "capture_helper", source = %module, message = %log.message)
        }
        L::Warn => {
            tracing::warn!(target: "capture_helper", source = %module, message = %log.message)
        }
        L::Error => {
            tracing::error!(target: "capture_helper", source = %module, message = %log.message)
        }
    }
}
