import type {
    AlignmentUri,
    Env,
    ListTypesResponse,
    PublishTypeRequest,
    TypeFieldAlignment,
    TypeListing,
    TypeMetadata,
    TypeVersion,
} from '../types'
import {
    getType,
    putType,
    getTypeVersion,
    putTypeVersion,
    getTypeHash,
    putTypeHash,
    indexAddType,
    indexAddUserType,
    indexAddAligned,
    indexRemoveAligned,
    listTypesIndex,
    ref,
    sha256Hex,
    splitRef,
} from '../storage'
import { identifyCaller, callerCanWrite } from '../auth'
import { json, jsonError } from '../http'
import { validateSegment } from './packages'

// Type name segments mirror the bare type name from `share type Name {…}`.
// Names start with an uppercase letter (the rest of the codebase already
// expects PascalCase type names — `notes/grammar.md` constrains the
// parser the same way), and use alphanumeric characters only. The 64-char
// ceiling matches the recipe-slug pragmatic limit.
const TYPE_NAME_RE = /^[A-Z][A-Za-z0-9]{0,63}$/

const MAX_TYPE_SOURCE_BYTES = 256 * 1024
const MAX_ALIGNMENTS = 32
const MAX_FIELDS = 256
const MAX_FIELD_NAME_LEN = 128
const MAX_TAGS = 16
const MAX_CATEGORY_LEN = 64
const MAX_DESCRIPTION_LEN = 2048
const ONTOLOGY_RE = /^[a-z][a-z0-9.\-]*$/
const TERM_RE = /^[A-Za-z0-9][A-Za-z0-9._:\-/]*$/
const FIELD_NAME_RE = /^[a-z_][a-zA-Z0-9_]*$/
const CATEGORY_RE = /^[a-z0-9][a-z0-9-]*$/

// Header form for a type-version artifact: the body must begin with
// `share type Name` (after comments / whitespace). Captures `Name` so
// the publish path can verify the URL `:name` segment matches.
const TYPE_HEAD_NAME_RE =
    /^\s*(?:\/\/[^\n]*\n|\/\*[\s\S]*?\*\/|\s)*share\s+type\s+([A-Z][A-Za-z0-9]*)/

export function validateTypeName(name: string): boolean {
    return TYPE_NAME_RE.test(name)
}

// `GET /v1/types` — listing of every published type. Filters mirror
// `listPackages` (category, q substring, sort). Pre-1.0 volume is small
// enough that a full scan over `idx:types` is fine.
export async function listTypes(
    request: Request,
    env: Env,
): Promise<Response> {
    const url = new URL(request.url)
    const q = url.searchParams.get('q')?.toLowerCase() ?? null
    const category = url.searchParams.get('category')
    const sort = url.searchParams.get('sort') ?? 'recent'
    const limit = clampInt(url.searchParams.get('limit'), 20, 1, 100)
    const cursor = url.searchParams.get('cursor')

    const refs = await listTypesIndex(env)
    const metas: TypeMetadata[] = []
    for (const r of refs) {
        const [a, n] = splitRef(r)
        const meta = await getType(env, a, n)
        if (meta === null) continue
        if (category !== null && meta.category !== category) continue
        if (q !== null) {
            const hay = `${meta.author}/${meta.name} ${meta.description}`.toLowerCase()
            if (!hay.includes(q)) continue
        }
        metas.push(meta)
    }

    if (sort === 'recent') {
        metas.sort((a, b) => b.created_at - a.created_at)
    } else {
        return jsonError(400, 'bad_sort', `unknown sort: ${sort}`, {}, request)
    }

    const startIdx = cursor !== null
        ? Math.max(0, metas.findIndex((m) => ref(m.author, m.name) === cursor) + 1)
        : 0
    const slice = metas.slice(startIdx, startIdx + limit)
    const nextCursor = startIdx + limit < metas.length
        ? ref(slice[slice.length - 1].author, slice[slice.length - 1].name)
        : null

    const body: ListTypesResponse = {
        items: slice.map(toListing),
        next_cursor: nextCursor,
    }
    return json(body, 200, request)
}

function toListing(meta: TypeMetadata): TypeListing {
    return {
        author: meta.author,
        name: meta.name,
        description: meta.description,
        category: meta.category,
        tags: meta.tags,
        created_at: meta.created_at,
        latest_version: meta.latest_version,
    }
}

