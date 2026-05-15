import type {
    Env,
    PackageMetadata,
    PackageVersion,
    Star,
    TypeMetadata,
    TypeVersion,
} from './types'

// Storage layout.
//
// KV namespace METADATA (one bucket; collisions avoided by prefix):
//   pkg:<author>:<slug>                  → PackageMetadata JSON
//   ver:<author>:<slug>:<n>              → PackageVersion JSON
//                                          OR { "r2_key": "..." } pointing
//                                          at the same JSON in R2 (used when
//                                          the artifact exceeds the R2
//                                          fallback threshold).
//   type:<author>:<name>                 → TypeMetadata JSON
//   tver:<author>:<name>:<n>             → TypeVersion JSON (small; never
//                                          spills to R2 — a single type
//                                          declaration plus alignments
//                                          easily fits inline)
//   thash:<author>:<name>:<sha>          → version number (content-hash dedup
//                                          index for re-publishing the same
//                                          source body)
//   star:<author>:<slug>:<user>          → "" (presence)
//   stars_by:<user>:<author>:<slug>      → ISO timestamp (presence + when)
//   idx:packages                         → JSON array of "<author>/<slug>"
//   idx:cat:<category>                   → JSON array of "<author>/<slug>"
//   idx:user_packages:<author>           → JSON array of "<author>/<slug>"
//   idx:categories                       → JSON array of category strings
//   idx:types                            → JSON array of "<author>/<name>"
//   idx:user_types:<author>              → JSON array of "<author>/<name>"
//   idx:producers:<author>/<name>        → JSON array of "<author>/<slug>"
//                                          (recipes whose latest version's
//                                          `output_type_refs` includes this
//                                          type)
//   idx:consumers:<author>/<name>        → JSON array of "<author>/<slug>"
//                                          (recipes whose latest version's
//                                          `input_type_refs` includes this
//                                          type)
//   idx:aligned:<ontology>/<term>        → JSON array of "<author>/<name>"
//                                          (types whose latest version
//                                          declares this alignment URI at
//                                          the type level)
//   idx:extends:<author>:<name>:<v>      → JSON array of "<author>/<name>"
//                                          (child types whose latest version
//                                          pins this exact parent
//                                          (author, name, version))
//
// R2 (only when the version artifact is large):
//   versions/<author>/<slug>/<n>.json    → PackageVersion JSON
//
// All counters (stars, downloads, fork_count) live on the
// PackageMetadata. They are bumped non-transactionally; a lost
// increment under contention is fine pre-1.0.
//
// Sorting by stars / downloads is a full scan over `idx:packages`
// in `listPackages`. Pre-1.0 volume is tiny; the cached top-N
// indexes that used to live here were dead code (never read by any
// surface) so they were dropped.

// Versions whose serialized JSON exceeds the threshold go to R2.
// Cloudflare KV's hard ceiling is 25 MiB; the default leaves
// headroom for the pointer wrapper. The threshold is overridable
// via `env.R2_FALLBACK_THRESHOLD_BYTES` so tests can exercise the
// R2 path with small payloads.
export const DEFAULT_R2_FALLBACK_THRESHOLD_BYTES = 20 * 1024 * 1024

function r2FallbackThreshold(env: Env): number {
    const raw = env.R2_FALLBACK_THRESHOLD_BYTES
    if (raw === undefined || raw === '') return DEFAULT_R2_FALLBACK_THRESHOLD_BYTES
    const n = parseInt(raw, 10)
    if (!Number.isFinite(n) || n < 0) {
        throw new Error(
            `R2_FALLBACK_THRESHOLD_BYTES must be a non-negative integer; got ${raw}`,
        )
    }
    return n
}

const PKG_KEY = (author: string, slug: string) => `pkg:${author}:${slug}`
const VER_KEY = (author: string, slug: string, n: number) =>
    `ver:${author}:${slug}:${n}`
const TYPE_KEY = (author: string, name: string) => `type:${author}:${name}`
const TYPE_VER_KEY = (author: string, name: string, n: number) =>
    `tver:${author}:${name}:${n}`
