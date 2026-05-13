import { describe, it, expect } from 'vitest'
import { Parser } from '../src/parser.js'

describe('parser', () => {
    it('parses a minimal recipe', () => {
        const src = `recipe "minimal"
engine http
type Item {
    id: String
    name: String
}
input baseUrl: String
step list {
    method "GET"
    url "{$input.baseUrl}/items"
}
for $item in $list {
    emit Item {
        id ← $item.id | toString
        name ← $item.name
    }
}`
        const recipe = Parser.parse(src)
        expect(recipe.name).toBe('minimal')
        expect(recipe.engineKind).toBe('http')
        expect(recipe.types).toHaveLength(1)
        expect(recipe.types[0].name).toBe('Item')
        expect(recipe.types[0].fields).toHaveLength(2)
        expect(recipe.inputs).toHaveLength(1)
        expect(recipe.body).toHaveLength(2)
        expect(recipe.body[0].tag).toBe('step')
        expect(recipe.body[1].tag).toBe('forLoop')
    })

    it('parses pagination strategies', () => {
        const src = `recipe "pagn"
engine http
step a {
    method "GET"; url "https://x.test/a"
    paginate pageWithTotal { items: $.list; total: $.total; pageParam: "page"; pageSize: 200 }
}
step b {
    method "GET"; url "https://x.test/b"
    paginate untilEmpty { items: $.data; pageParam: "n" }
}`
        const recipe = Parser.parse(src)
        expect(recipe.body).toHaveLength(2)
        const a = recipe.body[0]
        expect(a.tag).toBe('step')
        if (a.tag !== 'step') throw new Error('expected step')
        expect(a.step.pagination?.tag).toBe('pageWithTotal')
    })

    it('parses case-of in body and extraction', () => {
        const src = `recipe "c"
engine http
enum Mode { A, B }
input m: Mode
step s {
    method "POST"; url "https://x.test/s"
    body.json {
        sale: case $input.m of { A → "A"; B → "B" }
    }
}`
        const recipe = Parser.parse(src)
        expect(recipe.enums[0].variants).toEqual(['A', 'B'])
    })

    it('parses pipelines and function calls', () => {
        const src = `recipe "p"
engine http
type X { v: String }
step s { method "GET"; url "https://x.test/s" }
for $i in $s[*] {
    emit X { v ← coalesce($i.a, $i.b) | trim }
}`
        const recipe = Parser.parse(src)
        expect(recipe.body[1].tag).toBe('forLoop')
    })

    it('parses imports', () => {
        const src = `import alice/awesome v3
recipe "x"
engine http
step s { method "GET"; url "https://x.test/s" }`
        const recipe = Parser.parse(src)
        expect(recipe.imports).toHaveLength(1)
        const ref = recipe.imports[0]
        expect(ref.raw).toBe('alice/awesome')
        expect(ref.namespace).toBe('alice')
        expect(ref.name).toBe('awesome')
        expect(ref.registry).toBeNull()
        expect(ref.version).toBe(3)
    })

    it('parses bare-name imports (default namespace)', () => {
        const src = `import sweed

recipe "x"
engine http
step s { method "GET"; url "https://x.test/s" }`
        const recipe = Parser.parse(src)
        expect(recipe.imports).toHaveLength(1)
        const ref = recipe.imports[0]
        expect(ref.raw).toBe('sweed')
        expect(ref.registry).toBeNull()
        expect(ref.namespace).toBeNull()
        expect(ref.name).toBe('sweed')
        expect(ref.version).toBeNull()
    })

    it('parses custom-registry imports', () => {
        const src = `import hub.example.com/team/scraper v2

recipe "x"
engine http
step s { method "GET"; url "https://x.test/s" }`
        const recipe = Parser.parse(src)
        expect(recipe.imports).toHaveLength(1)
        const ref = recipe.imports[0]
        expect(ref.registry).toBe('hub.example.com')
        expect(ref.namespace).toBe('team')
        expect(ref.name).toBe('scraper')
        expect(ref.version).toBe(2)
    })

    it('throws on broken syntax', () => {
        expect(() => Parser.parse(`recipe "x"\nengine zorp`)).toThrow()
    })

    // Regression: legacy `recipe "X" { … }` block form must be rejected
    // explicitly, otherwise stale recipes would parse and silently produce
    // a recipe with an extra empty trailing `}` (or worse, parse fine
    // and only fail downstream).
    it('rejects legacy block syntax', () => {
        expect(() => Parser.parse(`recipe "x" {\n  engine http\n}`)).toThrow()
    })

    // Regression: a file may only declare one recipe. Two `recipe` headers
    // in one file is almost always copy-paste rot; refuse it loudly.
    it('rejects a second recipe header in the same file', () => {
        const src = `recipe "first"
engine http

recipe "second"
engine http`
        expect(() => Parser.parse(src)).toThrow(/only declare one recipe/)
    })

    it('parses expectations', () => {
        const src = `recipe "x"
engine http
type T { v: String }
step s { method "GET"; url "https://x.test/s" }
expect { records.where(typeName == "T").count >= 100 }`
        const recipe = Parser.parse(src)
        expect(recipe.expectations).toHaveLength(1)
        const k = recipe.expectations[0].kind
        expect(k.tag).toBe('recordCount')
        expect(k.typeName).toBe('T')
        expect(k.op).toBe('>=')
        expect(k.value).toBe(100)
    })
})
