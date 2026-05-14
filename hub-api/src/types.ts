// Wire types for the Forage Hub registry.
//
// All JSON keys on this API use `snake_case`. The same shapes are
// consumed by the hub IDE (TypeScript) and by Studio (Rust serde
// structs); snake_case is the lingua franca for the latter and keeping
// the JS side aligned avoids two name spaces. New fields follow the
// same convention.

export interface Env {
    // KV: package metadata, version artifacts, stars, indexes,
    // rate-limit counters, OAuth state. One namespace for everything
    // worker-controlled; collisions avoided by key prefix.
    METADATA: KVNamespace
    // R2: oversized version artifacts (the artifact itself does not
    // get split — only the storage location moves out of KV when the
    // serialized JSON would exceed the R2 fallback threshold).
    BLOBS: R2Bucket
    HUB_PUBLISH_TOKEN: string
    GITHUB_CLIENT_ID?: string
    GITHUB_CLIENT_SECRET?: string
    JWT_SIGNING_KEY?: string
    // Override the inline-vs-R2 split threshold (bytes). Defaults to
    // 20 MiB when unset. Tests set this very low to exercise the R2
    // path with small payloads.
    R2_FALLBACK_THRESHOLD_BYTES?: string
}

// --- Packages ------------------------------------------------------------

// One `.forage` declaration file shipped inside a version artifact.
// `name` is the in-package path (slash-separated, ending in `.forage`).
// `source` is the UTF-8 file body.
export interface PackageFile {
    name: string
    source: string
}

// One named fixture shipped inside a version artifact. `content` is the
// fixture's UTF-8 body (typically JSONL capture data).
export interface PackageFixture {
    name: string
    content: string
}

// One-shot lineage pointer. Recorded on the v1 metadata of a fork.
// Never updated after the fork point — pulls from upstream go through
// the regular publish path on the fork.
export interface ForkedFrom {
    author: string
    slug: string
    version: number
}

// The atomic package version artifact. recipe + decls + fixtures +
// snapshot ride together; there is no sub-resource that returns one
// without the others.
export interface PackageVersion {
    author: string
    slug: string
    version: number
    // The main recipe file's UTF-8 source. Required.
    recipe: string
    // Additional `.forage` files in the package (shared decls, etc.).
    // Empty array if the package has no extra files.
    decls: PackageFile[]
    // Captured replay fixtures. Empty array if the package was published
    // without fixtures.
    fixtures: PackageFixture[]
    // Result of running the recipe against the fixtures at publish
    // time. `null` if the package was published without a snapshot.
    snapshot: PackageSnapshot | null
    // The version this publish was rebased from. `null` for v1.
    base_version: number | null
    published_at: number
    published_by: string
}

// Captured run output. `records` is per-type record arrays as emitted
// by the runtime; `counts` summarises the totals for quick UI display.
export interface PackageSnapshot {
    records: Record<string, unknown[]>
    counts: Record<string, number>
}

// Package metadata. One record per (author, slug). Linear version
// history; counters move on stars / downloads / forks.
export interface PackageMetadata {
    author: string
    slug: string
    description: string
    category: string
    tags: string[]
    forked_from: ForkedFrom | null
    created_at: number
    latest_version: number
    stars: number
    downloads: number
    fork_count: number
    // GitHub login of whoever first published this package. Required.
    // Publish + delete require the caller to match (or be admin).
    owner_login: string
}

// `GET /v1/packages` and friends return arrays of these.
export interface PackageListing {
    author: string
    slug: string
    description: string
    category: string
    tags: string[]
    forked_from: ForkedFrom | null
    created_at: number
    latest_version: number
    stars: number
    downloads: number
    fork_count: number
}

export interface ListPackagesResponse {
    items: PackageListing[]
    next_cursor: string | null
}

// --- Requests ------------------------------------------------------------

// `POST /v1/packages/:author/:slug/versions` — publish a new version.
//
// `base_version` is the version the publisher rebased from. For first
// publish (v1) it is `null`; the server only succeeds if the (author,
// slug) does not exist yet. For subsequent versions, the server
// requires `base_version == latest_version`, else 409.
//
// `description`, `category`, `tags` update the package metadata. They
// are required on every publish — clients send the canonical values.
// `forked_from` is set on a v1 publish created by the fork endpoint
// (the regular publish path rejects non-null `forked_from`).
export interface PublishRequest {
    description: string
    category: string
    tags: string[]
    recipe: string
    decls: PackageFile[]
    fixtures: PackageFixture[]
    snapshot: PackageSnapshot | null
    base_version: number | null
    forked_from: ForkedFrom | null
}

// `POST /v1/packages/:author/:slug/fork` — create `@me/:as` from the
// upstream's latest version. `as` defaults to the upstream's slug.
export interface ForkRequest {
    as: string | null
}

// --- Stars + profile -----------------------------------------------------

// One row in `GET /v1/packages/:author/:slug/stars`. Returned in
// most-recent-first order; pagination via opaque cursor.
export interface Star {
    user: string
    starred_at: number
}

export interface ListStarsResponse {
    items: Star[]
    next_cursor: string | null
}

// `GET /v1/users/:author` — public profile.
export interface Profile {
    login: string
    name: string | null
    avatar_url: string | null
    created_at: number
    package_count: number
    star_count: number
}

export interface ListProfilePackagesResponse {
    items: PackageListing[]
    next_cursor: string | null
}

// `GET /v1/users/:author/stars` — packages this user has starred.
export interface ProfileStar {
    author: string
    slug: string
    starred_at: number
}

export interface ListProfileStarsResponse {
    items: ProfileStar[]
    next_cursor: string | null
}

// --- Errors --------------------------------------------------------------

// `{"error": {"code": "...", "message": "...", ...extras}}` — every
// non-2xx response. Stale-base publish 409 includes
// `latest_version` + `your_base` extras.
export interface ApiError {
    code: string
    message: string
    [extra: string]: unknown
}
