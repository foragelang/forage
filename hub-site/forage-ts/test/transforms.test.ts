import { describe, it, expect } from 'vitest'
import { TransformImpls } from '../src/transforms.js'
import type { JSONValue } from '../src/ast.js'

const t = new TransformImpls()

describe('transforms', () => {
    it('toString coerces primitives', () => {
        expect(t.apply('toString', { tag: 'int', value: 42 }, [])).toEqual({ tag: 'string', value: '42' })
        expect(t.apply('toString', { tag: 'bool', value: true }, [])).toEqual({ tag: 'string', value: 'true' })
        expect(t.apply('toString', { tag: 'null' }, [])).toEqual({ tag: 'null' })
    })

    it('parseSize parses suffixes', () => {
        const r = t.apply('parseSize', { tag: 'string', value: '3.5g' }, []) as JSONValue
        expect(r.tag).toBe('object')
        if (r.tag !== 'object') return
        expect(r.entries['value']).toEqual({ tag: 'double', value: 3.5 })
        expect(r.entries['unit']).toEqual({ tag: 'string', value: 'G' })
    })

    it('parseJaneWeight maps named weights', () => {
        expect(t.apply('parseJaneWeight', { tag: 'string', value: 'eighth ounce' }, [])).toEqual({ tag: 'double', value: 3.5 })
        expect(t.apply('parseJaneWeight', { tag: 'string', value: 'gram' }, [])).toEqual({ tag: 'double', value: 1 })
        expect(t.apply('parseJaneWeight', { tag: 'string', value: 'each' }, [])).toEqual({ tag: 'null' })
    })

    it('coalesce picks first non-null', () => {
        expect(t.apply('coalesce', { tag: 'null' }, [{ tag: 'string', value: 'a' }, { tag: 'string', value: 'b' }]))
            .toEqual({ tag: 'string', value: 'a' })
        expect(t.apply('coalesce', { tag: 'string', value: 'x' }, [{ tag: 'string', value: 'y' }]))
            .toEqual({ tag: 'string', value: 'x' })
    })

    it('dedup removes duplicates', () => {
        const r = t.apply('dedup', {
            tag: 'array',
            items: [{ tag: 'string', value: 'a' }, { tag: 'string', value: 'b' }, { tag: 'string', value: 'a' }],
        }, [])
        expect(r).toEqual({
            tag: 'array',
            items: [{ tag: 'string', value: 'a' }, { tag: 'string', value: 'b' }],
        })
    })

    it('prevalenceNormalize handles INDICA / NOT_APPLICABLE', () => {
        expect(t.apply('prevalenceNormalize', { tag: 'string', value: 'INDICA' }, []))
            .toEqual({ tag: 'string', value: 'Indica' })
        expect(t.apply('prevalenceNormalize', { tag: 'string', value: 'NOT_APPLICABLE' }, []))
            .toEqual({ tag: 'null' })
    })

    it('normalizeOzToGrams multiplies ounces', () => {
        const oz: JSONValue = {
            tag: 'object',
            entries: {
                value: { tag: 'double', value: 1 },
                unit: { tag: 'string', value: 'OZ' },
            },
        }
        const r = t.apply('normalizeOzToGrams', oz, []) as JSONValue
        expect(r.tag).toBe('object')
        if (r.tag !== 'object') return
        expect(r.entries['value']).toEqual({ tag: 'double', value: 28 })
        expect(r.entries['unit']).toEqual({ tag: 'string', value: 'G' })
    })
})