const TYPE_HASH_KEY = (author: string, name: string, sha: string) =>
    `thash:${author}:${name}:${sha}`
const STAR_KEY = (author: string, slug: string, user: string) =>
    `star:${author}:${slug}:${user}`
const STARS_BY_KEY = (user: string, author: string, slug: string) =>
    `stars_by:${user}:${author}:${slug}`
const STAR_PREFIX = (author: string, slug: string) =>
    `star:${author}:${slug}:`
const STARS_BY_PREFIX = (user: string) => `stars_by:${user}:`

const IDX_PACKAGES = 'idx:packages'
const IDX_CATEGORIES = 'idx:categories'
const IDX_CATEGORY = (category: string) => `idx:cat:${category}`
const IDX_USER_PACKAGES = (author: string) => `idx:user_packages:${author}`
const IDX_TYPES = 'idx:types'
const IDX_USER_TYPES = (author: string) => `idx:user_types:${author}`
const IDX_PRODUCERS = (author: string, name: string) =>
    `idx:producers:${author}/${name}`
const IDX_CONSUMERS = (author: string, name: string) =>
    `idx:consumers:${author}/${name}`
const IDX_ALIGNED = (ontology: string, term: string) =>
    `idx:aligned:${ontology}/${term}`
// One index slot per parent `(author, name, version)` triple holding
// the list of `<child_author>/<child_name>` refs that pin this exact
// parent version. Pinning by version not name means a child that
// publishes against `@upstream/JobPosting@v1` and one that pins
// `@v2` show up on different discover responses; the hub doesn't
// collapse them silently.
const IDX_EXTENDS = (author: string, name: string, version: number) =>
    `idx:extends:${author}:${name}:${version}`

const R2_VERSION_KEY = (author: string, slug: string, n: number) =>
    `versions/${author}/${slug}/${n}.json`

// SHA-256 hex digest of a UTF-8 string.
export async function sha256Hex(input: string): Promise<string> {
    const data = new TextEncoder().encode(input)
    const digest = await crypto.subtle.digest('SHA-256', data)
    return [...new Uint8Array(digest)]
        .map((b) => b.toString(16).padStart(2, '0'))
        .join('')
}

// --- Package metadata -----------------------------------------------------

export async function getPackage(
    env: Env,
    author: string,
    slug: string,
): Promise<PackageMetadata | null> {
    const raw = await env.METADATA.get(PKG_KEY(author, slug))
    if (raw === null) return null
    return JSON.parse(raw) as PackageMetadata
}

export async function putPackage(
    env: Env,
    pkg: PackageMetadata,
): Promise<void> {
    await env.METADATA.put(PKG_KEY(pkg.author, pkg.slug), JSON.stringify(pkg))
}

// Cheap presence check — KV.list with limit=1 returns just the key
// header, no value bytes. Use this when you don't need the metadata,
// only need to assert "does this package exist".
export async function packageExists(
    env: Env,
    author: string,
    slug: string,
): Promise<boolean> {
    const key = PKG_KEY(author, slug)
    const result = await env.METADATA.list({ prefix: key, limit: 1 })
    return result.keys.some((k) => k.name === key)
}

// --- Version artifacts (with R2 fallback) --------------------------------

// Wire shape of the KV version slot when the artifact lives in R2.
// The flat one-key envelope lets us probe stored JSON for the
// discriminant without a typed wrapper.
interface VersionR2Pointer {
    r2_key: string
}

function isR2Pointer(v: unknown): v is VersionR2Pointer {
    return (
        typeof v === 'object'
        && v !== null
        && typeof (v as Record<string, unknown>).r2_key === 'string'
        && Object.keys(v as Record<string, unknown>).length === 1
    )
}

