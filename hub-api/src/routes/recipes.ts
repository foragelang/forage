import type {
    Env,
    PackageFile,
    PublishRequest,
    PublishResponse,
    RecipeMetadata,
    ListingItem,
    ListingResponse,
    RecipeDetailResponse,
} from '../types'
import {
    sha256Hex,
    getRecipeMetadata,
    putRecipeMetadata,
    getRecipeVersions,
    putRecipeVersions,
    getSlugIndex,
    ensureSlugInIndex,
    blobKeyForFile,
    blobKeyForFixtures,
    blobKeyForSnapshot,
    blobKeyForMeta,
    putBlob,
    getBlob,
} from '../storage'
import { identifyCaller, callerCanWrite } from '../auth'
import { json, jsonError, streamFromR2 } from '../http'

// Each namespace / name segment matches the Worker's published shape.
const SEGMENT_RE = /^[a-z0-9][a-z0-9-]{1,63}$/
const SLUG_RE = /^[a-z0-9][a-z0-9-]{1,63}\/[a-z0-9][a-z0-9-]{1,63}$/
const RECIPE_HEAD_RE = /^\s*(?:\/\/[^\n]*\n|\/\*[\s\S]*?\*\/|\s)*recipe\s+"/

// Request body size limits (envelope JSON, not the raw .forage text).
// Each `.forage` file in the payload is bounded by `MAX_FILE_BODY`
// inside `validatePublish`; the overall envelope (sum of all files plus
// fixtures plus snapshot plus JSON overhead) is capped by
// `MAX_PUBLISH_PAYLOAD` so a 50MB publish never spins up a Worker.
const MAX_PUBLISH_PAYLOAD = 16 * 1024 * 1024 // 16 MiB
const MAX_FILE_BODY = 1 * 1024 * 1024 // 1 MiB per file

// --- Listing -------------------------------------------------------------

export async function listRecipes(
    request: Request,
    env: Env,
): Promise<Response> {
    const url = new URL(request.url)
    const author = url.searchParams.get('author')
    const tag = url.searchParams.get('tag')
    const platform = url.searchParams.get('platform')
    const limit = clampInt(url.searchParams.get('limit'), 20, 1, 100)
    const cursor = url.searchParams.get('cursor')

    const allSlugs = await getSlugIndex(env)
    const startIdx = cursor ? Math.max(0, allSlugs.indexOf(cursor) + 1) : 0
    const slice = allSlugs.slice(startIdx)

    const items: ListingItem[] = []
    let nextCursor: string | null = null

    for (const slug of slice) {
        const meta = await getRecipeMetadata(env, slug)
        if (!meta || meta.deleted) continue
        if (author && meta.author !== author) continue
        if (tag && !meta.tags.includes(tag)) continue
        if (platform && meta.platform !== platform) continue
        items.push(metaToListing(meta))
        if (items.length >= limit) {
            nextCursor = slug
            break
        }
    }

    const body: ListingResponse = { items, nextCursor }
    return json(body)
}

function metaToListing(meta: RecipeMetadata): ListingItem {
    return {
        slug: meta.slug,
        author: meta.author,
        displayName: meta.displayName,
        summary: meta.summary,
        tags: meta.tags,
        platform: meta.platform,
        version: meta.version,
        fileNames: meta.fileNames,
        sha256: meta.sha256,
        createdAt: meta.createdAt,
        updatedAt: meta.updatedAt,
    }
}

function clampInt(
    raw: string | null,
    fallback: number,
    min: number,
    max: number,
): number {
    if (!raw) return fallback
    const n = parseInt(raw, 10)
    if (!Number.isFinite(n)) return fallback
    return Math.min(max, Math.max(min, n))
}

// --- Detail --------------------------------------------------------------

