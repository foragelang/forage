import type { Env } from './types'
import { corsPreflight, json, jsonError } from './http'
import {
    listRecipes,
    getRecipe,
    getRecipeVersionsHandler,
    getRecipeFixtures,
    getRecipeSnapshot,
    publishRecipe,
    deleteRecipe,
    validateSlugSegments,
} from './routes/recipes'
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
        if (request.method === 'OPTIONS') return corsPreflight()

        const url = new URL(request.url)
        const path = url.pathname.replace(/\/+$/, '') || '/'

        try {
            return await route(request, env, path)
        } catch (err) {
            const message = err instanceof Error ? err.message : String(err)
            console.error('handler-error', message, err)
            return jsonError(500, 'internal', message)
        }
    },
} satisfies ExportedHandler<Env>

async function route(
    request: Request,
    env: Env,
    path: string,
): Promise<Response> {
    if (path === '/v1/health' && request.method === 'GET') {
        return json({ status: 'ok', time: new Date().toISOString() })
    }

    // M11 OAuth endpoints.
    if (path === '/v1/oauth/start' && request.method === 'POST') return oauthStart(request, env)
    if (path === '/v1/oauth/callback' && request.method === 'GET') return oauthCallback(request, env)
    if (path === '/v1/oauth/device' && request.method === 'POST') return oauthDevice(request, env)
    if (path === '/v1/oauth/device/poll' && request.method === 'POST') return oauthDevicePoll(request, env)
    if (path === '/v1/oauth/refresh' && request.method === 'POST') return oauthRefresh(request, env)
    if (path === '/v1/oauth/whoami' && request.method === 'GET') {
        const caller = await identifyCaller(request, env)
        const login = caller?.kind === 'user' ? caller.login : null
        return oauthWhoami(request, env, login)
    }
    if (path === '/v1/oauth/revoke' && request.method === 'POST') {
        const caller = await identifyCaller(request, env)
        if (caller?.kind !== 'user') return jsonError(401, 'unauthorized', 'sign-in required')
        return oauthRevoke(request, env, caller.login)
    }

    if (path === '/v1/recipes' && request.method === 'GET') {
        return listRecipes(request, env)
    }
    if (path === '/v1/recipes' && request.method === 'POST') {
        return publishRecipe(request, env)
    }

    // Versions / fixtures / snapshot live under `/v1/recipes/:ns/:name/:sub`.
    // Match these *before* the bare detail route below, since the detail
    // route is also a two-segment match.
    const subMatch = path.match(/^\/v1\/recipes\/([^/]+)\/([^/]+)\/(versions|fixtures|snapshot)$/)
    if (subMatch) {
        const ns = decodeURIComponent(subMatch[1])
        const name = decodeURIComponent(subMatch[2])
        const slug = validateSlugSegments(ns, name)
        if (!slug) return jsonError(400, 'bad_slug', `invalid namespace/name: ${ns}/${name}`)
        if (request.method !== 'GET') {
            return jsonError(405, 'method_not_allowed', `${request.method} not allowed`)
        }
        if (subMatch[3] === 'versions') return getRecipeVersionsHandler(request, env, slug)
        if (subMatch[3] === 'fixtures') return getRecipeFixtures(request, env, slug)
        if (subMatch[3] === 'snapshot') return getRecipeSnapshot(request, env, slug)
    }

    // Detail / delete: `/v1/recipes/:namespace/:name`.
    const detailMatch = path.match(/^\/v1\/recipes\/([^/]+)\/([^/]+)$/)
    if (detailMatch) {
        const ns = decodeURIComponent(detailMatch[1])
        const name = decodeURIComponent(detailMatch[2])
        const slug = validateSlugSegments(ns, name)
        if (!slug) return jsonError(400, 'bad_slug', `invalid namespace/name: ${ns}/${name}`)
        if (request.method === 'GET') {
            const url = new URL(request.url)
            return getRecipe(request, env, slug, url.searchParams.get('version'))
        }
        if (request.method === 'DELETE') {
            return deleteRecipe(request, env, slug)
        }
    }

    return jsonError(404, 'no_route', `no route for ${request.method} ${path}`)
}
