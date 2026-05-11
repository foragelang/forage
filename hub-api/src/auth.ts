import type { Env } from './types'

// Bearer-token check against the HUB_PUBLISH_TOKEN secret.
// Returns true if the request carries a valid token; false otherwise.
export function isAuthorized(request: Request, env: Env): boolean {
    const expected = env.HUB_PUBLISH_TOKEN
    if (!expected) return false
    const header = request.headers.get('Authorization')
    if (!header) return false
    const [scheme, token] = header.split(/\s+/, 2)
    if (scheme?.toLowerCase() !== 'bearer' || !token) return false
    // Constant-time comparison.
    return timingSafeEqual(token, expected)
}

function timingSafeEqual(a: string, b: string): boolean {
    if (a.length !== b.length) return false
    let diff = 0
    for (let i = 0; i < a.length; i++) {
        diff |= a.charCodeAt(i) ^ b.charCodeAt(i)
    }
    return diff === 0
}
