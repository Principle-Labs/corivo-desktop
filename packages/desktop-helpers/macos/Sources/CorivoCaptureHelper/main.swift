//
//  main.swift
//  Entry point for the macOS capture helper sidecar.
//
//  Top-level flow (per architecture spec § 6):
//    1. Parent-pid guard      — refuse to run unless launched by Rust main
//    2. Parse args            — `--echo-mode` for CI / protocol smoke
//    3. Emit `hello`          — first message after spawn (must be < 5s)
//    4. Await `hello_ack`     — Rust client confirms protocol selection
//    5. Start heartbeat       — periodic liveness emit
//    6. Dispatch loop         — read NDJSON requests, route through Router
//    7. EOF / shutdown        — exit cleanly
//
//  This file is the orchestration only; per-feature handlers live under
//  Control/ (Phase 0) and Audio/ Screen/ AX/ Foreground/ OCR/ (later).
//

import Foundation

let HELPER_VERSION = "0.1.0"

func boot() async {
    // 1. Parent-pid guard. CI / dev tooling can opt out via env.
    if ProcessInfo.processInfo.environment["CORIVO_HELPER_SKIP_PARENT_PID"] != "1" {
        guard let raw = ProcessInfo.processInfo.environment["CORIVO_HELPER_PARENT_PID"],
              Int32(raw) != nil
        else {
            FileHandle.standardError.write(Data(
                "CorivoCaptureHelper: refusing to start: missing CORIVO_HELPER_PARENT_PID\n".utf8
            ))
            exit(2)
        }
    }

    // 2. Parse args. `--echo-mode` is the protocol-only mode used by CI to
    //    exercise the IPC stack without any platform-native dependencies.
    //    For Phase 0 every code path is already protocol-only, so the flag
    //    is recorded for future phases (which will skip platform setup
    //    when echo mode is on).
    let echoMode = CommandLine.arguments.contains("--echo-mode")
    if echoMode {
        Log.info("starting in --echo-mode (protocol-only)")
    } else {
        Log.info("starting in normal mode")
    }

    // 3. Emit hello.
    let hello = HelloMessage(
        helperVersion: HELPER_VERSION,
        supportedProtocols: ["v1"],
        capabilities: Capabilities.current(),
        platform: PlatformInfo.current()
    )
    do {
        try await EventEmitter.shared.send(hello)
    } catch {
        Log.error("failed to send hello: \(error)")
        exit(3)
    }

    let stdin = LineStream(FileHandle.standardInput)

    // 4. Await hello_ack. We don't enforce a per-helper timeout here: the
    //    Rust client has its own 5s deadline on the handshake and will
    //    SIGKILL us if we take too long, so blocking forever is safe.
    do {
        guard let line = try await stdin.readLine() else {
            Log.error("stdin EOF before hello_ack")
            exit(4)
        }
        switch try InboundMessage.decode(from: line) {
        case .helloAck(let ack):
            Log.info("hello_ack received protocol=\(ack.selectedProtocol) client=\(ack.clientVersion)")
        case .request:
            Log.error("unexpected request before hello_ack")
            exit(5)
        }
    } catch {
        Log.error("hello_ack handshake failed: \(error)")
        exit(6)
    }

    // 5. Start heartbeat. Phase 0 has nothing in-flight to report, so the
    //    payload is the bare timestamp.
    Heartbeat.start()

    // 6. Dispatch loop. Each request gets its own detached task so the
    //    loop never stalls on a slow handler (e.g. a future `ax.query`
    //    that walks a deep Electron tree).
    do {
        while let line = try await stdin.readLine() {
            // Tolerate empty lines (shell or test fixtures sometimes send them).
            if line.isEmpty { continue }
            do {
                let inbound = try InboundMessage.decode(from: line)
                switch inbound {
                case .request(let req):
                    Task.detached(priority: .userInitiated) {
                        await Router.dispatch(req)
                    }
                case .helloAck:
                    Log.warn("unexpected hello_ack post-handshake (ignored)")
                }
            } catch {
                Log.warn("decode failed (line dropped): \(error)")
            }
        }
        Log.info("stdin EOF, exiting cleanly")
        exit(0)
    } catch {
        Log.error("dispatch loop failed: \(error)")
        exit(7)
    }
}

// We run boot() in a Task so the dispatch loop owns its async stack,
// then drive the main RunLoop forever. NSWorkspace observers (Phase 3
// foreground monitor) and AVCaptureSession (Phase 5 mic) both need a
// running main RunLoop to deliver their callbacks. boot() exits the
// process directly on EOF / fatal error so the RunLoop call only matters
// while async work is in flight.
Task.detached(priority: .userInitiated) {
    await boot()
}
RunLoop.main.run()
