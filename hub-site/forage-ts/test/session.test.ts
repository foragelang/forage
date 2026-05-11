import { describe, it, expect } from 'vitest'
import { Parser } from '../src/parser.js'
import { validate, hasErrors, referencedSecretsInPath } from '../src/validator.js'
import { run } from '../src/runner.js'

describe('parser: auth.session', () => {
    it('parses auth.session.formLogin with secret references', () => {
        const src = `recipe "p" {
            engine http
            secret username
            secret password
            type Item { id: String }
            auth.session.formLogin {
                url: "https://example.com/login"
                method: "POST"
                body.form {
                    "username": $secret.username
                    "password": $secret.password
                }
                captureCookies: true
                maxReauthRetries: 2
                cache: 3600
            }
            step items { method "GET"; url "https://example.com/items" }
            for $it in $items {
                emit Item { id ← $it.id }
            }
        }`
        const recipe = Parser.parse(src)
        expect(recipe.auth?.tag).toBe('session')
        if (recipe.auth?.tag !== 'session') throw new Error('not session')
        expect(recipe.auth.session.maxReauthRetries).toBe(2)
        expect(recipe.auth.session.cacheDuration).toBe(3600)
        expect(recipe.auth.session.kind.tag).toBe('formLogin')
        if (recipe.auth.session.kind.tag !== 'formLogin') throw new Error('not formLogin')
        expect(recipe.auth.session.kind.formLogin.captureCookies).toBe(true)
        expect(recipe.secrets).toEqual(['username', 'password'])
    })

    it('parses auth.session.bearerLogin with tokenPath', () => {
        const src = `recipe "p" {
            engine http
            secret clientId
            secret clientSecret
            type Item { id: String }
            auth.session.bearerLogin {
                url: "https://example.com/token"
                body.json {
                    client_id: $secret.clientId
                    client_secret: $secret.clientSecret
                }
                tokenPath: $.access_token
                headerPrefix: "Bearer "
            }
            step items { method "GET"; url "https://example.com/items" }
            for $it in $items {
                emit Item { id ← $it.id }
            }
        }`
        const recipe = Parser.parse(src)
        expect(recipe.auth?.tag).toBe('session')
        if (recipe.auth?.tag !== 'session') throw new Error('not session')
        if (recipe.auth.session.kind.tag !== 'bearerLogin') throw new Error('not bearerLogin')
        expect(recipe.auth.session.kind.bearerLogin.headerName).toBe('Authorization')
        expect(recipe.auth.session.kind.bearerLogin.headerPrefix).toBe('Bearer ')
        // tokenPath should be a $.access_token shape: field("current", "access_token")
        const tp = recipe.auth.session.kind.bearerLogin.tokenPath
        expect(tp.tag).toBe('field')
        if (tp.tag !== 'field') throw new Error('not field')
        expect(tp.name).toBe('access_token')
        expect(tp.base.tag).toBe('current')
    })

    it('parses auth.session.cookiePersist with netscape format', () => {
        const src = `recipe "p" {
            engine http
            secret cookieFile
            type Item { id: String }
            auth.session.cookiePersist {
                sourcePath: "{$secret.cookieFile}"
                format: netscape
            }
            step items { method "GET"; url "https://example.com/items" }
            for $it in $items {
                emit Item { id ← $it.id }
            }
        }`
        const recipe = Parser.parse(src)
        if (recipe.auth?.tag !== 'session') throw new Error('not session')
        if (recipe.auth.session.kind.tag !== 'cookiePersist') throw new Error('not cookiePersist')
        expect(recipe.auth.session.kind.cookiePersist.format).toBe('netscape')
    })
})

describe('validator: secrets', () => {
    it('warns on referenced-but-undeclared secret', () => {
        const src = `recipe "v" {
            engine http
            type Item { id: String }
            auth.session.formLogin {
                url: "https://example.com/login"
                body.form { "username": $secret.typoed }
            }
            step items { method "GET"; url "https://example.com/items" }
            for $it in $items { emit Item { id ← $it.id } }
        }`
        const recipe = Parser.parse(src)
        const issues = validate(recipe)
        const warnings = issues.filter(i => i.severity === 'warning').map(i => i.message)
        expect(warnings.some(m => m.includes('typoed') && m.includes('not declared'))).toBe(true)
    })

    it('warns on declared-but-unreferenced secret', () => {
        const src = `recipe "v" {
            engine http
            secret unused
            type Item { id: String }
            step items { method "GET"; url "https://example.com/items" }
            for $it in $items { emit Item { id ← $it.id } }
        }`
        const recipe = Parser.parse(src)
        const issues = validate(recipe)
        const warnings = issues.filter(i => i.severity === 'warning').map(i => i.message)
        expect(warnings.some(m => m.includes('unused') && m.includes('never referenced'))).toBe(true)
    })

    it('clean recipe: declared + referenced match → no secret warnings', () => {
        const src = `recipe "v" {
            engine http
            secret username
            secret password
            type Item { id: String }
            auth.session.formLogin {
                url: "https://example.com/login"
                body.form {
                    "username": $secret.username
                    "password": $secret.password
                }
            }
            step items { method "GET"; url "https://example.com/items" }
            for $it in $items { emit Item { id ← $it.id } }
        }`
        const recipe = Parser.parse(src)
        const issues = validate(recipe)
        expect(hasErrors(issues)).toBe(false)
        // No secret-related warnings.
        const warnings = issues.filter(i => i.severity === 'warning').map(i => i.message)
        expect(warnings.some(m => m.includes('secret'))).toBe(false)
    })
})

describe('referencedSecretsInPath', () => {
    it('extracts secret name from a $secret.<name> path', () => {
        const src = `recipe "v" {
            engine http
            secret apikey
            type Item { id: String }
            auth.session.formLogin {
                url: "https://example.com/login"
                body.form { "key": $secret.apikey }
            }
            step items { method "GET"; url "https://example.com/items" }
            for $it in $items { emit Item { id ← $it.id } }
        }`
        const recipe = Parser.parse(src)
        if (recipe.auth?.tag !== 'session') throw new Error('not session')
        if (recipe.auth.session.kind.tag !== 'formLogin') throw new Error('not formLogin')
        const body = recipe.auth.session.kind.formLogin.body
        if (body.tag !== 'form') throw new Error('not form')
        const value = body.entries[0].value
        if (value.tag !== 'path') throw new Error('not path')
        const secrets = referencedSecretsInPath(value.path)
        expect(secrets.has('apikey')).toBe(true)
    })
})

describe('runner: session unsupported', () => {
    it('refuses to run session-auth recipes with a clear message', async () => {
        const src = `recipe "v" {
            engine http
            secret password
            type Item { id: String }
            auth.session.formLogin {
                url: "https://example.com/login"
                body.form { "password": $secret.password }
            }
            step items { method "GET"; url "https://example.com/items" }
            for $it in $items { emit Item { id ← $it.id } }
        }`
        const recipe = Parser.parse(src)
        // Stub fetch — the runner should bail before any network call.
        let called = false
        const stubFetch = async () => { called = true; return new Response('') }
        const result = await run(recipe, {}, { fetch: stubFetch as any })
        expect(called).toBe(false)
        expect(result.diagnostic.stallReason).toMatch(/auth\.session\.\* not supported/)
        expect(result.records).toEqual([])
    })
})
