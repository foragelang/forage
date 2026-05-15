import type { Env, ListStarsResponse } from '../types'
import {
    getPackage,
    putPackage,
    putStar,
    deleteStar,
    listStars,
    packageExists,
} from '../storage'
import { identifyCaller } from '../auth'
import { json, jsonError } from '../http'

// `POST /v1/packages/:author/:slug/stars`
export async function addStar(
    request: Request,
    env: Env,
    author: string,
    slug: string,
): Promise<Response> {
    const caller = await identifyCaller(request, env)
    if (caller === null || caller.kind !== 'user') {
        return jsonError(401, 'unauthorized', 'sign-in required to star', {}, request)
    }
    const meta = await getPackage(env, author, slug)
    if (meta === null) {
        return jsonError(404, 'not_found', `unknown package: ${author}/${slug}`, {}, request)
    }
    const { added, starredAt } = await putStar(env, author, slug, caller.login)
    if (added) {
        meta.stars += 1
        await putPackage(env, meta)
    }
    return json(
        {
            user: caller.login,
            starred_at: starredAt,
            stars: meta.stars,
            already_starred: !added,
        },
        added ? 201 : 200,
        request,
    )
}

// `DELETE /v1/packages/:author/:slug/stars`
export async function removeStar(
    request: Request,
    env: Env,
    author: string,
    slug: string,
): Promise<Response> {
    const caller = await identifyCaller(request, env)
    if (caller === null || caller.kind !== 'user') {
        return jsonError(401, 'unauthorized', 'sign-in required to unstar', {}, request)
    }
    const meta = await getPackage(env, author, slug)
    if (meta === null) {
        return jsonError(404, 'not_found', `unknown package: ${author}/${slug}`, {}, request)
    }
    const removed = await deleteStar(env, author, slug, caller.login)
    if (removed && meta.stars > 0) {
        meta.stars -= 1
        await putPackage(env, meta)
    }
    return json(
        { user: caller.login, removed, stars: meta.stars },
        200,
        request,
    )
}

// `GET /v1/packages/:author/:slug/stars?cursor=&limit=`
export async function getStars(
    request: Request,
    env: Env,
    author: string,
    slug: string,
): Promise<Response> {
    if (!(await packageExists(env, author, slug))) {
        return jsonError(404, 'not_found', `unknown package: ${author}/${slug}`, {}, request)
    }
    const url = new URL(request.url)
    const cursor = url.searchParams.get('cursor')
    const rawLimit = url.searchParams.get('limit')
    const limit = clampInt(rawLimit, 50, 1, 200)
    const { items, nextCursor } = await listStars(env, author, slug, cursor, limit)
    const body: ListStarsResponse = { items, next_cursor: nextCursor }
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
