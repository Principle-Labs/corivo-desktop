//
//  DisplayEnumerator.swift
//  Lists displays known to ScreenCaptureKit. SCDisplay has no
//  human-friendly name, so we synthesize one from the displayID; future
//  phases can swap to CGDisplay localized name lookup if needed.
//

import Foundation
import ScreenCaptureKit
import CoreGraphics

@available(macOS 14.0, *)
enum DisplayEnumerator {

    struct Display {
        var id: String
        var name: String
        var width: Int
        var height: Int
        var isMain: Bool
    }

    static func list() async throws -> [Display] {
        let content: SCShareableContent
        do {
            content = try await SCShareableContent.current
        } catch {
            throw HelperError.osError(
                code: (error as NSError).code,
                detail: "SCShareableContent.current failed: \(error)"
            )
        }
        let main = CGMainDisplayID()
        return content.displays.map { d in
            Display(
                id: String(d.displayID),
                name: "Display \(d.displayID)",
                width: d.width,
                height: d.height,
                isMain: d.displayID == main
            )
        }
    }
}
