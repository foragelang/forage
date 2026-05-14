// Storage schema types for the Forage Hub registry.

export interface Env {
    METADATA: KVNamespace
    BLOBS: R2Bucket
    HUB_PUBLISH_TOKEN: string
    // M11 GitHub OAuth env. All three must be set for the OAuth
    // endpoints to function; the legacy HUB_PUBLISH_TOKEN flow keeps
    // working regardless (admin path).
    GITHUB_CLIENT_ID?: string
    GITHUB_CLIENT_SECRET?: string
    JWT_SIGNING_KEY?: string
}

// Metadata stored at `recipe:<namespace>/<name>` in KV. `slug` is the
// `<namespace>/<name>` composite used as the path under `/v1/packages/...`.
//
// `fileNames` is the on-disk manifest of every `.forage` file in this
// version. Detail responses bundle the bodies alongside (see
// `RecipeDetailResponse`); listings keep just the names.
//
// `ownerLogin` (M11) is the GitHub login of whoever first published the
// package via the OAuth path. Packages published before M11 (or via the
// legacy `HUB_PUBLISH_TOKEN` admin path) carry `ownerLogin: "admin"`.
// Publish + delete check this against the caller identity.
export interface RecipeMetadata {
    slug: string
    author: string | null
    displayName: string
    summary: string
    tags: string[]
    platform: string | null
    version: number
    /// List of `.forage` file paths published in this version, relative
    /// to the package root. Each name resolves to a blob under
    /// `recipes/<slug>/<version>/<name>`.
    fileNames: string[]
    sha256: string
    createdAt: string
    updatedAt: string
    deleted?: boolean
    ownerLogin?: string  // M11 — undefined on legacy entries; treated as "admin"
}

// One entry in `recipe:<namespace>/<name>:versions`. `fileNames` is the
// list of `.forage` file paths published in this version; resolves to
// blob keys via `blobKeyForFile(slug, version, name)`.
export interface VersionRecord {
    version: number
    fileNames: string[]
    publishedAt: string
    sha256: string
}

// `index:list` — a flat array of `<namespace>/<name>` slugs in publish
// order. Re-written on every publish.
export type SlugIndex = string[]

// One .forage file in a publish payload (or detail response). `name`
// is the path relative to the package root; `body` is the UTF-8 source.
export interface PackageFile {
    name: string
    body: string
}

// Request shape for POST /v1/packages/:namespace/:name. `slug` is
// `<namespace>/<name>`. A package is one or more `.forage` files; the
// legacy single-recipe shape ships as a 1-file package.
export interface PublishRequest {
    slug: string
    author?: string | null
    displayName: string
    summary: string
    tags?: string[]
    platform?: string | null
    files: PackageFile[]
    fixtures?: string
    snapshot?: unknown
}

// Response shape for POST /v1/packages/:namespace/:name.
export interface PublishResponse {
    slug: string
    version: number
    sha256: string
}

// Listing item — flat metadata returned by `GET /v1/packages` and
// reused by listing responses. Records the file *names* of the latest
// version; full bodies come from `RecipeDetailResponse`.
export interface ListingItem {
    slug: string
    author: string | null
    displayName: string
    summary: string
    tags: string[]
    platform: string | null
    version: number
    fileNames: string[]
    sha256: string
    createdAt: string
    updatedAt: string
}

export interface ListingResponse {
    items: ListingItem[]
    nextCursor: string | null
}

// Detail response (`GET /v1/packages/:namespace/:name`). One canonical
// `files` field carries every `.forage` file's name *and* body — there
// is no separate name-only manifest to deserialize against.
export interface RecipeDetailResponse extends Omit<ListingItem, 'fileNames'> {
    files: PackageFile[]
}
