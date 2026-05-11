// HS256 JWT signing and verification for hub-api authentication.
//
// We use HS256 (HMAC-SHA-256) with the Worker's `JWT_SIGNING_KEY`
// secret. JWTs are issued by the OAuth endpoints after a successful
// GitHub OAuth exchange and carry the user's GitHub login as `sub`.
//
// Refresh tokens are separate longer-lived JWTs with a `typ: "refresh"`
// claim, so the access-token verifier rejects them by audience.

export interface JWTClaims {
    sub: string                 // GitHub login (e.g. "alice")
    iat: number                 // Issued-at, unix seconds
    exp: number                 // Expires-at, unix seconds
    aud: string                 // "forage-hub:access" or "forage-hub:refresh"
    typ?: 'access' | 'refresh'  // Convenience; same info encoded in `aud`
}

export const ACCESS_TOKEN_TTL_SECONDS = 60 * 60          // 1 hour
export const REFRESH_TOKEN_TTL_SECONDS = 60 * 60 * 24 * 30  // 30 days

const ACCESS_AUD = 'forage-hub:access'
const REFRESH_AUD = 'forage-hub:refresh'

/// Mint an access token for the given GitHub login. Expires in 1 hour.
export async function signAccessToken(login: string, signingKey: string): Promise<string> {
    const now = Math.floor(Date.now() / 1000)
    return signJWT({
        sub: login,
        iat: now,
        exp: now + ACCESS_TOKEN_TTL_SECONDS,
        aud: ACCESS_AUD,
        typ: 'access',
    }, signingKey)
}

/// Mint a refresh token for the given GitHub login. Expires in 30 days.
export async function signRefreshToken(login: string, signingKey: string): Promise<string> {
    const now = Math.floor(Date.now() / 1000)
    return signJWT({
        sub: login,
        iat: now,
        exp: now + REFRESH_TOKEN_TTL_SECONDS,
        aud: REFRESH_AUD,
        typ: 'refresh',
    }, signingKey)
}

/// Verify an access token and return its claims. Returns `null` if the
/// token is malformed, fails signature, is expired, or carries the
/// wrong audience.
export async function verifyAccessToken(token: string, signingKey: string): Promise<JWTClaims | null> {
    return verifyJWT(token, signingKey, ACCESS_AUD)
}

/// Verify a refresh token and return its claims. Same rules; only
/// `forage-hub:refresh`-audience tokens validate here.
export async function verifyRefreshToken(token: string, signingKey: string): Promise<JWTClaims | null> {
    return verifyJWT(token, signingKey, REFRESH_AUD)
}

// MARK: - Internal

async function signJWT(claims: JWTClaims, signingKey: string): Promise<string> {
    const header = { alg: 'HS256', typ: 'JWT' }
    const headerB64 = base64UrlEncode(new TextEncoder().encode(JSON.stringify(header)))
    const payloadB64 = base64UrlEncode(new TextEncoder().encode(JSON.stringify(claims)))
    const signingInput = `${headerB64}.${payloadB64}`
    const sig = await hmacSha256(signingInput, signingKey)
    return `${signingInput}.${base64UrlEncode(sig)}`
}

async function verifyJWT(token: string, signingKey: string, expectedAud: string): Promise<JWTClaims | null> {
    const parts = token.split('.')
    if (parts.length !== 3) return null
    const [headerB64, payloadB64, sigB64] = parts
    const signingInput = `${headerB64}.${payloadB64}`
    const expectedSig = await hmacSha256(signingInput, signingKey)
    const actualSig = base64UrlDecode(sigB64)
    if (!timingSafeEqual(expectedSig, actualSig)) return null

    let claims: JWTClaims
    try {
        const payload = base64UrlDecode(payloadB64)
        claims = JSON.parse(new TextDecoder().decode(payload)) as JWTClaims
    } catch {
        return null
    }
    if (claims.aud !== expectedAud) return null
    const now = Math.floor(Date.now() / 1000)
    if (typeof claims.exp !== 'number' || claims.exp < now) return null
    if (typeof claims.iat !== 'number' || claims.iat > now + 60) return null  // tolerate 60s clock skew
    if (typeof claims.sub !== 'string' || claims.sub.length === 0) return null
    return claims
}

async function hmacSha256(message: string, key: string): Promise<Uint8Array> {
    const enc = new TextEncoder()
    const cryptoKey = await crypto.subtle.importKey(
        'raw',
        enc.encode(key),
        { name: 'HMAC', hash: 'SHA-256' },
        false,
        ['sign']
    )
    const sig = await crypto.subtle.sign('HMAC', cryptoKey, enc.encode(message))
    return new Uint8Array(sig)
}

function base64UrlEncode(bytes: Uint8Array): string {
    let s = ''
    for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i])
    return btoa(s).replace(/=+$/g, '').replace(/\+/g, '-').replace(/\//g, '_')
}

function base64UrlDecode(s: string): Uint8Array {
    const pad = s.length % 4 === 0 ? '' : '='.repeat(4 - (s.length % 4))
    const b64 = (s + pad).replace(/-/g, '+').replace(/_/g, '/')
    const binary = atob(b64)
    const bytes = new Uint8Array(binary.length)
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i)
    return bytes
}

function timingSafeEqual(a: Uint8Array, b: Uint8Array): boolean {
    if (a.length !== b.length) return false
    let diff = 0
    for (let i = 0; i < a.length; i++) diff |= a[i] ^ b[i]
    return diff === 0
}
