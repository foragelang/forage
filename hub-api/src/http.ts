import type { Env } from './types'

// CORS allowlist. The Worker only ever responds to known origins;
// `*` would expose the hub to any third-party site, which is fine for
// reads but a leak risk for the publish + auth endpoints that depend
// on the Authorization header.
const CORS_ORIGIN_ALLOWLIST = new Set<string>([
    'https://hub.foragelang.com',
    'https://foragelang.com',
    'http://localhost:5173',
    'http://localhost:4173',
    'tauri://localhost',
    'https://tauri.localhost',
])

function pickOrigin(req: Request | null): string {
    if (!req) return 'https://hub.foragelang.com'
    const origin = req.headers.get('Origin') ?? ''
    if (CORS_ORIGIN_ALLOWLIST.has(origin)) return origin
    // Default to the canonical hub UI; non-allowlisted origins still get a
    // valid CORS header (just not theirs) so debugging is obvious.
    return 'https://hub.foragelang.com'
}

function corsHeaders(req: Request | null): Record<string, string> {
    return {
        'Access-Control-Allow-Origin': pickOrigin(req),
        // Wider than the current route table so future PATCH / PUT /
        // HEAD endpoints don't silently CORS-fail on first deploy.
        // Pre-1.0; not worth wiring a generator off the routing table.
        'Access-Control-Allow-Methods': 'GET,HEAD,POST,PUT,PATCH,DELETE,OPTIONS',
        'Access-Control-Allow-Headers': 'Authorization,Content-Type',
        'Access-Control-Allow-Credentials': 'true',
        'Access-Control-Max-Age': '86400',
        'Vary': 'Origin',
    }
}

export function withCors(response: Response, req: Request | null = null): Response {
    const headers = new Headers(response.headers)
    for (const [k, v] of Object.entries(corsHeaders(req))) headers.set(k, v)
    return new Response(response.body, {
        status: response.status,
        statusText: response.statusText,
        headers,
    })
}

export function corsPreflight(req: Request | null = null): Response {
    return new Response(null, { status: 204, headers: corsHeaders(req) })
}

export function json(body: unknown, status: number = 200, req: Request | null = null): Response {
    return new Response(JSON.stringify(body), {
        status,
        headers: {
            'Content-Type': 'application/json; charset=utf-8',
            ...corsHeaders(req),
        },
    })
}

// `application/ld+json` response. Same shape as `json()` but with the
// JSON-LD media type so clients (and any caching proxies that vary on
// Content-Type) treat the response as the JSON-LD profile of JSON.
export function jsonLd(
    body: unknown,
    status: number = 200,
    req: Request | null = null,
): Response {
    return new Response(JSON.stringify(body), {
        status,
        headers: {
            'Content-Type': 'application/ld+json; charset=utf-8',
            ...corsHeaders(req),
        },
    })
}

export function jsonError(
    status: number,
    code: string,
    message: string,
    extra: Record<string, unknown> = {},
    req: Request | null = null,
): Response {
    const body: Record<string, unknown> = { error: { code, message, ...extra } }
    return json(body, status, req)
}

// ---------------------------------------------------------------------------
// Rate limiting (KV-backed sliding window)
//
// One counter per (bucket, key) — keyed by `rl:<bucket>:<key>` in the
// METADATA KV namespace, expiring at the end of the window. Cloudflare KV's
// TTL is in seconds; we store {count, started_at} and reset when the
// stored window has expired by wall clock.
//
// Not as precise as a Durable Object, but it correctly throttles repeat
// abusers and matches the recipe-hub threat model (anonymous reads are
// fine; the choke points are publish / auth / device-poll).

export type Bucket = {
    /// Maximum requests within one window, per key.
    max: number
    /// Sliding window length in seconds.
    windowSec: number
}

export const BUCKETS = {
    publish: { max: 30, windowSec: 60 } satisfies Bucket,
    deviceStart: { max: 5, windowSec: 60 } satisfies Bucket,
    devicePoll: { max: 120, windowSec: 60 } satisfies Bucket,
    oauthStart: { max: 20, windowSec: 60 } satisfies Bucket,
    read: { max: 300, windowSec: 60 } satisfies Bucket,
    // Star / unstar / fork from an authenticated caller, plus the
    // anonymous download counter bump. Cheaper to hit than publish,
    // but we want a brake on token-fueled or IP-fueled abuse.
    social: { max: 120, windowSec: 60 } satisfies Bucket,
}

export async function rateLimit(
    env: Env,
    bucketName: keyof typeof BUCKETS,
    key: string,
    req: Request | null = null,
): Promise<Response | null> {
    const bucket = BUCKETS[bucketName]
    const kvKey = `rl:${bucketName}:${key}`
    const now = Date.now()
    const raw = await env.METADATA.get(kvKey)
    let count = 0
    let startedAt = now
    if (raw) {
        try {
            const parsed = JSON.parse(raw) as { count: number; startedAt: number }
            if (now - parsed.startedAt < bucket.windowSec * 1000) {
                count = parsed.count
                startedAt = parsed.startedAt
            }
        } catch {
            /* corrupt entry → reset */
        }
    }
    if (count >= bucket.max) {
        const retryAfter = Math.max(1, bucket.windowSec - Math.floor((now - startedAt) / 1000))
        const resp = jsonError(
            429,
            'rate_limited',
            `too many ${bucketName} requests; retry in ${retryAfter}s`,
            { retryAfter },
            req,
        )
        resp.headers.set('Retry-After', String(retryAfter))
        return resp
    }
    const next = JSON.stringify({ count: count + 1, startedAt })
    // Set TTL slightly longer than the window so the entry expires on its own.
    await env.METADATA.put(kvKey, next, { expirationTtl: bucket.windowSec + 30 })
    return null
}

export function callerKey(req: Request, login: string | null): string {
    if (login) return `user:${login}`
    const ip =
        req.headers.get('CF-Connecting-IP') ||
        req.headers.get('X-Forwarded-For') ||
        'unknown'
    return `ip:${ip}`
}
