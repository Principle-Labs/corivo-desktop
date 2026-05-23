//
//  VisionOCR.swift
//  Apple Vision OCR — port of ocr_extractor.rs.
//
//  Settings (mirror of the Rust impl):
//    - recognitionLevel = .accurate
//    - usesLanguageCorrection = false (avoid "correcting" code)
//    - recognitionLanguages from caller (default zh-Hans / zh-Hant / en-US)
//
//  Synchronous blocking call wrapped in a `Task.detached` by the handler.
//

import Foundation
import Vision
import CoreGraphics
import ImageIO

enum VisionOCR {

    struct Request {
        var imagePath: String
        var languages: [String]
        var useLanguageCorrection: Bool
    }

    struct Response {
        var text: String
        var elapsedMs: UInt64
    }

    static func run(_ request: Request) throws -> Response {
        let url = URL(fileURLWithPath: request.imagePath)
        guard FileManager.default.fileExists(atPath: url.path) else {
            throw HelperError.notFound("image not found: \(request.imagePath)")
        }
        let started = Date()

        guard let imageSource = CGImageSourceCreateWithURL(url as CFURL, nil),
              let cgImage = CGImageSourceCreateImageAtIndex(imageSource, 0, nil)
        else {
            throw HelperError.osError(code: 0,
                                       detail: "failed to decode image at \(request.imagePath)")
        }

        let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
        let textRequest = VNRecognizeTextRequest()
        textRequest.recognitionLevel = .accurate
        textRequest.usesLanguageCorrection = request.useLanguageCorrection
        textRequest.recognitionLanguages = request.languages

        do {
            try handler.perform([textRequest])
        } catch {
            throw HelperError.osError(code: (error as NSError).code,
                                       detail: "Vision performRequests: \(error)")
        }

        let observations = textRequest.results ?? []
        let stitched = stitch(observations: observations)
        let elapsedMs = UInt64(Date().timeIntervalSince(started) * 1000)
        return Response(text: stitched, elapsedMs: elapsedMs)
    }

    /// Reading-order stitch — same algorithm as `output_format::stitch_observations`
    /// in the existing Rust extractor, simplified for Phase 4. Vision returns
    /// observations roughly in y-then-x order; we group within ~1 line height
    /// and join with newlines.
    private static func stitch(observations: [VNRecognizedTextObservation]) -> String {
        struct Item {
            var text: String
            var midY: Double
            var midX: Double
            var height: Double
        }
        var items: [Item] = []
        items.reserveCapacity(observations.count)

        for obs in observations {
            guard let top = obs.topCandidates(1).first else { continue }
            let bbox = obs.boundingBox
            let text = top.string
            if text.isEmpty { continue }
            items.append(Item(
                text: text,
                midY: Double(bbox.origin.y + bbox.size.height / 2.0),
                midX: Double(bbox.origin.x + bbox.size.width / 2.0),
                height: Double(bbox.size.height)
            ))
        }

        // Vision normalizes to bottom-up [0,1]. Sort top-to-bottom (highest
        // midY first), then within a line group sort left-to-right.
        items.sort { (a, b) -> Bool in
            if abs(a.midY - b.midY) > max(a.height, b.height) * 0.6 {
                return a.midY > b.midY
            }
            return a.midX < b.midX
        }

        var out = ""
        var lastMidY: Double? = nil
        var lastHeight: Double? = nil
        for item in items {
            if let prev = lastMidY, let h = lastHeight {
                if abs(item.midY - prev) > h * 0.6 {
                    out.append("\n")
                } else if !out.isEmpty {
                    out.append(" ")
                }
            }
            out.append(item.text)
            lastMidY = item.midY
            lastHeight = item.height
        }
        return out
    }
}
