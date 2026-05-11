import Foundation

/// Renders a `Template` against a `Scope`. Each interpolation evaluates its
/// extraction expression (path, pipeline, or function-call transform) and
/// stringifies the result.
public enum TemplateRenderer {
    public static func render(
        _ template: Template,
        in scope: Scope,
        evaluator: ExtractionEvaluator = ExtractionEvaluator()
    ) throws -> String {
        var out = ""
        for part in template.parts {
            switch part {
            case .literal(let s):
                out.append(s)
            case .interp(let expr):
                let v = try evaluator.evaluateToJSON(expr, in: scope)
                out.append(stringify(v))
            }
        }
        return out
    }

    public static func stringify(_ v: JSONValue) -> String {
        switch v {
        case .null: return ""
        case .bool(let b): return String(b)
        case .int(let i): return String(i)
        case .double(let d):
            // Prefer integer-style for whole doubles
            if d == d.rounded() && abs(d) < 1e15 { return String(Int(d)) }
            return String(d)
        case .string(let s): return s
        case .array, .object:
            // Best-effort JSON; rarely useful in a URL template, but avoids crashes.
            if let data = try? JSONSerialization.data(withJSONObject: v.asAny, options: [.fragmentsAllowed]),
               let s = String(data: data, encoding: .utf8) {
                return s
            }
            return ""
        case .node(let n):
            return (try? n.element.outerHtml()) ?? ""
        }
    }
}
