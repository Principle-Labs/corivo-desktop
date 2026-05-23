//
//  MicrophoneSource.swift
//  Microphone capture via AVCaptureSession.
//

import Foundation
import AVFoundation
import CoreMedia

@available(macOS 13.0, *)
final class MicrophoneSource: NSObject, AVCaptureAudioDataOutputSampleBufferDelegate {
    weak var sink: SegmentWriter?

    private let session = AVCaptureSession()
    private let queue = DispatchQueue(label: "corivo.helper.mic", qos: .userInteractive)

    func start(deviceID: String?) throws {
        session.beginConfiguration()

        let device: AVCaptureDevice
        if let id = deviceID, let d = AVCaptureDevice(uniqueID: id) {
            device = d
        } else if let d = AVCaptureDevice.default(for: .audio) {
            device = d
        } else {
            session.commitConfiguration()
            throw HelperError.notFound("no microphone device available")
        }

        let input: AVCaptureDeviceInput
        do {
            input = try AVCaptureDeviceInput(device: device)
        } catch {
            session.commitConfiguration()
            throw HelperError.osError(
                code: (error as NSError).code,
                detail: "AVCaptureDeviceInput init: \(error)"
            )
        }
        guard session.canAddInput(input) else {
            session.commitConfiguration()
            throw HelperError.internal("microphone input rejected by AVCaptureSession")
        }
        session.addInput(input)

        let output = AVCaptureAudioDataOutput()
        output.setSampleBufferDelegate(self, queue: queue)
        guard session.canAddOutput(output) else {
            session.commitConfiguration()
            throw HelperError.internal("microphone output rejected by AVCaptureSession")
        }
        session.addOutput(output)

        session.commitConfiguration()
        session.startRunning()
    }

    func stop() {
        session.stopRunning()
    }

    func captureOutput(_: AVCaptureOutput,
                       didOutput sampleBuffer: CMSampleBuffer,
                       from _: AVCaptureConnection) {
        sink?.appendMicrophone(sampleBuffer)
    }

    /// `recording.list_microphones` data source.
    static func enumerateDevices() -> [(id: String, name: String, isDefault: Bool)] {
        // macOS 14.0 introduced `.microphone` / `.external`; older 13.x
        // only knows `.builtInMicrophone`.
        let deviceTypes: [AVCaptureDevice.DeviceType]
        if #available(macOS 14.0, *) {
            deviceTypes = [.microphone, .external]
        } else {
            deviceTypes = [.builtInMicrophone]
        }
        let session = AVCaptureDevice.DiscoverySession(
            deviceTypes: deviceTypes,
            mediaType: .audio,
            position: .unspecified
        )
        let defaultID = AVCaptureDevice.default(for: .audio)?.uniqueID
        return session.devices.map { d in
            (id: d.uniqueID, name: d.localizedName, isDefault: d.uniqueID == defaultID)
        }
    }
}