// `GET /v1/types/:author/:name` — metadata.
export async function getTypeDetail(
    request: Request,
    env: Env,
    author: string,
    name: string,
): Promise<Response> {
    const meta = await getType(env, author, name)
    if (meta === null) {
        return jsonError(404, 'not_found', `unknown type: ${author}/${name}`, {}, request)
    }
    return json(meta, 200, request)
}

// `GET /v1/types/:author/:name/versions` — linear version history.
export async function listTypeVersions(
    request: Request,
    env: Env,
    author: string,
    name: string,
): Promise<Response> {
    const meta = await getType(env, author, name)
    if (meta === null) {
        return jsonError(404, 'not_found', `unknown type: ${author}/${name}`, {}, request)
    }
    const items: Array<{ version: number; published_at: number; published_by: string }> = []
    for (let n = 1; n <= meta.latest_version; n++) {
        const v = await getTypeVersion(env, author, name, n)
        if (v === null) continue
        items.push({
            version: v.version,
            published_at: v.published_at,
            published_by: v.published_by,
        })
    }
    return json({ items }, 200, request)
}

// `GET /v1/types/:author/:name/versions/:n` (n = number | 'latest')
export async function getTypeVersionArtifact(
    request: Request,
    env: Env,
    author: string,
    name: string,
    versionSpec: string,
): Promise<Response> {
    const meta = await getType(env, author, name)
    if (meta === null) {
        return jsonError(404, 'not_found', `unknown type: ${author}/${name}`, {}, request)
    }
    const n = versionSpec === 'latest'
        ? meta.latest_version
        : parseInt(versionSpec, 10)
    if (!Number.isFinite(n) || n < 1) {
        return jsonError(400, 'bad_version', `invalid version: ${versionSpec}`, {}, request)
    }
    const artifact = await getTypeVersion(env, author, name, n)
    if (artifact === null) {
        return jsonError(404, 'not_found', `unknown version ${author}/${name}@${n}`, {}, request)
    }
    return json(artifact, 200, request)
}

