import type {
    Env,
    PackageListing,
    Profile,
    ListProfilePackagesResponse,
    ListProfileStarsResponse,
    ProfileStar,
} from '../types'
import {
    getPackage,
    listUserPackagesIndex,
    listStarsByUser,
    countStarsByUser,
    splitRef,
} from '../storage'
import { json, jsonError } from '../http'

// User record stored by the OAuth flow at `user:<login>` (lowercase
// login). We read it for profile metadata; never write it from this
// module.
interface OAuthUserRecord {
    login: string
    name?: string
    avatarUrl?: string
    createdAt: string
}

// `GET /v1/users/:author`
export async function getProfile(
    request: Request,
    env: Env,
    author: string,
): Promise<Response> {
    const userRaw = await env.METADATA.get(`user:${author.toLowerCase()}`)

    // We synthesize a profile even when the OAuth record is missing
    // (admin / legacy publishes). The shape stays consistent for
    // clients; only the auxiliary fields go null.
    let name: string | null = null
    let avatarUrl: string | null = null
    let createdAt: number | null = null
    if (userRaw !== null) {
        const u = JSON.parse(userRaw) as OAuthUserRecord
        if (typeof u.name === 'string') name = u.name
        if (typeof u.avatarUrl === 'string') avatarUrl = u.avatarUrl
        if (typeof u.createdAt === 'string') {
            const t = Date.parse(u.createdAt)
            if (!Number.isNaN(t)) createdAt = t
        }
    }

    const packageRefs = await listUserPackagesIndex(env, author)
    const starCount = await countStarsByUser(env, author)

    // No packages and no OAuth record => 404. Otherwise we have
    // something to show.
    if (packageRefs.length === 0 && userRaw === null) {
        return jsonError(404, 'not_found', `unknown user: ${author}`, {}, request)
    }

    const body: Profile = {
        login: author,
        name,
        avatar_url: avatarUrl,
        created_at: createdAt ?? 0,
        package_count: packageRefs.length,
        star_count: starCount,
    }
    return json(body, 200, request)
}

// `GET /v1/users/:author/packages?cursor=&limit=`
//
// Cursor-paginated to bound the response when a user has many
// packages. The cursor is the `author/slug` ref of the last returned
// item; the next page starts after it. Default limit 50, max 100.
export async function getProfilePackages(
    request: Request,
    env: Env,
    author: string,
): Promise<Response> {
    const url = new URL(request.url)
    const cursor = url.searchParams.get('cursor')
    const limit = clampInt(url.searchParams.get('limit'), 50, 1, 100)

    const refs = await listUserPackagesIndex(env, author)
    const startIdx = cursor !== null
        ? Math.max(0, refs.indexOf(cursor) + 1)
        : 0
    const slice = refs.slice(startIdx, startIdx + limit)

    const items: PackageListing[] = []
    for (const r of slice) {
        const [a, s] = splitRef(r)
        const meta = await getPackage(env, a, s)
        if (meta === null) continue
        items.push({
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
        })
    }
    const nextCursor = startIdx + limit < refs.length
        ? slice[slice.length - 1]
        : null
    const body: ListProfilePackagesResponse = { items, next_cursor: nextCursor }
    return json(body, 200, request)
}

// `GET /v1/users/:author/stars`
export async function getProfileStars(
    request: Request,
    env: Env,
    author: string,
): Promise<Response> {
    const url = new URL(request.url)
    const cursor = url.searchParams.get('cursor')
    const rawLimit = url.searchParams.get('limit')
    const limit = clampInt(rawLimit, 100, 1, 500)
    const { items, nextCursor } = await listStarsByUser(env, author, cursor, limit)
    const formatted: ProfileStar[] = items.map((s) => ({
        author: s.author,
        slug: s.slug,
        starred_at: s.starred_at,
    }))
    const body: ListProfileStarsResponse = {
        items: formatted,
        next_cursor: nextCursor,
    }
    return json(body, 200, request)
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
