//
//  HelperError.swift
//  Internal Swift error → wire ResponseError mapping.
//

import Foundation

enum HelperError: Error {
    case invalidRequest(String)
    case unsupported(String)
    case permissionDenied(String, kind: String)
    case resourceBusy(String)
    case notFound(String)
    case timeout(String)
    case osError(code: Int, detail: String)
    case `internal`(String)
    case protocolViolation(String)
}

extension HelperError {
    func toResponseError() -> ResponseError {
        switch self {
        case .invalidRequest(let m):
            return ResponseError(code: ErrorCodes.invalidRequest, message: m)
        case .unsupported(let m):
            return ResponseError(code: ErrorCodes.unsupported, message: m)
        case .permissionDenied(let m, let kind):
            return ResponseError(
                code: ErrorCodes.permissionDenied,
                message: m,
                detail: AnyCodable(["kind": kind])
            )
        case .resourceBusy(let m):
            return ResponseError(code: ErrorCodes.resourceBusy, message: m)
        case .notFound(let m):
            return ResponseError(code: ErrorCodes.notFound, message: m)
        case .timeout(let m):
            return ResponseError(code: ErrorCodes.timeout, message: m)
        case .osError(let code, let detail):
            return ResponseError(
                code: ErrorCodes.osError,
                message: detail,
                detail: AnyCodable(["os_code": code])
            )
        case .internal(let m):
            return ResponseError(code: ErrorCodes.internal, message: m)
        case .protocolViolation(let m):
            // Protocol violations from the client get reported as
            // INVALID_REQUEST per the wire enum.
            return ResponseError(code: ErrorCodes.invalidRequest, message: m)
        }
    }
}
