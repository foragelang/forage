import type { Env } from './types'
import { verifyAccessToken } from './jwt'

/// Caller identity resolved from the request's `Authorization` header
/// (or a `forage_at` cookie, for the web IDE). Values:
///
///   - `{ kind: 'user', login }` — verified JWT issued by /v1/oauth/*.
///     `login` is always lowercase (GitHub logins are case-insensitive
///     and the rest of the system keys everything by the lowercase
///     form). The normalization happens here at the verification
///     boundary so every caller downstream can rely on the invariant.
///   - `{ kind: 'admin' }`       — legacy HUB_PUBLISH_TOKEN bearer
///   - `null`                    — unauthenticated
export type Caller =
    | { kind: 'user'; login: string }
    | { kind: 'admin' }
    | null

/// Identify the caller. Tries JWT first (Authorization bearer or
/// forage_at cookie), then falls back to the legacy HUB_PUBLISH_TOKEN
/// admin check. Returns `null` for unauthenticated.
export async function identifyCaller(request: Request, env: Env): Promise<Caller> {
    // 1) JWT path — Authorization: Bearer <jwt>
    const auth = request.headers.get('Authorization')
    if (auth) {
        const [scheme, token] = auth.split(/\s+/, 2)
        if (scheme?.toLowerCase() === 'bearer' && token) {
            if (env.JWT_SIGNING_KEY) {
                const claims = await verifyAccessToken(token, env.JWT_SIGNING_KEY)
                if (claims) return { kind: 'user', login: claims.sub.toLowerCase() }
            }
            // 2) Legacy admin token — same Authorization scheme.
            if (env.HUB_PUBLISH_TOKEN && timingSafeEqual(token, env.HUB_PUBLISH_TOKEN)) {
                return { kind: 'admin' }
            }
        }
    }
    // 3) Cookie path — forage_at=<jwt> (web IDE).
    if (env.JWT_SIGNING_KEY) {
        const cookieHeader = request.headers.get('Cookie') ?? ''
        const match = /(?:^|;\s*)forage_at=([^;]+)/.exec(cookieHeader)
        if (match) {
            const claims = await verifyAccessToken(match[1], env.JWT_SIGNING_KEY)
            if (claims) return { kind: 'user', login: claims.sub.toLowerCase() }
        }
    }
    return null
}

/// True when the caller may write to a recipe owned by `ownerLogin`.
/// Admin can write to anything; the original owner can write to theirs;
/// `undefined` owner (legacy / pre-M11 recipes) is admin-owned by
/// convention. Both sides of the comparison are lowercase by contract
/// — `caller.login` from `identifyCaller`, `ownerLogin` from the KV
/// `pkg:` record (which is also keyed lowercase).
export function callerCanWrite(caller: Caller, ownerLogin: string | undefined): boolean {
    if (caller === null) return false
    if (caller.kind === 'admin') return true
    const owner = ownerLogin ?? 'admin'
    if (owner === 'admin') return false  // only admin may rewrite legacy entries
    return caller.kind === 'user' && caller.login === owner
}

/// Back-compat: existing code calls `isAuthorized(...)` for the publish
/// gate. Returns true iff the caller is admin OR any authenticated
/// user. Routes that need ownership checks call `identifyCaller` +
/// `callerCanWrite` directly.
export async function isAuthorized(request: Request, env: Env): Promise<boolean> {
    return (await identifyCaller(request, env)) !== null
}

function timingSafeEqual(a: string, b: string): boolean {
    if (a.length !== b.length) return false
    let diff = 0
    for (let i = 0; i < a.length; i++) {
        diff |= a.charCodeAt(i) ^ b.charCodeAt(i)
    }
    return diff === 0
}
