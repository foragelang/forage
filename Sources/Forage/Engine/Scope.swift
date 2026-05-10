import Foundation

/// Runtime variable scope. A `Scope` is a stacked binding of names → values
/// that path expressions resolve against. The runtime pushes a new scope
/// frame for each `for` loop iteration, step result, and emit context.
public struct Scope: Sendable {
    /// Inputs the consumer supplied. Resolved by `$input.<name>`.
    public let inputs: [String: JSONValue]
    /// Anything else (`$cat`, `$products`, `$ajaxNonce`, …) — a flat map per
    /// frame; lookups walk frames inside-out.
    public let frames: [[String: JSONValue]]
    /// The "current value" — what `$` (no name) resolves to. Set by the
    /// surrounding for-loop or capture-extract iteration; nil at top level.
    public let current: JSONValue?

    public init(
        inputs: [String: JSONValue] = [:],
        frames: [[String: JSONValue]] = [],
        current: JSONValue? = nil
    ) {
        self.inputs = inputs
        self.frames = frames
        self.current = current
    }

    /// Push a new (named) variable; returns a new scope.
    public func with(_ name: String, _ value: JSONValue) -> Scope {
        var newFrames = frames
        if newFrames.isEmpty { newFrames.append([:]) }
        newFrames[newFrames.count - 1][name] = value
        return Scope(inputs: inputs, frames: newFrames, current: current)
    }

    /// Push a new frame.
    public func pushed() -> Scope {
        Scope(inputs: inputs, frames: frames + [[:]], current: current)
    }

    /// Set the current ($) value.
    public func withCurrent(_ value: JSONValue?) -> Scope {
        Scope(inputs: inputs, frames: frames, current: value)
    }

    /// Resolve a named variable. Walks frames inside-out.
    public func variable(_ name: String) -> JSONValue? {
        for frame in frames.reversed() {
            if let v = frame[name] { return v }
        }
        return nil
    }
}

/// Errors thrown by the runtime when a recipe references something undefined.
public enum ScopeError: Error, CustomStringConvertible {
    case undefinedInput(String)
    case undefinedVariable(String)
    case noCurrentValue

    public var description: String {
        switch self {
        case .undefinedInput(let n): return "undefined input $input.\(n)"
        case .undefinedVariable(let n): return "undefined variable $\(n)"
        case .noCurrentValue: return "no current value bound for $"
        }
    }
}
