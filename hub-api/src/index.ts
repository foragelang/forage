import type { Env } from './types'
import { BUCKETS, callerKey, corsPreflight, json, jsonError, rateLimit } from './http'
import {
    listPackages,
    getPackageDetail,
    listVersions,
    getVersionArtifact,
    publishVersion,
    validateSegment,
    validateSegments,
} from './routes/packages'
import { addStar, removeStar, getStars } from './routes/stars'
import { recordDownload } from './routes/downloads'
import { createFork } from './routes/forks'
import { getProfile, getProfilePackages, getProfileStars } from './routes/users'
import { listCategories } from './routes/categories'
import {
    oauthStart,
    oauthCallback,
    oauthDevice,
    oauthDevicePoll,
    oauthRefresh,
    oauthRevoke,
    oauthWhoami,
} from './oauth'
import { identifyCaller } from './auth'

export default {
    async fetch(request: Request, env: Env): Promise<Response> {
        if (request.method === 'OPTIONS') return corsPreflight(request)
        const url = new URL(request.url)
        const path = url.pathname.replace(/\/+$/, '') || '/'
        try {
            return await route(request, env, path)
        } catch (err) {
            const message = err instanceof Error ? err.message : String(err)
            console.error('handler-error', message, err)
            return jsonError(500, 'internal', message, {}, request)
        }
    },
} satisfies ExportedHandler<Env>

async function route(
    request: Request,
    env: Env,
    path: string,
): Promise<Response> {
    if (path === '/v1/health' && request.method === 'GET') {
        return json({ status: 'ok', time: new Date().toISOString() }, 200, request)
    }

    // ----- OAuth ---------------------------------------------------------
    if (path === '/v1/oauth/start' && request.method === 'POST') {
        const limit = await rateLimit(env, 'oauthStart', callerKey(request, null), request)
        if (limit !== null) return limit
        return oauthStart(request, env)
    }
    if (path === '/v1/oauth/callback' && request.method === 'GET') {
        return oauthCallback(request, env)
    }
    if (path === '/v1/oauth/device' && request.method === 'POST') {
        const limit = await rateLimit(env, 'deviceStart', callerKey(request, null), request)
        if (limit !== null) return limit
        return oauthDevice(request, env)
    }
    if (path === '/v1/oauth/device/poll' && request.method === 'POST') {
        const limit = await rateLimit(env, 'devicePoll', callerKey(request, null), request)
        if (limit !== null) return limit
        return oauthDevicePoll(request, env)
    }
    if (path === '/v1/oauth/refresh' && request.method === 'POST') {
        return oauthRefresh(request, env)
    }
    if (path === '/v1/oauth/whoami' && request.method === 'GET') {
        const caller = await identifyCaller(request, env)
        const login = caller !== null && caller.kind === 'user' ? caller.login : null
        return oauthWhoami(request, env, login)
    }
    if (path === '/v1/oauth/revoke' && request.method === 'POST') {
        const caller = await identifyCaller(request, env)
        if (caller === null || caller.kind !== 'user') {
            return jsonError(401, 'unauthorized', 'sign-in required', {}, request)
        }
        return oauthRevoke(request, env, caller.login)
    }

    // ----- Top-level lists ----------------------------------------------
    if (path === '/v1/packages' && request.method === 'GET') {
        const limit = await rateLimit(env, 'read', callerKey(request, null), request)
        if (limit !== null) return limit
        return listPackages(request, env)
    }
    if (path === '/v1/categories' && request.method === 'GET') {
        const limit = await rateLimit(env, 'read', callerKey(request, null), request)
        if (limit !== null) return limit
        return listCategories(request, env)
    }

    // ----- /v1/users/:author* -------------------------------------------
    const userMatch = path.match(/^\/v1\/users\/([^/]+)(?:\/(packages|stars))?$/)
    if (userMatch !== null) {
        if (request.method !== 'GET') {
            return jsonError(405, 'method_not_allowed', `${request.method} not allowed`, {}, request)
        }
        // GitHub logins are case-insensitive and the rest of the
        // system keys users by the lowercase form. Canonicalize at the
        // boundary so handlers can rely on the invariant.
        const author = decodeURIComponent(userMatch[1]).toLowerCase()
        if (!validateSegment(author)) {
            return jsonError(400, 'bad_slug', `invalid author: ${author}`, {}, request)
        }
        const sub = userMatch[2] ?? null
        const limit = await rateLimit(env, 'read', callerKey(request, null), request)
        if (limit !== null) return limit
        if (sub === null) return getProfile(request, env, author)
        if (sub === 'packages') return getProfilePackages(request, env, author)
        if (sub === 'stars') return getProfileStars(request, env, author)
    }

    // ----- /v1/packages/:author/:slug[/...] ------------------------------
    const pkgMatch = path.match(/^\/v1\/packages\/([^/]+)\/([^/]+)(?:\/(versions|stars|downloads|fork)(?:\/([^/]+))?)?$/)
    if (pkgMatch !== null) {
        const author = decodeURIComponent(pkgMatch[1])
        const slug = decodeURIComponent(pkgMatch[2])
        if (!validateSegments(author, slug)) {
            return jsonError(400, 'bad_slug', `invalid author/slug: ${author}/${slug}`, {}, request)
        }
        const sub = pkgMatch[3] ?? null
        const subArg = pkgMatch[4] ?? null

        // Bare /:author/:slug — detail
        if (sub === null) {
            if (request.method !== 'GET') {
                return jsonError(405, 'method_not_allowed', `${request.method} not allowed`, {}, request)
            }
            const limit = await rateLimit(env, 'read', callerKey(request, null), request)
            if (limit !== null) return limit
            return getPackageDetail(request, env, author, slug)
        }

        // /versions[/:n|latest]
        if (sub === 'versions') {
            if (subArg === null && request.method === 'GET') {
                const limit = await rateLimit(env, 'read', callerKey(request, null), request)
                if (limit !== null) return limit
                return listVersions(request, env, author, slug)
            }
            if (subArg === null && request.method === 'POST') {
                const caller = await identifyCaller(request, env)
                const key = caller !== null && caller.kind === 'user'
                    ? caller.login
                    : callerKey(request, null)
                const limit = await rateLimit(env, 'publish', key, request)
                if (limit !== null) return limit
                return publishVersion(request, env, author, slug)
            }
            if (subArg !== null && request.method === 'GET') {
                const limit = await rateLimit(env, 'read', callerKey(request, null), request)
                if (limit !== null) return limit
                return getVersionArtifact(request, env, author, slug, subArg)
            }
            return jsonError(405, 'method_not_allowed', `${request.method} not allowed`, {}, request)
        }

        // /stars
        if (sub === 'stars' && subArg === null) {
            if (request.method === 'GET') {
                const limit = await rateLimit(env, 'read', callerKey(request, null), request)
                if (limit !== null) return limit
                return getStars(request, env, author, slug)
            }
            if (request.method === 'POST') return addStar(request, env, author, slug)
            if (request.method === 'DELETE') return removeStar(request, env, author, slug)
            return jsonError(405, 'method_not_allowed', `${request.method} not allowed`, {}, request)
        }

        // /downloads
        if (sub === 'downloads' && subArg === null && request.method === 'POST') {
            return recordDownload(request, env, author, slug)
        }

        // /fork
        if (sub === 'fork' && subArg === null && request.method === 'POST') {
            return createFork(request, env, author, slug)
        }
    }

    return jsonError(404, 'no_route', `no route for ${request.method} ${path}`, {}, request)
}

export { BUCKETS }
