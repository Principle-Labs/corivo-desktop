//! Mock capture helper used by integration tests.
//!
//! Implements the v1 control plane (hello / heartbeat / ping / shutdown)
//! using the same Rust types as the production client. This lets us exercise
//! the entire IPC stack — process spawn, NDJSON framing, handshake, router,
//! health monitor — without depending on a built Swift / C++ helper.
//!
//! Test hooks via env vars (read at startup):
//!
//! | Env                               | Effect                                                        |
//! |-----------------------------------|---------------------------------------------------------------|
//! | `CORIVO_MOCK_FAST_HEARTBEAT=1`    | Emit heartbeat every 50ms instead of 5s                       |
//! | `CORIVO_MOCK_NO_HEARTBEAT=1`      | Don't emit heartbeats at all (for crash-detection tests)      |
//! | `CORIVO_MOCK_REJECT_PING=1`       | Respond to `ping` with an `INTERNAL` error                    |
//! | `CORIVO_MOCK_DELAY_PING_MS=N`     | Delay ping response by N ms (for timeout tests)               |
//! | `CORIVO_MOCK_NO_HELLO=1`          | Skip sending hello (forces handshake timeout)                 |
//! | `CORIVO_MOCK_BAD_FIRST_MSG=1`     | Send heartbeat as the first message instead of hello          |
//! | `CORIVO_MOCK_NO_PROTOCOL_OVERLAP=1` | Advertise unknown protocol "v999" only                      |
//! | `CORIVO_MOCK_SKIP_PARENT_PID=1`   | Bypass the parent-pid env requirement                         |
//! | `CORIVO_MOCK_EXIT_ON_PING=1`      | Exit (code 1) immediately on receiving ping (crash test)      |
//! | `CORIVO_MOCK_ADVERTISE_SCREEN=1`  | Advertise `screen_capture: true` and serve `screen.*` methods |
//! | `CORIVO_MOCK_ADVERTISE_AX=1`      | Advertise `ax_query: true` and serve `ax.*` methods            |
//! | `CORIVO_MOCK_AX_TEXT=...`         | Override the canned `ax.query` response text                   |
//! | `CORIVO_MOCK_AX_SELECTION=...`    | Make `ax.probe_selection` return this string instead of null   |
//! | `CORIVO_MOCK_ADVERTISE_FOREGROUND=1` | Advertise `foreground_monitor: true`, serve foreground.*    |
//! | `CORIVO_MOCK_ADVERTISE_AX_EVENTS=1`  | Advertise `ax_events: true`, serve ax.subscribe/unsubscribe |
//! | `CORIVO_MOCK_ADVERTISE_OCR=1`        | Advertise `ocr_local: true`, serve ocr.run                  |
//! | `CORIVO_MOCK_ADVERTISE_RECORDING=1`  | Advertise `audio_record: true`, serve recording.*           |

use std::time::Duration;

use chrono::Utc;
use corivo_app_lib::services::capture_client::codec;
use corivo_app_lib::services::capture_client::protocol::{
    methods, Arch, Capabilities, ErrorCode, Heartbeat, Hello, Message, Os, PlatformInfo,
    ProtocolVersion, Request, Response, ResponseError,
};
use serde_json::json;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::time::sleep;

#[derive(Clone, Debug)]
struct MockConfig {
    fast_heartbeat: bool,
    no_heartbeat: bool,
    reject_ping: bool,
    delay_ping_ms: u64,
    no_hello: bool,
    bad_first_msg: bool,
    no_protocol_overlap: bool,
    exit_on_ping: bool,
    advertise_screen: bool,
    advertise_ax: bool,
    advertise_foreground: bool,
    advertise_ax_events: bool,
    advertise_ocr: bool,
    advertise_recording: bool,
}

