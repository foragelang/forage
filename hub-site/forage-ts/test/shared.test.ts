// Drift-detection: parses the shared `.forage` files and asserts the same
// structural summary as the Rust `shared_recipes` harness. Both
// implementations load `tests/shared-recipes/expected.json`; if a
// parser/validator change in one drifts from the other, one of these
// tests fails first. The optional `runSnapshot` block also drives a
// real run through the TS runner so eval-side drift gets caught too.

import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { Parser } from '../src/parser.js'
import { validate } from '../src/validator.js'
import { run } from '../src/runner.js'
import type { FetchLike } from '../src/runner.js'
import type { Recipe, Statement } from '../src/ast.js'

const __dirname = dirname(fileURLToPath(import.meta.url))
const SHARED_DIR = resolve(__dirname, '../../../tests/shared-recipes')

interface ExpectedFile {
    description: string
    recipes: ExpectedRecipe[]
}

interface ExpectedRecipe {
    file: string
    parses: boolean
    summary?: Summary
    types?: ExpectedType[]
    enums?: ExpectedEnum[]
    paginationModes?: string[]
    secrets?: string[]
    authSessionVariant?: string
    functionCount?: number
    validation: ExpectedValidation
    runSnapshot?: RunSnapshot
}

interface RunSnapshot {
    inputs?: Record<string, unknown>
    httpFixtures?: HttpFixture[]
    records: ExpectedRecord[]
}

interface HttpFixture {
    url: string
    method: string
    status: number
    body: string
}

interface ExpectedRecord {
    typeName: string
    fields: Record<string, unknown>
}

interface Summary {
    name: string
    engineKind: string
    typeCount: number
    enumCount: number
    inputCount: number
    bodyStatementCount: number
    stepNames: string[]
    topLevelEmits: number
    forLoopCount: number
    expectationCount: number
}

interface ExpectedType {
    name: string
    fieldNames: string[]
    requiredFieldCount: number
}

interface ExpectedEnum {
    name: string
    variants: string[]
}

interface ExpectedValidation {
    errorCount?: number
    warningCount?: number
    errorCountMin?: number
    expectedErrorKeywords?: string[]
}

const expectedFile: ExpectedFile = JSON.parse(
    readFileSync(resolve(SHARED_DIR, 'expected.json'), 'utf8'),
)

describe('shared recipes', () => {
    for (const rec of expectedFile.recipes) {
        if (rec.runSnapshot) {
            it(`runs ${rec.file} to expected records`, async () => {
                const source = readFileSync(resolve(SHARED_DIR, rec.file), 'utf8')
                const recipe = Parser.parse(source)
                const snapshot = rec.runSnapshot!
                const fetchImpl = makeFetch(snapshot.httpFixtures ?? [])
                const result = await run(recipe, snapshot.inputs ?? {}, { fetch: fetchImpl })
                expect(result.diagnostic.stallReason, JSON.stringify(result.diagnostic)).toBe('completed')
                expect(result.records).toEqual(snapshot.records)
            })
        }

        it(`parses+validates ${rec.file} consistently`, () => {
            const source = readFileSync(resolve(SHARED_DIR, rec.file), 'utf8')

            let recipe: Recipe
            try {
                recipe = Parser.parse(source)
            } catch (e) {
                if (rec.parses) throw e
                return
            }

            if (rec.summary) {
                expect(recipe.name).toBe(rec.summary.name)
                expect(recipe.engineKind).toBe(rec.summary.engineKind)
                expect(recipe.types).toHaveLength(rec.summary.typeCount)
                expect(recipe.enums).toHaveLength(rec.summary.enumCount)
                expect(recipe.inputs).toHaveLength(rec.summary.inputCount)
                expect(recipe.body).toHaveLength(rec.summary.bodyStatementCount)
                expect(recipe.expectations).toHaveLength(rec.summary.expectationCount)
                expect(collectStepNames(recipe.body)).toEqual(rec.summary.stepNames)
                expect(countTopLevelEmits(recipe.body)).toBe(rec.summary.topLevelEmits)
                expect(countForLoops(recipe.body)).toBe(rec.summary.forLoopCount)
            }

            if (rec.types) {
                for (const et of rec.types) {
                    const t = recipe.types.find(t => t.name === et.name)
                    expect(t, `${rec.file}: missing type ${et.name}`).toBeTruthy()
                    if (!t) continue
                    expect(t.fields.map(f => f.name)).toEqual(et.fieldNames)
                    expect(t.fields.filter(f => !f.optional)).toHaveLength(et.requiredFieldCount)
                }
            }

            if (rec.enums) {
                for (const ee of rec.enums) {
                    const e = recipe.enums.find(e => e.name === ee.name)
                    expect(e, `${rec.file}: missing enum ${ee.name}`).toBeTruthy()
                    if (!e) continue
                    expect(e.variants).toEqual(ee.variants)
                }
            }

            if (rec.paginationModes) {
                expect(collectPaginationModes(recipe.body)).toEqual(rec.paginationModes)
            }

            if (rec.secrets) {
                expect(recipe.secrets).toEqual(rec.secrets)
            }

            if (rec.authSessionVariant) {
                expect(recipe.auth?.tag).toBe('session')
                if (recipe.auth?.tag === 'session') {
                    expect(recipe.auth.session.kind.tag).toBe(rec.authSessionVariant)
                }
            }

            if (rec.functionCount !== undefined) {
                expect(recipe.functions).toHaveLength(rec.functionCount)
            }

            const issues = validate(recipe)
            const errors = issues.filter(i => i.severity === 'error')
            const warnings = issues.filter(i => i.severity === 'warning')

            if (rec.validation.errorCount !== undefined) {
                expect(errors.length, `errors: ${errors.map(e => e.message).join('; ')}`)
                    .toBe(rec.validation.errorCount)
            }
            if (rec.validation.warningCount !== undefined) {
                expect(warnings).toHaveLength(rec.validation.warningCount)
            }
            if (rec.validation.errorCountMin !== undefined) {
                expect(errors.length).toBeGreaterThanOrEqual(rec.validation.errorCountMin)
            }
            if (rec.validation.expectedErrorKeywords) {
                for (const kw of rec.validation.expectedErrorKeywords) {
                    expect(
                        errors.some(e => e.message.includes(kw)),
                        `expected an error containing '${kw}'; got: ${errors.map(e => e.message).join('; ')}`,
                    ).toBe(true)
                }
            }
        })
    }
})

