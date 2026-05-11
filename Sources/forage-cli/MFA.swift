import Foundation
import Forage

/// MFA provider for the CLI: prompts on stderr, reads one line from stdin.
/// Used by `forage run` when the recipe declares `auth.session.requiresMFA: true`
/// and the user hasn't passed `--no-mfa`.
public struct StdinMFAProvider: MFAProvider {
    public init() {}

    public func mfaCode() async throws -> String {
        // Prompt on stderr so stdout stays clean for the snapshot output.
        FileHandle.standardError.write("Enter MFA code: ".data(using: .utf8)!)
        guard let line = readLine(strippingNewline: true), !line.isEmpty else {
            throw MFAError.cancelled
        }
        return line
    }
}
