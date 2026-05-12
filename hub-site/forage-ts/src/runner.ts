// HTTP-engine recipe runner. Mirrors HTTPEngine.swift but uses the browser
// `fetch` API. Browser-engine recipes are not supported.

import type {
    BodyValue,
    HTTPBody,
    HTTPRequest,
    HTTPStep,
    HTTPBodyKV,
    JSONValue,
    Pagination,
    Recipe,
    Statement,
} from './ast.js'
import { ExtractionEvaluator, Scope, resolvePath, stringifyJSON } from './extraction.js'
import { fromRawJSON, toRawJSON, TransformImpls } from './transforms.js'

export interface RunDiagnostic {
    stallReason: string
    unmetExpectations: string[]
}

export interface ScrapedRecord {
    typeName: string
    fields: Record<string, unknown>
}

export interface RunResult {
    records: ScrapedRecord[]
    diagnostic: RunDiagnostic
}

export type FetchLike = (url: string, init: RequestInit) => Promise<Response>

export interface RunOptions {
    /** Replace `fetch` for tests or alternative transports. */
    fetch?: FetchLike
}

interface PaginationOverride {
    param: string
    value: JSONValue
}

export async function run(
    recipe: Recipe,
    inputs: Record<string, unknown>,
    opts: RunOptions = {},
): Promise<RunResult> {
    if (recipe.engineKind !== 'http') {
        return {
            records: [],
            diagnostic: {
                stallReason: 'failed: browser-engine recipes are not supported in the web runner',
                unmetExpectations: [],
            },
        }
    }

    const fetchImpl: FetchLike = opts.fetch ?? ((u, i) => fetch(u, i))
    const evaluator = new ExtractionEvaluator(new TransformImpls())
    const inputsJSON: Record<string, JSONValue> = {}
    for (const [k, v] of Object.entries(inputs)) inputsJSON[k] = fromRawJSON(v)

    let scope = new Scope(inputsJSON, [{}], null)
    const collector: ScrapedRecord[] = []

    try {
        // Auth: htmlPrime not supported on the web (response is plain HTML
        // that fetch can return, but regex-based capture is fiddly and most
        // recipes targeting the web IDE will use staticHeader). staticHeader
        // is applied per-request in `buildRequest`.
        if (recipe.auth?.tag === 'htmlPrime') {
            return {
                records: [],
                diagnostic: {
                    stallReason: 'failed: htmlPrime auth not supported in web runner — use Studio',
                    unmetExpectations: [],
                },
            }
        }
        // Session auth is a stateful flow with credentials, cookie threading,
        // re-auth on 401, and an MFA hook — not viable inside a browser tab
        // without leaking creds into localStorage. Sessioned recipes belong
        // in the CLI or Studio; the web IDE just parses + validates them.
        if (recipe.auth?.tag === 'session') {
            return {
                records: [],
                diagnostic: {
                    stallReason: 'failed: auth.session.* not supported in web runner — run via CLI or Studio',
                    unmetExpectations: [],
                },
            }
        }

        scope = await runStatements(recipe.body, recipe, scope, collector, evaluator, fetchImpl)
        return {
            records: collector,
            diagnostic: {
                stallReason: 'completed',
                unmetExpectations: checkExpectations(recipe, collector),
            },
        }
    } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        return {
            records: collector,
            diagnostic: {
                stallReason: `failed: ${message}`,
                unmetExpectations: checkExpectations(recipe, collector),
            },
        }
    }
}

function checkExpectations(recipe: Recipe, records: ScrapedRecord[]): string[] {
    const unmet: string[] = []
    for (const expectation of recipe.expectations) {
        const k = expectation.kind
        if (k.tag === 'recordCount') {
            const count = records.filter(r => r.typeName === k.typeName).length
            const ok = compare(count, k.op, k.value)
            if (!ok) {
                unmet.push(`records.where(typeName == "${k.typeName}").count ${k.op} ${k.value} — got ${count}`)
            }
        }
    }
    return unmet
}

function compare(a: number, op: string, b: number): boolean {
    switch (op) {
        case '>=': return a >= b
        case '>': return a > b
        case '<=': return a <= b
        case '<': return a < b
        case '==': return a === b
        case '!=': return a !== b
        default: return false
    }
}

async function runStatements(
    statements: Statement[],
    recipe: Recipe,
    scope: Scope,
    collector: ScrapedRecord[],
    evaluator: ExtractionEvaluator,
    fetchImpl: FetchLike,
): Promise<Scope> {
    for (const stmt of statements) {
        scope = await runStatement(stmt, recipe, scope, collector, evaluator, fetchImpl)
    }
    return scope
}

