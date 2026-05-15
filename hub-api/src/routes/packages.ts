import type {
    Env,
    PackageFile,
    PackageFixture,
    PackageListing,
    PackageMetadata,
    PackageSnapshot,
    PackageVersion,
    PublishRequest,
    ForkedFrom,
    ListPackagesResponse,
} from '../types'
import {
    getPackage,
    putPackage,
    getVersion,
    putVersion,
    indexAddPackage,
    indexAddUserPackage,
    indexAddCategory,
    indexRemoveCategory,
    listPackagesIndex,
    listCategoryIndex,
    ref,
    splitRef,
} from '../storage'
import { identifyCaller, callerCanWrite } from '../auth'
import { json, jsonError } from '../http'

// Segment shapes. `author` matches a GitHub login; `slug` matches a
// package slug. Both are lowercase to keep KV keys deterministic.
const SEGMENT_RE = /^[a-z0-9][a-z0-9-]{0,38}$/

// Body shape limits. The whole publish envelope is bounded so a 50 MB
// drive-by never spins up a worker. The recipe + one decls file +
// fixtures + snapshot ride together; the largest known fixture (the
// cannabis captures) is several MB.
const MAX_PUBLISH_PAYLOAD = 64 * 1024 * 1024 // 64 MiB envelope ceiling
const MAX_RECIPE_BYTES = 1 * 1024 * 1024 // 1 MiB per .forage file
const MAX_DECLS_FILES = 64
const MAX_FIXTURES_FILES = 16
const MAX_TAGS = 16
const MAX_CATEGORY_LEN = 64
const MAX_DESCRIPTION_LEN = 2048

// Captures the recipe header name out of the publish payload's
// `recipe` field. The hub-side slug equals the recipe's header name —
// keying data dirs, sidecars, and daemon state off a value that
// silently drifts from the URL slug would break the round-trip — so
// publish-time validation enforces the match.
const RECIPE_HEAD_NAME_RE = /^\s*(?:\/\/[^\n]*\n|\/\*[\s\S]*?\*\/|\s)*recipe\s+"([^"]+)"/

const FILE_NAME_RE = /^[a-z0-9][a-z0-9._\-]*(?:\/[a-z0-9][a-z0-9._\-]*)*\.forage$/i
const FIXTURE_NAME_RE = /^[a-zA-Z0-9][a-zA-Z0-9._\-]{0,127}$/
const CATEGORY_RE = /^[a-z0-9][a-z0-9-]*$/

// --- Listing ------------------------------------------------------------

// `GET /v1/packages?category=&sort=&q=&cursor=&limit=`
//
// Filters: optional `category` exact match, optional `q` substring
// match against slug + description (case-insensitive), optional `sort`
// = `recent` (default), `stars`, `downloads`.
export async function listPackages(
    request: Request,
    env: Env,
): Promise<Response> {
    const url = new URL(request.url)
    const category = url.searchParams.get('category')
    const q = url.searchParams.get('q')?.toLowerCase() ?? null
    const sort = url.searchParams.get('sort') ?? 'recent'
    const limit = clampInt(url.searchParams.get('limit'), 20, 1, 100)
    const cursor = url.searchParams.get('cursor')

    const refs = category !== null
        ? await listCategoryIndex(env, category)
        : await listPackagesIndex(env)

    const metas: PackageMetadata[] = []
    for (const r of refs) {
        const [a, s] = splitRef(r)
        const meta = await getPackage(env, a, s)
        if (meta === null) continue
        if (q !== null) {
            const hay = `${meta.author}/${meta.slug} ${meta.description}`.toLowerCase()
            if (!hay.includes(q)) continue
        }
        metas.push(meta)
    }

    if (sort === 'stars') {
        metas.sort((a, b) => b.stars - a.stars)
    } else if (sort === 'downloads') {
        metas.sort((a, b) => b.downloads - a.downloads)
    } else if (sort === 'recent') {
        metas.sort((a, b) => b.created_at - a.created_at)
    } else {
        return jsonError(400, 'bad_sort', `unknown sort: ${sort}`, {}, request)
    }

    const startIdx = cursor !== null
        ? Math.max(0, metas.findIndex((m) => ref(m.author, m.slug) === cursor) + 1)
        : 0
    const slice = metas.slice(startIdx, startIdx + limit)
    const nextCursor = startIdx + limit < metas.length
        ? ref(slice[slice.length - 1].author, slice[slice.length - 1].slug)
        : null

    const body: ListPackagesResponse = {
        items: slice.map(toListing),
        next_cursor: nextCursor,
    }
    return json(body, 200, request)
}

