//
//  AXObserverThread.swift
//  Dedicated CFRunLoop thread that owns the AXObserver for the
//  currently-targeted pid. Port of the Rust `ax_observer.rs`.
//
//  refcon discipline: AXObserverAddNotification takes a void* refcon
//  the C-level callback gets back when it fires. We pass an opaque
//  pointer to the active `AXSubscription` (not retained — the thread's
//  `current` strong ref keeps it alive). On uninstall we call
//  AXObserverRemoveNotification *first* (Apple guarantees no further
//  callbacks after that returns) and only then drop the strong ref.
//

import ApplicationServices
import AppKit
import CoreFoundation
import Foundation

/// Notification kinds we expose to the wire (`ax.subscribe`'s
/// `notifications: ["focused_window", "title"]`).
enum AxNotification: String, CaseIterable {
    case focusedWindow = "focused_window"
    case title

    var axConstant: String {
        switch self {
        case .focusedWindow: return kAXFocusedWindowChangedNotification
        case .title: return kAXTitleChangedNotification
        }
    }

    var wireEventName: String {
        switch self {
        case .focusedWindow: return "ax.focused_window_changed"
        case .title: return "ax.title_changed"
        }
    }
}

final class AXObserverThread: @unchecked Sendable {
    static let shared = AXObserverThread()

    private let cmdLock = NSLock()
    private var pendingCommands: [Command] = []
    private var thread: Thread?
    private var threadRunLoop: CFRunLoop?
    private var shouldStop = false

    private enum Command {
        case setPid(pid_t, [AxNotification])
        case clear(pid_t)
        case stop
    }

    func subscribe(pid: pid_t, notifications: [AxNotification]) {
        startIfNeeded()
        enqueue(.setPid(pid, notifications))
    }

    func unsubscribe(pid: pid_t) {
        enqueue(.clear(pid))
    }

    func shutdown() {
        enqueue(.stop)
    }

    private func enqueue(_ cmd: Command) {
        cmdLock.lock()
        pendingCommands.append(cmd)
        cmdLock.unlock()
        if let rl = threadRunLoop {
            CFRunLoopWakeUp(rl)
        }
    }

    private func startIfNeeded() {
        cmdLock.lock()
        let alreadyStarted = thread != nil
        cmdLock.unlock()
        if alreadyStarted { return }
        let t = Thread { [weak self] in
            self?.runLoop()
        }
        t.name = "corivo.helper.ax-observer"
        t.start()
        // Spin a tiny moment so threadRunLoop is set before the first
        // command lands. In the worst case the first wake misses and
        // the runloop's own pump catches it on the next 0.1s tick.
        cmdLock.lock()
        thread = t
        cmdLock.unlock()
    }

    private func runLoop() {
        threadRunLoop = CFRunLoopGetCurrent()
        Log.info("ax_observer.thread_started")
        var current: AXSubscription? = nil

        while !shouldStop {
            let batch = drainCommands()
            for cmd in batch {
                switch cmd {
                case .setPid(let pid, let notifications):
                    if current?.pid == pid {
                        // Same target — just refresh notifications if
                        // the requested set differs.
                        current?.refreshNotifications(notifications)
                    } else {
                        current?.uninstall()
                        current = AXSubscription.install(
                            pid: pid,
                            notifications: notifications
                        )
                    }
                case .clear(let pid):
                    if current?.pid == pid {
                        current?.uninstall()
                        current = nil
                    }
                case .stop:
                    current?.uninstall()
                    shouldStop = true
                }
            }

            // Pump CFRunLoop: AX callbacks fire here. Short timeout so
            // we revisit the command queue between batches.
            CFRunLoopRunInMode(.defaultMode, 0.1, false)
        }

        Log.info("ax_observer.thread_exit")
    }

    private func drainCommands() -> [Command] {
        cmdLock.lock()
        let batch = pendingCommands
        pendingCommands.removeAll()
        cmdLock.unlock()
        return batch
    }
}

/// One AXObserver registration for a single pid. Keeps the AXObserver,
/// the AXUIElement(application), and the runloop source alive until
/// uninstall.
final class AXSubscription {
    let pid: pid_t
    private let app: AXUIElement
    private let observer: AXObserver
    private var subscribedNotifications: Set<AxNotification> = []

    private init(pid: pid_t, app: AXUIElement, observer: AXObserver) {
        self.pid = pid
        self.app = app
        self.observer = observer
    }