// Store a version artifact. Inline in KV when small, R2 when large.
// On large writes the KV slot holds a `{"r2_key": "..."}` pointer.
export async function putVersion(
    env: Env,
    version: PackageVersion,
): Promise<void> {
    const serialized = JSON.stringify(version)
    const kvKey = VER_KEY(version.author, version.slug, version.version)
    if (byteLengthUtf8(serialized) <= r2FallbackThreshold(env)) {
        await env.METADATA.put(kvKey, serialized)
        return
    }
    const r2Key = R2_VERSION_KEY(version.author, version.slug, version.version)
    await env.BLOBS.put(r2Key, serialized, {
        httpMetadata: { contentType: 'application/json; charset=utf-8' },
    })
    const pointer: VersionR2Pointer = { r2_key: r2Key }
    await env.METADATA.put(kvKey, JSON.stringify(pointer))
}

// Read a version artifact. Resolves R2 pointers transparently.
// Returns `null` when the version slot is missing; throws when the
// slot points at an R2 object that has gone missing (storage
// corruption — surface it, never paper over).
export async function getVersion(
    env: Env,
    author: string,
    slug: string,
    n: number,
): Promise<PackageVersion | null> {
    const raw = await env.METADATA.get(VER_KEY(author, slug, n))
    if (raw === null) return null
    const parsed = JSON.parse(raw) as unknown
    if (isR2Pointer(parsed)) {
        const obj = await env.BLOBS.get(parsed.r2_key)
        if (obj === null) {
            throw new Error(
                `version slot ${author}/${slug}@${n} points at missing R2 key ${parsed.r2_key}`,
            )
        }
        const body = await obj.text()
        return JSON.parse(body) as PackageVersion
    }
    return parsed as PackageVersion
}

// --- Type metadata + versions --------------------------------------------

export async function getType(
    env: Env,
    author: string,
    name: string,
): Promise<TypeMetadata | null> {
    const raw = await env.METADATA.get(TYPE_KEY(author, name))
    if (raw === null) return null
    return JSON.parse(raw) as TypeMetadata
}

export async function putType(
    env: Env,
    meta: TypeMetadata,
): Promise<void> {
    await env.METADATA.put(TYPE_KEY(meta.author, meta.name), JSON.stringify(meta))
}

// Type-version artifacts are small (one type declaration plus alignment
// arrays) and ride exclusively in KV — the R2 fallback that recipe
// versions use for fixture-heavy artifacts has no analog here.
export async function getTypeVersion(
    env: Env,
    author: string,
    name: string,
    n: number,
): Promise<TypeVersion | null> {
    const raw = await env.METADATA.get(TYPE_VER_KEY(author, name, n))
    if (raw === null) return null
    return JSON.parse(raw) as TypeVersion
}

export async function putTypeVersion(
    env: Env,
    version: TypeVersion,
): Promise<void> {
    await env.METADATA.put(
        TYPE_VER_KEY(version.author, version.name, version.version),
        JSON.stringify(version),
    )
}

// Content-hash dedup: record `(sha → version)` so a re-publish of the
// same source body short-circuits to the existing version number
// instead of allocating v(N+1). Recipes that re-publish unchanged
// types get stable pins this way.
export async function putTypeHash(
    env: Env,
    author: string,
    name: string,
    sha: string,
    version: number,
): Promise<void> {
    await env.METADATA.put(
        TYPE_HASH_KEY(author, name, sha),
        String(version),
    )
}

export async function getTypeHash(
    env: Env,
    author: string,
    name: string,
    sha: string,
): Promise<number | null> {
    const raw = await env.METADATA.get(TYPE_HASH_KEY(author, name, sha))
    if (raw === null) return null
    const n = parseInt(raw, 10)
    if (!Number.isFinite(n) || n < 1) {
        throw new Error(
            `type-hash index for ${author}/${name}/${sha} holds invalid version: ${raw}`,
        )
    }
    return n
}

// --- Star presence + reverse index ---------------------------------------

interface StarRecord {
    starred_at: number
}

export async function putStar(
    env: Env,
    author: string,
    slug: string,
    user: string,
): Promise<{ added: boolean; starredAt: number }> {
    const existing = await env.METADATA.get(STAR_KEY(author, slug, user))
    if (existing !== null) {
        // Already starred. Return the existing timestamp so the
        // caller's response stays idempotent.
        const parsed = JSON.parse(existing) as StarRecord
        return { added: false, starredAt: parsed.starred_at }
    }
    const starredAt = Date.now()
    const record: StarRecord = { starred_at: starredAt }
    const serialized = JSON.stringify(record)
    await Promise.all([
        env.METADATA.put(STAR_KEY(author, slug, user), serialized),
        env.METADATA.put(
            STARS_BY_KEY(user, author, slug),
            serialized,
        ),
    ])
    return { added: true, starredAt }
}

