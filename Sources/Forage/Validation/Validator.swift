import Foundation

/// Static validation for a parsed `Recipe`. Catches errors before the
/// runtime sees them: type/enum/transform references, unbound variables,
/// missing required emit fields, malformed pagination configs.
///
/// Returns a list of `ValidationIssue` (errors + warnings). An empty list
/// means the recipe is good to run; the host typically refuses to run a
/// recipe with errors, but warnings are surfaced and ignored.
public enum Validator {

    public static func validate(_ recipe: Recipe) -> [ValidationIssue] {
        var issues: [ValidationIssue] = []
        let typeNames = Set(recipe.types.map(\.name))
        let enumNames = Set(recipe.enums.map(\.name))
        var topLevelNames = Set(recipe.inputs.map(\.name))
        // htmlPrime auth captures variables that become top-level scope —
        // recipe steps reference them just like inputs.
        if case .htmlPrime(_, let captures) = recipe.auth {
            for v in captures { topLevelNames.insert(v.varName) }
        }
        let inputNames = topLevelNames
        let transforms = TransformImpls()

        // 1. Type field references resolve.
        for type in recipe.types {
            for field in type.fields {
                checkFieldType(field.type, knownTypes: typeNames, knownEnums: enumNames, location: "type \(type.name).\(field.name)", issues: &issues)
            }
        }
        // 2. Inputs reference real types/enums.
        for input in recipe.inputs {
            checkFieldType(input.type, knownTypes: typeNames, knownEnums: enumNames, location: "input \(input.name)", issues: &issues)
        }

        // 3. Statement walker — validates emit type/field references,
        //    transform names, path-variable scope.
        var stepNames: Set<String> = []
        // Track variables in scope: inputs are always in scope; for-loop
        // variables push and pop frames.
        var varStack: [Set<String>] = [inputNames]

        func varInScope(_ name: String) -> Bool {
            for frame in varStack.reversed() where frame.contains(name) { return true }
            return false
        }

        // Predeclare all step names anywhere in the body so a path expression
        // referring to a step result that appears later still validates.
        func collectStepNames(_ stmts: [Statement]) {
            for s in stmts {
                switch s {
                case .step(let st):
                    if stepNames.contains(st.name) {
                        issues.append(.error("duplicate step name '\(st.name)'", "step decl"))
                    }
                    stepNames.insert(st.name)
                case .forLoop(_, _, let body):
                    collectStepNames(body)
                case .emit:
                    break
                }
            }
        }
        collectStepNames(recipe.body)

        func walk(_ stmts: [Statement]) {
            for stmt in stmts {
                switch stmt {
                case .step(let s):
                    validatePathExpr(s.request.url, knownVars: { varInScope($0) || stepNames.contains($0) }, knownInputs: inputNames, location: "step \(s.name).url", issues: &issues)
                    if let body = s.request.body { validateBody(body, location: "step \(s.name).body", knownVars: varInScope, knownInputs: inputNames, issues: &issues) }
                    if let p = s.pagination { validatePagination(p, location: "step \(s.name).paginate", issues: &issues) }
                case .emit(let em):
                    guard typeNames.contains(em.typeName) else {
                        issues.append(.error("emit references unknown type '\(em.typeName)'", "emit"))
                        continue
                    }
                    let recipeType = recipe.type(em.typeName)!
                    let typeFields = Set(recipeType.fields.map(\.name))
                    let boundFields = Set(em.bindings.map(\.fieldName))
                    for fb in em.bindings where !typeFields.contains(fb.fieldName) {
                        issues.append(.error("emit \(em.typeName).\(fb.fieldName): unknown field on type", "emit"))
                    }
                    for f in recipeType.fields where !f.optional && !boundFields.contains(f.name) {
                        issues.append(.warning("emit \(em.typeName) doesn't bind required field '\(f.name)'", "emit"))
                    }
                    for fb in em.bindings {
                        validateExtraction(fb.expr, transforms: transforms, knownVars: varInScope, knownInputs: inputNames, knownTypes: typeNames, knownStepNames: stepNames, location: "emit \(em.typeName).\(fb.fieldName)", issues: &issues)
                    }
                case .forLoop(let v, let coll, let body):
                    validatePath(coll, knownVars: { varInScope($0) || stepNames.contains($0) }, knownInputs: inputNames, location: "for \(v) in <coll>", issues: &issues)
                    var frame = Set<String>()
                    frame.insert(v)
                    varStack.append(frame)
                    walk(body)
                    varStack.removeLast()
                }
            }
        }

        walk(recipe.body)

        return issues
    }

    // MARK: - Sub-validators

    private static func checkFieldType(_ type: FieldType, knownTypes: Set<String>, knownEnums: Set<String>, location: String, issues: inout [ValidationIssue]) {
        switch type {
        case .string, .int, .double, .bool: return
        case .array(let inner):
            checkFieldType(inner, knownTypes: knownTypes, knownEnums: knownEnums, location: location, issues: &issues)
        case .record(let n):
            // Could be a type or enum reference; the parser doesn't disambiguate.
            if !knownTypes.contains(n) && !knownEnums.contains(n) {
                issues.append(.error("\(location): unknown type/enum '\(n)'", location))
            }
        case .enumRef(let n):
            if !knownEnums.contains(n) {
                issues.append(.error("\(location): unknown enum '\(n)'", location))
            }
        }
    }

    private static func validatePagination(_ p: Pagination, location: String, issues: inout [ValidationIssue]) {
        // Just check that `pageSize > 0` etc. — value sanity.
        switch p {
        case .pageWithTotal(_, _, _, let pageSize, _):
            if pageSize <= 0 { issues.append(.error("\(location): pageSize must be > 0", location)) }
        case .untilEmpty: break
        case .cursor: break
        }
    }