export async function getRecipe(
    _request: Request,
    env: Env,
    slug: string,
    versionParam: string | null,
): Promise<Response> {
    const meta = await getRecipeMetadata(env, slug)
    if (!meta) return jsonError(404, 'not_found', `unknown slug: ${slug}`)
    if (meta.deleted) return jsonError(410, 'gone', `slug deleted: ${slug}`)

    let version = meta.version
    let fileNames = meta.fileNames

    if (versionParam) {
        const requested = parseInt(versionParam, 10)
        if (!Number.isFinite(requested) || requested < 1) {
            return jsonError(400, 'bad_version', `invalid version: ${versionParam}`)
        }
        if (requested !== meta.version) {
            const versions = await getRecipeVersions(env, slug)
            const v = versions.find((x) => x.version === requested)
            if (!v) return jsonError(404, 'not_found', `version ${requested} unknown`)
            version = v.version
            fileNames = v.fileNames
        }
    }

    // Pull every .forage file for the requested version. Returning
    // bodies inline is fine — packages are bounded by `MAX_FILE_BODY`
    // per file and a small ceiling on file count, so the response stays
    // a few MB at most.
    const files: PackageFile[] = []
    for (const name of fileNames) {
        const obj = await getBlob(env, blobKeyForFile(slug, version, name))
        if (!obj) {
            return jsonError(500, 'blob_missing', `package file missing from R2: ${name}`)
        }
        files.push({ name, body: await obj.text() })
    }

    const { fileNames: _drop, ...listing } = metaToListing(meta)
    const detail: RecipeDetailResponse = {
        ...listing,
        version,
        files,
    }
    return json(detail)
}

// --- Version history ------------------------------------------------------

export async function getRecipeVersionsHandler(
    _request: Request,
    env: Env,
    slug: string,
): Promise<Response> {
    const meta = await getRecipeMetadata(env, slug)
    if (!meta) return jsonError(404, 'not_found', `unknown slug: ${slug}`)
    if (meta.deleted) return jsonError(410, 'gone', `slug deleted: ${slug}`)
    const versions = await getRecipeVersions(env, slug)
    return json(
        versions.map((v) => ({
            version: v.version,
            fileNames: v.fileNames,
            publishedAt: v.publishedAt,
            sha256: v.sha256,
        })),
    )
}

// --- Fixtures + snapshot streams -----------------------------------------

export async function getRecipeFixtures(
    request: Request,
    env: Env,
    slug: string,
): Promise<Response> {
    const meta = await getRecipeMetadata(env, slug)
    if (!meta) return jsonError(404, 'not_found', `unknown slug: ${slug}`)
    if (meta.deleted) return jsonError(410, 'gone', `slug deleted: ${slug}`)
    const version = parseVersionOrLatest(request, meta.version)
    return streamFromR2(
        env,
        blobKeyForFixtures(slug, version),
        'application/x-jsonlines',
    )
}

export async function getRecipeSnapshot(
    request: Request,
    env: Env,
    slug: string,
): Promise<Response> {
    const meta = await getRecipeMetadata(env, slug)
    if (!meta) return jsonError(404, 'not_found', `unknown slug: ${slug}`)
    if (meta.deleted) return jsonError(410, 'gone', `slug deleted: ${slug}`)
    const version = parseVersionOrLatest(request, meta.version)
    return streamFromR2(
        env,
        blobKeyForSnapshot(slug, version),
        'application/json',
    )
}

function parseVersionOrLatest(request: Request, latest: number): number {
    const url = new URL(request.url)
    const raw = url.searchParams.get('version')
    if (!raw) return latest
    const n = parseInt(raw, 10)
    return Number.isFinite(n) && n >= 1 ? n : latest
}

// --- Publish --------------------------------------------------------------

