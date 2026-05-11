// Minimal hub-api client. Mirrors `hub-api/src/routes/recipes.ts` endpoints.

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
}

export class HubClient {
    private readonly base: string
    private readonly token: string | null
    private readonly fetchImpl: typeof fetch

    constructor(opts: HubClientOptions = {}) {
        this.base = opts.base ?? DEFAULT_HUB_API
        this.token = opts.token ?? null
        this.fetchImpl = opts.fetch ?? globalThis.fetch.bind(globalThis)
    }

    async list(): Promise<RecipeListItem[]> {
        const r = await this.fetchImpl(`${this.base}/v1/recipes?limit=100`)
        if (!r.ok) throw new Error(`HTTP ${r.status} on GET /v1/recipes`)
        const data = await r.json()
        return Array.isArray(data.items) ? data.items : []
    }

    async get(slug: string, version?: number): Promise<RecipeDetail> {
        const url = `${this.base}/v1/recipes/${encodeURIComponent(slug)}${version ? `?version=${version}` : ''}`
        const r = await this.fetchImpl(url)
        if (!r.ok) throw new Error(`HTTP ${r.status} on GET ${url}`)
        return await r.json()
    }

    async publish(payload: PublishPayload): Promise<PublishResult> {
        if (!this.token) throw new Error('hub: publish requires an API token')
        const r = await this.fetchImpl(`${this.base}/v1/recipes`, {
            method: 'POST',
            headers: {
                'Authorization': `Bearer ${this.token}`,
                'Content-Type': 'application/json',
            },
            body: JSON.stringify(payload),
        })
        if (!r.ok) {
            const text = await r.text()
            throw new Error(`HTTP ${r.status} on POST /v1/recipes: ${text}`)
        }
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