impl MockConfig {
    fn from_env() -> Self {
        fn flag(name: &str) -> bool {
            std::env::var(name).ok().as_deref() == Some("1")
        }
        Self {
            fast_heartbeat: flag("CORIVO_MOCK_FAST_HEARTBEAT"),
            no_heartbeat: flag("CORIVO_MOCK_NO_HEARTBEAT"),
            reject_ping: flag("CORIVO_MOCK_REJECT_PING"),
            delay_ping_ms: std::env::var("CORIVO_MOCK_DELAY_PING_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            no_hello: flag("CORIVO_MOCK_NO_HELLO"),
            bad_first_msg: flag("CORIVO_MOCK_BAD_FIRST_MSG"),
            no_protocol_overlap: flag("CORIVO_MOCK_NO_PROTOCOL_OVERLAP"),
            exit_on_ping: flag("CORIVO_MOCK_EXIT_ON_PING"),
            advertise_screen: flag("CORIVO_MOCK_ADVERTISE_SCREEN"),
            advertise_ax: flag("CORIVO_MOCK_ADVERTISE_AX"),
            advertise_foreground: flag("CORIVO_MOCK_ADVERTISE_FOREGROUND"),
            advertise_ax_events: flag("CORIVO_MOCK_ADVERTISE_AX_EVENTS"),
            advertise_ocr: flag("CORIVO_MOCK_ADVERTISE_OCR"),
            advertise_recording: flag("CORIVO_MOCK_ADVERTISE_RECORDING"),
        }
    }
}

/// Two flavors of outbound message: the typed envelope (used 99% of the
/// time) plus a raw JSON escape hatch so test hooks like
/// `CORIVO_MOCK_NO_PROTOCOL_OVERLAP` can produce protocol values that
/// the typed enum (intentionally) can't represent.
enum Outbound {
    Typed(Message),
    Raw(serde_json::Value),
}

fn main() {
    if std::env::var("CORIVO_MOCK_SKIP_PARENT_PID").ok().as_deref() != Some("1")
        && std::env::var("CORIVO_HELPER_PARENT_PID").is_err()
    {
        eprintln!("capture_helper_mock: refusing to start: missing CORIVO_HELPER_PARENT_PID");
        std::process::exit(2);
    }

    let cfg = MockConfig::from_env();
    eprintln!("capture_helper_mock: starting with cfg={cfg:?}");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    rt.block_on(run(cfg));
}

async fn run(cfg: MockConfig) {
    let (out_tx, out_rx) = mpsc::channel::<Outbound>(64);
    let writer_handle = tokio::spawn(stdout_writer(out_rx));

    send_initial_message(&cfg, &out_tx).await;

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut buf = Vec::with_capacity(4096);

    // If we sent something other than a real hello, the client will close
    // the connection and we should bail. Skip waiting for hello_ack in
    // those modes.
    let expect_ack = !cfg.no_hello && !cfg.bad_first_msg && !cfg.no_protocol_overlap;
    if expect_ack {
        match codec::read_message(&mut reader, &mut buf).await {
            Ok(Message::HelloAck(ack)) => {
                eprintln!(
                    "capture_helper_mock: hello_ack received protocol={:?} client={}",
                    ack.selected_protocol, ack.client_version
                );
            }
            Ok(other) => {
                eprintln!("capture_helper_mock: expected hello_ack, got {other:?}");
                std::process::exit(3);
            }
            Err(e) => {
                eprintln!("capture_helper_mock: hello_ack read failed: {e}");
                std::process::exit(4);
            }
        }
    }

    let hb_handle = if !cfg.no_heartbeat {
        let tx = out_tx.clone();
        let interval = if cfg.fast_heartbeat {
            Duration::from_millis(50)
        } else {
            Duration::from_secs(5)
        };
        Some(tokio::spawn(heartbeat_loop(tx, interval)))
    } else {
        None
    };

    loop {
        match codec::read_message(&mut reader, &mut buf).await {
            Ok(Message::Request(req)) => {
                let cfg = cfg.clone();
                let tx = out_tx.clone();
                tokio::spawn(async move {
                    handle_request(req, &cfg, tx).await;
                });
            }
            Ok(_other) => {
                // mock helper ignores non-request messages from client
            }
            Err(codec::CodecError::Eof) => {
                eprintln!("capture_helper_mock: stdin eof, shutting down");
                break;
            }
            Err(e) => {
                eprintln!("capture_helper_mock: stdin read error: {e}");
                break;
            }
        }
    }

    drop(out_tx);
    if let Some(h) = hb_handle {
        h.abort();
    }
    let _ = writer_handle.await;
}

async fn send_initial_message(cfg: &MockConfig, out: &mpsc::Sender<Outbound>) {
    if cfg.no_hello {
        // Stay silent until the client times out the handshake.
        return;
    }
    if cfg.bad_first_msg {
        let hb = Message::Heartbeat(Heartbeat {
            ts: Utc::now(),
            ..Default::default()
        });
        let _ = out.send(Outbound::Typed(hb)).await;
        return;
    }
    if cfg.no_protocol_overlap {
        let raw = json!({
            "type": "hello",
            "helper_version": env!("CARGO_PKG_VERSION"),
            "supported_protocols": ["v999"],
            "capabilities": {},
            "platform": { "os": "macos", "os_version": "test", "arch": "arm64" }
        });
        let _ = out.send(Outbound::Raw(raw)).await;
        return;
    }

    let hello = Message::Hello(Hello {
        helper_version: env!("CARGO_PKG_VERSION").into(),
        supported_protocols: vec![ProtocolVersion::V1],
        capabilities: Capabilities {
            audio_record: cfg.advertise_recording,
            audio_per_app: false,
            screen_capture: cfg.advertise_screen,
            ax_query: cfg.advertise_ax,
            ax_events: cfg.advertise_ax_events,
            foreground_monitor: cfg.advertise_foreground,
            ocr_local: cfg.advertise_ocr,
            ..Default::default()
        },
        platform: PlatformInfo {
            os: if cfg!(target_os = "windows") {
                Os::Windows
            } else {
                Os::Macos
            },
            os_version: "mock".into(),
            arch: if cfg!(target_arch = "x86_64") {
                Arch::X86_64
            } else {
                Arch::Arm64
            },
        },
    });
    let _ = out.send(Outbound::Typed(hello)).await;
}

async fn handle_request(req: Request, cfg: &MockConfig, out: mpsc::Sender<Outbound>) {
    match req.method.as_str() {
        methods::SCREEN_CAPTURE => {
            handle_screen_capture(req, out).await;
        }
        methods::SCREEN_LIST_DISPLAYS => {
            handle_screen_list_displays(req, out).await;
        }
        methods::AX_QUERY => {
            handle_ax_query(req, out).await;
        }
        methods::AX_PROBE_SELECTION => {
            handle_ax_probe_selection(req, out).await;
        }
        methods::FOREGROUND_SUBSCRIBE => {
            handle_foreground_subscribe(req, out).await;
        }
        methods::FOREGROUND_UNSUBSCRIBE => {
            handle_simple_ack(req, out).await;
        }
        methods::FOREGROUND_CURRENT => {
            handle_foreground_current(req, out).await;
        }
        methods::AX_SUBSCRIBE => {
            handle_ax_subscribe(req, out).await;
        }
        methods::AX_UNSUBSCRIBE => {
            handle_simple_ack(req, out).await;
        }
        methods::OCR_RUN => {
            handle_ocr_run(req, out).await;
        }
        methods::RECORDING_START => {
            handle_recording_start(req, out).await;
        }
        methods::RECORDING_STOP => {
            handle_recording_stop(req, out).await;
        }
        methods::RECORDING_LIST_MICROPHONES => {
            handle_recording_list_microphones(req, out).await;
        }
        methods::PING => {
            if cfg.exit_on_ping {
                eprintln!("capture_helper_mock: exit_on_ping triggered");
                std::process::exit(1);
            }
            if cfg.delay_ping_ms > 0 {
                sleep(Duration::from_millis(cfg.delay_ping_ms)).await;
            }
            let resp = if cfg.reject_ping {
                Response {
                    id: req.id,
                    ok: false,
                    result: None,
                    error: Some(ResponseError {
                        code: ErrorCode::Internal,
                        message: "test hook: ping rejected".into(),
                        detail: serde_json::Value::Null,
                    }),
                }
            } else {
                Response {
                    id: req.id,
                    ok: true,
                    result: Some(json!({ "pong": true })),
                    error: None,
                }
            };
            let _ = out.send(Outbound::Typed(Message::Response(resp))).await;
        }
        methods::SHUTDOWN => {
            let resp = Response {
                id: req.id,
                ok: true,
                result: Some(json!({ "ack": true })),
                error: None,
            };
            let _ = out.send(Outbound::Typed(Message::Response(resp))).await;
            // Give the writer task a moment to flush before we tear down.
            sleep(Duration::from_millis(50)).await;
            std::process::exit(0);
        }
        unknown => {
            let resp = Response {
                id: req.id,
                ok: false,
                result: None,
                error: Some(ResponseError {
                    code: ErrorCode::InvalidRequest,
                    message: format!("unknown method: {unknown}"),
                    detail: serde_json::Value::Null,
                }),
            };
            let _ = out.send(Outbound::Typed(Message::Response(resp))).await;
        }
    }
}

/// Generate a synthetic 16x16 image at the requested path. Width/height
/// are deliberately tiny — tests only validate "file exists, has the
/// declared dimensions" not pixel content.
async fn handle_screen_capture(req: Request, out: mpsc::Sender<Outbound>) {
    use image::{ImageBuffer, Rgb};

    let payload = req.payload.unwrap_or_else(|| json!({}));
    let format_str = payload
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("jpeg");
    let display_id = payload
        .get("display_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let extension = match format_str {
        "png" => "png",
        _ => "jpg",
    };
    let path = payload
        .get("output_path")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!(
                "corivo-mock-{}.{}",
                uuid::Uuid::new_v4(),
                extension
            ))
        });

    let path_for_write = path.clone();
    let format_for_write = format_str.to_string();
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(16, 16, Rgb([100, 150, 200]));
        let dyn_img = image::DynamicImage::ImageRgb8(img);
        let fmt = match format_for_write.as_str() {
            "png" => image::ImageFormat::Png,
            _ => image::ImageFormat::Jpeg,
        };
        dyn_img
            .save_with_format(&path_for_write, fmt)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        Ok(())
    })
    .await;

    let response = match result {
        Ok(Ok(())) => {
            let mut result_value = json!({
                "path": path.to_string_lossy(),
                "width": 16,
                "height": 16,
                "captured_at": chrono::Utc::now(),
            });
            if let Some(id) = display_id {
                result_value["display_id"] = json!(id);
            }
            Response {
                id: req.id,
                ok: true,
                result: Some(result_value),
                error: None,
            }
        }
        Ok(Err(e)) => Response {
            id: req.id,
            ok: false,
            result: None,
            error: Some(ResponseError {
                code: ErrorCode::OsError,
                message: format!("mock screen.capture write failed: {e}"),
                detail: serde_json::Value::Null,
            }),
        },
        Err(join_err) => Response {
            id: req.id,
            ok: false,
            result: None,
            error: Some(ResponseError {
                code: ErrorCode::Internal,
                message: format!("mock screen.capture join failed: {join_err}"),
                detail: serde_json::Value::Null,
            }),
        },
    };

    let _ = out.send(Outbound::Typed(Message::Response(response))).await;
}

