//! Type-shaped discovery: `producers_of(T)`, `consumers_of(T)`,
//! `aligned_with(<ontology>/<term>)`.
//!
//! Each route reads a per-key inverse index in KV; the index is
//! maintained by the publish paths (see
//! `routes/packages.ts::diffProducerConsumerIndexes` and
//! `routes/types.ts::diffAlignmentIndex`). The indexes are
//! "latest-version is canonical" — a recipe drops out of
//! `producers_of(T)` the moment its next publish removes `T` from the
//! output signature.
//!
//! Today the assignability relation is the identity relation: type `T`
//! is only assignable to itself. The expansion step in
//! `producersOf` / `consumersOf` is therefore a one-element walk over
//! `{T}`. Sub-plan 9 introduces `extends` and widens this to a closure
//! over the extension graph; the rest of the route stays unchanged.
//! The query takes an optional `version` parameter so callers can pin
//! to a specific type version (today the index is per-name; the
//! version filter checks every returned recipe's `output_type_refs` /
//! `input_type_refs` for an exact match).

import type {
    Env,
    PackageListing,
    PackageMetadata,
    TypeListing,
    TypeMetadata,
} from '../types'
import {
    getPackage,
    getType,
    getVersion,
    listProducersIndex,
    listConsumersIndex,
    listAlignedIndex,
    splitRef,
} from '../storage'
import { json, jsonError } from '../http'

/// `<author>/<Name>` segment shape. Mirrors the rest of the codebase:
/// author is lowercase alphanumeric-with-dashes; type name is
/// PascalCase. Validated before touching KV so a malformed query
/// fails loudly rather than walking an empty index.
const TYPE_ID_RE = /^([a-z0-9][a-z0-9-]{0,38})\/([A-Z][A-Za-z0-9]{0,63})$/

/// `<ontology>/<term>` segment shape. The hub stores alignment URIs
/// opaquely (per the "alignment ontology registry: open" decision in
/// the program plan), so this is a presence check, not a curated
/// validator. The ontology and term are each one or more characters
/// from the same charset the parser accepts; either may contain dots.
const ALIGNMENT_URI_RE =
    /^([a-z][a-z0-9.\-]*)\/([A-Za-z0-9][A-Za-z0-9._:\-/]*)$/

/// `GET /v1/discover/producers?type=<author>/<Name>[&version=<v>]`
///
/// Returns every recipe whose latest version produces the given hub
/// type. Optional `version` narrows to recipes pinning exactly that
/// type-version.
export async function discoverProducers(
    request: Request,
    env: Env,
): Promise<Response> {
    const url = new URL(request.url)
    const typeId = url.searchParams.get('type')
    if (typeId === null) {
        return jsonError(400, 'missing_query', 'missing required query param: type', {}, request)
    }
    const match = TYPE_ID_RE.exec(typeId)
    if (match === null) {
        return jsonError(
            400,
            'bad_type_id',
            `type must be "<author>/<Name>"; got ${JSON.stringify(typeId)}`,
            {},
            request,
        )
    }
    const versionStr = url.searchParams.get('version')
    const version = versionStr !== null ? parseVersion(versionStr) : null
    if (versionStr !== null && version === null) {
        return jsonError(
            400,
            'bad_version',
            `version must be a positive integer; got ${JSON.stringify(versionStr)}`,
            {},
            request,
        )
    }
    const [, typeAuthor, typeName] = match
    const refs = await listProducersIndex(env, typeAuthor, typeName)
    const items = await collectRecipeListings(env, refs, async (artifact) => {
        if (version === null) return true
        return artifact.output_type_refs.some(
            (r) => r.author === typeAuthor && r.name === typeName && r.version === version,
        )
    })
    return json({ items }, 200, request)
}

