import type {
    Env,
    RecipeMetadata,
    VersionRecord,
    SlugIndex,
} from './types'

// Key conventions. `slug` is `<namespace>/<name>`; KV keys keep it whole so
// the rest of the code can treat the composite as one opaque string.
const METADATA_KEY = (slug: string) => `recipe:${slug}`
const VERSIONS_KEY = (slug: string) => `recipe:${slug}:versions`
const INDEX_KEY = 'index:list'

const BLOB_PREFIX = (slug: string, version: number) =>
    `recipes/${slug}/${version}`

// SHA-256 hex digest of a UTF-8 string or ArrayBuffer.
export async function sha256Hex(input: string | ArrayBuffer): Promise<string> {
    const data =
        typeof input === 'string' ? new TextEncoder().encode(input) : input
    const digest = await crypto.subtle.digest('SHA-256', data)
    return [...new Uint8Array(digest)]
        .map((b) => b.toString(16).padStart(2, '0'))
        .join('')
}

// --- KV: metadata pointer (`recipe:<namespace>/<name>`) -------------------

export async function getRecipeMetadata(
    env: Env,
    slug: string,
): Promise<RecipeMetadata | null> {
    const raw = await env.METADATA.get(METADATA_KEY(slug))
    if (!raw) return null
    return JSON.parse(raw) as RecipeMetadata
}

export async function putRecipeMetadata(
    env: Env,
    meta: RecipeMetadata,
): Promise<void> {
    await env.METADATA.put(METADATA_KEY(meta.slug), JSON.stringify(meta))
}

// --- KV: version log (`recipe:<namespace>/<name>:versions`) ---------------

export async function getRecipeVersions(
    env: Env,
    slug: string,
): Promise<VersionRecord[]> {
    const raw = await env.METADATA.get(VERSIONS_KEY(slug))
    if (!raw) return []
    return JSON.parse(raw) as VersionRecord[]
}

export async function putRecipeVersions(
    env: Env,
    slug: string,
    versions: VersionRecord[],
): Promise<void> {
    await env.METADATA.put(VERSIONS_KEY(slug), JSON.stringify(versions))
}

// --- KV: denormalized slug index (`index:list`) ---------------------------

export async function getSlugIndex(env: Env): Promise<SlugIndex> {
    const raw = await env.METADATA.get(INDEX_KEY)
    if (!raw) return []
    return JSON.parse(raw) as SlugIndex
}

export async function putSlugIndex(env: Env, slugs: SlugIndex): Promise<void> {
    await env.METADATA.put(INDEX_KEY, JSON.stringify(slugs))
}

export async function ensureSlugInIndex(
    env: Env,
    slug: string,
): Promise<void> {
    const slugs = await getSlugIndex(env)
    if (slugs.includes(slug)) return
    slugs.push(slug)
    await putSlugIndex(env, slugs)
}

// --- R2: package blobs ----------------------------------------------------

/// On-disk key for one `.forage` file inside a published package. `name`
/// is the file's path relative to the package root — `recipe.forage`
/// for top-level recipes, `cannabis.forage` for shared declarations,
/// etc.
export function blobKeyForFile(slug: string, version: number, name: string): string {
    return `${BLOB_PREFIX(slug, version)}/${name}`
}

export function blobKeyForFixtures(slug: string, version: number): string {
    return `${BLOB_PREFIX(slug, version)}/fixtures.jsonl`
}

export function blobKeyForSnapshot(slug: string, version: number): string {
    return `${BLOB_PREFIX(slug, version)}/snapshot.json`
}

export function blobKeyForMeta(slug: string, version: number): string {
    return `${BLOB_PREFIX(slug, version)}/meta.json`
}

export async function putBlob(
    env: Env,
    key: string,
    body: string | ArrayBuffer,
    contentType: string,
): Promise<void> {
    await env.BLOBS.put(key, body, {
        httpMetadata: { contentType },
    })
}

export async function getBlob(
    env: Env,
    key: string,
): Promise<R2ObjectBody | null> {
    return env.BLOBS.get(key)
}
