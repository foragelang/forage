// User-defined function tests for the TS port — mirror the Rust
// `eval_smoke.rs` cases against the same engine surface.

import { describe, it, expect } from 'vitest'
import { Parser } from '../src/parser.js'
import { validate, hasErrors } from '../src/validator.js'
import { run } from '../src/runner.js'
import type { FetchLike } from '../src/runner.js'

describe('user-defined functions', () => {
    it('parses a fn declaration with one param', () => {
        const recipe = Parser.parse(`recipe "ok"
engine http
fn double($x) { $x }
type T { id: String }
step s { method "GET"; url "https://x.test" }
emit T { id ← "a" }`)
        expect(recipe.functions).toHaveLength(1)
        expect(recipe.functions[0].name).toBe('double')
        expect(recipe.functions[0].params).toEqual(['x'])
    })

    it('rejects a fn with a non-dollar parameter', () => {
        expect(() => Parser.parse(`recipe "bad"
engine http
fn nope(x) { $x }
type T { id: String }
step s { method "GET"; url "https://x.test" }
emit T { id ← "a" }`)).toThrow()
    })

    it('validates a user fn calling a built-in transform', () => {
        const recipe = Parser.parse(`recipe "ok"
engine http
fn shouty($x) { $x | upper }
type T { id: String }
step s { method "GET"; url "https://x.test" }
emit T { id ← "a" }`)
        expect(hasErrors(validate(recipe))).toBe(false)
    })

    it('flags wrong arity at a pipe call site', () => {
        const recipe = Parser.parse(`recipe "bad"
engine http
fn two($a, $b) { $a }
type T { id: String }
step s { method "GET"; url "https://x.test" }
for $x in $s[*] { emit T { id ← $x.id | two } }`)
        const issues = validate(recipe)
        expect(
            issues.some(i => i.severity === 'error' && i.message.includes("'two'")),
        ).toBe(true)
    })

    it('warns on direct recursion without erroring', () => {
        const recipe = Parser.parse(`recipe "ok"
engine http
fn loopy($x) { $x | loopy }
type T { id: String }
step s { method "GET"; url "https://x.test" }
emit T { id ← "a" }`)
        const issues = validate(recipe)
        expect(hasErrors(issues)).toBe(false)
        expect(
            issues.some(i => i.severity === 'warning' && i.message.includes('calls itself')),
        ).toBe(true)
    })

    it('runs a recipe that calls a zero-param fn directly', async () => {
        // `answer()` carries no head and no args; the body evaluates as
        // a literal. Regression for the eval bug that synthesized a
        // bogus head and rejected with `expects 0 arguments, got 1`.
        const fakeFetch: FetchLike = async () =>
            new Response(JSON.stringify({ items: [{}] }), { status: 200 })
        const src = `recipe "demo"
engine http
fn answer() { 42 }
type T { value: Int }
step list { method "GET"; url "https://x.test/items" }
for $i in $list.items[*] {
    emit T { value ← answer() }
}`
        const recipe = Parser.parse(src)
        const result = await run(recipe, {}, { fetch: fakeFetch })
        expect(result.diagnostic.stallReason).toBe('completed')
        expect(result.records.map(r => r.fields.value)).toEqual([42])
    })

    it('rejects a pipe call into a zero-param fn at validation time', () => {
        // A pipe always carries a head as param 0; a zero-param fn has
        // nowhere to bind it. Validator must catch this before eval.
        const recipe = Parser.parse(`recipe "bad"
engine http
fn answer() { 42 }
type T { id: Int }
step s { method "GET"; url "https://x.test" }
for $x in $s[*] { emit T { id ← $x.id | answer } }`)
        const issues = validate(recipe)
        expect(
            issues.some(
                i => i.severity === 'error'
                    && i.message.includes("'answer'")
                    && i.message.includes('0 arguments'),
            ),
        ).toBe(true)
    })

    it('runs a recipe that uses a user fn at a pipe site', async () => {
        const fakeFetch: FetchLike = async () =>
            new Response(JSON.stringify({ items: [{ id: 'abc' }, { id: 'def' }] }), { status: 200 })
        const src = `recipe "demo"
engine http
fn shout($x) { $x | upper }
type Item { id: String }
step list { method "GET"; url "https://x.test/items" }
for $i in $list.items[*] {
    emit Item { id ← $i.id | shout }
}`
        const recipe = Parser.parse(src)
        const result = await run(recipe, {}, { fetch: fakeFetch })
        expect(result.diagnostic.stallReason).toBe('completed')
        expect(result.records.map(r => r.fields.id)).toEqual(['ABC', 'DEF'])
    })

    it('a fn body cannot see the caller\'s for-loop variable', () => {
        const recipe = Parser.parse(`recipe "scoped"
engine http
fn leaky($x) { $item }
type T { id: String }
step list { method "GET"; url "https://x.test" }
for $item in $list[*] {
    emit T { id ← $item.id | leaky }
}`)
        const issues = validate(recipe)
        expect(
            issues.some(i => i.severity === 'error' && i.message.includes('item')),
        ).toBe(true)
    })
})