function toListing(meta: PackageMetadata): PackageListing {
    return {
        author: meta.author,
        slug: meta.slug,
        description: meta.description,
        category: meta.category,
        tags: meta.tags,
        forked_from: meta.forked_from,
        created_at: meta.created_at,
        latest_version: meta.latest_version,
        stars: meta.stars,
        downloads: meta.downloads,
        fork_count: meta.fork_count,
    }
}

// --- Detail -------------------------------------------------------------

// `GET /v1/packages/:author/:slug`
export async function getPackageDetail(
    request: Request,
    env: Env,
    author: string,
    slug: string,
): Promise<Response> {
    const meta = await getPackage(env, author, slug)
    if (meta === null) {
        return jsonError(404, 'not_found', `unknown package: ${author}/${slug}`, {}, request)
    }
    return json(meta, 200, request)
}

// --- Versions list -------------------------------------------------------

// `GET /v1/packages/:author/:slug/versions`
// Returns the linear version history with light metadata (numbers +
// timestamps). The full artifacts are at `/versions/:n`.
export async function listVersions(
    request: Request,
    env: Env,
    author: string,
    slug: string,
): Promise<Response> {
    const meta = await getPackage(env, author, slug)
    if (meta === null) {
        return jsonError(404, 'not_found', `unknown package: ${author}/${slug}`, {}, request)
    }
    const items: Array<{ version: number; published_at: number; published_by: string }> = []
    for (let n = 1; n <= meta.latest_version; n++) {
        const v = await getVersion(env, author, slug, n)
        if (v === null) continue
        items.push({
            version: v.version,
            published_at: v.published_at,
            published_by: v.published_by,
        })
    }
    return json({ items }, 200, request)
}

// --- Single version artifact --------------------------------------------

// `GET /v1/packages/:author/:slug/versions/:n` (n = number | 'latest')
export async function getVersionArtifact(
    request: Request,
    env: Env,
    author: string,
    slug: string,
    versionSpec: string,
): Promise<Response> {
    const meta = await getPackage(env, author, slug)
    if (meta === null) {
        return jsonError(404, 'not_found', `unknown package: ${author}/${slug}`, {}, request)
    }
    const n = versionSpec === 'latest'
        ? meta.latest_version
        : parseInt(versionSpec, 10)
    if (!Number.isFinite(n) || n < 1) {
        return jsonError(400, 'bad_version', `invalid version: ${versionSpec}`, {}, request)
    }
    const artifact = await getVersion(env, author, slug, n)
    if (artifact === null) {
        return jsonError(404, 'not_found', `unknown version ${author}/${slug}@${n}`, {}, request)
    }
    return json(artifact, 200, request)
}

// --- Publish -------------------------------------------------------------