// `POST /v1/types/:author/:name/versions`
//
// First publish (`base_version: null`) accepts iff (author, name) doesn't
// exist; creates v1. Subsequent publishes require
// `base_version == latest_version`; mismatch returns 409.
//
// Content-hash dedup: if the new `source` hashes to the same digest as
// the current latest version, returns 200 with the existing version
// number rather than allocating v(N+1). Recipes that re-publish
// unchanged types get stable type-version pins this way.
export async function publishTypeVersion(
    request: Request,
    env: Env,
    author: string,
    name: string,
): Promise<Response> {
    const caller = await identifyCaller(request, env)
    if (caller === null) {
        return jsonError(401, 'unauthorized', 'missing or invalid bearer token', {}, request)
    }
    const callerLogin = caller.kind === 'user' ? caller.login : null
    if (callerLogin !== null && callerLogin !== author) {
        return jsonError(
            403,
            'forbidden',
            `you are signed in as @${callerLogin}; cannot publish under @${author}`,
            {},
            request,
        )
    }
    if (!validateTypeName(name)) {
        return jsonError(400, 'bad_type_name', `invalid type name: ${name}`, {}, request)
    }

    let payload: PublishTypeRequest
    try {
        payload = (await request.json()) as PublishTypeRequest
    } catch {
        return jsonError(400, 'bad_json', 'request body is not valid JSON', {}, request)
    }

    const validation = validateTypePublish(payload)
    if (typeof validation === 'string') {
        return jsonError(400, 'invalid', validation, {}, request)
    }
    if (validation.typeName !== name) {
        return jsonError(
            400,
            'name_mismatch',
            `publish name ${name} does not match type header name ${validation.typeName}`,
            {},
            request,
        )
    }

    const existing = await getType(env, author, name)

    // Stale-base check (same shape as recipe publish).
    if (existing === null) {
        if (payload.base_version !== null) {
            return jsonError(
                409,
                'stale_base',
                `type ${author}/${name} does not exist yet; first publish must use base_version: null`,
                { latest_version: 0, your_base: payload.base_version },
                request,
            )
        }
    } else {
        if (!callerCanWrite(caller, existing.owner_login)) {
            return jsonError(
                403,
                'forbidden',
                `${author}/${name} is owned by @${existing.owner_login}`,
                {},
                request,
            )
        }
        if (payload.base_version !== existing.latest_version) {
            return jsonError(
                409,
                'stale_base',
                `base is stale, rebase to v${existing.latest_version} and retry`,
                {
                    latest_version: existing.latest_version,
                    your_base: payload.base_version,
                },
                request,
            )
        }
    }

    // Content-hash dedup. The hash is over the canonical UTF-8 source
    // bytes only — alignments / metadata are not part of the dedup
    // key because re-publishing the same source with a richer alignment
    // set is still meaningful and should land as a new version.
    const sha = await sha256Hex(payload.source)
    const dedup = await getTypeHash(env, author, name, sha)
    if (dedup !== null && existing !== null && dedup === existing.latest_version) {
        const cached = await getTypeVersion(env, author, name, dedup)
        if (cached === null) {
            return jsonError(
                500,
                'corrupt',
                `type-hash index points at missing version ${author}/${name}@${dedup}`,
                {},
                request,
            )
        }
        // Same content as the current latest; short-circuit so the
        // recipe pinning this type gets a stable version reference.
        // The HTTP status differs from a real publish (200, not 201)
        // so callers can render "type unchanged, reused v{N}" if they
        // want — same shape otherwise.
        return json(
            {
                author,
                name,
                version: dedup,
                latest_version: existing.latest_version,
                deduped: true,
            },
            200,
            request,
        )
    }

    const ownerLogin = existing?.owner_login
        ?? (caller.kind === 'user' ? caller.login : 'admin')
    const publishedBy = caller.kind === 'user' ? caller.login : 'admin'

    const now = Date.now()
    const nextVersion = existing === null ? 1 : existing.latest_version + 1

    // Read the prior latest's alignments before overwriting so the
    // alignment index can be diffed below. `[]` on first publish.
    const priorAlignments: AlignmentUri[] = existing !== null
        ? (await getTypeVersion(env, author, name, existing.latest_version))?.alignments ?? []
        : []

    const artifact: TypeVersion = {
        author,
        name,
        version: nextVersion,
        source: payload.source,
        alignments: payload.alignments,
        field_alignments: payload.field_alignments,
        base_version: payload.base_version,
        published_at: now,
        published_by: publishedBy,
    }

    await putTypeVersion(env, artifact)
    await putTypeHash(env, author, name, sha, nextVersion)

    const meta: TypeMetadata = {
        author,
        name,
        description: payload.description,
        category: payload.category,
        tags: payload.tags,
        created_at: existing?.created_at ?? now,
        latest_version: nextVersion,
        owner_login: ownerLogin,
    }

    await putType(env, meta)
    if (existing === null) {
        await indexAddType(env, author, name)
        await indexAddUserType(env, author, name)
    }

    // Diff the alignment index against the prior latest. The index
    // tracks the current canonical view: a re-publish that drops an
    // `aligns` clause removes this type from
    // `aligned_with(<ontology>/<term>)`. Adding new ontologies adds it.
    await diffAlignmentIndex(env, author, name, priorAlignments, payload.alignments)

    return json(
        {
            author,
            name,
            version: nextVersion,
            latest_version: nextVersion,
            deduped: false,
        },
        201,
        request,
    )
}

/// Add to / remove from the `idx:aligned:<ontology>/<term>` index for
/// every alignment that's only on one side of the diff. Field-level
/// alignments don't participate — the hub's `aligned_with` query is
/// type-level by design (a field matching schema.org/name doesn't
/// make the *type* a schema.org/Thing).
async function diffAlignmentIndex(
    env: Env,
    typeAuthor: string,
    typeName: string,
    prior: AlignmentUri[],
    next: AlignmentUri[],
): Promise<void> {
    const priorKeys = new Set(prior.map((a) => `${a.ontology}/${a.term}`))
    const nextKeys = new Set(next.map((a) => `${a.ontology}/${a.term}`))
    for (const a of next) {
        if (!priorKeys.has(`${a.ontology}/${a.term}`)) {
            await indexAddAligned(env, a.ontology, a.term, typeAuthor, typeName)
        }
    }
    for (const a of prior) {
        if (!nextKeys.has(`${a.ontology}/${a.term}`)) {
            await indexRemoveAligned(env, a.ontology, a.term, typeAuthor, typeName)
        }
    }
}

