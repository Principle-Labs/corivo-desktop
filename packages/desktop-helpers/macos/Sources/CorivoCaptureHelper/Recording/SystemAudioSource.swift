//
//  SystemAudioSource.swift
//  System audio capture via ScreenCaptureKit. SCK 13.x doesn't expose a
//  pure audio stream — the workaround per the macOS spec is to subscribe
//  to a 1×1 video at extremely low frame rate (and never hook the video
//  output handler), so the GPU side is essentially free.
//

import Foundation
import ScreenCaptureKit
import CoreMedia

@available(macOS 13.0, *)
final class SystemAudioSource: NSObject, SCStreamDelegate, SCStreamOutput {
    weak var sink: SegmentWriter?
    private let queue = DispatchQueue(label: "corivo.helper.sysaudio", qos: .userInteractive)
    private var stream: SCStream?

    func start() throws {
        let semaphore = DispatchSemaphore(value: 0)
        var startError: Error?
        Task {
            do {
                try await startAsync()
            } catch {
                startError = error
            }
            semaphore.signal()
        }
        semaphore.wait()
        if let error = startError {
            throw error
        }
    }

    private func startAsync() async throws {
        let content: SCShareableContent
        do {
            content = try await SCShareableContent.current
        } catch {
            throw HelperError.osError(code: (error as NSError).code,
                                       detail: "SCShareableContent.current: \(error)")
        }
        guard let display = content.displays.first else {
            throw HelperError.notFound("no displays available for SCK system audio")
        }
        let filter = SCContentFilter(display: display, excludingWindows: [])

        let config = SCStreamConfiguration()
        config.capturesAudio = true
        config.excludesCurrentProcessAudio = true
        config.sampleRate = 48000
        config.channelCount = 2
        // 1×1 video + 1fps + drop video output: minimal GPU cost.
        config.width = 2
        config.height = 2
        config.minimumFrameInterval = CMTime(value: 1, timescale: 1)

        let stream = SCStream(filter: filter, configuration: config, delegate: self)
        try stream.addStreamOutput(self, type: .audio, sampleHandlerQueue: queue)
        do {
            try await stream.startCapture()
        } catch {
            throw HelperError.osError(code: (error as NSError).code,
                                       detail: "SCStream.startCapture: \(error)")
        }
        self.stream = stream
    }

    func stop() async {
        if let stream = stream {
            try? await stream.stopCapture()
        }
        stream = nil
    }

    func stream(_: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
                of type: SCStreamOutputType) {
        guard type == .audio, sampleBuffer.isValid else { return }
        sink?.appendSystemAudio(sampleBuffer)
    }
}
