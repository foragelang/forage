import type {
    Env,
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
    blobKeyForBody,
    blobKeyForFixtures,
    blobKeyForSnapshot,
    blobKeyForMeta,
    putBlob,
    getBlob,
} from '../storage'
import { isAuthorized } from '../auth'
import { json, jsonError, streamFromR2 } from '../http'

const SLUG_RE = /^[a-z0-9][a-z0-9-]{1,63}$/
const RECIPE_HEAD_RE = /^\s*(?:\/\/[^\n]*\n|\/\*[\s\S]*?\*\/|\s)*recipe\s+"/

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
    let blobKey = meta.latestBlobKey

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
            blobKey = v.blobKey
        }
    }

    const obj = await getBlob(env, blobKey)
    if (!obj) return jsonError(500, 'blob_missing', 'recipe body missing from R2')
    const body = await obj.text()

    const detail: RecipeDetailResponse = { ...metaToListing(meta), version, body }
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
    if (!isAuthorized(request, env)) {
        return jsonError(401, 'unauthorized', 'missing or invalid bearer token')
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
    const now = new Date().toISOString()
    const version = (existing?.version ?? 0) + 1
    const sha256 = await sha256Hex(payload.body)
    const blobKey = blobKeyForBody(payload.slug, version)

    // Write the body first; on failure we won't have updated metadata yet.
    await putBlob(env, blobKey, payload.body, 'text/plain; charset=utf-8')

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

    const meta: RecipeMetadata = {
        slug: payload.slug,
        author: payload.author ?? existing?.author ?? null,
        displayName: payload.displayName,
        summary: payload.summary,
        tags: payload.tags ?? [],
        platform: payload.platform ?? existing?.platform ?? null,
        version,
        latestBlobKey: blobKey,
        sha256,
        createdAt: existing?.createdAt ?? now,
        updatedAt: now,
        deleted: false,
    }

    await putBlob(
        env,
        blobKeyForMeta(payload.slug, version),
        JSON.stringify(meta),
        'application/json',
    )

    const versions = await getRecipeVersions(env, payload.slug)
    versions.push({ version, blobKey, publishedAt: now, sha256 })
    await putRecipeVersions(env, payload.slug, versions)
    await putRecipeMetadata(env, meta)
    await ensureSlugInIndex(env, payload.slug)

    const response: PublishResponse = { slug: payload.slug, version, sha256 }
    return json(response, 201)
}

function validatePublish(payload: PublishRequest): string | null {
    if (!payload || typeof payload !== 'object') return 'body must be an object'
    if (typeof payload.slug !== 'string' || !SLUG_RE.test(payload.slug)) {
        return 'slug must match ^[a-z0-9][a-z0-9-]{1,63}$'
    }
    if (typeof payload.displayName !== 'string' || !payload.displayName.trim()) {
        return 'displayName is required'
    }
    if (typeof payload.summary !== 'string') {
        return 'summary is required'
    }
    if (typeof payload.body !== 'string' || !payload.body.trim()) {
        return 'body is required'
    }
    if (!RECIPE_HEAD_RE.test(payload.body)) {
        return 'body does not look like a Forage recipe (expected `recipe "..."`)'
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

// --- Delete (soft) --------------------------------------------------------

export async function deleteRecipe(
    request: Request,
    env: Env,
    slug: string,
): Promise<Response> {
    if (!isAuthorized(request, env)) {
        return jsonError(401, 'unauthorized', 'missing or invalid bearer token')
    }
    const meta = await getRecipeMetadata(env, slug)
    if (!meta) return jsonError(404, 'not_found', `unknown slug: ${slug}`)
    if (meta.deleted) return new Response(null, { status: 204 })
    meta.deleted = true
    meta.updatedAt = new Date().toISOString()
    await putRecipeMetadata(env, meta)
    return new Response(null, { status: 204 })
}
