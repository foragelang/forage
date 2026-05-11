// GitHub OAuth flow endpoints (M11). Two flows supported:
//
//   - Web (authorization-code with PKCE): `/v1/oauth/start` returns the
//     GitHub authorize URL + state; `/v1/oauth/callback` exchanges the
//     code, mints an httpOnly cookie + JWT, redirects back to the
//     supplied `returnTo` URL.
//   - Device-code (CLI / Toolkit): `/v1/oauth/device` initiates the
//     flow; `/v1/oauth/device/poll` returns 200+tokens once the user
//     completes the GitHub side, 202+pending otherwise.
//
// Both flows store user metadata + refresh token at `user:<gh-login>`
// in KV. Access tokens are stateless HS256 JWTs; refresh tokens are
// also JWTs (longer-lived) but tied to the stored refresh-token
// fingerprint so the server can revoke them by rotating the user's
// record.

import type { Env } from './types'
import { json, jsonError } from './http'
import {
    signAccessToken,
    signRefreshToken,
    verifyRefreshToken,
    ACCESS_TOKEN_TTL_SECONDS,
} from './jwt'

const GH_AUTHORIZE_URL = 'https://github.com/login/oauth/authorize'
const GH_TOKEN_URL = 'https://github.com/login/oauth/access_token'
const GH_DEVICE_CODE_URL = 'https://github.com/login/device/code'
const GH_USER_URL = 'https://api.github.com/user'

const STATE_TTL_SECONDS = 60 * 10  // 10 minutes for an in-flight web flow
const DEVICE_PENDING_TTL_SECONDS = 60 * 15  // GitHub device codes last ~15 min

interface UserRecord {
    login: string
    name?: string
    avatarUrl?: string
    refreshTokenFingerprint: string  // last issued refresh token's sha256
    createdAt: string
    updatedAt: string
}

// --------- Helpers ---------

function userKey(login: string): string {
    return `user:${login.toLowerCase()}`
}

function stateKey(state: string): string {
    return `oauth:state:${state}`
}

function deviceKey(deviceCode: string): string {
    return `oauth:device:${deviceCode}`
}

async function sha256Hex(s: string): Promise<string> {
    const buf = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(s))
    return Array.from(new Uint8Array(buf))
        .map((b) => b.toString(16).padStart(2, '0'))
        .join('')
}

function randomToken(bytes: number = 32): string {
    const arr = new Uint8Array(bytes)
    crypto.getRandomValues(arr)
    return Array.from(arr).map((b) => b.toString(16).padStart(2, '0')).join('')
}

