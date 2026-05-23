//
//  ForegroundHandlers.swift
//  RPC handlers + NSWorkspace observer for `foreground.*`. Mirrors the
//  pre-helper `foreground_monitor/macos.rs` behaviour: dedup by bundle id,
//  emit one `foreground.app_activated` event per real switch, never on a
//  re-activation of the same app.
//

import Foundation
import AppKit

enum ForegroundHandlers {

    private static let inner = ForegroundMonitor()

    static func subscribe(_ request: RequestMessage) async throws {
        await inner.start()
        try await EventEmitter.shared.send(
            ResponseMessage(id: request.id, result: AnyCodable(["subscribed": true]))
        )
    }

    static func unsubscribe(_ request: RequestMessage) async throws {
        await inner.stop()
        try await EventEmitter.shared.send(
            ResponseMessage(id: request.id, result: AnyCodable(["ack": true]))
        )
    }

    static func current(_ request: RequestMessage) async throws {
        let workspace = NSWorkspace.shared
        let app = workspace.frontmostApplication
        let json: [String: Any] = [
            "pid": app?.processIdentifier as Any,
            "bundle_id": (app?.bundleIdentifier ?? "") as String,
            "app_name": (app?.localizedName ?? "") as String,
            "window_title": "" as String,
        ]
        try await EventEmitter.shared.send(
            ResponseMessage(id: request.id, result: AnyCodable(json))
        )
    }
}

actor ForegroundMonitor {
    private var observerToken: Any?
    private var lastBundle: String?
    private var ourPid: Int32 = 0

    func start() {
        guard observerToken == nil else { return }
        ourPid = Int32(ProcessInfo.processInfo.processIdentifier)
        let center = NSWorkspace.shared.notificationCenter
        let token = center.addObserver(
            forName: NSWorkspace.didActivateApplicationNotification,
            object: nil,
            queue: .main
        ) { _ in
            // Hop back into the actor so observation state stays sync'd.
            Task { await Self.process() }
        }
        observerToken = token
    }

    func stop() {
        if let t = observerToken {
            NSWorkspace.shared.notificationCenter.removeObserver(t)
            observerToken = nil
        }
    }

    /// Static hop because the notification block can't reference `self`
    /// without inviting a Sendable warning under Swift 6.
    private static func process() async {
        await ForegroundHandlers.shared.processActivation()
    }

    fileprivate func processActivation() async {
        let workspace = NSWorkspace.shared
        guard let app = workspace.frontmostApplication else { return }
        let bundleID = app.bundleIdentifier
        if bundleID == lastBundle { return }
        lastBundle = bundleID

        // Skip self-activation (helper / Corivo brought to front).
        if app.processIdentifier == ourPid { return }

        let json: [String: Any] = [
            "pid": app.processIdentifier,
            "bundle_id": bundleID ?? "",
            "app_name": app.localizedName ?? "",
            "window_title": "",
        ]
        let evt = EventMessage(
            type: "event",
            name: "foreground.app_activated",
            ts: Heartbeat.iso8601(Date()),
            payload: AnyCodable(json)
        )
        try? await EventEmitter.shared.send(evt)
    }
}

extension ForegroundHandlers {
    /// The single shared `ForegroundMonitor` exposed for the static
    /// notification-block hop. Lives for the helper's lifetime.
    fileprivate static let shared = inner
}
