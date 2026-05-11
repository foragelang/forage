// Runtime scope, path resolver, template renderer, extraction evaluator.
// Ports Sources/Forage/Engine/{Scope,PathResolver,TemplateRenderer,ExtractionEvaluator}.swift.

import type {
    Emission,
    ExtractionExpr,
    JSONValue,
    PathExpr,
    Template,
} from './ast.js'
import { TransformImpls } from './transforms.js'

export class ScopeError extends Error {
    constructor(message: string) { super(message); this.name = 'ScopeError' }
}

export class EvaluationError extends Error {
    constructor(message: string) { super(message); this.name = 'EvaluationError' }
}

export class Scope {
    constructor(
        public readonly inputs: Record<string, JSONValue> = {},
        public readonly frames: Array<Record<string, JSONValue>> = [],
        public readonly current: JSONValue | null = null,
        public readonly secrets: Record<string, string> = {},
    ) {}

    with(name: string, value: JSONValue): Scope {
        const newFrames = this.frames.map(f => ({ ...f }))
        if (newFrames.length === 0) newFrames.push({})
        newFrames[newFrames.length - 1][name] = value
        return new Scope(this.inputs, newFrames, this.current, this.secrets)
    }

    pushed(): Scope {
        return new Scope(this.inputs, [...this.frames, {}], this.current, this.secrets)
    }

    withCurrent(value: JSONValue | null): Scope {
        return new Scope(this.inputs, this.frames, value, this.secrets)
    }

    withSecrets(secrets: Record<string, string>): Scope {
        return new Scope(this.inputs, this.frames, this.current, secrets)
    }

    variable(name: string): JSONValue | null {
        for (let i = this.frames.length - 1; i >= 0; i--) {
            if (name in this.frames[i]) return this.frames[i][name]
        }
        return null
    }
}

export function resolvePath(expr: PathExpr, scope: Scope): JSONValue {
    switch (expr.tag) {
        case 'current':
            if (scope.current === null) throw new ScopeError('no current value bound for $')
            return scope.current
        case 'input':
            return { tag: 'object', entries: scope.inputs }
        case 'secret': {
            const v = scope.secrets[expr.name]
            if (v === undefined) throw new ScopeError(`secret '${expr.name}' was not pre-resolved (was it declared and supplied?)`)
            return { tag: 'string', value: v }
        }
        case 'variable': {
            const v = scope.variable(expr.name)
            if (v === null) throw new ScopeError(`undefined variable $${expr.name}`)
            return v
        }
        case 'field': {
            const v = resolvePath(expr.base, scope)
            return fieldOf(v, expr.name) ?? { tag: 'null' }
        }
        case 'optField': {
            let v: JSONValue
            try { v = resolvePath(expr.base, scope) } catch { return { tag: 'null' } }
            return fieldOf(v, expr.name) ?? { tag: 'null' }
        }
        case 'index': {
            const v = resolvePath(expr.base, scope)
            return arrayIndex(v, expr.idx) ?? { tag: 'null' }
        }
        case 'wildcard': {
            const v = resolvePath(expr.base, scope)
            return arrayWidened(v)
        }
    }
}

function fieldOf(v: JSONValue, name: string): JSONValue | null {
    if (v.tag === 'object') return v.entries[name] ?? null
    return null
}

function arrayIndex(v: JSONValue, i: number): JSONValue | null {
    if (v.tag === 'array' && i >= 0 && i < v.items.length) return v.items[i]
    return null
}

function arrayWidened(v: JSONValue): JSONValue {
    if (v.tag === 'array') return v
    if (v.tag === 'null') return { tag: 'array', items: [] }
    return { tag: 'array', items: [v] }
}

export class ExtractionEvaluator {
    constructor(public readonly transforms: TransformImpls = new TransformImpls()) {}

    evaluate(expr: ExtractionExpr, scope: Scope): JSONValue {
        return this.evaluateToJSON(expr, scope)
    }

    evaluateToJSON(expr: ExtractionExpr, scope: Scope): JSONValue {
        switch (expr.tag) {
            case 'path': return resolvePath(expr.path, scope)
            case 'literal': return expr.value
            case 'template': return { tag: 'string', value: this.renderTemplate(expr.template, scope) }
            case 'pipe': {
                let v = this.evaluateToJSON(expr.inner, scope)
                for (const call of expr.calls) {
                    const args = call.args.map(a => this.evaluateToJSON(a, scope))
                    v = this.transforms.apply(call.name, v, args)
                }
                return v
            }
            case 'caseOf': {
                const label = this.resolveEnumLabel(expr.scrutinee, scope)
                for (const br of expr.branches) {
                    if (br.label === label) return this.evaluateToJSON(br.expr, scope)
                }
                throw new EvaluationError(`case-of: no branch matched '${label}' (available: ${expr.branches.map(b => b.label).join(', ')})`)
            }
            case 'mapTo': {
                const listValue = resolvePath(expr.path, scope)
                if (listValue.tag === 'null') return { tag: 'array', items: [] }
                if (listValue.tag !== 'array') throw new EvaluationError('map-to-record: path must resolve to a list')
                const out: JSONValue[] = []
                for (const item of listValue.items) {
                    const itemScope = scope.withCurrent(item)
                    const record = this.emit(expr.emission, itemScope)
                    out.push(record)
                }
                return { tag: 'array', items: out }
            }
            case 'call': {
                const args = expr.args.map(a => this.evaluateToJSON(a, scope))
                const value = args[0] ?? { tag: 'null' }
                const rest = args.slice(1)
                return this.transforms.apply(expr.name, value, rest)
            }
        }
    }

    emit(emission: Emission, scope: Scope): JSONValue {
        const entries: Record<string, JSONValue> = { _typeName: { tag: 'string', value: emission.typeName } }
        for (const binding of emission.bindings) {
            entries[binding.fieldName] = this.evaluate(binding.expr, scope)
        }
        return { tag: 'object', entries }
    }

    renderTemplate(t: Template, scope: Scope): string {
        let out = ''
        for (const part of t.parts) {
            if (part.tag === 'literal') {
                out += part.value
            } else {
                const v = this.evaluateToJSON(part.expr, scope)
                out += stringifyJSON(v)
            }
        }
        return out
    }

    private resolveEnumLabel(p: PathExpr, scope: Scope): string {
        const v = resolvePath(p, scope)
        switch (v.tag) {
            case 'string': return v.value
            case 'bool': return String(v.value)
            case 'int': return String(v.value)
            case 'double': return String(v.value)
            case 'null': return 'null'
            default: throw new EvaluationError(`case-of: scrutinee must resolve to a string enum label, got ${v.tag}`)
        }
    }
}

export function stringifyJSON(v: JSONValue): string {
    switch (v.tag) {
        case 'null': return ''
        case 'bool': return String(v.value)
        case 'int': return String(v.value)
        case 'double':
            if (v.value === Math.trunc(v.value) && Math.abs(v.value) < 1e15) return String(Math.trunc(v.value))
            return String(v.value)
        case 'string': return v.value
        case 'array': case 'object':
            try { return JSON.stringify(toRaw(v)) } catch { return '' }
    }
}

function toRaw(v: JSONValue): unknown {
    switch (v.tag) {
        case 'null': return null
        case 'bool': return v.value
        case 'int': return v.value
        case 'double': return v.value
        case 'string': return v.value
        case 'array': return v.items.map(toRaw)
        case 'object': {
            const obj: Record<string, unknown> = {}
            for (const [k, e] of Object.entries(v.entries)) obj[k] = toRaw(e)
            return obj
        }
    }
}