/// Fetch the authenticated user's GitHub profile.
async function fetchGitHubUser(ghAccessToken: string): Promise<{ login: string; name?: string; avatarUrl?: string }> {
    const resp = await fetch(GH_USER_URL, {
        headers: {
            'Authorization': `Bearer ${ghAccessToken}`,
            'User-Agent': 'forage-hub-api',
            'Accept': 'application/vnd.github+json',
        },
    })
    if (!resp.ok) {
        throw new Error(`github user fetch failed: ${resp.status}`)
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const data = (await resp.json()) as any
    return {
        login: String(data.login),
        name: typeof data.name === 'string' ? data.name : undefined,
        avatarUrl: typeof data.avatar_url === 'string' ? data.avatar_url : undefined,
    }
}

async function upsertUser(env: Env, gh: { login: string; name?: string; avatarUrl?: string }, refreshToken: string): Promise<void> {
    const fingerprint = await sha256Hex(refreshToken)
    const now = new Date().toISOString()
    const existingRaw = await env.METADATA.get(userKey(gh.login))
    let createdAt = now
    if (existingRaw) {
        try {
            const existing = JSON.parse(existingRaw) as UserRecord
            createdAt = existing.createdAt
        } catch {
            // fall through with fresh createdAt
        }
    }
    const rec: UserRecord = {
        login: gh.login,
        name: gh.name,
        avatarUrl: gh.avatarUrl,
        refreshTokenFingerprint: fingerprint,
        createdAt,
        updatedAt: now,
    }
    await env.METADATA.put(userKey(gh.login), JSON.stringify(rec))
}

// --------- Web flow ---------

/// `POST /v1/oauth/start` — body: `{ returnTo: string }`. Stores state
/// + returnTo in KV (short TTL), returns the GitHub authorize URL.
export async function oauthStart(request: Request, env: Env): Promise<Response> {
    if (!env.GITHUB_CLIENT_ID) {
        return jsonError(503, 'oauth_not_configured', 'GITHUB_CLIENT_ID not set on the Worker')
    }
    let body: { returnTo?: string }
    try {
        body = await request.json() as { returnTo?: string }
    } catch {
        body = {}
    }
    const returnTo = body.returnTo ?? '/'
    const state = randomToken(16)
    await env.METADATA.put(
        stateKey(state),
        JSON.stringify({ returnTo, createdAt: new Date().toISOString() }),
        { expirationTtl: STATE_TTL_SECONDS }
    )
    const callbackURL = `${new URL(request.url).origin}/v1/oauth/callback`
    const authURL = new URL(GH_AUTHORIZE_URL)
    authURL.searchParams.set('client_id', env.GITHUB_CLIENT_ID)
    authURL.searchParams.set('redirect_uri', callbackURL)
    authURL.searchParams.set('state', state)
    authURL.searchParams.set('scope', 'read:user')
    return json({ authorizeURL: authURL.toString(), state })
}

/// `GET /v1/oauth/callback?code=…&state=…` — GitHub redirects here
/// after the user authorizes. Exchanges the code for a GH access
/// token, fetches the user, mints our JWTs, redirects to `returnTo`
/// with a cookie set.
export async function oauthCallback(request: Request, env: Env): Promise<Response> {
    if (!env.GITHUB_CLIENT_ID || !env.GITHUB_CLIENT_SECRET || !env.JWT_SIGNING_KEY) {
        return jsonError(503, 'oauth_not_configured', 'OAuth env vars not set')
    }
    const url = new URL(request.url)
    const code = url.searchParams.get('code')
    const state = url.searchParams.get('state')
    if (!code || !state) return jsonError(400, 'bad_request', 'missing code or state')

    const stored = await env.METADATA.get(stateKey(state))
    if (!stored) return jsonError(400, 'bad_state', 'state expired or unknown')
    await env.METADATA.delete(stateKey(state))
    let stateData: { returnTo: string }
    try {
        stateData = JSON.parse(stored)
    } catch {
        return jsonError(400, 'bad_state', 'state payload malformed')
    }

    const callbackURL = `${url.origin}/v1/oauth/callback`
    const tokenResp = await fetch(GH_TOKEN_URL, {
        method: 'POST',
        headers: {
            'Accept': 'application/json',
            'Content-Type': 'application/x-www-form-urlencoded',
            'User-Agent': 'forage-hub-api',
        },
        body: new URLSearchParams({
            client_id: env.GITHUB_CLIENT_ID,
            client_secret: env.GITHUB_CLIENT_SECRET,
            code,
            redirect_uri: callbackURL,
        }).toString(),
    })
    if (!tokenResp.ok) return jsonError(502, 'github_token_exchange_failed', `status ${tokenResp.status}`)
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const tokenJSON = (await tokenResp.json()) as any
    const ghAccessToken: string | undefined = tokenJSON.access_token
    if (!ghAccessToken) {
        return jsonError(502, 'github_token_exchange_failed', String(tokenJSON.error_description ?? 'no access_token'))
    }

    const gh = await fetchGitHubUser(ghAccessToken)
    const refreshToken = await signRefreshToken(gh.login, env.JWT_SIGNING_KEY)
    const accessToken = await signAccessToken(gh.login, env.JWT_SIGNING_KEY)
    await upsertUser(env, gh, refreshToken)

    // Redirect back to returnTo with the access token in an httpOnly cookie.
    // The refresh token is held by the client; the web IDE keeps it in a
    // separate Secure;HttpOnly;SameSite=Strict cookie too.
    const redirect = new URL(stateData.returnTo, url.origin).toString()
    const headers = new Headers({ 'Location': redirect })
    headers.append('Set-Cookie', cookie('forage_at', accessToken, ACCESS_TOKEN_TTL_SECONDS))
    headers.append('Set-Cookie', cookie('forage_rt', refreshToken, 60 * 60 * 24 * 30))
    return new Response(null, { status: 302, headers })
}

function cookie(name: string, value: string, maxAge: number): string {
    return `${name}=${value}; Path=/; Max-Age=${maxAge}; HttpOnly; Secure; SameSite=Lax`
}

// --------- Device-code flow (CLI / Toolkit) ---------

/// `POST /v1/oauth/device` — initiates the GitHub device-code flow.
/// Returns `{ deviceCode, userCode, verificationURL, interval, expiresIn }`.
/// The CLI shows the userCode + URL, then polls `/v1/oauth/device/poll`.
export async function oauthDevice(_request: Request, env: Env): Promise<Response> {
    if (!env.GITHUB_CLIENT_ID) {
        return jsonError(503, 'oauth_not_configured', 'GITHUB_CLIENT_ID not set on the Worker')
    }
    const resp = await fetch(GH_DEVICE_CODE_URL, {
        method: 'POST',
        headers: {
            'Accept': 'application/json',
            'Content-Type': 'application/x-www-form-urlencoded',
            'User-Agent': 'forage-hub-api',
        },
        body: new URLSearchParams({
            client_id: env.GITHUB_CLIENT_ID,
            scope: 'read:user',
        }).toString(),
    })
    if (!resp.ok) return jsonError(502, 'github_device_code_failed', `status ${resp.status}`)
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const data = (await resp.json()) as any
    if (!data.device_code) return jsonError(502, 'github_device_code_failed', 'no device_code')

    // Cache the device code so /device/poll knows it's an active flow.
    await env.METADATA.put(
        deviceKey(String(data.device_code)),
        JSON.stringify({ createdAt: new Date().toISOString() }),
        { expirationTtl: DEVICE_PENDING_TTL_SECONDS }
    )
    return json({
        deviceCode: data.device_code,
        userCode: data.user_code,
        verificationURL: data.verification_uri,
        interval: data.interval ?? 5,
        expiresIn: data.expires_in,
    })
}

/// `POST /v1/oauth/device/poll` — body: `{ deviceCode: string }`.
/// Returns:
///   200 + tokens if the user finished the GitHub side;
///   202 if still pending (slow_down or authorization_pending);
///   400 if expired / denied.
export async function oauthDevicePoll(request: Request, env: Env): Promise<Response> {
    if (!env.GITHUB_CLIENT_ID || !env.JWT_SIGNING_KEY) {
        return jsonError(503, 'oauth_not_configured', 'OAuth env vars not set')
    }
    let body: { deviceCode?: string }
    try {
        body = await request.json() as { deviceCode?: string }
    } catch {
        return jsonError(400, 'bad_request', 'invalid JSON body')
    }
    if (!body.deviceCode) return jsonError(400, 'bad_request', 'missing deviceCode')

    const tokenResp = await fetch(GH_TOKEN_URL, {
        method: 'POST',
        headers: {
            'Accept': 'application/json',
            'Content-Type': 'application/x-www-form-urlencoded',
            'User-Agent': 'forage-hub-api',
        },
        body: new URLSearchParams({
            client_id: env.GITHUB_CLIENT_ID,
            device_code: body.deviceCode,
            grant_type: 'urn:ietf:params:oauth:grant-type:device_code',
        }).toString(),
    })
    if (!tokenResp.ok) return jsonError(502, 'github_token_exchange_failed', `status ${tokenResp.status}`)
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const tokenJSON = (await tokenResp.json()) as any

    if (tokenJSON.error) {
        if (tokenJSON.error === 'authorization_pending' || tokenJSON.error === 'slow_down') {
            return json({ status: 'pending', error: tokenJSON.error }, 202)
        }
        return jsonError(400, 'oauth_error', String(tokenJSON.error))
    }
    const ghAccessToken: string | undefined = tokenJSON.access_token
    if (!ghAccessToken) {
        return jsonError(502, 'github_token_exchange_failed', 'no access_token')
    }

    const gh = await fetchGitHubUser(ghAccessToken)
    const refreshToken = await signRefreshToken(gh.login, env.JWT_SIGNING_KEY)
    const accessToken = await signAccessToken(gh.login, env.JWT_SIGNING_KEY)
    await upsertUser(env, gh, refreshToken)
    await env.METADATA.delete(deviceKey(body.deviceCode))

    return json({
        status: 'ok',
        accessToken,
        refreshToken,
        expiresIn: ACCESS_TOKEN_TTL_SECONDS,
        user: { login: gh.login, name: gh.name, avatarUrl: gh.avatarUrl },
    })
}

// --------- Refresh / revoke ---------

/// `POST /v1/oauth/refresh` — body: `{ refreshToken: string }`. Verifies
/// the refresh token, checks the stored fingerprint, mints a fresh
/// access token (and rotates the refresh token).
export async function oauthRefresh(request: Request, env: Env): Promise<Response> {
    if (!env.JWT_SIGNING_KEY) return jsonError(503, 'oauth_not_configured', 'JWT_SIGNING_KEY not set')
    let body: { refreshToken?: string }
    try {
        body = await request.json() as { refreshToken?: string }
    } catch {
        return jsonError(400, 'bad_request', 'invalid JSON body')
    }
    if (!body.refreshToken) return jsonError(400, 'bad_request', 'missing refreshToken')

    const claims = await verifyRefreshToken(body.refreshToken, env.JWT_SIGNING_KEY)
    if (!claims) return jsonError(401, 'invalid_refresh', 'refresh token invalid or expired')

    const userRaw = await env.METADATA.get(userKey(claims.sub))
    if (!userRaw) return jsonError(401, 'invalid_refresh', 'user record missing')
    let user: UserRecord
    try { user = JSON.parse(userRaw) as UserRecord }
    catch { return jsonError(500, 'corrupt_user', 'user record malformed') }

    const fp = await sha256Hex(body.refreshToken)
    if (fp !== user.refreshTokenFingerprint) {
        // Stale refresh token — user has rotated. Treat as revoked.
        return jsonError(401, 'invalid_refresh', 'refresh token rotated')
    }

    const newRefresh = await signRefreshToken(claims.sub, env.JWT_SIGNING_KEY)
    const newAccess = await signAccessToken(claims.sub, env.JWT_SIGNING_KEY)
    user.refreshTokenFingerprint = await sha256Hex(newRefresh)
    user.updatedAt = new Date().toISOString()
    await env.METADATA.put(userKey(claims.sub), JSON.stringify(user))

    return json({
        accessToken: newAccess,
        refreshToken: newRefresh,
        expiresIn: ACCESS_TOKEN_TTL_SECONDS,
        user: { login: user.login, name: user.name, avatarUrl: user.avatarUrl },
    })
}

/// `POST /v1/oauth/revoke` — invalidates the user's refresh token by
/// rotating the fingerprint to a fresh random value. The caller is
/// authenticated via Bearer access token.
export async function oauthRevoke(_request: Request, env: Env, callerLogin: string): Promise<Response> {
    const userRaw = await env.METADATA.get(userKey(callerLogin))
    if (!userRaw) return jsonError(404, 'no_user', `no record for ${callerLogin}`)
    let user: UserRecord
    try { user = JSON.parse(userRaw) as UserRecord }
    catch { return jsonError(500, 'corrupt_user', 'user record malformed') }
    user.refreshTokenFingerprint = await sha256Hex(randomToken(32))
    user.updatedAt = new Date().toISOString()
    await env.METADATA.put(userKey(callerLogin), JSON.stringify(user))
    return json({ status: 'revoked' })
}

/// `GET /v1/oauth/whoami` — returns the caller's identity (if any).
/// Useful for the web IDE to detect the sign-in state on page load.
export async function oauthWhoami(_request: Request, env: Env, callerLogin: string | null): Promise<Response> {
    if (!callerLogin) return json({ authenticated: false })
    const userRaw = await env.METADATA.get(userKey(callerLogin))
    if (!userRaw) return json({ authenticated: false })
    try {
        const u = JSON.parse(userRaw) as UserRecord
        return json({
            authenticated: true,
            user: { login: u.login, name: u.name, avatarUrl: u.avatarUrl },
        })
    } catch {
        return json({ authenticated: false })
    }
}
