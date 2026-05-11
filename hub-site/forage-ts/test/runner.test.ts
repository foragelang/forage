import { describe, it, expect } from 'vitest'
import { Parser } from '../src/parser.js'
import { run } from '../src/runner.js'
import type { FetchLike } from '../src/runner.js'

describe('runner', () => {
    it('runs a one-step recipe end to end', async () => {
        const fakeFetch: FetchLike = async (_url, _init) => {
            const body = JSON.stringify({
                hits: [
                    { title: 'Foo', url: 'https://a', points: 100, author: 'a', num_comments: 1 },
                    { title: 'Bar', url: 'https://b', points: 50, author: 'b', num_comments: 2 },
                ],
            })
            return new Response(body, { status: 200, headers: { 'Content-Type': 'application/json' } })
        }
        const src = `recipe "hn-mini" {
            engine http
            type Story { title: String; url: String?; points: Int; author: String; comments: Int }
            step front { method "GET"; url "https://hn.algolia.com/api/v1/search?tags=front_page" }
            for $hit in $front.hits[*] {
                emit Story {
                    title    ← $hit.title
                    url      ← $hit.url
                    points   ← $hit.points
                    author   ← $hit.author
                    comments ← $hit.num_comments
                }
            }
        }`
        const recipe = Parser.parse(src)
        const result = await run(recipe, {}, { fetch: fakeFetch })
        expect(result.diagnostic.stallReason).toBe('completed')
        expect(result.records).toHaveLength(2)
        expect(result.records[0].typeName).toBe('Story')
        expect(result.records[0].fields.title).toBe('Foo')
        expect(result.records[1].fields.points).toBe(50)
    })

    it('uses static-header auth', async () => {
        let captured: Headers | undefined
        const fakeFetch: FetchLike = async (_url, init) => {
            captured = new Headers((init.headers as Record<string, string>) ?? {})
            return new Response(JSON.stringify({ list: [], total: 0 }), { status: 200 })
        }
        const src = `recipe "auth" {
            engine http
            type X { v: String }
            input storeId: String
            auth.staticHeader { name: "storeId"; value: "{$input.storeId}" }
            step list {
                method "GET"
                url "https://x.test/list"
                paginate pageWithTotal { items: $.list; total: $.total; pageParam: "page"; pageSize: 50 }
            }
        }`
        const recipe = Parser.parse(src)
        await run(recipe, { storeId: 'abc123' }, { fetch: fakeFetch })
        expect(captured?.get('storeId')).toBe('abc123')
    })

    it('reports failed expectations', async () => {
        const fakeFetch: FetchLike = async () =>
            new Response(JSON.stringify({ hits: [] }), { status: 200 })
        const src = `recipe "fail-expect" {
            engine http
            type Story { title: String }
            step s { method "GET"; url "https://x.test/s" }
            for $h in $s.hits[*] {
                emit Story { title ← $h.title }
            }
            expect { records.where(typeName == "Story").count >= 1 }
        }`
        const recipe = Parser.parse(src)
        const result = await run(recipe, {}, { fetch: fakeFetch })
        expect(result.diagnostic.unmetExpectations).toHaveLength(1)
    })

    it('rejects browser-engine recipes', async () => {
        const src = `recipe "b" {
            engine browser
            browser { initialURL: "https://x.test/" }
        }`
        const recipe = Parser.parse(src)
        const result = await run(recipe, {})
        expect(result.diagnostic.stallReason).toContain('browser-engine')
    })
})