    private static func validatePath(_ p: PathExpr, knownVars: (String) -> Bool, knownInputs: Set<String>, location: String, issues: inout [ValidationIssue]) {
        switch p {
        case .current, .input: return
        case .variable(let n):
            if !knownVars(n) { issues.append(.error("\(location): unbound variable $\(n)", location)) }
        case .field(let inner, _), .optField(let inner, _), .index(let inner, _), .wildcard(let inner):
            validatePath(inner, knownVars: knownVars, knownInputs: knownInputs, location: location, issues: &issues)
        }
    }

    private static func validatePathExpr(_ template: Template, knownVars: (String) -> Bool, knownInputs: Set<String>, location: String, issues: inout [ValidationIssue]) {
        for part in template.parts {
            if case .interp(let p) = part {
                validatePath(p, knownVars: knownVars, knownInputs: knownInputs, location: location, issues: &issues)
            }
        }
    }

    private static func validateBody(_ body: HTTPBody, location: String, knownVars: (String) -> Bool, knownInputs: Set<String>, issues: inout [ValidationIssue]) {
        switch body {
        case .jsonObject(let kvs):
            for kv in kvs { validateBodyValue(kv.value, location: "\(location).\(kv.key)", knownVars: knownVars, knownInputs: knownInputs, issues: &issues) }
        case .form(let kvs):
            for (k, v) in kvs { validateBodyValue(v, location: "\(location).\(k)", knownVars: knownVars, knownInputs: knownInputs, issues: &issues) }
        case .raw(let t):
            validatePathExpr(t, knownVars: knownVars, knownInputs: knownInputs, location: location, issues: &issues)
        }
    }

    private static func validateBodyValue(_ bv: BodyValue, location: String, knownVars: (String) -> Bool, knownInputs: Set<String>, issues: inout [ValidationIssue]) {
        switch bv {
        case .templateString(let t):
            validatePathExpr(t, knownVars: knownVars, knownInputs: knownInputs, location: location, issues: &issues)
        case .literal: break
        case .path(let p):
            validatePath(p, knownVars: knownVars, knownInputs: knownInputs, location: location, issues: &issues)
        case .object(let kvs):
            for kv in kvs { validateBodyValue(kv.value, location: "\(location).\(kv.key)", knownVars: knownVars, knownInputs: knownInputs, issues: &issues) }
        case .array(let xs):
            for v in xs { validateBodyValue(v, location: "\(location)[]", knownVars: knownVars, knownInputs: knownInputs, issues: &issues) }
        case .caseOf(let s, let branches):
            validatePath(s, knownVars: knownVars, knownInputs: knownInputs, location: "\(location).case", issues: &issues)
            for (_, v) in branches { validateBodyValue(v, location: location, knownVars: knownVars, knownInputs: knownInputs, issues: &issues) }
        }
    }

    private static func validateExtraction(_ expr: ExtractionExpr, transforms: TransformImpls, knownVars: (String) -> Bool, knownInputs: Set<String>, knownTypes: Set<String>, knownStepNames: Set<String>, location: String, issues: inout [ValidationIssue]) {
        switch expr {
        case .path(let p):
            validatePath(p, knownVars: { knownVars($0) || knownStepNames.contains($0) }, knownInputs: knownInputs, location: location, issues: &issues)
        case .pipe(let inner, let calls):
            validateExtraction(inner, transforms: transforms, knownVars: knownVars, knownInputs: knownInputs, knownTypes: knownTypes, knownStepNames: knownStepNames, location: location, issues: &issues)
            for c in calls {
                if !transforms.has(c.name) {
                    issues.append(.error("\(location): unknown transform '\(c.name)'", location))
                }
            }
        case .caseOf(let scrutinee, let branches):
            validatePath(scrutinee, knownVars: { knownVars($0) || knownStepNames.contains($0) }, knownInputs: knownInputs, location: "\(location).case", issues: &issues)
            for (_, e) in branches {
                validateExtraction(e, transforms: transforms, knownVars: knownVars, knownInputs: knownInputs, knownTypes: knownTypes, knownStepNames: knownStepNames, location: location, issues: &issues)
            }
        case .mapTo(let p, let em):
            validatePath(p, knownVars: { knownVars($0) || knownStepNames.contains($0) }, knownInputs: knownInputs, location: location, issues: &issues)
            if !knownTypes.contains(em.typeName) {
                issues.append(.error("\(location): map-to references unknown type '\(em.typeName)'", location))
            }
        case .literal, .template: break
        case .call(let name, let args):
            if !transforms.has(name) {
                issues.append(.error("\(location): unknown transform '\(name)'", location))
            }
            for a in args { validateExtraction(a, transforms: transforms, knownVars: knownVars, knownInputs: knownInputs, knownTypes: knownTypes, knownStepNames: knownStepNames, location: location, issues: &issues) }
        }
    }
}

public struct ValidationIssue: Hashable, Sendable {
    public enum Severity: Sendable { case error, warning }
    public let severity: Severity
    public let message: String
    public let location: String

    public static func error(_ message: String, _ location: String) -> Self {
        ValidationIssue(severity: .error, message: message, location: location)
    }
    public static func warning(_ message: String, _ location: String) -> Self {
        ValidationIssue(severity: .warning, message: message, location: location)
    }
}

extension Array where Element == ValidationIssue {
    public var hasErrors: Bool { contains(where: { $0.severity == .error }) }
    public var errors: [ValidationIssue] { filter { $0.severity == .error } }
    public var warnings: [ValidationIssue] { filter { $0.severity == .warning } }
}