async function runStatement(
    statement: Statement,
    recipe: Recipe,
    scope: Scope,
    collector: ScrapedRecord[],
    evaluator: ExtractionEvaluator,
    fetchImpl: FetchLike,
): Promise<Scope> {
    switch (statement.tag) {
        case 'step': {
            const result = await runStep(statement.step, recipe, scope, evaluator, fetchImpl)
            return scope.with(statement.step.name, result)
        }
        case 'emit': {
            const record = evaluator.emit(statement.emission, scope)
            if (record.tag === 'object') {
                const fields: Record<string, unknown> = {}
                for (const [k, v] of Object.entries(record.entries)) {
                    if (k === '_typeName') continue
                    fields[k] = toRawJSON(v)
                }
                collector.push({ typeName: statement.emission.typeName, fields })
            }
            return scope
        }
        case 'forLoop': {
            const listValue = evaluator.evaluateToJSON(statement.collection, scope)
            let items: JSONValue[]
            if (listValue.tag === 'array') items = listValue.items
            else if (listValue.tag === 'null') items = []
            else items = [listValue]
            for (const item of items) {
                let inner = scope.with(statement.variable, item).withCurrent(item)
                inner = await runStatements(statement.body, recipe, inner, collector, evaluator, fetchImpl)
            }
            return scope
        }
    }
}

async function runStep(
    step: HTTPStep,
    recipe: Recipe,
    scope: Scope,
    evaluator: ExtractionEvaluator,
    fetchImpl: FetchLike,
): Promise<JSONValue> {
    if (step.pagination) return runPaginated(step, recipe, scope, step.pagination, evaluator, fetchImpl)
    const { url, init } = buildRequest(step.request, recipe, scope, null, evaluator)
    const response = await fetchImpl(url, init)
    const text = await response.text()
    return parseResponse(text)
}

async function runPaginated(
    step: HTTPStep,
    recipe: Recipe,
    scope: Scope,
    pagination: Pagination,
    evaluator: ExtractionEvaluator,
    fetchImpl: FetchLike,
): Promise<JSONValue> {
    switch (pagination.tag) {
        case 'pageWithTotal': {
            const collected: JSONValue[] = []
            let page = pagination.pageZeroIndexed ? 0 : 1
            for (let iter = 0; iter < 200; iter++) {
                const override: PaginationOverride = { param: pagination.pageParam, value: { tag: 'int', value: page } }
                const { url, init } = buildRequest(step.request, recipe, scope, override, evaluator)
                const response = await fetchImpl(url, init)
                const text = await response.text()
                const json = parseResponse(text)
                const pageScope = scope.withCurrent(json)
                const items = resolvePath(pagination.itemsPath, pageScope)
                const total = resolvePath(pagination.totalPath, pageScope)
                const totalCount = total.tag === 'int' ? total.value : (total.tag === 'double' ? Math.trunc(total.value) : 0)
                if (items.tag === 'array') {
                    for (const x of items.items) collected.push(x)
                    if (collected.length >= totalCount || items.items.length === 0) break
                } else break
                page += 1
            }
            return { tag: 'array', items: collected }
        }
        case 'untilEmpty': {
            const collected: JSONValue[] = []
            let page = pagination.pageZeroIndexed ? 0 : 1
            for (let iter = 0; iter < 500; iter++) {
                const override: PaginationOverride = { param: pagination.pageParam, value: { tag: 'int', value: page } }
                const { url, init } = buildRequest(step.request, recipe, scope, override, evaluator)
                const response = await fetchImpl(url, init)
                const text = await response.text()
                const json = parseResponse(text)
                const items = resolvePath(pagination.itemsPath, scope.withCurrent(json))
                if (items.tag === 'array' && items.items.length > 0) {
                    for (const x of items.items) collected.push(x)
                } else break
                page += 1
            }
            return { tag: 'array', items: collected }
        }
        case 'cursor': {
            const collected: JSONValue[] = []
            let cursor: JSONValue = { tag: 'null' }
            for (let iter = 0; iter < 500; iter++) {
                let override: PaginationOverride | null = null
                if (cursor.tag === 'string' && cursor.value !== '') {
                    override = { param: pagination.cursorParam, value: cursor }
                }
                const { url, init } = buildRequest(step.request, recipe, scope, override, evaluator)
                const response = await fetchImpl(url, init)
                const text = await response.text()
                const json = parseResponse(text)
                const pageScope = scope.withCurrent(json)
                const items = resolvePath(pagination.itemsPath, pageScope)
                if (items.tag === 'array') for (const x of items.items) collected.push(x)
                try {
                    cursor = resolvePath(pagination.cursorPath, pageScope)
                } catch {
                    cursor = { tag: 'null' }
                }
                if (cursor.tag === 'null') break
                if (cursor.tag === 'string' && cursor.value === '') break
            }
            return { tag: 'array', items: collected }
        }
    }
}

