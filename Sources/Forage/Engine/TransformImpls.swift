import Foundation

/// Built-in transform vocabulary. Recipes call these by name in pipelines
/// (`$.x | parseSize | normalizeOzToGrams`) or function-call form
/// (`coalesce(a, b)`, `default(...)`).
///
/// Each transform takes a `JSONValue` (the piped value) and an optional list
/// of evaluated `JSONValue` args, returns a `JSONValue`. Adding a new
/// transform is a deliberate Swift extension to `register()` — the DSL
/// doesn't grow primitives ad-hoc.
public struct TransformImpls: Sendable {

    public typealias Impl = @Sendable (JSONValue, [JSONValue]) throws -> JSONValue

    private var registry: [String: Impl] = [:]

    public init() {
        registerDefaults()
    }

    public func has(_ name: String) -> Bool {
        registry[name] != nil
    }

    public func apply(_ name: String, value: JSONValue, args: [JSONValue]) throws -> JSONValue {
        guard let impl = registry[name] else {
            throw TransformError.unknown(name)
        }
        return try impl(value, args)
    }

    private mutating func register(_ name: String, _ impl: @escaping Impl) {
        registry[name] = impl
    }

    private mutating func registerDefaults() {
        // MARK: - Type coercion

        register("toString") { v, _ in
            switch v {
            case .null: return .null
            case .bool(let b): return .string(String(b))
            case .int(let i): return .string(String(i))
            case .double(let d): return .string(String(d))
            case .string(let s): return .string(s)
            default: return .string("\(v)")
            }
        }
        register("parseInt") { v, _ in
            if case .string(let s) = v, let i = Int(s) { return .int(i) }
            if case .double(let d) = v { return .int(Int(d)) }
            if case .int = v { return v }
            return .null
        }
        register("parseFloat") { v, _ in
            if case .string(let s) = v, let d = Double(s) { return .double(d) }
            if case .int(let i) = v { return .double(Double(i)) }
            if case .double = v { return v }
            return .null
        }
        register("parseBool") { v, _ in
            if case .string(let s) = v {
                switch s.lowercased() {
                case "true", "yes", "1": return .bool(true)
                case "false", "no", "0": return .bool(false)
                default: return .null
                }
            }
            if case .bool = v { return v }
            return .null
        }

        // MARK: - String

        register("lower") { v, _ in
            if case .string(let s) = v { return .string(s.lowercased()) }
            return v
        }
        register("upper") { v, _ in
            if case .string(let s) = v { return .string(s.uppercased()) }
            return v
        }
        register("trim") { v, _ in
            if case .string(let s) = v {
                return .string(s.trimmingCharacters(in: .whitespacesAndNewlines))
            }
            return v
        }
        register("capitalize") { v, _ in
            if case .string(let s) = v { return .string(s.capitalized) }
            return v
        }
        register("titleCase") { v, _ in
            if case .string(let s) = v { return .string(s.capitalized) }
            return v
        }

        // MARK: - Array

        register("length") { v, _ in
            if case .array(let a) = v { return .int(a.count) }
            if case .string(let s) = v { return .int(s.count) }
            if case .null = v { return .int(0) }
            return .int(1)
        }
        register("dedup") { v, _ in
            if case .array(let a) = v {
                var seen: [JSONValue] = []
                for x in a where !seen.contains(x) { seen.append(x) }
                return .array(seen)
            }
            return v
        }

        // MARK: - Cannabis-domain helpers

        // parseSize: "3.5g" → {value: 3.5, unit: "G"}, "1oz" → {value: 1, unit: "OZ"}, "100mg" → {…}
        // Returns an object with `.value` and `.unit` keys. Recipes typically
        // pipe through normalizeOzToGrams next.
        register("parseSize") { v, _ in
            guard case .string(let s) = v else { return .null }
            let pattern = #"^([0-9]+(?:\.[0-9]+)?)\s*(g|mg|oz|ml)\b"#
            guard let regex = try? NSRegularExpression(pattern: pattern, options: .caseInsensitive),
                  let match = regex.firstMatch(in: s, range: NSRange(s.startIndex..., in: s)),
                  let valueRange = Range(match.range(at: 1), in: s),
                  let unitRange = Range(match.range(at: 2), in: s),
                  let value = Double(s[valueRange])
            else { return .null }
            let unit = String(s[unitRange]).uppercased()
            return .object(["value": .double(value), "unit": .string(unit)])
        }

        // normalizeOzToGrams: takes the {value, unit} object from parseSize (or
        // similar), returns the value with OZ × 28 = G. Pass-through for non-OZ.
        register("normalizeOzToGrams") { v, _ in
            guard case .object(let obj) = v,
                  case .double(let value) = (obj["value"] ?? .null) else { return v }
            let unit = (obj["unit"]).flatMap { if case .string(let s) = $0 { return s } else { return nil as String? } } ?? ""
            if unit.uppercased() == "OZ" {
                return .object([
                    "value": .double(value * 28),
                    "unit": .string("G")
                ])
            }
            return v
        }

        // sizeValue / sizeUnit: project the parts out of the parseSize object.
        register("sizeValue") { v, _ in
            if case .object(let obj) = v, let val = obj["value"] { return val }
            return .null
        }
        register("sizeUnit") { v, _ in
            if case .object(let obj) = v, let unit = obj["unit"] { return unit }
            return .null
        }

        // normalizeUnitToGrams: for a unit string, OZ → G, else passthrough.
        register("normalizeUnitToGrams") { v, _ in
            if case .string(let s) = v, s.uppercased() == "OZ" { return .string("G") }
            return v
        }

        // prevalenceNormalize: "INDICA" / "Indica" / "indica" → "Indica";
        // "NOT_APPLICABLE" → null; null / empty → null.
        register("prevalenceNormalize") { v, _ in
            guard case .string(let raw) = v, !raw.isEmpty, raw != "NOT_APPLICABLE" else {
                return .null
            }
            return .string(raw.capitalized)
        }

        // parseJaneWeight: Jane uses "half ounce" / "gram" / "two gram" /
        // "eighth ounce" / "quarter ounce" / "ounce" / "half gram" / "each".
        // Returns the value in canonical units. "each" → null (use the weight
        // unit "EA").
        register("parseJaneWeight") { v, _ in
            guard case .string(let s) = v else { return .null }
            switch s.lowercased() {
            case "half gram":     return .double(0.5)
            case "gram":          return .double(1.0)
            case "two gram":      return .double(2.0)
            case "eighth ounce":  return .double(3.5)
            case "quarter ounce": return .double(7.0)
            case "half ounce":    return .double(14.0)
            case "ounce":         return .double(28.0)
            case "each":          return .null
            default:
                // Try numeric prefix fallback ("1g", "3.5g")
                let parts = s.split(separator: " ").first.map(String.init) ?? s
                return .double(Double(parts) ?? 0)
            }
        }
        register("janeWeightUnit") { v, _ in
            guard case .string(let s) = v else { return .null }
            switch s.lowercased() {
            case "each": return .string("EA")
            default:     return .string("G")
            }
        }

        // MARK: - Object / dynamic-field access

        // getField(obj, "fieldName") — dynamic field lookup (where the field
        // name is computed). Used for Jane's per-weight price columns:
        // `getField($product.search_attributes, "price_{$weight}")`.
        register("getField") { v, args in
            guard args.count >= 1 else { return .null }
            let obj = v
            let keyArg = args[0]
            guard case .string(let key) = keyArg else { return .null }
            if case .object(let o) = obj { return o[key] ?? .null }
            return .null
        }

        // MARK: - Coalesce / default

        register("coalesce") { v, args in
            if !v.isNull { return v }
            for a in args where !a.isNull { return a }
            return .null
        }
        register("default") { v, args in
            if !v.isNull { return v }
            return args.first ?? .null
        }
    }
}

public enum TransformError: Error, CustomStringConvertible {
    case unknown(String)
    case wrongArgCount(name: String, expected: Int, got: Int)

    public var description: String {
        switch self {
        case .unknown(let n): return "unknown transform '\(n)'"
        case .wrongArgCount(let n, let e, let g): return "transform '\(n)' expects \(e) args, got \(g)"
        }
    }
}

extension JSONValue {
    fileprivate var isNull: Bool {
        if case .null = self { return true }
        return false
    }
}
