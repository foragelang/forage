// Transform vocabulary — port of Sources/Forage/Engine/TransformImpls.swift.

import type { JSONValue } from './ast.js'

export type TransformImpl = (value: JSONValue, args: JSONValue[]) => JSONValue

export class TransformError extends Error {
    constructor(message: string) {
        super(message)
        this.name = 'TransformError'
    }
}

export class TransformImpls {
    private readonly registry = new Map<string, TransformImpl>()

    constructor() {
        this.registerDefaults()
    }

    has(name: string): boolean {
        return this.registry.has(name)
    }

    apply(name: string, value: JSONValue, args: JSONValue[]): JSONValue {
        const impl = this.registry.get(name)
        if (!impl) throw new TransformError(`unknown transform '${name}'`)
        return impl(value, args)
    }

    private register(name: string, impl: TransformImpl): void {
        this.registry.set(name, impl)
    }

    private registerDefaults(): void {
        // ---- Type coercion ----
        this.register('toString', (v) => {
            switch (v.tag) {
                case 'null': return { tag: 'null' }
                case 'bool': return { tag: 'string', value: String(v.value) }
                case 'int': return { tag: 'string', value: String(v.value) }
                case 'double': return { tag: 'string', value: String(v.value) }
                case 'string': return v
                default: return { tag: 'string', value: jsonToString(v) }
            }
        })
        this.register('parseInt', (v) => {
            if (v.tag === 'string') {
                const i = parseInt(v.value, 10)
                if (!isNaN(i) && String(i) === v.value.trim()) return { tag: 'int', value: i }
            }
            if (v.tag === 'double') return { tag: 'int', value: Math.trunc(v.value) }
            if (v.tag === 'int') return v
            return { tag: 'null' }
        })
        this.register('parseFloat', (v) => {
            if (v.tag === 'string') {
                const d = parseFloat(v.value)
                if (!isNaN(d)) return { tag: 'double', value: d }
            }
            if (v.tag === 'int') return { tag: 'double', value: v.value }
            if (v.tag === 'double') return v
            return { tag: 'null' }
        })
        this.register('parseBool', (v) => {
            if (v.tag === 'string') {
                const s = v.value.toLowerCase()
                if (s === 'true' || s === 'yes' || s === '1') return { tag: 'bool', value: true }
                if (s === 'false' || s === 'no' || s === '0') return { tag: 'bool', value: false }
                return { tag: 'null' }
            }
            if (v.tag === 'bool') return v
            return { tag: 'null' }
        })

        // ---- String ----
        this.register('lower', (v) => v.tag === 'string' ? { tag: 'string', value: v.value.toLowerCase() } : v)
        this.register('upper', (v) => v.tag === 'string' ? { tag: 'string', value: v.value.toUpperCase() } : v)
        this.register('trim', (v) => v.tag === 'string' ? { tag: 'string', value: v.value.trim() } : v)
        this.register('capitalize', (v) => v.tag === 'string' ? { tag: 'string', value: titleCase(v.value) } : v)
        this.register('titleCase', (v) => v.tag === 'string' ? { tag: 'string', value: titleCase(v.value) } : v)

        // ---- Array ----
        this.register('length', (v) => {
            if (v.tag === 'array') return { tag: 'int', value: v.items.length }
            if (v.tag === 'string') return { tag: 'int', value: v.value.length }
            if (v.tag === 'null') return { tag: 'int', value: 0 }
            return { tag: 'int', value: 1 }
        })
        this.register('dedup', (v) => {
            if (v.tag !== 'array') return v
            const seen: JSONValue[] = []
            for (const x of v.items) {
                if (!seen.some(s => jsonEquals(s, x))) seen.push(x)
            }
            return { tag: 'array', items: seen }
        })

        // ---- Cannabis-domain helpers ----
        this.register('parseSize', (v) => {
            if (v.tag !== 'string') return { tag: 'null' }
            return parseSizeString(v.value)
        })
        this.register('normalizeOzToGrams', (v) => {
            if (v.tag !== 'object') return v
            const value = v.entries['value']
            if (!value || value.tag !== 'double') return v
            const unitVal = v.entries['unit']
            const unit = unitVal && unitVal.tag === 'string' ? unitVal.value : ''
            if (unit.toUpperCase() === 'OZ') {
                return {
                    tag: 'object',
                    entries: {
                        value: { tag: 'double', value: value.value * 28 },
                        unit: { tag: 'string', value: 'G' },
                    },
                }
            }
            return v
        })
        this.register('sizeValue', (v) => {
            if (v.tag === 'object') return v.entries['value'] ?? { tag: 'null' }
            return { tag: 'null' }
        })
        this.register('sizeUnit', (v) => {
            if (v.tag === 'object') return v.entries['unit'] ?? { tag: 'null' }
            return { tag: 'null' }
        })
        this.register('normalizeUnitToGrams', (v) => {
            if (v.tag === 'string' && v.value.toUpperCase() === 'OZ') return { tag: 'string', value: 'G' }
            return v
        })
        this.register('prevalenceNormalize', (v) => {
            if (v.tag !== 'string' || v.value === '' || v.value === 'NOT_APPLICABLE') return { tag: 'null' }
            return { tag: 'string', value: titleCase(v.value) }
        })
        this.register('parseJaneWeight', (v) => {
            if (v.tag !== 'string') return { tag: 'null' }
            const s = v.value.toLowerCase()
            switch (s) {
                case 'half gram': return { tag: 'double', value: 0.5 }
                case 'gram': return { tag: 'double', value: 1.0 }
                case 'two gram': return { tag: 'double', value: 2.0 }
                case 'eighth ounce': return { tag: 'double', value: 3.5 }
                case 'quarter ounce': return { tag: 'double', value: 7.0 }
                case 'half ounce': return { tag: 'double', value: 14.0 }
                case 'ounce': return { tag: 'double', value: 28.0 }
                case 'each': return { tag: 'null' }
                default: {
                    const parsed = parseSizeString(v.value)
                    if (parsed.tag === 'object') {
                        const value = parsed.entries['value']
                        if (value && value.tag === 'double') return { tag: 'double', value: value.value }
                    }
                    const f = parseFloat(v.value)
                    return { tag: 'double', value: isNaN(f) ? 0 : f }
                }
            }
        })
        this.register('janeWeightUnit', (v) => {
            if (v.tag !== 'string') return { tag: 'null' }
            return v.value.toLowerCase() === 'each'
                ? { tag: 'string', value: 'EA' }
                : { tag: 'string', value: 'G' }
        })
        this.register('janeWeightKey', (v) => {
            if (v.tag !== 'string') return { tag: 'null' }
            return { tag: 'string', value: v.value.replace(/ /g, '_') }
        })

        // ---- Object / dynamic field ----
        this.register('getField', (v, args) => {
            if (args.length < 1) return { tag: 'null' }
            const keyArg = args[0]
            if (keyArg.tag !== 'string') return { tag: 'null' }
            if (v.tag === 'object') return v.entries[keyArg.value] ?? { tag: 'null' }
            return { tag: 'null' }
        })

        // ---- Coalesce / default ----
        this.register('coalesce', (v, args) => {
            if (v.tag !== 'null') return v
            for (const a of args) if (a.tag !== 'null') return a
            return { tag: 'null' }
        })
        this.register('default', (v, args) => {
            if (v.tag !== 'null') return v
            return args[0] ?? { tag: 'null' }
        })
    }
}

