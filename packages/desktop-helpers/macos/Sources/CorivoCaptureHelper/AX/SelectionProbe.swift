//
//  SelectionProbe.swift
//  Port of the Rust selection_probe to Swift.
//
//  Reads the user's currently-highlighted text from `pid`'s focused
//  application via the AXTextMarker family of APIs. AXSelectedText alone
//  doesn't survive multi-node selections (Safari paragraph, Twitter
//  thread, multi-line TextEdit); the marker-range pair does.
//

import ApplicationServices
import Foundation

enum SelectionProbe {

    private static let MAX_ANCESTOR_WALK = 16

    private static let AX_SELECTED_TEXT_MARKER_RANGE = "AXSelectedTextMarkerRange"
    private static let AX_STRING_FOR_TEXT_MARKER_RANGE = "AXStringForTextMarkerRange"

    static func probe(pid: pid_t, deadlineSeconds: TimeInterval) throws -> String? {
        guard AXIsProcessTrusted() else {
            // Probe is best-effort: missing permission → return nil instead
            // of erroring, matching the existing Rust behaviour.
            return nil
        }

        let started = Date()
        let app = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(app, Float(deadlineSeconds))

        guard let focused = copyAttribute(app, kAXFocusedUIElementAttribute as CFString),
              CFGetTypeID(focused as CFTypeRef) == AXUIElementGetTypeID()
        else { return nil }

        var current: AXUIElement = focused as! AXUIElement
        var walked = 0

        while true {
            if Date().timeIntervalSince(started) > deadlineSeconds {
                return nil
            }
            if let range = copyAttribute(current, AX_SELECTED_TEXT_MARKER_RANGE as CFString) {
                let str = copyParameterizedString(
                    current,
                    AX_STRING_FOR_TEXT_MARKER_RANGE as CFString,
                    parameter: range
                )
                guard let raw = str else { return nil }
                let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
                if trimmed.isEmpty { return nil }
                return raw
            }
            if walked >= MAX_ANCESTOR_WALK { return nil }
            guard let parentAny = copyAttribute(current, kAXParentAttribute as CFString),
                  CFGetTypeID(parentAny as CFTypeRef) == AXUIElementGetTypeID()
            else { return nil }
            current = parentAny as! AXUIElement
            walked += 1
        }
    }

    private static func copyAttribute(
        _ element: AXUIElement,
        _ attribute: CFString
    ) -> AnyObject? {
        var value: AnyObject?
        let err = AXUIElementCopyAttributeValue(element, attribute, &value)
        return err == .success ? value : nil
    }

    private static func copyParameterizedString(
        _ element: AXUIElement,
        _ attribute: CFString,
        parameter: AnyObject
    ) -> String? {
        var value: AnyObject?
        let err = AXUIElementCopyParameterizedAttributeValue(
            element, attribute, parameter as CFTypeRef, &value
        )
        guard err == .success, let v = value else { return nil }
        return v as? String
    }
}
