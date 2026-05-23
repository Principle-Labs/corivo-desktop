//
//  ScreenCapture.swift
//  Single-frame screen capture via ScreenCaptureKit (`SCScreenshotManager`,
//  macOS 14.0+). Encodes via ImageIO into JPEG / PNG at the requested
//  output path.
//
//  v1 protocol mapping:
//    request  screen.capture { display_id?, output_path?, format, quality? }
//    response { path, width, height, captured_at, display_id }
//

import Foundation
import ImageIO
import ScreenCaptureKit
import CoreGraphics
import UniformTypeIdentifiers

@available(macOS 14.0, *)
enum ScreenCapture {

    enum Format: String {
        case jpeg = "jpeg"
        case png = "png"

        var fileExtension: String { self == .jpeg ? "jpg" : "png" }
        var utiIdentifier: CFString {
            (self == .jpeg ? UTType.jpeg : UTType.png).identifier as CFString
        }
    }

    struct Request {
        var displayId: String?
        var outputPath: String?
        var format: Format
        var quality: Int?
    }

    struct Response {
        var path: String
        var width: Int
        var height: Int
        var capturedAt: String
        var displayId: String
    }

    static func capture(_ request: Request) async throws -> Response {
        let content: SCShareableContent
        do {
            content = try await SCShareableContent.current
        } catch {
            throw HelperError.osError(
                code: (error as NSError).code,
                detail: "SCShareableContent.current failed: \(error)"
            )
        }

        let display = try resolveDisplay(displayId: request.displayId, content: content)

        let filter = SCContentFilter(display: display, excludingWindows: [])
        let config = SCStreamConfiguration()
        config.width = display.width
        config.height = display.height
        // Phase 1 doesn't need cursor capture toggling; SCScreenshotManager
        // honours the system default. SCK 14.4+ exposes captureDynamicRange
        // / showsCursor; we'll plumb those when the product asks.

        let cgImage: CGImage
        do {
            cgImage = try await SCScreenshotManager.captureImage(
                contentFilter: filter, configuration: config)
        } catch {
            throw HelperError.osError(
                code: (error as NSError).code,
                detail: "SCScreenshotManager.captureImage failed: \(error)"
            )
        }

        let url = resolveOutputURL(
            outputPath: request.outputPath,
            format: request.format
        )
        try encode(
            image: cgImage,
            to: url,
            format: request.format,
            quality: request.quality
        )

        return Response(
            path: url.path,
            width: cgImage.width,
            height: cgImage.height,
            capturedAt: Heartbeat.iso8601(Date()),
            displayId: String(display.displayID)
        )
    }

    private static func resolveDisplay(
        displayId: String?,
        content: SCShareableContent
    ) throws -> SCDisplay {
        if let id = displayId {
            if let match = content.displays.first(where: { String($0.displayID) == id }) {
                return match
            }
            throw HelperError.notFound("display id not found: \(id)")
        }
        let main = CGMainDisplayID()
        if let m = content.displays.first(where: { $0.displayID == main }) {
            return m
        }
        if let first = content.displays.first {
            return first
        }
        throw HelperError.notFound("no displays available")
    }

    private static func resolveOutputURL(outputPath: String?, format: Format) -> URL {
        if let path = outputPath {
            return URL(fileURLWithPath: path)
        }
        let temp = NSTemporaryDirectory()
        let name = "corivo-helper-\(UUID().uuidString).\(format.fileExtension)"
        return URL(fileURLWithPath: temp).appendingPathComponent(name)
    }

    private static func encode(
        image: CGImage,
        to url: URL,
        format: Format,
        quality: Int?
    ) throws {
        // Ensure the parent directory exists; misconfigured callers
        // sometimes pass a path under a not-yet-created session dir.
        try? FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )

        guard let dest = CGImageDestinationCreateWithURL(
            url as CFURL, format.utiIdentifier, 1, nil
        ) else {
            throw HelperError.internal("CGImageDestinationCreateWithURL failed for \(url.path)")
        }

        var props: [CFString: Any] = [:]
        if format == .jpeg, let q = quality {
            let clamped = max(0, min(100, q))
            props[kCGImageDestinationLossyCompressionQuality] = Double(clamped) / 100.0
        }
        CGImageDestinationAddImage(dest, image, props as CFDictionary)

        if !CGImageDestinationFinalize(dest) {
            throw HelperError.internal("CGImageDestinationFinalize failed for \(url.path)")
        }
    }
}
