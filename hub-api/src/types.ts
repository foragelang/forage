// Storage schema types for the Forage Hub registry.

export interface Env {
    METADATA: KVNamespace
    BLOBS: R2Bucket
    HUB_PUBLISH_TOKEN: string
}

// Metadata stored at `recipe:<slug>` in KV. Latest pointer for the recipe.
export interface RecipeMetadata {
    slug: string
    author: string | null
    displayName: string
    summary: string
    tags: string[]
    platform: string | null
    version: number
    latestBlobKey: string
    sha256: string
    createdAt: string
    updatedAt: string
    deleted?: boolean
}

// One entry in `recipe:<slug>:versions`.
export interface VersionRecord {
    version: number
    blobKey: string
    publishedAt: string
    sha256: string
}

// `index:list` — a flat array of slugs in publish-order. Re-written on every publish.
export type SlugIndex = string[]

// Request shape for POST /v1/recipes.
export interface PublishRequest {
    slug: string
    author?: string | null
    displayName: string
    summary: string
    tags?: string[]
    platform?: string | null
    body: string
    fixtures?: string
    snapshot?: unknown
}

// Response shape for POST /v1/recipes.
export interface PublishResponse {
    slug: string
    version: number
    sha256: string
}

// Listing item — the same shape as RecipeMetadata sans `latestBlobKey`.
export interface ListingItem {
    slug: string
    author: string | null
    displayName: string
    summary: string
    tags: string[]
    platform: string | null
    version: number
    sha256: string
    createdAt: string
    updatedAt: string
}

export interface ListingResponse {
    items: ListingItem[]
    nextCursor: string | null
}

// Recipe-detail response (GET /v1/recipes/:slug).
export interface RecipeDetailResponse extends ListingItem {
    body: string
}
