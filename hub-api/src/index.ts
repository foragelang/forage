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
} from './routes/recipes'

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

    if (path === '/v1/recipes' && request.method === 'GET') {
        return listRecipes(request, env)
    }
    if (path === '/v1/recipes' && request.method === 'POST') {
        return publishRecipe(request, env)
    }

    const detailMatch = path.match(/^\/v1\/recipes\/([^/]+)$/)
    if (detailMatch) {
        const slug = decodeURIComponent(detailMatch[1])
        if (request.method === 'GET') {
            const url = new URL(request.url)
            return getRecipe(request, env, slug, url.searchParams.get('version'))
        }
        if (request.method === 'DELETE') {
            return deleteRecipe(request, env, slug)
        }
    }

    const versionsMatch = path.match(/^\/v1\/recipes\/([^/]+)\/versions$/)
    if (versionsMatch && request.method === 'GET') {
        return getRecipeVersionsHandler(
            request,
            env,
            decodeURIComponent(versionsMatch[1]),
        )
    }

    const fixturesMatch = path.match(/^\/v1\/recipes\/([^/]+)\/fixtures$/)
    if (fixturesMatch && request.method === 'GET') {
        return getRecipeFixtures(
            request,
            env,
            decodeURIComponent(fixturesMatch[1]),
        )
    }

    const snapshotMatch = path.match(/^\/v1\/recipes\/([^/]+)\/snapshot$/)
    if (snapshotMatch && request.method === 'GET') {
        return getRecipeSnapshot(
            request,
            env,
            decodeURIComponent(snapshotMatch[1]),
        )
    }

    return jsonError(404, 'no_route', `no route for ${request.method} ${path}`)
}
