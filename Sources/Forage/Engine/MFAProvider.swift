import Foundation

/// Resolves an MFA code when an `auth.session.<...>` block declares
/// `requiresMFA: true`. The host (CLI / Toolkit / web IDE) supplies an
/// implementation; the engine awaits `mfaCode()` and re-runs the login
/// with the code attached.
public protocol MFAProvider: Sendable {
    /// Block until the user supplies a code. Throws `MFAError.cancelled`
    /// if the user dismisses the prompt — the engine surfaces that as
    /// `stallReason: "auth-mfa-cancelled"`.
    func mfaCode() async throws -> String
}

public enum MFAError: Error, Sendable, CustomStringConvertible {
    case cancelled

    public var description: String {
        switch self {
        case .cancelled: return "MFA prompt cancelled"
        }
    }
}

/// Test/dictionary MFA provider — yields a pre-configured code.
public struct StaticMFAProvider: MFAProvider {
    public let code: String

    public init(code: String) {
        self.code = code
    }

    public func mfaCode() async throws -> String {
        code
    }
}