// --- Validation ----------------------------------------------------------

interface ValidatedTypePublish {
    typeName: string
}

function validateTypePublish(
    payload: PublishTypeRequest,
): ValidatedTypePublish | string {
    if (payload === null || typeof payload !== 'object') {
        return 'body must be an object'
    }
    if (typeof payload.description !== 'string'
        || payload.description.length > MAX_DESCRIPTION_LEN
    ) {
        return `description must be a string up to ${MAX_DESCRIPTION_LEN} chars`
    }
    if (typeof payload.category !== 'string'
        || payload.category.length === 0
        || payload.category.length > MAX_CATEGORY_LEN
        || !CATEGORY_RE.test(payload.category)
    ) {
        return `category must match ${CATEGORY_RE}`
    }
    if (!Array.isArray(payload.tags) || payload.tags.length > MAX_TAGS) {
        return `tags must be an array of at most ${MAX_TAGS} strings`
    }
    for (const t of payload.tags) {
        if (typeof t !== 'string') return 'tags must be strings'
    }
    if (typeof payload.source !== 'string') {
        return 'source must be a string (the type declaration body)'
    }
    if (payload.source.length === 0) {
        return 'source must be non-empty'
    }
    if (payload.source.length > MAX_TYPE_SOURCE_BYTES) {
        return `source exceeds ${MAX_TYPE_SOURCE_BYTES} bytes`
    }
    const headerMatch = payload.source.match(TYPE_HEAD_NAME_RE)
    if (headerMatch === null) {
        return 'source must start with `share type <Name>` (after comments / whitespace)'
    }
    const typeName = headerMatch[1]

    if (!Array.isArray(payload.alignments) || payload.alignments.length > MAX_ALIGNMENTS) {
        return `alignments must be an array of at most ${MAX_ALIGNMENTS} entries`
    }
    for (const a of payload.alignments) {
        const err = validateAlignment(a)
        if (err !== null) return `alignments: ${err}`
    }
    if (
        !Array.isArray(payload.field_alignments)
        || payload.field_alignments.length > MAX_FIELDS
    ) {
        return `field_alignments must be an array of at most ${MAX_FIELDS} entries`
    }
    const seenFields = new Set<string>()
    for (const fa of payload.field_alignments) {
        const err = validateFieldAlignment(fa, seenFields)
        if (err !== null) return `field_alignments: ${err}`
    }
    if (
        payload.base_version !== null
        && (typeof payload.base_version !== 'number'
            || !Number.isInteger(payload.base_version)
            || payload.base_version < 0)
    ) {
        return 'base_version must be null or a non-negative integer'
    }
    return { typeName }
}

function validateAlignment(a: AlignmentUri | undefined): string | null {
    if (a === null || typeof a !== 'object') return 'each alignment must be an object'
    if (typeof a.ontology !== 'string' || !ONTOLOGY_RE.test(a.ontology)) {
        return `invalid ontology: ${JSON.stringify(a.ontology)}`
    }
    if (typeof a.term !== 'string' || !TERM_RE.test(a.term)) {
        return `invalid term: ${JSON.stringify(a.term)}`
    }
    return null
}

function validateFieldAlignment(
    fa: TypeFieldAlignment | undefined,
    seen: Set<string>,
): string | null {
    if (fa === null || typeof fa !== 'object') return 'each entry must be an object'
    if (typeof fa.field !== 'string'
        || fa.field.length === 0
        || fa.field.length > MAX_FIELD_NAME_LEN
        || !FIELD_NAME_RE.test(fa.field)
    ) {
        return `invalid field name: ${JSON.stringify(fa.field)}`
    }
    if (seen.has(fa.field)) return `duplicate field: ${fa.field}`
    seen.add(fa.field)
    if (fa.alignment !== null) {
        const err = validateAlignment(fa.alignment)
        if (err !== null) return `${fa.field}: ${err}`
    }
    return null
}

function clampInt(
    raw: string | null,
    fallback: number,
    min: number,
    max: number,
): number {
    if (raw === null) return fallback
    const n = parseInt(raw, 10)
    if (!Number.isFinite(n)) return fallback
    return Math.min(max, Math.max(min, n))
}

// Re-export so the router can reuse the same name check.
export { validateSegment }