function parseResponse(text: string): JSONValue {
    if (text.trim() === '') return { tag: 'null' }
    try {
        return fromRawJSON(JSON.parse(text))
    } catch {
        return { tag: 'string', value: text }
    }
}

function buildRequest(
    template: HTTPRequest,
    recipe: Recipe,
    scope: Scope,
    paginationOverride: PaginationOverride | null,
    evaluator: ExtractionEvaluator,
): { url: string; init: RequestInit } {
    const url = evaluator.renderTemplate(template.url, scope)
    const headers: Record<string, string> = {}
    if (recipe.auth?.tag === 'staticHeader') {
        headers[recipe.auth.name] = evaluator.renderTemplate(recipe.auth.value, scope)
    }
    for (const h of template.headers) {
        headers[h.key] = evaluator.renderTemplate(h.value, scope)
    }

    let body: BodyInit | null = null
    if (template.body) {
        const built = buildBody(template.body, scope, paginationOverride, evaluator)
        body = built.body
        if (!headers['Content-Type'] && built.contentType) {
            headers['Content-Type'] = built.contentType
        }
    }
    return {
        url,
        init: {
            method: template.method,
            headers,
            body,
        },
    }
}

function buildBody(
    body: HTTPBody,
    scope: Scope,
    paginationOverride: PaginationOverride | null,
    evaluator: ExtractionEvaluator,
): { body: BodyInit; contentType: string } {
    switch (body.tag) {
        case 'jsonObject': {
            let entries = body.entries
            if (paginationOverride) entries = upsertedJSON(entries, paginationOverride)
            const obj: Record<string, unknown> = {}
            for (const kv of entries) obj[kv.key] = renderBodyValue(kv.value, scope, evaluator)
            return { body: JSON.stringify(obj), contentType: 'application/json' }
        }
        case 'form': {
            let entries = body.entries
            if (paginationOverride) entries = upsertedForm(entries, paginationOverride)
            const parts: string[] = []
            for (const kv of entries) {
                const any = renderBodyValue(kv.value, scope, evaluator)
                parts.push(`${encodeURIComponent(kv.key)}=${encodeURIComponent(stringifyAny(any))}`)
            }
            return { body: parts.join('&'), contentType: 'application/x-www-form-urlencoded' }
        }
        case 'raw': {
            return { body: evaluator.renderTemplate(body.template, scope), contentType: 'text/plain' }
        }
    }
}

function renderBodyValue(bv: BodyValue, scope: Scope, evaluator: ExtractionEvaluator): unknown {
    switch (bv.tag) {
        case 'templateString': return evaluator.renderTemplate(bv.template, scope)
        case 'literal': return toRawJSON(bv.value)
        case 'path': return toRawJSON(resolvePath(bv.path, scope))
        case 'object': {
            const obj: Record<string, unknown> = {}
            for (const kv of bv.entries) obj[kv.key] = renderBodyValue(kv.value, scope, evaluator)
            return obj
        }
        case 'array': return bv.items.map(v => renderBodyValue(v, scope, evaluator))
        case 'caseOf': {
            const v = resolvePath(bv.scrutinee, scope)
            if (v.tag !== 'string') throw new Error('case-of: scrutinee did not resolve to an enum label')
            for (const br of bv.branches) {
                if (br.label === v.value) return renderBodyValue(br.value, scope, evaluator)
            }
            throw new Error(`case-of: no branch matched '${v.value}'`)
        }
    }
}

function upsertedJSON(kvs: HTTPBodyKV[], p: PaginationOverride): HTTPBodyKV[] {
    const out = kvs.filter(k => k.key !== p.param)
    out.push({ key: p.param, value: { tag: 'literal', value: p.value } })
    return out
}

function upsertedForm(
    kvs: Array<{ key: string; value: BodyValue }>,
    p: PaginationOverride,
): Array<{ key: string; value: BodyValue }> {
    const out = kvs.filter(k => k.key !== p.param)
    out.push({ key: p.param, value: { tag: 'literal', value: p.value } })
    return out
}

function stringifyAny(v: unknown): string {
    if (v === null || v === undefined) return ''
    if (typeof v === 'string') return v
    if (typeof v === 'number') {
        if (Number.isInteger(v)) return String(v)
        if (v === Math.trunc(v) && Math.abs(v) < 1e15) return String(Math.trunc(v))
        return String(v)
    }
    if (typeof v === 'boolean') return String(v)
    return JSON.stringify(v)
}
