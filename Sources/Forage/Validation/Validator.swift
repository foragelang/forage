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
        let declaredSecrets = Set(recipe.secrets)

        // Collect every `$secret.<name>` referenced anywhere in the recipe
        // (auth block, step requests, emit bindings). Any reference whose
        // name isn't declared via `secret <name>` surfaces as a warning;
        // a `secret` declaration that isn't referenced surfaces too.
        let referencedSecrets = collectReferencedSecrets(recipe: recipe)
        for n in referencedSecrets where !declaredSecrets.contains(n) {
            issues.append(.warning(
                "referenced secret '\(n)' is not declared via `secret \(n)` at the top of the recipe",
                "secrets"
            ))
        }
        for n in declaredSecrets where !referencedSecrets.contains(n) {
            issues.append(.warning(
                "declared secret '\(n)' is never referenced",
                "secrets"
            ))
        }

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
                    validateTemplate(s.request.url, transforms: transforms, knownVars: { varInScope($0) || stepNames.contains($0) }, knownInputs: inputNames, knownTypes: typeNames, knownStepNames: stepNames, location: "step \(s.name).url", issues: &issues)
                    if let body = s.request.body { validateBody(body, transforms: transforms, location: "step \(s.name).body", knownVars: varInScope, knownInputs: inputNames, knownTypes: typeNames, knownStepNames: stepNames, issues: &issues) }
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
        case .current, .input, .secret: return
        case .variable(let n):
            if !knownVars(n) { issues.append(.error("\(location): unbound variable $\(n)", location)) }
        case .field(let inner, _), .optField(let inner, _), .index(let inner, _), .wildcard(let inner):
            validatePath(inner, knownVars: knownVars, knownInputs: knownInputs, location: location, issues: &issues)
        }
    }

    // MARK: - Secret collection

    /// Walk the recipe's auth block, step requests, and emit bindings to
    /// gather every `$secret.<name>` referenced anywhere. Used by validation
    /// to warn on undeclared secrets and unused declarations.
    private static func collectReferencedSecrets(recipe: Recipe) -> Set<String> {
        var out = Set<String>()
        if case .session(let s) = recipe.auth {
            switch s.kind {
            case .formLogin(let f):
                out.formUnion(secretsIn(template: f.url))
                out.formUnion(secretsIn(body: f.body))
            case .bearerLogin(let b):
                out.formUnion(secretsIn(template: b.url))
                out.formUnion(secretsIn(body: b.body))
                out.formUnion(b.tokenPath.referencedSecrets)
            case .cookiePersist(let c):
                out.formUnion(secretsIn(template: c.sourcePath))
            }
        }
        if case .staticHeader(_, let v) = recipe.auth {
            out.formUnion(secretsIn(template: v))
        }
        for stmt in recipe.body { collectInStatement(stmt, into: &out) }
        return out
    }

    private static func collectInStatement(_ stmt: Statement, into out: inout Set<String>) {
        switch stmt {
        case .step(let s):
            out.formUnion(secretsIn(template: s.request.url))
            for (_, hv) in s.request.headers { out.formUnion(secretsIn(template: hv)) }
            if let body = s.request.body { out.formUnion(secretsIn(body: body)) }
        case .emit(let em):
            for b in em.bindings { out.formUnion(secretsIn(expr: b.expr)) }
        case .forLoop(_, let coll, let body):
            out.formUnion(coll.referencedSecrets)
            for s in body { collectInStatement(s, into: &out) }
        }
    }

    private static func secretsIn(template t: Template) -> Set<String> {
        var out = Set<String>()
        for part in t.parts {
            if case .interp(let expr) = part {
                out.formUnion(secretsIn(expr: expr))
            }
        }
        return out
    }

    private static func secretsIn(body: HTTPBody) -> Set<String> {
        var out = Set<String>()
        switch body {
        case .jsonObject(let kvs):
            for kv in kvs { out.formUnion(secretsIn(bodyValue: kv.value)) }
        case .form(let kvs):
            for (_, v) in kvs { out.formUnion(secretsIn(bodyValue: v)) }
        case .raw(let t):
            out.formUnion(secretsIn(template: t))
        }
        return out
    }

    private static func secretsIn(bodyValue: BodyValue) -> Set<String> {
        switch bodyValue {
        case .templateString(let t): return secretsIn(template: t)
        case .literal: return []
        case .path(let p): return p.referencedSecrets
        case .object(let kvs):
            var out = Set<String>()
            for kv in kvs { out.formUnion(secretsIn(bodyValue: kv.value)) }
            return out
        case .array(let xs):
            var out = Set<String>()
            for v in xs { out.formUnion(secretsIn(bodyValue: v)) }
            return out
        case .caseOf(let scrutinee, let branches):
            var out = scrutinee.referencedSecrets
            for (_, v) in branches { out.formUnion(secretsIn(bodyValue: v)) }
            return out
        }
    }

    private static func secretsIn(expr: ExtractionExpr) -> Set<String> {
        switch expr {
        case .path(let p): return p.referencedSecrets
        case .pipe(let inner, let calls):
            var out = secretsIn(expr: inner)
            for c in calls { for a in c.args { out.formUnion(secretsIn(expr: a)) } }
            return out
        case .caseOf(let scrutinee, let branches):
            var out = scrutinee.referencedSecrets
            for (_, e) in branches { out.formUnion(secretsIn(expr: e)) }
            return out
        case .mapTo(let p, _): return p.referencedSecrets
        case .literal: return []
        case .template(let t): return secretsIn(template: t)
        case .call(_, let args):
            var out = Set<String>()
            for a in args { out.formUnion(secretsIn(expr: a)) }
            return out
        }
    }

    private static func validateTemplate(_ template: Template, transforms: TransformImpls, knownVars: (String) -> Bool, knownInputs: Set<String>, knownTypes: Set<String>, knownStepNames: Set<String>, location: String, issues: inout [ValidationIssue]) {
        for part in template.parts {
            if case .interp(let expr) = part {
                validateExtraction(expr, transforms: transforms, knownVars: knownVars, knownInputs: knownInputs, knownTypes: knownTypes, knownStepNames: knownStepNames, location: location, issues: &issues)
            }
        }
    }

    private static func validateBody(_ body: HTTPBody, transforms: TransformImpls, location: String, knownVars: (String) -> Bool, knownInputs: Set<String>, knownTypes: Set<String>, knownStepNames: Set<String>, issues: inout [ValidationIssue]) {
        switch body {
        case .jsonObject(let kvs):
            for kv in kvs { validateBodyValue(kv.value, transforms: transforms, location: "\(location).\(kv.key)", knownVars: knownVars, knownInputs: knownInputs, knownTypes: knownTypes, knownStepNames: knownStepNames, issues: &issues) }
        case .form(let kvs):
            for (k, v) in kvs { validateBodyValue(v, transforms: transforms, location: "\(location).\(k)", knownVars: knownVars, knownInputs: knownInputs, knownTypes: knownTypes, knownStepNames: knownStepNames, issues: &issues) }
        case .raw(let t):
            validateTemplate(t, transforms: transforms, knownVars: knownVars, knownInputs: knownInputs, knownTypes: knownTypes, knownStepNames: knownStepNames, location: location, issues: &issues)
        }
    }

    private static func validateBodyValue(_ bv: BodyValue, transforms: TransformImpls, location: String, knownVars: (String) -> Bool, knownInputs: Set<String>, knownTypes: Set<String>, knownStepNames: Set<String>, issues: inout [ValidationIssue]) {
        switch bv {
        case .templateString(let t):
            validateTemplate(t, transforms: transforms, knownVars: knownVars, knownInputs: knownInputs, knownTypes: knownTypes, knownStepNames: knownStepNames, location: location, issues: &issues)
        case .literal: break
        case .path(let p):
            validatePath(p, knownVars: knownVars, knownInputs: knownInputs, location: location, issues: &issues)
        case .object(let kvs):
            for kv in kvs { validateBodyValue(kv.value, transforms: transforms, location: "\(location).\(kv.key)", knownVars: knownVars, knownInputs: knownInputs, knownTypes: knownTypes, knownStepNames: knownStepNames, issues: &issues) }
        case .array(let xs):
            for v in xs { validateBodyValue(v, transforms: transforms, location: "\(location)[]", knownVars: knownVars, knownInputs: knownInputs, knownTypes: knownTypes, knownStepNames: knownStepNames, issues: &issues) }
        case .caseOf(let s, let branches):
            validatePath(s, knownVars: knownVars, knownInputs: knownInputs, location: "\(location).case", issues: &issues)
            for (_, v) in branches { validateBodyValue(v, transforms: transforms, location: location, knownVars: knownVars, knownInputs: knownInputs, knownTypes: knownTypes, knownStepNames: knownStepNames, issues: &issues) }
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
        case .literal: break
        case .template(let t):
            validateTemplate(t, transforms: transforms, knownVars: knownVars, knownInputs: knownInputs, knownTypes: knownTypes, knownStepNames: knownStepNames, location: location, issues: &issues)
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