/// Build a deterministic `fetch` that matches request URL against the
/// declared fixtures (exact match first, then path+sorted-query to mirror
/// `forage-http::transport::url_matches`). Falls back to 404 so a missing
/// fixture surfaces as a real failure rather than a hung test.
function makeFetch(fixtures: HttpFixture[]): FetchLike {
    return async (url: string, init: RequestInit) => {
        const method = (init.method ?? 'GET').toUpperCase()
        const match = fixtures.find(f =>
            f.method.toUpperCase() === method && urlMatches(f.url, url),
        )
        if (!match) {
            return new Response(`no fixture for ${method} ${url}`, { status: 404 })
        }
        return new Response(match.body, { status: match.status })
    }
}

function urlMatches(fixtureUrl: string, reqUrl: string): boolean {
    if (fixtureUrl === reqUrl) return true
    return stripOrigin(fixtureUrl) === stripOrigin(reqUrl)
}

function stripOrigin(u: string): string {
    const qIdx = u.indexOf('?')
    let path = qIdx === -1 ? u : u.slice(0, qIdx)
    const query = qIdx === -1 ? null : u.slice(qIdx + 1)
    const slashIdx = path.indexOf('//')
    if (slashIdx !== -1) {
        const rest = path.slice(slashIdx + 2)
        const next = rest.indexOf('/')
        path = next === -1 ? rest : rest.slice(next)
    }
    if (query === null) return path
    const parts = query.split('&').sort()
    return `${path}?${parts.join('&')}`
}

function collectStepNames(stmts: Statement[]): string[] {
    const names: string[] = []
    for (const s of stmts) {
        if (s.tag === 'step') names.push(s.step.name)
        if (s.tag === 'forLoop') names.push(...collectStepNames(s.body))
    }
    return names
}

function countTopLevelEmits(stmts: Statement[]): number {
    return stmts.filter(s => s.tag === 'emit').length
}

function countForLoops(stmts: Statement[]): number {
    let n = 0
    for (const s of stmts) {
        if (s.tag === 'forLoop') n += 1 + countForLoops(s.body)
    }
    return n
}

function collectPaginationModes(stmts: Statement[]): string[] {
    const modes: string[] = []
    for (const s of stmts) {
        if (s.tag === 'step' && s.step.pagination) modes.push(s.step.pagination.tag)
        if (s.tag === 'forLoop') modes.push(...collectPaginationModes(s.body))
    }
    return modes
}
