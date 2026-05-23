//
//  ActiveSession.swift
//  One in-flight recording session. Owns the audio sources and the
//  segment writer; lives until `stop()` resolves.
//
//  v1 simplification: a session writes a single segment file
//  `segment-0000.m4a` for its entire duration. Segment rotation lands
//  in v2; the wire contract already accommodates it.
//

import Foundation
import AVFoundation
import ScreenCaptureKit

@available(macOS 13.0, *)
final class ActiveSession {
    let sessionID: String
    let startedAt: Date

    struct ClosedSegment {
        var sessionID: String
        var index: Int
        var path: URL
        var duration: TimeInterval
        var startedAt: Date
        var endedAt: Date
        var sysTrack: Bool
        var micTrack: Bool
    }

    private let outputDir: URL
    private let captureSystemAudio: Bool
    private let captureMicrophone: Bool
    private let onSegmentClosed: (ClosedSegment) -> Void

    private var segmentWriter: SegmentWriter?
    private var systemSource: SystemAudioSource?
    private var microphoneSource: MicrophoneSource?
    private var totalSegments = 0

    init(
        request: AudioRecorder.StartRequest,
        onSegmentClosed: @escaping (ClosedSegment) -> Void
    ) throws {
        self.sessionID = request.sessionID
        self.startedAt = Date()
        self.outputDir = request.outputDir
        self.captureSystemAudio = request.captureSystemAudio
        self.captureMicrophone = request.captureMicrophone
        self.onSegmentClosed = onSegmentClosed

        let writer = try SegmentWriter(
            outputDir: request.outputDir,
            captureSystemAudio: request.captureSystemAudio,
            captureMicrophone: request.captureMicrophone
        )
        self.segmentWriter = writer

        if request.captureSystemAudio {
            let src = SystemAudioSource()
            src.sink = writer
            try src.start()
            self.systemSource = src
        }
        if request.captureMicrophone {
            let mic = MicrophoneSource()
            mic.sink = writer
            try mic.start(deviceID: request.microphoneDeviceID)
            self.microphoneSource = mic
        }
    }

    /// Resolves with the total number of segments written. v1 always 1
    /// (single-segment session); v2 with rotation will report the real
    /// count.
    func stop() async -> Int {
        await systemSource?.stop()
        microphoneSource?.stop()

        guard let writer = segmentWriter else { return 0 }
        let endedAt = Date()
        let duration = endedAt.timeIntervalSince(startedAt)
        let path = writer.outputPath
        await writer.finalize()
        totalSegments += 1

        let closed = ClosedSegment(
            sessionID: sessionID,
            index: 0,
            path: path,
            duration: duration,
            startedAt: startedAt,
            endedAt: endedAt,
            sysTrack: captureSystemAudio,
            micTrack: captureMicrophone
        )
        onSegmentClosed(closed)
        return totalSegments
    }
}