    static func install(
        pid: pid_t,
        notifications: [AxNotification]
    ) -> AXSubscription? {
        let app = AXUIElementCreateApplication(pid)

        var observerOpt: AXObserver?
        let createErr = AXObserverCreate(pid, axCallback, &observerOpt)
        guard createErr == .success, let observer = observerOpt else {
            Log.warn("ax_observer.observer_create_failed pid=\(pid) err=\(createErr.rawValue)")
            return nil
        }

        let subscription = AXSubscription(pid: pid, app: app, observer: observer)
        let opaque = UnsafeMutableRawPointer(
            Unmanaged.passUnretained(subscription).toOpaque())

        for n in notifications {
            let err = AXObserverAddNotification(
                observer, app, n.axConstant as CFString, opaque)
            if err != .success {
                Log.warn(
                    "ax_observer.subscribe_failed pid=\(pid) notification=\(n.rawValue) err=\(err.rawValue)"
                )
            } else {
                subscription.subscribedNotifications.insert(n)
            }
        }

        let source = AXObserverGetRunLoopSource(observer)
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, .defaultMode)

        Log.info("ax_observer.installed pid=\(pid)")
        return subscription
    }

    func refreshNotifications(_ desired: [AxNotification]) {
        let desiredSet = Set(desired)
        // Remove no-longer-wanted ones first.
        for n in subscribedNotifications.subtracting(desiredSet) {
            _ = AXObserverRemoveNotification(observer, app, n.axConstant as CFString)
            subscribedNotifications.remove(n)
        }
        let opaque = UnsafeMutableRawPointer(
            Unmanaged.passUnretained(self).toOpaque())
        // Add the newly-wanted ones.
        for n in desiredSet.subtracting(subscribedNotifications) {
            let err = AXObserverAddNotification(
                observer, app, n.axConstant as CFString, opaque)
            if err == .success {
                subscribedNotifications.insert(n)
            }
        }
    }

    func uninstall() {
        // Apple guarantees no further callbacks fire after
        // AXObserverRemoveNotification returns on the same thread —
        // do that first, so the strong-ref drop at function exit can
        // never race a queued callback.
        for n in subscribedNotifications {
            _ = AXObserverRemoveNotification(observer, app, n.axConstant as CFString)
        }
        subscribedNotifications.removeAll()

        let source = AXObserverGetRunLoopSource(observer)
        CFRunLoopRemoveSource(CFRunLoopGetCurrent(), source, .defaultMode)

        Log.info("ax_observer.uninstalled pid=\(pid)")
    }
}

/// The C-level AX callback. Runs on the AX runloop thread (whichever
/// thread installed the observer). Must be `@convention(c)` — it can't
/// capture environment, so all state comes via `refcon`.
private func axCallback(
    observer: AXObserver,
    element: AXUIElement,
    notification: CFString,
    refcon: UnsafeMutableRawPointer?
) {
    guard let refcon = refcon else { return }
    let subscription = Unmanaged<AXSubscription>.fromOpaque(refcon).takeUnretainedValue()
    let name = notification as String

    let wireEvent: String
    if name == kAXFocusedWindowChangedNotification {
        wireEvent = AxNotification.focusedWindow.wireEventName
    } else if name == kAXTitleChangedNotification {
        wireEvent = AxNotification.title.wireEventName
    } else {
        return
    }

    let pid = subscription.pid
    var payload: [String: Any] = [
        "pid": pid,
    ]
    if let app = NSRunningApplication(processIdentifier: pid) {
        if let bundleID = app.bundleIdentifier, !bundleID.isEmpty {
            payload["bundle_id"] = bundleID
        }
    }
    if let title = windowTitle(for: element, pid: pid), !title.isEmpty {
        payload["window_title"] = title
    }
    Task.detached {
        let evt = EventMessage(
            type: "event",
            name: wireEvent,
            ts: Heartbeat.iso8601(Date()),
            payload: AnyCodable(payload)
        )
        try? await EventEmitter.shared.send(evt)
    }
}

private func windowTitle(for element: AXUIElement, pid: pid_t) -> String? {
    if let title = stringAttribute(element, kAXTitleAttribute as CFString),
       !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        return title
    }

    let app = AXUIElementCreateApplication(pid)
    AXUIElementSetMessagingTimeout(app, 0.2)
    guard let focused = copyAttribute(app, kAXFocusedWindowAttribute as CFString),
          CFGetTypeID(focused as CFTypeRef) == AXUIElementGetTypeID()
    else {
        return nil
    }
    return stringAttribute(focused as! AXUIElement, kAXTitleAttribute as CFString)
}

private func stringAttribute(_ element: AXUIElement, _ attribute: CFString) -> String? {
    copyAttribute(element, attribute) as? String
}

private func copyAttribute(_ element: AXUIElement, _ attribute: CFString) -> AnyObject? {
    var value: AnyObject?
    let err = AXUIElementCopyAttributeValue(element, attribute, &value)
    return err == .success ? value : nil
}