async fn handle_ax_query(req: Request, out: mpsc::Sender<Outbound>) {
    let payload = req.payload.unwrap_or_else(|| json!({}));
    let pid = payload.get("pid").and_then(|v| v.as_i64()).unwrap_or(0);
    let max_chars = payload
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(64_000) as usize;

    let canned = std::env::var("CORIVO_MOCK_AX_TEXT").unwrap_or_else(|_| {
        format!("[TITLE] Mock Window (pid {pid})\n[BUTTON] OK\nbody text from mock helper")
    });
    let truncated = canned.len() > max_chars;
    let text = if truncated {
        canned[..max_chars].to_string()
    } else {
        canned
    };

    let resp = Response {
        id: req.id,
        ok: true,
        result: Some(json!({
            "text": text,
            "elapsed_ms": 5_u64,
            "truncated": truncated,
        })),
        error: None,
    };
    let _ = out.send(Outbound::Typed(Message::Response(resp))).await;
}

async fn handle_ax_probe_selection(req: Request, out: mpsc::Sender<Outbound>) {
    let selection = std::env::var("CORIVO_MOCK_AX_SELECTION").ok();
    let resp = Response {
        id: req.id,
        ok: true,
        result: Some(json!({ "selection": selection })),
        error: None,
    };
    let _ = out.send(Outbound::Typed(Message::Response(resp))).await;
}

