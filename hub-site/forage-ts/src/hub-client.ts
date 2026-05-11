// Minimal hub-api client. Mirrors `hub-api/src/routes/recipes.ts` endpoints.
// Slugs are always `<namespace>/<name>`; the URL path encodes them as two
// segments so the Worker can route on `:namespace/:name`.

export const DEFAULT_HUB_API = 'https://api.foragelang.com'

export interface RecipeListItem {
    slug: string
    displayName: string
    summary: string
    author?: string
    platform?: string
    tags?: string[]
    version?: number
    sha256?: string
    createdAt?: string
    updatedAt?: string
}

export interface RecipeDetail extends RecipeListItem {
    body: string
}

export interface PublishPayload {
    slug: string
    displayName: string
    summary: string
    tags: string[]
    body: string
    author?: string
    platform?: string
    license?: string
    fixtures?: string
    snapshot?: unknown
}

export interface PublishResult {
    slug: string
    version: number
    sha256?: string
    publishedAt?: string
}

export interface HubClientOptions {
    base?: string
    token?: string
    fetch?: typeof fetch
    /// When true, include credentials (cookies) on every request. Used
    /// by the web IDE so the httpOnly `forage_at` cookie from the
    /// OAuth flow authenticates publish/delete without an explicit
    /// Bearer token.
    useCredentials?: boolean
}

export class HubClient {
    private readonly base: string
    private readonly token: string | null
    private readonly fetchImpl: typeof fetch
    private readonly useCredentials: boolean

    constructor(opts: HubClientOptions = {}) {
        this.base = opts.base ?? DEFAULT_HUB_API
        this.token = opts.token ?? null
        this.fetchImpl = opts.fetch ?? globalThis.fetch.bind(globalThis)
        this.useCredentials = opts.useCredentials ?? false
    }

    private fetchInit(extra: RequestInit = {}): RequestInit {
        const init: RequestInit = { ...extra }
        if (this.useCredentials) init.credentials = 'include'
        return init
    }

    async list(): Promise<RecipeListItem[]> {
        const r = await this.fetchImpl(`${this.base}/v1/recipes?limit=100`, this.fetchInit())
        if (!r.ok) throw new Error(`HTTP ${r.status} on GET /v1/recipes`)
        const data = await r.json()
        return Array.isArray(data.items) ? data.items : []
    }

    async get(slug: string, version?: number): Promise<RecipeDetail> {
        const path = encodeSlugPath(slug)
        const url = `${this.base}/v1/recipes/${path}${version ? `?version=${version}` : ''}`
        const r = await this.fetchImpl(url, this.fetchInit())
        if (!r.ok) throw new Error(`HTTP ${r.status} on GET ${url}`)
        return await r.json()
    }

    async publish(payload: PublishPayload): Promise<PublishResult> {
        // Either an explicit Bearer token OR the cookie path (useCredentials)
        // is required; if neither is set, the server will 401.
        if (!this.token && !this.useCredentials) {
            throw new Error('hub: publish requires an API token or signed-in session')
        }
        const headers: Record<string, string> = { 'Content-Type': 'application/json' }
        if (this.token) headers['Authorization'] = `Bearer ${this.token}`
        const r = await this.fetchImpl(`${this.base}/v1/recipes`, this.fetchInit({
            method: 'POST',
            headers,
            body: JSON.stringify(payload),
        }))
        if (!r.ok) {
            const text = await r.text()
            throw new Error(`HTTP ${r.status} on POST /v1/recipes: ${text}`)
        }
        return await r.json()
    }

    /// Whoami endpoint — for the web IDE to detect a signed-in user
    /// without needing the Bearer token in JS.
    async whoami(): Promise<{ authenticated: boolean; user?: { login: string; name?: string; avatarUrl?: string } }> {
        const r = await this.fetchImpl(`${this.base}/v1/oauth/whoami`, this.fetchInit())
        if (!r.ok) return { authenticated: false }
        return await r.json()
    }

    /// Initiate the web (authorization-code) flow. Returns the GitHub
    /// authorize URL; the caller redirects the browser to it.
    async oauthStart(returnTo: string): Promise<{ authorizeURL: string; state: string }> {
        const r = await this.fetchImpl(`${this.base}/v1/oauth/start`, this.fetchInit({
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ returnTo }),
        }))
        if (!r.ok) throw new Error(`HTTP ${r.status} on POST /v1/oauth/start`)
        return await r.json()
    }

    /** Build the POST body without sending — used by the IDE to preview. */
    buildPublishRequest(payload: PublishPayload): { url: string; init: RequestInit } {
        return {
            url: `${this.base}/v1/recipes`,
            init: {
                method: 'POST',
                headers: {
                    Authorization: this.token ? `Bearer ${this.token}` : '',
                    'Content-Type': 'application/json',
                },
                body: JSON.stringify(payload),
            },
        }
    }
}

/// URL-encode a `<namespace>/<name>` slug while preserving the `/`. The
/// Worker routes on two path segments.
function encodeSlugPath(slug: string): string {
    return slug.split('/').map(encodeURIComponent).join('/')
}
