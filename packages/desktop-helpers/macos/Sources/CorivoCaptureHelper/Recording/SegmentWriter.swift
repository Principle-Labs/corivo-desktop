//
//  SegmentWriter.swift
//  AVAssetWriter wrapper. v1 writes a single multi-track .m4a per session;
//  v2 will rotate every `segment_seconds`. Multi-track means sys + mic
//  each get their own `AVAssetWriterInput` so downstream transcription
//  can diarize without separate files.
//

import Foundation
import AVFoundation
import CoreMedia

@available(macOS 13.0, *)
final class SegmentWriter: @unchecked Sendable {
    let outputPath: URL

    private let queue = DispatchQueue(label: "corivo.helper.segment", qos: .userInteractive)
    private let writer: AVAssetWriter
    private let sysInput: AVAssetWriterInput?
    private let micInput: AVAssetWriterInput?
    private var sessionStarted = false
    private var origin: CMTime = .invalid

    init(
        outputDir: URL,
        captureSystemAudio: Bool,
        captureMicrophone: Bool
    ) throws {
        let path = outputDir.appendingPathComponent("segment-0000.m4a")
        // AVAssetWriter rejects an existing file at the destination path.
        try? FileManager.default.removeItem(at: path)
        self.outputPath = path

        do {
            self.writer = try AVAssetWriter(outputURL: path, fileType: .m4a)
        } catch {
            throw HelperError.osError(code: (error as NSError).code,
                                       detail: "AVAssetWriter init failed: \(error)")
        }

        let aacSettings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 48000,
            AVNumberOfChannelsKey: 2,
            AVEncoderBitRateKey: 128_000,
        ]

        if captureSystemAudio {
            let input = AVAssetWriterInput(mediaType: .audio, outputSettings: aacSettings)
            input.expectsMediaDataInRealTime = true
            guard writer.canAdd(input) else {
                throw HelperError.internal("AVAssetWriter rejected sys-audio input")
            }
            writer.add(input)
            self.sysInput = input
        } else {
            self.sysInput = nil
        }

        if captureMicrophone {
            let input = AVAssetWriterInput(mediaType: .audio, outputSettings: aacSettings)
            input.expectsMediaDataInRealTime = true
            guard writer.canAdd(input) else {
                throw HelperError.internal("AVAssetWriter rejected microphone input")
            }
            writer.add(input)
            self.micInput = input
        } else {
            self.micInput = nil
        }

        if !writer.startWriting() {
            throw HelperError.internal(
                "AVAssetWriter.startWriting failed: \(writer.error?.localizedDescription ?? "unknown")")
        }
    }

    func appendSystemAudio(_ buffer: CMSampleBuffer) {
        queue.async { [weak self] in self?.appendInternal(buffer, input: self?.sysInput) }
    }

    func appendMicrophone(_ buffer: CMSampleBuffer) {
        queue.async { [weak self] in self?.appendInternal(buffer, input: self?.micInput) }
    }

    private func appendInternal(_ buffer: CMSampleBuffer, input: AVAssetWriterInput?) {
        guard let input = input, CMSampleBufferDataIsReady(buffer) else { return }
        let pts = CMSampleBufferGetPresentationTimeStamp(buffer)
        if !sessionStarted {
            origin = pts
            writer.startSession(atSourceTime: pts)
            sessionStarted = true
        }
        guard input.isReadyForMoreMediaData else {
            // Backpressure: drop. Reported once per session via warning.
            return
        }
        if !input.append(buffer) {
            Log.warn("AVAssetWriterInput.append failed (status=\(writer.status.rawValue))")
        }
    }

    func finalize() async {
        await withCheckedContinuation { (cont: CheckedContinuation<Void, Never>) in
            queue.async { [weak self] in
                guard let self = self else { cont.resume(); return }
                self.sysInput?.markAsFinished()
                self.micInput?.markAsFinished()
                self.writer.finishWriting {
                    cont.resume()
                }
            }
        }
    }
}