async fn handle_ocr_run(req: Request, out: mpsc::Sender<Outbound>) {
    let payload = req.payload.unwrap_or_else(|| json!({}));
    let path = payload
        .get("image_path")
        .and_then(|v| v.as_str())
        .unwrap_or("(none)");
    let exists = std::path::Path::new(path).exists();
    let basename = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("(none)");
    let text = if exists {
        format!("MOCK OCR text from {basename}")
    } else {
        format!("MOCK OCR (file not found): {basename}")
    };
    let resp = Response {
        id: req.id,
        ok: true,
        result: Some(json!({
            "text": text,
            "elapsed_ms": 12_u64,
        })),
        error: None,
    };
    let _ = out.send(Outbound::Typed(Message::Response(resp))).await;
}

async fn handle_recording_start(req: Request, out: mpsc::Sender<Outbound>) {
    let payload = req.payload.clone().unwrap_or_else(|| json!({}));
    let session_id = payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("mock-session")
        .to_string();
    let output_dir = payload
        .get("output_dir")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from);
    let cap_sys = payload
        .get("capture_system_audio")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let cap_mic = payload
        .get("capture_microphone")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Ack immediately so the caller proceeds.
    let resp = Response {
        id: req.id,
        ok: true,
        result: Some(json!({
            "session_id": session_id,
            "started_at": chrono::Utc::now(),
        })),
        error: None,
    };
    let _ = out.send(Outbound::Typed(Message::Response(resp))).await;

    // Then asynchronously simulate one segment closing. Real helpers do
    // this from their encoder thread; mock just spawns a task.
    if let Some(dir) = output_dir {
        let session_id = session_id.clone();
        let out = out.clone();
        tokio::spawn(async move {
            let _ = tokio::fs::create_dir_all(&dir).await;
            let path = dir.join("segment-0000.m4a");
            // Touch a placeholder file so callers can assert on its existence.
            let _ = tokio::fs::write(&path, b"mock m4a").await;
            sleep(Duration::from_millis(50)).await;
            let evt = Message::Event(corivo_app_lib::services::capture_client::Event {
                name: "recording.segment_closed".into(),
                ts: chrono::Utc::now(),
                payload: Some(json!({
                    "session_id": session_id,
                    "segment_index": 0,
                    "path": path.to_string_lossy(),
                    "duration_ms": 10_000_u64,
                    "started_at": chrono::Utc::now(),
                    "ended_at": chrono::Utc::now(),
                    "sys_track_present": cap_sys,
                    "mic_track_present": cap_mic,
                })),
            });
            let _ = out.send(Outbound::Typed(evt)).await;
        });
    }
}

