import Foundation

/// Resolves `$secret.<name>` references at runtime. The recipe text never
/// contains the resolved value; the resolver is the only place credential
/// material enters the engine.
///
/// Three default implementations:
/// - `EnvironmentSecretResolver` — `FORAGE_SECRET_<NAME>` env vars (CLI).
/// - `DictionarySecretResolver` — in-memory map (tests).
/// - Studio composes its own keychain-backed resolver.
public protocol SecretResolver: Sendable {
    func resolve(_ name: String) async throws -> String
}

/// Reads `FORAGE_SECRET_<NAME>` (uppercase) from the process environment.
/// Used by the CLI by default.
public struct EnvironmentSecretResolver: SecretResolver {
    public init() {}

    public func resolve(_ name: String) async throws -> String {
        let envKey = "FORAGE_SECRET_\(name.uppercased())"
        guard let v = ProcessInfo.processInfo.environment[envKey] else {
            throw SecretError.notFound(name)
        }
        return v
    }
}

/// In-memory map, useful for tests and host code that already has the
/// secrets in hand (e.g. Studio after a Keychain read).
public struct DictionarySecretResolver: SecretResolver {
    public let secrets: [String: String]

    public init(_ secrets: [String: String]) {
        self.secrets = secrets
    }

    public func resolve(_ name: String) async throws -> String {
        guard let v = secrets[name] else { throw SecretError.notFound(name) }
        return v
    }
}

/// Errors thrown by `SecretResolver` implementations.
public enum SecretError: Error, Sendable, CustomStringConvertible {
    case notFound(String)

    public var description: String {
        switch self {
        case .notFound(let n): return "secret '\(n)' not found"
        }
    }
}