export async function deleteStar(
    env: Env,
    author: string,
    slug: string,
    user: string,
): Promise<boolean> {
    const existing = await env.METADATA.get(STAR_KEY(author, slug, user))
    if (existing === null) return false
    await Promise.all([
        env.METADATA.delete(STAR_KEY(author, slug, user)),
        env.METADATA.delete(STARS_BY_KEY(user, author, slug)),
    ])
    return true
}

export async function hasStar(
    env: Env,
    author: string,
    slug: string,
    user: string,
): Promise<boolean> {
    const existing = await env.METADATA.get(STAR_KEY(author, slug, user))
    return existing !== null
}

// List who starred (author, slug). Paginated via KV's prefix-list
// cursor.
export async function listStars(
    env: Env,
    author: string,
    slug: string,
    cursor: string | null,
    limit: number,
): Promise<{ items: Star[]; nextCursor: string | null }> {
    const prefix = STAR_PREFIX(author, slug)
    const opts: KVNamespaceListOptions = { prefix, limit }
    if (cursor !== null) opts.cursor = cursor
    const list = await env.METADATA.list(opts)
    const items: Star[] = []
    for (const k of list.keys) {
        const user = k.name.slice(prefix.length)
        const raw = await env.METADATA.get(k.name)
        if (raw === null) continue
        const rec = JSON.parse(raw) as StarRecord
        items.push({ user, starred_at: rec.starred_at })
    }
    const nextCursor = list.list_complete ? null : list.cursor ?? null
    return { items, nextCursor }
}

// List (author, slug) starred by a user.
export async function listStarsByUser(
    env: Env,
    user: string,
    cursor: string | null,
    limit: number,
): Promise<{
    items: Array<{ author: string; slug: string; starred_at: number }>
    nextCursor: string | null
}> {
    const prefix = STARS_BY_PREFIX(user)
    const opts: KVNamespaceListOptions = { prefix, limit }
    if (cursor !== null) opts.cursor = cursor
    const list = await env.METADATA.list(opts)
    const items: Array<{ author: string; slug: string; starred_at: number }> = []
    for (const k of list.keys) {
        const rest = k.name.slice(prefix.length)
        const split = rest.indexOf(':')
        if (split < 0) continue
        const a = rest.slice(0, split)
        const s = rest.slice(split + 1)
        const raw = await env.METADATA.get(k.name)
        if (raw === null) continue
        const rec = JSON.parse(raw) as StarRecord
        items.push({ author: a, slug: s, starred_at: rec.starred_at })
    }
    const nextCursor = list.list_complete ? null : list.cursor ?? null
    return { items, nextCursor }
}

export async function countStarsByUser(
    env: Env,
    user: string,
): Promise<number> {
    // KV prefix-count by paging through. Volume tiny pre-1.0.
    const prefix = STARS_BY_PREFIX(user)
    let cursor: string | undefined = undefined
    let total = 0
    for (;;) {
        const list: KVNamespaceListResult<unknown, string> = await env.METADATA.list({
            prefix,
            cursor,
        })
        total += list.keys.length
        if (list.list_complete) break
        cursor = list.cursor
    }
    return total
}

// --- Indexes (eventually-consistent) -------------------------------------

// Add (author, slug) to the all-packages index. Idempotent.
export async function indexAddPackage(
    env: Env,
    author: string,
    slug: string,
): Promise<void> {
    await appendToIndex(env, IDX_PACKAGES, ref(author, slug))
}

// Add (author, slug) to the per-user packages index. Idempotent.
export async function indexAddUserPackage(
    env: Env,
    author: string,
    slug: string,
): Promise<void> {
    await appendToIndex(env, IDX_USER_PACKAGES(author), ref(author, slug))
}

