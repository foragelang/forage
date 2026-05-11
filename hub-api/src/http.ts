import type { Env } from './types'
import { getBlob } from './storage'

const CORS_HEADERS: Record<string, string> = {
    'Access-Control-Allow-Origin': '*',
    'Access-Control-Allow-Methods': 'GET,POST,DELETE,OPTIONS',
    'Access-Control-Allow-Headers': 'Authorization,Content-Type',
    'Access-Control-Max-Age': '86400',
}

export function withCors(response: Response): Response {
    const headers = new Headers(response.headers)
    for (const [k, v] of Object.entries(CORS_HEADERS)) headers.set(k, v)
    return new Response(response.body, {
        status: response.status,
        statusText: response.statusText,
        headers,
    })
}

export function corsPreflight(): Response {
    return new Response(null, { status: 204, headers: CORS_HEADERS })
}

export function json(body: unknown, status: number = 200): Response {
    return new Response(JSON.stringify(body), {
        status,
        headers: {
            'Content-Type': 'application/json; charset=utf-8',
            ...CORS_HEADERS,
        },
    })
}

export function jsonError(
    status: number,
    code: string,
    message: string,
): Response {
    return json({ error: { code, message } }, status)
}

export async function streamFromR2(
    env: Env,
    key: string,
    contentType: string,
): Promise<Response> {
    const obj = await getBlob(env, key)
    if (!obj) return jsonError(404, 'not_found', `blob not found: ${key}`)
    return new Response(obj.body, {
        status: 200,
        headers: {
            'Content-Type': contentType,
            'Cache-Control': 'public, max-age=300',
            ...CORS_HEADERS,
        },
    })
}
