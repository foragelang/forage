import { describe, it, expect } from 'vitest'
import { TransformImpls } from '../src/transforms.js'
import type { JSONValue } from '../src/ast.js'
import { Parser } from '../src/parser.js'
import { validate } from '../src/validator.js'
import { run as runRecipe } from '../src/runner.js'

const t = new TransformImpls()

describe('html extraction primitives', () => {
    it('parseHtml produces a node', () => {
        const result = t.apply('parseHtml', { tag: 'string', value: '<p>hi</p>' }, [])
        expect(result.tag).toBe('node')
    })

    it('parseHtml passes non-string through', () => {
        const result = t.apply('parseHtml', { tag: 'int', value: 42 }, [])
        expect(result).toEqual({ tag: 'int', value: 42 })
    })

    it('parseJson round-trips an object', () => {
        // Avoid 0/1 to match the Swift port's NSNumber-to-Bool quirk
        // (parity matters: both ports normalize 0/1 to bool).
        const result = t.apply('parseJson', { tag: 'string', value: '{"x":42,"y":[10,20]}' }, []) as JSONValue
        expect(result.tag).toBe('object')
        if (result.tag !== 'object') return
        expect(result.entries['x']).toEqual({ tag: 'int', value: 42 })
    })

    it('parseJson returns null on malformed', () => {
        const result = t.apply('parseJson', { tag: 'string', value: 'not json' }, [])
        expect(result).toEqual({ tag: 'null' })
    })

    it('select returns matching nodes as an array', () => {
        const doc = t.apply('parseHtml', {
            tag: 'string',
            value: '<ul><li class="a">1</li><li class="a">2</li><li class="b">3</li></ul>',
        }, [])
        const result = t.apply('select', doc, [{ tag: 'string', value: 'li.a' }]) as JSONValue
        expect(result.tag).toBe('array')
        if (result.tag !== 'array') return
        expect(result.items.length).toBe(2)
        for (const item of result.items) expect(item.tag).toBe('node')
    })

    it('text extracts text from a node', () => {
        const doc = t.apply('parseHtml', { tag: 'string', value: '<p>hello <b>world</b></p>' }, [])
        const p = t.apply('select', doc, [{ tag: 'string', value: 'p' }])
        expect(t.apply('text', p, [])).toEqual({ tag: 'string', value: 'hello world' })
    })

    it('text auto-flattens a single-element array (jQuery convention)', () => {
        const doc = t.apply('parseHtml', { tag: 'string', value: '<h1>Title</h1>' }, [])
        const pipeline = t.apply('select', doc, [{ tag: 'string', value: 'h1' }])
        expect(t.apply('text', pipeline, [])).toEqual({ tag: 'string', value: 'Title' })
    })

    it('text on empty array returns null', () => {
        expect(t.apply('text', { tag: 'array', items: [] }, [])).toEqual({ tag: 'null' })
    })

    it('attr returns attribute value', () => {
        const doc = t.apply('parseHtml', { tag: 'string', value: '<a href="/x">link</a>' }, [])
        const a = t.apply('select', doc, [{ tag: 'string', value: 'a' }])
        expect(t.apply('attr', a, [{ tag: 'string', value: 'href' }])).toEqual({ tag: 'string', value: '/x' })
    })

    it('attr returns null for missing attribute', () => {
        const doc = t.apply('parseHtml', { tag: 'string', value: '<a>link</a>' }, [])
        const a = t.apply('select', doc, [{ tag: 'string', value: 'a' }])
        expect(t.apply('attr', a, [{ tag: 'string', value: 'href' }])).toEqual({ tag: 'null' })
    })

    it('first picks head of array', () => {
        expect(t.apply('first', { tag: 'array', items: [{ tag: 'int', value: 1 }, { tag: 'int', value: 2 }] }, []))
            .toEqual({ tag: 'int', value: 1 })
    })

    it('first on empty array returns null', () => {
        expect(t.apply('first', { tag: 'array', items: [] }, [])).toEqual({ tag: 'null' })
    })
})

describe('html extraction end-to-end', () => {
    it('iterates HTML via for-loop with pipe-driven collection', async () => {
        const source = `
            recipe "html-listings" {
                engine http

                type Item {
                    title: String
                    url:   String?
                }

                input page: String

                step fetch {
                    method "GET"
                    url    "{$input.page}"
                }

                for $li in $fetch | parseHtml | select("li.story") {
                    emit Item {
                        title ← $li | select("a") | text
                        url   ← $li | select("a") | attr("href")
                    }
                }
            }
        `
        const recipe = Parser.parse(source)
        const issues = validate(recipe)
        expect(issues.filter(i => i.severity === 'error')).toHaveLength(0)

        const html = `
            <ul>
              <li class="story"><a href="/a">Story A</a></li>
              <li class="story"><a href="/b">Story B</a></li>
              <li class="other"><a href="/c">Other</a></li>
            </ul>
        `
        const fetchImpl: any = async () =>
            new Response(html, { status: 200, headers: { 'Content-Type': 'text/html' } })

        const result = await runRecipe(recipe, { page: 'http://example.com/' }, { fetch: fetchImpl })

        expect(result.diagnostic.stallReason).toBe('completed')
        expect(result.records).toHaveLength(2)
        expect(result.records[0]).toMatchObject({
            typeName: 'Item',
            fields: { title: 'Story A', url: '/a' },
        })
        expect(result.records[1]).toMatchObject({
            typeName: 'Item',
            fields: { title: 'Story B', url: '/b' },
        })
    })
})