// `POST /v1/packages/:author/:slug/versions`
//
// First publish (`base_version: null`) accepts iff (author, slug)
// doesn't exist; creates v1. Subsequent publishes require
// `base_version == latest_version`; mismatch returns 409 with the
// current latest in the body.
export async function publishVersion(
    request: Request,
    env: Env,
    author: string,
    slug: string,
): Promise<Response> {
    const caller = await identifyCaller(request, env)
    if (caller === null) {
        return jsonError(401, 'unauthorized', 'missing or invalid bearer token', {}, request)
    }
    // Caller may only publish under their own author namespace (or be admin).
    // Both `caller.login` and `author` are guaranteed lowercase here —
    // `identifyCaller` normalizes the JWT subject, and `author` already
    // passed the lowercase-only `SEGMENT_RE` in the router.
    const callerLogin = caller.kind === 'user' ? caller.login : null
    if (callerLogin !== null && callerLogin !== author) {
        return jsonError(
            403,
            'forbidden',
            `you are signed in as @${callerLogin}; cannot publish to @${author}`,
            {},
            request,
        )
    }

    const declaredLen = request.headers.get('content-length')
    if (declaredLen !== null && Number(declaredLen) > MAX_PUBLISH_PAYLOAD) {
        return jsonError(
            413,
            'payload_too_large',
            `publish envelope exceeds ${MAX_PUBLISH_PAYLOAD} bytes`,
            {},
            request,
        )
    }

    let payload: PublishRequest
    try {
        payload = (await request.json()) as PublishRequest
    } catch {
        return jsonError(400, 'bad_json', 'request body is not valid JSON', {}, request)
    }

    const validation = validatePublish(payload)
    if (typeof validation === 'string') {
        return jsonError(400, 'invalid', validation, {}, request)
    }
    if (validation.recipeName !== slug) {
        return jsonError(
            400,
            'slug_mismatch',
            `publish slug ${slug} does not match recipe header name ${validation.recipeName}`,
            {},
            request,
        )
    }

    const existing = await getPackage(env, author, slug)

    // Stale-base check.
    if (existing === null) {
        if (payload.base_version !== null) {
            return jsonError(
                409,
                'stale_base',
                `package ${author}/${slug} does not exist yet; first publish must use base_version: null`,
                { latest_version: 0, your_base: payload.base_version },
                request,
            )
        }
    } else {
        if (!callerCanWrite(caller, existing.owner_login)) {
            return jsonError(
                403,
                'forbidden',
                `${author}/${slug} is owned by @${existing.owner_login}`,
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

    const ownerLogin = existing?.owner_login
        ?? (caller.kind === 'user' ? caller.login : 'admin')
    const publishedBy = caller.kind === 'user' ? caller.login : 'admin'

    const now = Date.now()
    const nextVersion = existing === null ? 1 : existing.latest_version + 1

    const artifact: PackageVersion = {
        author,
        slug,
        version: nextVersion,
        recipe: payload.recipe,
        decls: payload.decls,
        fixtures: payload.fixtures,
        snapshot: payload.snapshot,
        base_version: payload.base_version,
        published_at: now,
        published_by: publishedBy,
    }

    await putVersion(env, artifact)

    let oldCategory: string | null = null
    if (existing !== null && existing.category !== payload.category) {
        oldCategory = existing.category
    }

    // `forked_from` is server-owned: it's stamped at fork-creation
    // time and preserved across subsequent publishes against the
    // fork. The publish path never accepts it from the caller — the
    // `PublishRequest` wire type doesn't carry the field at all.
    const meta: PackageMetadata = {
        author,
        slug,
        description: payload.description,
        category: payload.category,
        tags: payload.tags,
        forked_from: existing?.forked_from ?? null,
        created_at: existing?.created_at ?? now,
        latest_version: nextVersion,
        stars: existing?.stars ?? 0,
        downloads: existing?.downloads ?? 0,
        fork_count: existing?.fork_count ?? 0,
        owner_login: ownerLogin,
    }

    await putPackage(env, meta)
    if (existing === null) {
        await indexAddPackage(env, author, slug)
        await indexAddUserPackage(env, author, slug)
    }
    await indexAddCategory(env, payload.category, author, slug)
    if (oldCategory !== null) {
        await indexRemoveCategory(env, oldCategory, author, slug)
    }

    return json(
        {
            author,
            slug,
            version: nextVersion,
            latest_version: nextVersion,
        },
        201,
        request,
    )
}

// --- Validation ----------------------------------------------------------

interface ValidatedPublish {
    recipeName: string
}

function validatePublish(payload: PublishRequest): ValidatedPublish | string {
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
        return `category must match ${CATEGORY_RE} (e.g. "dispensary")`
    }
    if (!Array.isArray(payload.tags) || payload.tags.length > MAX_TAGS) {
        return `tags must be an array of at most ${MAX_TAGS} strings`
    }
    for (const t of payload.tags) {
        if (typeof t !== 'string') return 'tags must be strings'
    }
    if (typeof payload.recipe !== 'string') {
        return 'recipe must be a string (the main .forage source)'
    }
    if (payload.recipe.length > MAX_RECIPE_BYTES) {
        return `recipe exceeds ${MAX_RECIPE_BYTES} bytes`
    }
    const headerMatch = payload.recipe.match(RECIPE_HEAD_NAME_RE)
    if (headerMatch === null) {
        return 'recipe must start with `recipe "..."` (after comments / whitespace)'
    }
    const recipeName = headerMatch[1]
    if (!Array.isArray(payload.decls) || payload.decls.length > MAX_DECLS_FILES) {
        return `decls must be an array of at most ${MAX_DECLS_FILES} files`
    }
    const seenDecls = new Set<string>()
    for (const f of payload.decls) {
        const err = validateFile(f, seenDecls, MAX_RECIPE_BYTES)
        if (err !== null) return `decls: ${err}`
    }
    if (!Array.isArray(payload.fixtures) || payload.fixtures.length > MAX_FIXTURES_FILES) {
        return `fixtures must be an array of at most ${MAX_FIXTURES_FILES} entries`
    }
    const seenFixtures = new Set<string>()
    for (const f of payload.fixtures) {
        const err = validateFixture(f, seenFixtures)
        if (err !== null) return `fixtures: ${err}`
    }
    if (payload.snapshot !== null) {
        const err = validateSnapshot(payload.snapshot)
        if (err !== null) return `snapshot: ${err}`
    }
    if (
        payload.base_version !== null
        && (typeof payload.base_version !== 'number'
            || !Number.isInteger(payload.base_version)
            || payload.base_version < 0)
    ) {
        return 'base_version must be null or a non-negative integer'
    }
    return { recipeName }
}

function validateFile(
    f: PackageFile | undefined,
    seen: Set<string>,
    maxBytes: number,
): string | null {
    if (f === null || typeof f !== 'object') return 'each file must be an object'
    if (typeof f.name !== 'string' || !FILE_NAME_RE.test(f.name) || f.name.includes('..')) {
        return `invalid file name: ${JSON.stringify(f.name)}`
    }
    if (seen.has(f.name)) return `duplicate file name: ${f.name}`
    seen.add(f.name)
    if (typeof f.source !== 'string') return `${f.name}: source must be a string`
    if (f.source.length > maxBytes) return `${f.name}: source exceeds ${maxBytes} bytes`
    return null
}

function validateFixture(
    f: PackageFixture | undefined,
    seen: Set<string>,
): string | null {
    if (f === null || typeof f !== 'object') return 'each fixture must be an object'
    if (typeof f.name !== 'string' || !FIXTURE_NAME_RE.test(f.name)) {
        return `invalid fixture name: ${JSON.stringify(f.name)}`
    }
    if (seen.has(f.name)) return `duplicate fixture name: ${f.name}`
    seen.add(f.name)
    if (typeof f.content !== 'string') return `${f.name}: content must be a string`
    return null
}

function validateSnapshot(s: PackageSnapshot): string | null {
    if (typeof s !== 'object') return 'must be an object'
    if (s.records === null || typeof s.records !== 'object' || Array.isArray(s.records)) {
        return 'records must be an object keyed by type name'
    }
    if (s.counts === null || typeof s.counts !== 'object' || Array.isArray(s.counts)) {
        return 'counts must be an object keyed by type name'
    }
    for (const [k, v] of Object.entries(s.counts)) {
        if (typeof v !== 'number' || !Number.isInteger(v) || v < 0) {
            return `counts.${k} must be a non-negative integer`
        }
    }
    return null
}

// --- Helpers -------------------------------------------------------------

export function validateSegment(s: string): boolean {
    return SEGMENT_RE.test(s)
}

export function validateSegments(author: string, slug: string): boolean {
    return SEGMENT_RE.test(author) && SEGMENT_RE.test(slug)
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

// `forked_from` reused by `forks.ts` when stamping the v1 metadata.
export function newForkedFrom(
    author: string,
    slug: string,
    version: number,
): ForkedFrom {
    return { author, slug, version }
}