// Add (author, slug) to the category index. Idempotent. Also adds the
// category name to the global category list.
export async function indexAddCategory(
    env: Env,
    category: string,
    author: string,
    slug: string,
): Promise<void> {
    await appendToIndex(env, IDX_CATEGORY(category), ref(author, slug))
    await appendToIndex(env, IDX_CATEGORIES, category)
}

// Remove (author, slug) from the category index. Used when a publish
// updates a package's category. If the category goes empty as a
// result, drop its name from the global category list too so
// `GET /v1/categories` stops returning phantoms.
export async function indexRemoveCategory(
    env: Env,
    category: string,
    author: string,
    slug: string,
): Promise<void> {
    await removeFromIndex(env, IDX_CATEGORY(category), ref(author, slug))
    const remaining = await listIndex(env, IDX_CATEGORY(category))
    if (remaining.length === 0) {
        await removeFromIndex(env, IDX_CATEGORIES, category)
    }
}

// Type indexes. Same eventually-consistent pattern as the package
// indexes above: append-only `string[]` lists in KV, scanned in full
// by `listTypes`.
export async function indexAddType(
    env: Env,
    author: string,
    name: string,
): Promise<void> {
    await appendToIndex(env, IDX_TYPES, ref(author, name))
}

// --- Producer / consumer / alignment indexes -----------------------------
//
// Idempotent add / remove on per-type-id ("producers"/"consumers") and
// per-alignment-URI ("aligned") inverse lookups. The producer index
// keys on the (author, name) of an output type and points at every
// recipe whose latest version emits it; consumers mirror that for
// inputs. The alignment index keys on the alignment URI's
// ontology/term path and points at every type whose latest version
// declares that alignment.
//
// The hub mutates these on every publish so the index reflects the
// *current* canonical view: producers_of(T) returns recipes whose
// latest version emits T, not historical versions. The discover
// routes read the indexes directly; nothing scans the version corpus.

export async function indexAddProducer(
    env: Env,
    typeAuthor: string,
    typeName: string,
    recipeAuthor: string,
    recipeSlug: string,
): Promise<void> {
    await appendToIndex(
        env,
        IDX_PRODUCERS(typeAuthor, typeName),
        ref(recipeAuthor, recipeSlug),
    )
}

export async function indexRemoveProducer(
    env: Env,
    typeAuthor: string,
    typeName: string,
    recipeAuthor: string,
    recipeSlug: string,
): Promise<void> {
    await removeFromIndex(
        env,
        IDX_PRODUCERS(typeAuthor, typeName),
        ref(recipeAuthor, recipeSlug),
    )
}

export async function indexAddConsumer(
    env: Env,
    typeAuthor: string,
    typeName: string,
    recipeAuthor: string,
    recipeSlug: string,
): Promise<void> {
    await appendToIndex(
        env,
        IDX_CONSUMERS(typeAuthor, typeName),
        ref(recipeAuthor, recipeSlug),
    )
}

export async function indexRemoveConsumer(
    env: Env,
    typeAuthor: string,
    typeName: string,
    recipeAuthor: string,
    recipeSlug: string,
): Promise<void> {
    await removeFromIndex(
        env,
        IDX_CONSUMERS(typeAuthor, typeName),
        ref(recipeAuthor, recipeSlug),
    )
}

export async function indexAddAligned(
    env: Env,
    ontology: string,
    term: string,
    typeAuthor: string,
    typeName: string,
): Promise<void> {
    await appendToIndex(
        env,
        IDX_ALIGNED(ontology, term),
        ref(typeAuthor, typeName),
    )
}

export async function indexRemoveAligned(
    env: Env,
    ontology: string,
    term: string,
    typeAuthor: string,
    typeName: string,
): Promise<void> {
    await removeFromIndex(
        env,
        IDX_ALIGNED(ontology, term),
        ref(typeAuthor, typeName),
    )
}

export async function listProducersIndex(
    env: Env,
    typeAuthor: string,
    typeName: string,
): Promise<string[]> {
    return listIndex(env, IDX_PRODUCERS(typeAuthor, typeName))
}

