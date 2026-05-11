// Validator — port of Sources/Forage/Validation/Validator.swift.

import type {
    BodyValue,
    ExtractionExpr,
    FieldType,
    HTTPBody,
    Pagination,
    PathExpr,
    Recipe,
    Statement,
    Template,
} from './ast.js'
import { TransformImpls } from './transforms.js'

export interface ValidationIssue {
    severity: 'error' | 'warning'
    message: string
    location: string
}

export function validate(recipe: Recipe): ValidationIssue[] {
    const issues: ValidationIssue[] = []
    const typeNames = new Set(recipe.types.map(t => t.name))
    const enumNames = new Set(recipe.enums.map(e => e.name))
    const topLevelNames = new Set(recipe.inputs.map(i => i.name))
    if (recipe.auth?.tag === 'htmlPrime') {
        for (const v of recipe.auth.capturedVars) topLevelNames.add(v.varName)
    }
    const transforms = new TransformImpls()

    // 1. Type field references resolve.
    for (const type of recipe.types) {
        for (const field of type.fields) {
            checkFieldType(field.type, typeNames, enumNames, `type ${type.name}.${field.name}`, issues)
        }
    }
    // 2. Inputs reference real types/enums.
    for (const input of recipe.inputs) {
        checkFieldType(input.type, typeNames, enumNames, `input ${input.name}`, issues)
    }

    // 3. Statement walker.
    const stepNames = new Set<string>()
    const varStack: Set<string>[] = [new Set(topLevelNames)]

    const varInScope = (name: string): boolean => {
        for (let i = varStack.length - 1; i >= 0; i--) {
            if (varStack[i].has(name)) return true
        }
        return false
    }

    const collectStepNames = (stmts: Statement[]): void => {
        for (const s of stmts) {
            switch (s.tag) {
                case 'step':
                    if (stepNames.has(s.step.name)) {
                        issues.push(error(`duplicate step name '${s.step.name}'`, 'step decl'))
                    }
                    stepNames.add(s.step.name)
                    break
                case 'forLoop': collectStepNames(s.body); break
                case 'emit': break
            }
        }
    }
    collectStepNames(recipe.body)

    const walk = (stmts: Statement[]): void => {
        for (const stmt of stmts) {
            switch (stmt.tag) {
                case 'step': {
                    const s = stmt.step
                    validateTemplate(
                        s.request.url, transforms,
                        n => varInScope(n) || stepNames.has(n),
                        topLevelNames, typeNames, stepNames,
                        `step ${s.name}.url`, issues,
                    )
                    if (s.request.body) {
                        validateBody(
                            s.request.body, transforms,
                            `step ${s.name}.body`, varInScope,
                            topLevelNames, typeNames, stepNames, issues,
                        )
                    }
                    if (s.pagination) validatePagination(s.pagination, `step ${s.name}.paginate`, issues)
                    break
                }
                case 'emit': {
                    const em = stmt.emission
                    if (!typeNames.has(em.typeName)) {
                        issues.push(error(`emit references unknown type '${em.typeName}'`, 'emit'))
                        break
                    }
                    const recipeType = recipe.types.find(t => t.name === em.typeName)!
                    const typeFields = new Set(recipeType.fields.map(f => f.name))
                    const boundFields = new Set(em.bindings.map(b => b.fieldName))
                    for (const fb of em.bindings) {
                        if (!typeFields.has(fb.fieldName)) {
                            issues.push(error(`emit ${em.typeName}.${fb.fieldName}: unknown field on type`, 'emit'))
                        }
                    }
                    for (const f of recipeType.fields) {
                        if (!f.optional && !boundFields.has(f.name)) {
                            issues.push(warning(`emit ${em.typeName} doesn't bind required field '${f.name}'`, 'emit'))
                        }
                    }
                    for (const fb of em.bindings) {
                        validateExtraction(
                            fb.expr, transforms, varInScope,
                            topLevelNames, typeNames, stepNames,
                            `emit ${em.typeName}.${fb.fieldName}`, issues,
                        )
                    }
                    break
                }
                case 'forLoop': {
                    validatePath(
                        stmt.collection,
                        n => varInScope(n) || stepNames.has(n),
                        topLevelNames, `for ${stmt.variable} in <coll>`, issues,
                    )
                    const frame = new Set<string>()
                    frame.add(stmt.variable)
                    varStack.push(frame)
                    walk(stmt.body)
                    varStack.pop()
                    break
                }
            }
        }
    }
    walk(recipe.body)

    return issues
}

function checkFieldType(
    type: FieldType,
    knownTypes: Set<string>,
    knownEnums: Set<string>,
    location: string,
    issues: ValidationIssue[],
): void {
    switch (type.tag) {
        case 'string': case 'int': case 'double': case 'bool': return
        case 'array': return checkFieldType(type.element, knownTypes, knownEnums, location, issues)
        case 'record':
            if (!knownTypes.has(type.name) && !knownEnums.has(type.name)) {
                issues.push(error(`${location}: unknown type/enum '${type.name}'`, location))
            }
            return
        case 'enumRef':
            if (!knownEnums.has(type.name)) {
                issues.push(error(`${location}: unknown enum '${type.name}'`, location))
            }
            return
    }
}

function validatePagination(p: Pagination, location: string, issues: ValidationIssue[]): void {
    if (p.tag === 'pageWithTotal' && p.pageSize <= 0) {
        issues.push(error(`${location}: pageSize must be > 0`, location))
    }
}

