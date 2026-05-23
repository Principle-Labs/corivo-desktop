//
//  Router.swift
//  Top-level dispatcher for inbound RPC. Phase 0 only handles control-plane
//  methods (`ping` / `shutdown`). Domain methods are added in their own
//  phases by extending the switch.
//

import Foundation

enum Router {
    static func dispatch(_ request: RequestMessage) async {
        do {
            switch request.method {
            // Phase 0 — control plane.
            case Methods.ping:
                try await ControlHandlers.ping(request)
            case Methods.shutdown:
                try await ControlHandlers.shutdown(request)

            // Phase 1 — screen.
            case Methods.screenCapture:
                try await ScreenHandlers.capture(request)
            case Methods.screenListDisplays:
                try await ScreenHandlers.listDisplays(request)

            // Phase 2 — accessibility.
            case Methods.axQuery:
                try await AxHandlers.query(request)
            case Methods.axProbeSelection:
                try await AxHandlers.probeSelection(request)

            // Phase 3 — async subscriptions.
            case Methods.axSubscribe:
                try await AxSubscribeHandlers.subscribe(request)
            case Methods.axUnsubscribe:
                try await AxSubscribeHandlers.unsubscribe(request)
            case Methods.foregroundSubscribe:
                try await ForegroundHandlers.subscribe(request)
            case Methods.foregroundUnsubscribe:
                try await ForegroundHandlers.unsubscribe(request)
            case Methods.foregroundCurrent:
                try await ForegroundHandlers.current(request)

            // Phase 4 — OCR.
            case Methods.ocrRun:
                try await OcrHandlers.run(request)

            // Phase 5 — meeting audio recording.
            case Methods.recordingStart:
                try await RecordingHandlers.start(request)
            case Methods.recordingStop:
                try await RecordingHandlers.stop(request)
            case Methods.recordingListMicrophones:
                try await RecordingHandlers.listMicrophones(request)

            // Phase 7 — permission status.
            case Methods.permissionStatus:
                try await PermissionHandlers.status(request)

            default:
                let err = HelperError.unsupported("unknown method: \(request.method)")
                try await EventEmitter.shared.send(
                    ResponseMessage(id: request.id, error: err.toResponseError())
                )
            }
        } catch let e as HelperError {
            do {
                try await EventEmitter.shared.send(
                    ResponseMessage(id: request.id, error: e.toResponseError())
                )
            } catch {
                Log.error("response write failed for \(request.method): \(error)")
            }
        } catch {
            do {
                let internalErr = HelperError.internal(String(describing: error))
                try await EventEmitter.shared.send(
                    ResponseMessage(id: request.id, error: internalErr.toResponseError())
                )
            } catch {
                Log.error("response write failed for \(request.method): \(error)")
            }
        }
    }
}