export async function listConsumersIndex(
    env: Env,
    typeAuthor: string,
    typeName: string,
): Promise<string[]> {
    return listIndex(env, IDX_CONSUMERS(typeAuthor, typeName))
}

export async function listAlignedIndex(
    env: Env,
    ontology: string,
    term: string,
): Promise<string[]> {
    return listIndex(env, IDX_ALIGNED(ontology, term))
}

export async function indexAddUserType(
    env: Env,
    author: string,
    name: string,
): Promise<void> {
    await appendToIndex(env, IDX_USER_TYPES(author), ref(author, name))
}

export async function listTypesIndex(env: Env): Promise<string[]> {
    return listIndex(env, IDX_TYPES)
}

export async function listUserTypesIndex(
    env: Env,
    author: string,
): Promise<string[]> {
    return listIndex(env, IDX_USER_TYPES(author))
}

// Index a child type as extending the given parent pin. Idempotent —
// republishing a child against the same parent keeps the index slot
// at one entry. The child is stored as `<child_author>/<child_name>`
// so `discover/extends` can resolve each into a `TypeListing`.
export async function indexAddExtends(
    env: Env,
    parentAuthor: string,
    parentName: string,
    parentVersion: number,
    childAuthor: string,
    childName: string,
): Promise<void> {
    await appendToIndex(
        env,
        IDX_EXTENDS(parentAuthor, parentName, parentVersion),
        ref(childAuthor, childName),
    )
}

// Remove a child from a parent's extends index. Used when republishing
// a child version that changes its parent pin (or drops `extends`
// entirely): the new publish stamps the new slot, this clears the
// old one so a discover query against the old parent stops returning
// the now-stale child.
export async function indexRemoveExtends(
    env: Env,
    parentAuthor: string,
    parentName: string,
    parentVersion: number,
    childAuthor: string,
    childName: string,
): Promise<void> {
    await removeFromIndex(
        env,
        IDX_EXTENDS(parentAuthor, parentName, parentVersion),
        ref(childAuthor, childName),
    )
}

export async function listExtendsIndex(
    env: Env,
    parentAuthor: string,
    parentName: string,
    parentVersion: number,
): Promise<string[]> {
    return listIndex(env, IDX_EXTENDS(parentAuthor, parentName, parentVersion))
}

export async function listIndex(env: Env, key: string): Promise<string[]> {
    const raw = await env.METADATA.get(key)
    if (raw === null) return []
    return JSON.parse(raw) as string[]
}

export async function listPackagesIndex(env: Env): Promise<string[]> {
    return listIndex(env, IDX_PACKAGES)
}

export async function listCategoryIndex(
    env: Env,
    category: string,
): Promise<string[]> {
    return listIndex(env, IDX_CATEGORY(category))
}

export async function listCategoriesIndex(env: Env): Promise<string[]> {
    return listIndex(env, IDX_CATEGORIES)
}

export async function listUserPackagesIndex(
    env: Env,
    author: string,
): Promise<string[]> {
    return listIndex(env, IDX_USER_PACKAGES(author))
}

// --- Helpers -------------------------------------------------------------

export function ref(author: string, slug: string): string {
    return `${author}/${slug}`
}

export function splitRef(r: string): [string, string] {
    const i = r.indexOf('/')
    if (i < 0) throw new Error(`bad ref: ${r}`)
    return [r.slice(0, i), r.slice(i + 1)]
}

async function appendToIndex(
    env: Env,
    key: string,
    value: string,
): Promise<void> {
    const current = await listIndex(env, key)
    if (current.includes(value)) return
    current.push(value)
    await env.METADATA.put(key, JSON.stringify(current))
}

async function removeFromIndex(
    env: Env,
    key: string,
    value: string,
): Promise<void> {
    const current = await listIndex(env, key)
    const i = current.indexOf(value)
    if (i < 0) return
    current.splice(i, 1)
    await env.METADATA.put(key, JSON.stringify(current))
}

// UTF-8 byte length of a string. Uses TextEncoder rather than relying
// on `string.length` (which counts UTF-16 code units).
function byteLengthUtf8(s: string): number {
    return new TextEncoder().encode(s).length
}