async fn handle_recording_stop(req: Request, out: mpsc::Sender<Outbound>) {
    let payload = req.payload.unwrap_or_else(|| json!({}));
    let session_id = payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("mock-session");
    let resp = Response {
        id: req.id,
        ok: true,
        result: Some(json!({
            "session_id": session_id,
            "stopped_at": chrono::Utc::now(),
            "total_segments": 1_u32,
        })),
        error: None,
    };
    let _ = out.send(Outbound::Typed(Message::Response(resp))).await;
}

async fn handle_recording_list_microphones(req: Request, out: mpsc::Sender<Outbound>) {
    let resp = Response {
        id: req.id,
        ok: true,
        result: Some(json!({
            "devices": [
                {"id": "mock-mic-builtin", "name": "Mock Built-in Microphone", "is_default": true},
                {"id": "mock-mic-airpods", "name": "Mock AirPods Pro", "is_default": false},
            ]
        })),
        error: None,
    };
    let _ = out.send(Outbound::Typed(Message::Response(resp))).await;
}

async fn handle_simple_ack(req: Request, out: mpsc::Sender<Outbound>) {
    let resp = Response {
        id: req.id,
        ok: true,
        result: Some(json!({ "ack": true })),
        error: None,
    };
    let _ = out.send(Outbound::Typed(Message::Response(resp))).await;
}

