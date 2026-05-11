import { describe, it, expect } from 'vitest'
import { Lexer } from '../src/lexer.js'

describe('lexer', () => {
    it('lexes keywords, literals, and operators', () => {
        const lex = new Lexer(`recipe "x" { engine http; type Foo { name: String?; age: Int }
            // comment
            emit Foo { name ← $.x; age ← 42 }
        }`)
        const toks = lex.tokenize()
        expect(toks.some(t => t.lexeme === 'recipe')).toBe(true)
        expect(toks.some(t => t.kind.tag === 'stringLit' && t.kind.value === 'x')).toBe(true)
        expect(toks.some(t => t.kind.tag === 'arrow')).toBe(true)
        expect(toks.some(t => t.kind.tag === 'question')).toBe(true)
    })

    it('lexes double, date, bool, null', () => {
        const lex = new Lexer('1.5 1990-01-01 true false null')
        const toks = lex.tokenize()
        expect(toks.some(t => t.kind.tag === 'doubleLit' && t.kind.value === 1.5)).toBe(true)
        expect(toks.some(t => t.kind.tag === 'dateLit' && t.kind.year === 1990)).toBe(true)
        expect(toks.some(t => t.kind.tag === 'boolLit' && t.kind.value === true)).toBe(true)
        expect(toks.some(t => t.kind.tag === 'boolLit' && t.kind.value === false)).toBe(true)
        expect(toks.some(t => t.kind.tag === 'nullLit')).toBe(true)
    })

    it('handles docker-style import refs', () => {
        const lex = new Lexer('import alice/awesome v3')
        const toks = lex.tokenize()
        const refTok = toks.find(t => t.kind.tag === 'refLit')
        expect(refTok).toBeTruthy()
        expect((refTok!.kind as { tag: 'refLit'; raw: string }).raw).toBe('alice/awesome')
    })

    it('handles custom-registry import refs', () => {
        const lex = new Lexer('import hub.example.com/team/scraper')
        const toks = lex.tokenize()
        const refTok = toks.find(t => t.kind.tag === 'refLit')
        expect((refTok!.kind as { tag: 'refLit'; raw: string }).raw).toBe('hub.example.com/team/scraper')
    })

    it('handles localhost-port import refs', () => {
        const lex = new Lexer('import localhost:5000/me/test')
        const toks = lex.tokenize()
        const refTok = toks.find(t => t.kind.tag === 'refLit')
        expect((refTok!.kind as { tag: 'refLit'; raw: string }).raw).toBe('localhost:5000/me/test')
    })

    it('handles wildcards [*]', () => {
        const lex = new Lexer('$x[*]')
        const toks = lex.tokenize()
        expect(toks.some(t => t.kind.tag === 'wildcard')).toBe(true)
    })

    it('handles arrows ←  →  ?.', () => {
        const lex = new Lexer('a ← b → c ?. d')
        const toks = lex.tokenize()
        expect(toks.some(t => t.kind.tag === 'arrow')).toBe(true)
        expect(toks.some(t => t.kind.tag === 'caseArrow')).toBe(true)
        expect(toks.some(t => t.kind.tag === 'qDot')).toBe(true)
    })

    it('handles string escapes', () => {
        const lex = new Lexer(`"hello\\nworld\\t!"`)
        const toks = lex.tokenize()
        const s = toks.find(t => t.kind.tag === 'stringLit')
        expect(s).toBeTruthy()
        expect((s!.kind as { tag: 'stringLit'; value: string }).value).toBe('hello\nworld\t!')
    })

    it('handles negative integers', () => {
        const lex = new Lexer('-42')
        const toks = lex.tokenize()
        expect(toks[0].kind.tag).toBe('intLit')
        expect((toks[0].kind as { tag: 'intLit'; value: number }).value).toBe(-42)
    })
})