export async function publishRecipe(
    request: Request,
    env: Env,
): Promise<Response> {
    const caller = await identifyCaller(request, env)
    if (caller === null) {
        return jsonError(401, 'unauthorized', 'missing or invalid bearer token')
    }

    // Reject oversize payloads up front so a 50MB request doesn't make
    // it past the gate. Cloudflare's hard ceiling is much higher; this
    // is a polite floor that catches accidents.
    const declaredLen = request.headers.get('content-length')
    if (declaredLen && Number(declaredLen) > MAX_PUBLISH_PAYLOAD) {
        return jsonError(
            413,
            'payload_too_large',
            `publish payload exceeds limit (${MAX_PUBLISH_PAYLOAD} bytes)`,
        )
    }

    let payload: PublishRequest
    try {
        payload = (await request.json()) as PublishRequest
    } catch {
        return jsonError(400, 'bad_json', 'request body is not valid JSON')
    }

    const validationError = validatePublish(payload)
    if (validationError) return jsonError(400, 'invalid', validationError)

    const existing = await getRecipeMetadata(env, payload.slug)
    if (existing && !callerCanWrite(caller, existing.ownerLogin)) {
        return jsonError(
            403,
            'forbidden',
            `package ${payload.slug} is owned by ${existing.ownerLogin ?? 'admin'}; sign in as that user to publish a new version`
        )
    }
    const now = new Date().toISOString()
    const version = (existing?.version ?? 0) + 1
    const fileNames = payload.files.map((f) => f.name)
    // SHA over the concatenated package contents, in declared order.
    // Stable wire-side hash so consumers can verify package integrity
    // independent of R2's internal etags.
    const concat = payload.files.map((f) => `${f.name}\n${f.body}\n`).join('')
    const sha256 = await sha256Hex(concat)

    // Write each .forage file. On failure we won't have updated
    // metadata yet, so a partial publish is a no-op from the catalog's
    // point of view (orphan blobs only).
    for (const f of payload.files) {
        await putBlob(
            env,
            blobKeyForFile(payload.slug, version, f.name),
            f.body,
            'text/plain; charset=utf-8',
        )
    }

    if (payload.fixtures && payload.fixtures.length > 0) {
        await putBlob(
            env,
            blobKeyForFixtures(payload.slug, version),
            payload.fixtures,
            'application/x-jsonlines',
        )
    }
    if (payload.snapshot !== undefined) {
        await putBlob(
            env,
            blobKeyForSnapshot(payload.slug, version),
            JSON.stringify(payload.snapshot),
            'application/json',
        )
    }

    // M11: first-time publish via OAuth establishes ownership; subsequent
    // versions retain the existing owner. Admin publishes (legacy
    // HUB_PUBLISH_TOKEN) stamp `ownerLogin: "admin"` unless overriding
    // an existing user-owned package (which the ownership check above
    // already permits for admin).
    const ownerLogin: string = existing?.ownerLogin
        ?? (caller.kind === 'user' ? caller.login : 'admin')

    const meta: RecipeMetadata = {
        slug: payload.slug,
        author: payload.author ?? existing?.author ?? null,
        displayName: payload.displayName,
        summary: payload.summary,
        tags: payload.tags ?? [],
        platform: payload.platform ?? existing?.platform ?? null,
        version,
        fileNames,
        sha256,
        createdAt: existing?.createdAt ?? now,
        updatedAt: now,
        deleted: false,
        ownerLogin,
    }

    await putBlob(
        env,
        blobKeyForMeta(payload.slug, version),
        JSON.stringify(meta),
        'application/json',
    )

    const versions = await getRecipeVersions(env, payload.slug)
    versions.push({ version, fileNames, publishedAt: now, sha256 })
    await putRecipeVersions(env, payload.slug, versions)
    await putRecipeMetadata(env, meta)
    await ensureSlugInIndex(env, payload.slug)

    const response: PublishResponse = { slug: payload.slug, version, sha256 }
    return json(response, 201)
}

// `name` must be a `.forage` path made of one or more `/`-joined
// segments. Each segment begins with `[a-z0-9]` and is built from
// `[a-z0-9._-]+`; segments are joined by single `/` separators (no
// `//`, no `/./`, no traversal). Total length is bounded separately
// (`name.length <= 128`) so the regex doesn't need a `{0,N}` ceiling.
const FILE_NAME_RE = /^[a-z0-9][a-z0-9._\-]*(?:\/[a-z0-9][a-z0-9._\-]*)*\.forage$/i
const MAX_FILE_NAME_LEN = 128
const MAX_FILES_PER_PACKAGE = 64