async fn handle_foreground_subscribe(req: Request, out: mpsc::Sender<Outbound>) {
    // Ack immediately, then emit one synthetic foreground.app_activated event
    // so subscribers can verify wiring without needing real OS state.
    let resp = Response {
        id: req.id,
        ok: true,
        result: Some(json!({ "subscribed": true })),
        error: None,
    };
    let _ = out.send(Outbound::Typed(Message::Response(resp))).await;
    let evt = Message::Event(corivo_app_lib::services::capture_client::Event {
        name: "foreground.app_activated".into(),
        ts: chrono::Utc::now(),
        payload: Some(json!({
            "pid": 4321,
            "bundle_id": "com.mock.app",
            "app_name": "Mock App",
            "window_title": "Mock Window",
        })),
    });
    let _ = out.send(Outbound::Typed(evt)).await;
}

async fn handle_foreground_current(req: Request, out: mpsc::Sender<Outbound>) {
    let resp = Response {
        id: req.id,
        ok: true,
        result: Some(json!({
            "pid": 4321,
            "bundle_id": "com.mock.app",
            "app_name": "Mock App",
            "window_title": "Mock Window",
        })),
        error: None,
    };
    let _ = out.send(Outbound::Typed(Message::Response(resp))).await;
}

async fn handle_ax_subscribe(req: Request, out: mpsc::Sender<Outbound>) {
    let payload = req.payload.clone().unwrap_or_else(|| json!({}));
    let pid = payload.get("pid").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let notifications: Vec<String> = payload
        .get("notifications")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    let resp = Response {
        id: req.id,
        ok: true,
        result: Some(json!({ "subscribed": notifications })),
        error: None,
    };
    let _ = out.send(Outbound::Typed(Message::Response(resp))).await;

    // Emit one synthetic ax.focused_window_changed for the subscribed pid.
    let evt = Message::Event(corivo_app_lib::services::capture_client::Event {
        name: "ax.focused_window_changed".into(),
        ts: chrono::Utc::now(),
        payload: Some(json!({
            "pid": pid,
            "bundle_id": "com.mock.app",
            "window_title": "Mock Focused Window",
        })),
    });
    let _ = out.send(Outbound::Typed(evt)).await;
}

async fn handle_screen_list_displays(req: Request, out: mpsc::Sender<Outbound>) {
    let displays = json!({
        "displays": [
            {
                "id": "mock-display-1",
                "name": "Mock Display 1",
                "width": 1920,
                "height": 1080,
                "is_main": true,
            },
            {
                "id": "mock-display-2",
                "name": "Mock Display 2",
                "width": 2560,
                "height": 1440,
                "is_main": false,
            }
        ]
    });
    let resp = Response {
        id: req.id,
        ok: true,
        result: Some(displays),
        error: None,
    };
    let _ = out.send(Outbound::Typed(Message::Response(resp))).await;
}

async fn heartbeat_loop(out: mpsc::Sender<Outbound>, interval: Duration) {
    loop {
        sleep(interval).await;
        let hb = Message::Heartbeat(Heartbeat {
            ts: Utc::now(),
            ..Default::default()
        });
        if out.send(Outbound::Typed(hb)).await.is_err() {
            break;
        }
    }
}

async fn stdout_writer(mut rx: mpsc::Receiver<Outbound>) {
    let mut stdout = tokio::io::stdout();
    while let Some(outbound) = rx.recv().await {
        let result: Result<(), String> = match outbound {
            Outbound::Typed(msg) => codec::write_message(&mut stdout, &msg)
                .await
                .map_err(|e| e.to_string()),
            Outbound::Raw(value) => {
                async {
                    let mut bytes = serde_json::to_vec(&value).expect("raw json serialization");
                    bytes.push(b'\n');
                    stdout.write_all(&bytes).await.map_err(|e| e.to_string())?;
                    stdout.flush().await.map_err(|e| e.to_string())?;
                    Ok(())
                }
                .await
            }
        };
        if let Err(e) = result {
            eprintln!("capture_helper_mock: stdout write failed: {e}");
            break;
        }
    }
}