/// `GET /v1/discover/consumers?type=<author>/<Name>[&version=<v>]`
///
/// Mirror of `discoverProducers` for the input side.
export async function discoverConsumers(
    request: Request,
    env: Env,
): Promise<Response> {
    const url = new URL(request.url)
    const typeId = url.searchParams.get('type')
    if (typeId === null) {
        return jsonError(400, 'missing_query', 'missing required query param: type', {}, request)
    }
    const match = TYPE_ID_RE.exec(typeId)
    if (match === null) {
        return jsonError(
            400,
            'bad_type_id',
            `type must be "<author>/<Name>"; got ${JSON.stringify(typeId)}`,
            {},
            request,
        )
    }
    const versionStr = url.searchParams.get('version')
    const version = versionStr !== null ? parseVersion(versionStr) : null
    if (versionStr !== null && version === null) {
        return jsonError(
            400,
            'bad_version',
            `version must be a positive integer; got ${JSON.stringify(versionStr)}`,
            {},
            request,
        )
    }
    const [, typeAuthor, typeName] = match
    const refs = await listConsumersIndex(env, typeAuthor, typeName)
    const items = await collectRecipeListings(env, refs, async (artifact) => {
        if (version === null) return true
        return artifact.input_type_refs.some(
            (r) => r.author === typeAuthor && r.name === typeName && r.version === version,
        )
    })
    return json({ items }, 200, request)
}

/// `GET /v1/discover/aligned-with?term=<ontology>/<term>`
///
/// Returns every hub type whose latest version declares the given
/// alignment URI at the type level. The hub treats any ontology
/// prefix as opaque — well-known prefixes (schema.org, wikidata,
/// dublin-core, foaf) share the index with newly-coined ones.
export async function discoverAlignedWith(
    request: Request,
    env: Env,
): Promise<Response> {
    const url = new URL(request.url)
    const termId = url.searchParams.get('term')
    if (termId === null) {
        return jsonError(400, 'missing_query', 'missing required query param: term', {}, request)
    }
    const match = ALIGNMENT_URI_RE.exec(termId)
    if (match === null) {
        return jsonError(
            400,
            'bad_term',
            `term must be "<ontology>/<term>"; got ${JSON.stringify(termId)}`,
            {},
            request,
        )
    }
    const [, ontology, term] = match
    const refs = await listAlignedIndex(env, ontology, term)
    const items: TypeListing[] = []
    for (const r of refs) {
        const [a, n] = splitRef(r)
        const meta = await getType(env, a, n)
        if (meta === null) continue
        items.push(toTypeListing(meta))
    }
    return json({ items }, 200, request)
}

/// Walk an `<author>/<slug>` ref list, resolve each to its package
/// metadata and (when a version filter is active) the latest version
/// artifact, and emit one `PackageListing` per surviving entry. The
/// predicate runs against the version artifact; entries whose package
/// metadata or version artifact is missing are silently dropped (the
/// index can lag the canonical state under contention pre-1.0).
async function collectRecipeListings(
    env: Env,
    refs: string[],
    keep: (artifact: import('../types').PackageVersion) => Promise<boolean>,
): Promise<PackageListing[]> {
    const items: PackageListing[] = []
    for (const r of refs) {
        const [a, s] = splitRef(r)
        const meta = await getPackage(env, a, s)
        if (meta === null) continue
        const artifact = await getVersion(env, a, s, meta.latest_version)
        if (artifact === null) continue
        if (!(await keep(artifact))) continue
        items.push(toPackageListing(meta))
    }
    return items
}

function toPackageListing(meta: PackageMetadata): PackageListing {
    return {
        author: meta.author,
        slug: meta.slug,
        description: meta.description,
        category: meta.category,
        tags: meta.tags,
        forked_from: meta.forked_from,
        created_at: meta.created_at,
        latest_version: meta.latest_version,
        stars: meta.stars,
        downloads: meta.downloads,
        fork_count: meta.fork_count,
    }
}

function toTypeListing(meta: TypeMetadata): TypeListing {
    return {
        author: meta.author,
        name: meta.name,
        description: meta.description,
        category: meta.category,
        tags: meta.tags,
        created_at: meta.created_at,
        latest_version: meta.latest_version,
    }
}

function parseVersion(raw: string): number | null {
    const n = parseInt(raw, 10)
    if (!Number.isFinite(n) || n < 1 || String(n) !== raw) return null
    return n
}