function validatePath(
    p: PathExpr,
    knownVars: (n: string) => boolean,
    _knownInputs: Set<string>,
    location: string,
    issues: ValidationIssue[],
): void {
    switch (p.tag) {
        case 'current': case 'input': return
        case 'variable':
            if (!knownVars(p.name)) issues.push(error(`${location}: unbound variable $${p.name}`, location))
            return
        case 'field': case 'optField':
            validatePath(p.base, knownVars, _knownInputs, location, issues); return
        case 'index': case 'wildcard':
            validatePath(p.base, knownVars, _knownInputs, location, issues); return
    }
}

function validateTemplate(
    t: Template,
    transforms: TransformImpls,
    knownVars: (n: string) => boolean,
    knownInputs: Set<string>,
    knownTypes: Set<string>,
    knownStepNames: Set<string>,
    location: string,
    issues: ValidationIssue[],
): void {
    for (const part of t.parts) {
        if (part.tag === 'interp') {
            validateExtraction(part.expr, transforms, knownVars, knownInputs, knownTypes, knownStepNames, location, issues)
        }
    }
}

function validateBody(
    body: HTTPBody,
    transforms: TransformImpls,
    location: string,
    knownVars: (n: string) => boolean,
    knownInputs: Set<string>,
    knownTypes: Set<string>,
    knownStepNames: Set<string>,
    issues: ValidationIssue[],
): void {
    switch (body.tag) {
        case 'jsonObject':
            for (const kv of body.entries) {
                validateBodyValue(kv.value, transforms, `${location}.${kv.key}`, knownVars, knownInputs, knownTypes, knownStepNames, issues)
            }
            return
        case 'form':
            for (const kv of body.entries) {
                validateBodyValue(kv.value, transforms, `${location}.${kv.key}`, knownVars, knownInputs, knownTypes, knownStepNames, issues)
            }
            return
        case 'raw':
            validateTemplate(body.template, transforms, knownVars, knownInputs, knownTypes, knownStepNames, location, issues)
            return
    }
}

function validateBodyValue(
    bv: BodyValue,
    transforms: TransformImpls,
    location: string,
    knownVars: (n: string) => boolean,
    knownInputs: Set<string>,
    knownTypes: Set<string>,
    knownStepNames: Set<string>,
    issues: ValidationIssue[],
): void {
    switch (bv.tag) {
        case 'templateString':
            validateTemplate(bv.template, transforms, knownVars, knownInputs, knownTypes, knownStepNames, location, issues); return
        case 'literal': return
        case 'path':
            validatePath(bv.path, knownVars, knownInputs, location, issues); return
        case 'object':
            for (const kv of bv.entries) {
                validateBodyValue(kv.value, transforms, `${location}.${kv.key}`, knownVars, knownInputs, knownTypes, knownStepNames, issues)
            }
            return
        case 'array':
            for (const v of bv.items) {
                validateBodyValue(v, transforms, `${location}[]`, knownVars, knownInputs, knownTypes, knownStepNames, issues)
            }
            return
        case 'caseOf':
            validatePath(bv.scrutinee, knownVars, knownInputs, `${location}.case`, issues)
            for (const br of bv.branches) {
                validateBodyValue(br.value, transforms, location, knownVars, knownInputs, knownTypes, knownStepNames, issues)
            }
            return
    }
}

function validateExtraction(
    expr: ExtractionExpr,
    transforms: TransformImpls,
    knownVars: (n: string) => boolean,
    knownInputs: Set<string>,
    knownTypes: Set<string>,
    knownStepNames: Set<string>,
    location: string,
    issues: ValidationIssue[],
): void {
    switch (expr.tag) {
        case 'path':
            validatePath(expr.path, n => knownVars(n) || knownStepNames.has(n), knownInputs, location, issues); return
        case 'pipe':
            validateExtraction(expr.inner, transforms, knownVars, knownInputs, knownTypes, knownStepNames, location, issues)
            for (const c of expr.calls) {
                if (!transforms.has(c.name)) {
                    issues.push(error(`${location}: unknown transform '${c.name}'`, location))
                }
                for (const a of c.args) {
                    validateExtraction(a, transforms, knownVars, knownInputs, knownTypes, knownStepNames, location, issues)
                }
            }
            return
        case 'caseOf':
            validatePath(expr.scrutinee, n => knownVars(n) || knownStepNames.has(n), knownInputs, `${location}.case`, issues)
            for (const br of expr.branches) {
                validateExtraction(br.expr, transforms, knownVars, knownInputs, knownTypes, knownStepNames, location, issues)
            }
            return
        case 'mapTo':
            validatePath(expr.path, n => knownVars(n) || knownStepNames.has(n), knownInputs, location, issues)
            if (!knownTypes.has(expr.emission.typeName)) {
                issues.push(error(`${location}: map-to references unknown type '${expr.emission.typeName}'`, location))
            }
            return
        case 'literal': return
        case 'template':
            validateTemplate(expr.template, transforms, knownVars, knownInputs, knownTypes, knownStepNames, location, issues); return
        case 'call':
            if (!transforms.has(expr.name)) {
                issues.push(error(`${location}: unknown transform '${expr.name}'`, location))
            }
            for (const a of expr.args) {
                validateExtraction(a, transforms, knownVars, knownInputs, knownTypes, knownStepNames, location, issues)
            }
            return
    }
}

function error(message: string, location: string): ValidationIssue {
    return { severity: 'error', message, location }
}

function warning(message: string, location: string): ValidationIssue {
    return { severity: 'warning', message, location }
}

export function hasErrors(issues: ValidationIssue[]): boolean {
    return issues.some(i => i.severity === 'error')
}
