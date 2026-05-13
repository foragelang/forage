import { describe, it, expect } from 'vitest'
import { Parser } from '../src/parser.js'
import { validate, hasErrors } from '../src/validator.js'

describe('validator', () => {
    it('passes a clean recipe', () => {
        const src = `recipe "ok"
engine http
type Item { id: String; name: String }
step list { method "GET"; url "https://x.test/items" }
for $i in $list[*] {
    emit Item { id ← $i.id | toString; name ← $i.name }
}`
        const recipe = Parser.parse(src)
        const issues = validate(recipe)
        expect(hasErrors(issues)).toBe(false)
    })

    it('catches unknown type in emit', () => {
        const src = `recipe "bad"
engine http
type Item { id: String }
step s { method "GET"; url "https://x.test/s" }
for $i in $s[*] {
    emit Widget { id ← $i.id }
}`
        const recipe = Parser.parse(src)
        const issues = validate(recipe)
        expect(hasErrors(issues)).toBe(true)
        expect(issues.some(i => i.message.includes("unknown type 'Widget'"))).toBe(true)
    })

    it('catches unknown transform', () => {
        const src = `recipe "bad"
engine http
type Item { id: String }
step s { method "GET"; url "https://x.test/s" }
for $i in $s[*] {
    emit Item { id ← $i.id | mysteriousTransform }
}`
        const recipe = Parser.parse(src)
        const issues = validate(recipe)
        expect(hasErrors(issues)).toBe(true)
        expect(issues.some(i => i.message.includes('mysteriousTransform'))).toBe(true)
    })

    it('catches unbound variable', () => {
        const src = `recipe "bad"
engine http
type Item { id: String }
step s { method "GET"; url "https://x.test/s" }
for $i in $s[*] {
    emit Item { id ← $unboundVar.x }
}`
        const recipe = Parser.parse(src)
        const issues = validate(recipe)
        expect(hasErrors(issues)).toBe(true)
        expect(issues.some(i => i.message.includes('unbound variable'))).toBe(true)
    })

    it('warns about unbound required field', () => {
        const src = `recipe "warn"
engine http
type Item { id: String; name: String }
step s { method "GET"; url "https://x.test/s" }
for $i in $s[*] {
    emit Item { id ← $i.id }
}`
        const recipe = Parser.parse(src)
        const issues = validate(recipe)
        expect(issues.some(i => i.severity === 'warning' && i.message.includes('name'))).toBe(true)
    })
})
