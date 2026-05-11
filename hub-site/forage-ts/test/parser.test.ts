import { describe, it, expect } from 'vitest'
import { Parser } from '../src/parser.js'

describe('parser', () => {
    it('parses a minimal recipe', () => {
        const src = `recipe "minimal" {
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
        const src = `recipe "pagn" {
            engine http
            step a {
                method "GET"; url "https://x.test/a"
                paginate pageWithTotal { items: $.list; total: $.total; pageParam: "page"; pageSize: 200 }
            }
            step b {
                method "GET"; url "https://x.test/b"
                paginate untilEmpty { items: $.data; pageParam: "n" }
            }
        }`
        const recipe = Parser.parse(src)
        expect(recipe.body).toHaveLength(2)
        const a = recipe.body[0]
        expect(a.tag).toBe('step')
        if (a.tag !== 'step') throw new Error('expected step')
        expect(a.step.pagination?.tag).toBe('pageWithTotal')
    })

    it('parses case-of in body and extraction', () => {
        const src = `recipe "c" {
            engine http
            enum Mode { A, B }
            input m: Mode
            step s {
                method "POST"; url "https://x.test/s"
                body.json {
                    sale: case $input.m of { A → "A"; B → "B" }
                }
            }
        }`
        const recipe = Parser.parse(src)
        expect(recipe.enums[0].variants).toEqual(['A', 'B'])
    })

    it('parses pipelines and function calls', () => {
        const src = `recipe "p" {
            engine http
            type X { v: String }
            step s { method "GET"; url "https://x.test/s" }
            for $i in $s[*] {
                emit X { v ← coalesce($i.a, $i.b) | trim }
            }
        }`
        const recipe = Parser.parse(src)
        expect(recipe.body[1].tag).toBe('forLoop')
    })

    it('parses imports', () => {
        const src = `import hub://alice/awesome v3
recipe "x" {
    engine http
    step s { method "GET"; url "https://x.test/s" }
}`
        const recipe = Parser.parse(src)
        expect(recipe.imports).toHaveLength(1)
        expect(recipe.imports[0].slug).toBe('alice/awesome')
        expect(recipe.imports[0].version).toBe(3)
    })

    it('throws on broken syntax', () => {
        expect(() => Parser.parse(`recipe "x" { engine zorp }`)).toThrow()
    })

    it('parses expectations', () => {
        const src = `recipe "x" {
            engine http
            type T { v: String }
            step s { method "GET"; url "https://x.test/s" }
            expect { records.where(typeName == "T").count >= 100 }
        }`
        const recipe = Parser.parse(src)
        expect(recipe.expectations).toHaveLength(1)
        const k = recipe.expectations[0].kind
        expect(k.tag).toBe('recordCount')
        expect(k.typeName).toBe('T')
        expect(k.op).toBe('>=')
        expect(k.value).toBe(100)
    })
})