function parseSizeString(s: string): JSONValue {
    const re = /^([0-9]+(?:\.[0-9]+)?)\s*(g|mg|oz|ml)\b/i
    const m = re.exec(s)
    if (!m) return { tag: 'null' }
    const value = parseFloat(m[1])
    const unit = m[2].toUpperCase()
    return {
        tag: 'object',
        entries: {
            value: { tag: 'double', value },
            unit: { tag: 'string', value: unit },
        },
    }
}

function titleCase(s: string): string {
    // Swift's `String.capitalized` capitalizes the first letter of each
    // whitespace-separated word, lowercases the rest. We replicate that.
    return s.split(/(\s+)/).map(w => {
        if (w.length === 0) return w
        if (/^\s+$/.test(w)) return w
        return w[0].toUpperCase() + w.slice(1).toLowerCase()
    }).join('')
}

export function jsonEquals(a: JSONValue, b: JSONValue): boolean {
    if (a.tag !== b.tag) return false
    switch (a.tag) {
        case 'null': return true
        case 'bool': return a.value === (b as typeof a).value
        case 'int': return a.value === (b as typeof a).value
        case 'double': return a.value === (b as typeof a).value
        case 'string': return a.value === (b as typeof a).value
        case 'array': {
            const bi = (b as typeof a).items
            if (a.items.length !== bi.length) return false
            for (let i = 0; i < a.items.length; i++) if (!jsonEquals(a.items[i], bi[i])) return false
            return true
        }
        case 'object': {
            const be = (b as typeof a).entries
            const aKeys = Object.keys(a.entries)
            const bKeys = Object.keys(be)
            if (aKeys.length !== bKeys.length) return false
            for (const k of aKeys) if (!(k in be) || !jsonEquals(a.entries[k], be[k])) return false
            return true
        }
    }
}

function jsonToString(v: JSONValue): string {
    switch (v.tag) {
        case 'null': return 'null'
        case 'bool': return String(v.value)
        case 'int': return String(v.value)
        case 'double': return String(v.value)
        case 'string': return v.value
        case 'array': return JSON.stringify(toRawJSON(v))
        case 'object': return JSON.stringify(toRawJSON(v))
    }
}

export function toRawJSON(v: JSONValue): unknown {
    switch (v.tag) {
        case 'null': return null
        case 'bool': return v.value
        case 'int': return v.value
        case 'double': return v.value
        case 'string': return v.value
        case 'array': return v.items.map(toRawJSON)
        case 'object': {
            const obj: Record<string, unknown> = {}
            for (const [k, e] of Object.entries(v.entries)) obj[k] = toRawJSON(e)
            return obj
        }
    }
}

export function fromRawJSON(raw: unknown): JSONValue {
    if (raw === null || raw === undefined) return { tag: 'null' }
    if (typeof raw === 'boolean') return { tag: 'bool', value: raw }
    if (typeof raw === 'number') {
        if (Number.isInteger(raw)) return { tag: 'int', value: raw }
        return { tag: 'double', value: raw }
    }
    if (typeof raw === 'string') return { tag: 'string', value: raw }
    if (Array.isArray(raw)) return { tag: 'array', items: raw.map(fromRawJSON) }
    if (typeof raw === 'object') {
        const entries: Record<string, JSONValue> = {}
        for (const [k, v] of Object.entries(raw as Record<string, unknown>)) {
            entries[k] = fromRawJSON(v)
        }
        return { tag: 'object', entries }
    }
    return { tag: 'null' }
}