function validatePublish(payload: PublishRequest): string | null {
    if (!payload || typeof payload !== 'object') return 'body must be an object'
    if (typeof payload.slug !== 'string' || !SLUG_RE.test(payload.slug)) {
        return 'slug must be <namespace>/<name>; each segment matches ^[a-z0-9][a-z0-9-]{1,63}$'
    }
    if (typeof payload.displayName !== 'string' || !payload.displayName.trim()) {
        return 'displayName is required'
    }
    if (typeof payload.summary !== 'string') {
        return 'summary is required'
    }
    if (!Array.isArray(payload.files) || payload.files.length === 0) {
        return 'files must be a non-empty array of { name, body }'
    }
    if (payload.files.length > MAX_FILES_PER_PACKAGE) {
        return `packages may contain at most ${MAX_FILES_PER_PACKAGE} files`
    }
    let sawRecipe = false
    const seen = new Set<string>()
    for (const f of payload.files) {
        if (!f || typeof f !== 'object') return 'each file must be an object'
        if (
            typeof f.name !== 'string'
            || f.name.length > MAX_FILE_NAME_LEN
            || !FILE_NAME_RE.test(f.name)
            || f.name.includes('..')
        ) {
            return `invalid file name: ${JSON.stringify(f.name)} (expected a .forage path)`
        }
        if (seen.has(f.name)) return `duplicate file name in package: ${f.name}`
        seen.add(f.name)
        if (typeof f.body !== 'string') return `file ${f.name}: body must be a string`
        if (f.body.length > MAX_FILE_BODY) {
            return `file ${f.name}: source exceeds limit (${MAX_FILE_BODY} bytes)`
        }
        if (RECIPE_HEAD_RE.test(f.body)) sawRecipe = true
    }
    if (!sawRecipe) {
        return 'package must contain at least one recipe file (starting with `recipe "..."`)'
    }
    if (payload.tags !== undefined && !Array.isArray(payload.tags)) {
        return 'tags must be an array if provided'
    }
    if (payload.tags) {
        for (const tag of payload.tags) {
            if (typeof tag !== 'string') return 'tags must be strings'
        }
    }
    if (payload.fixtures !== undefined && typeof payload.fixtures !== 'string') {
        return 'fixtures must be a string if provided'
    }
    return null
}

// --- Slug routing helpers -------------------------------------------------

/// Validate that a parsed `<namespace>/<name>` slug matches the published
/// shape. Returns the validated slug or `null` if either segment is bad.
export function validateSlugSegments(
    namespace: string,
    name: string,
): string | null {
    if (!SEGMENT_RE.test(namespace) || !SEGMENT_RE.test(name)) return null
    return `${namespace}/${name}`
}

// --- Delete (soft) --------------------------------------------------------

export async function deleteRecipe(
    request: Request,
    env: Env,
    slug: string,
): Promise<Response> {
    const caller = await identifyCaller(request, env)
    if (caller === null) {
        return jsonError(401, 'unauthorized', 'missing or invalid bearer token')
    }
    const meta = await getRecipeMetadata(env, slug)
    if (!meta) return jsonError(404, 'not_found', `unknown slug: ${slug}`)
    if (!callerCanWrite(caller, meta.ownerLogin)) {
        return jsonError(
            403,
            'forbidden',
            `recipe ${slug} is owned by ${meta.ownerLogin ?? 'admin'}; sign in as that user to delete it`
        )
    }
    if (meta.deleted) return new Response(null, { status: 204 })
    meta.deleted = true
    meta.updatedAt = new Date().toISOString()
    await putRecipeMetadata(env, meta)
    return new Response(null, { status: 204 })
}
