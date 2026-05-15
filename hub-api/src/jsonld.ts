// JSON-LD projection of a `PackageSnapshot`. Mirrors the Rust
// implementation in `crates/forage-core/src/snapshot/jsonld.rs` for the
// hub's stored snapshot shape (`records: Record<string, unknown[]>`)
// and the hub-published `TypeVersion` alignment metadata.
//
// The hub stores snapshots in their canonical JSON shape. Conversion to
// JSON-LD happens on the fly when a client asks for it via
// `Accept: application/ld+json`. The conversion reads alignment
// metadata from the recipe's `type_refs` — published TypeVersions in
// `/v1/types/` — *not* from the recipe source's local AST. The recipe
// is the consumer of those types; the type identity (and its
// alignments) lives at the hub.

import type {
    AlignmentUri,
    Env,
    PackageSnapshot,
    TypeFieldAlignment,
    TypeRef,
} from './types'
import { getTypeVersion } from './storage'

// Curated table of well-known ontologies → canonical base IRIs. Mirrors
// the Rust writer's table; the two sides have to agree on the lowering
// for the round-trip to be stable.
const WELL_KNOWN_BASES: Record<string, string> = {
    'schema.org': 'https://schema.org/',
    wikidata: 'http://www.wikidata.org/entity/',
    foaf: 'http://xmlns.com/foaf/0.1/',
    'dublin-core': 'http://purl.org/dc/terms/',
}

// Resolve an `<ontology>/<term>` URI to a full IRI when the ontology is
// in the curated set; otherwise return the CURIE `ontology:term`. The
// CURIE round-trips when a downstream consumer has the prefix table,
// and the JSON-LD envelope is still well-formed either way.
export function alignmentIri(uri: AlignmentUri): string {
    const base = WELL_KNOWN_BASES[uri.ontology]
    if (base !== undefined) return base + uri.term
    return `${uri.ontology}:${uri.term}`
}

export interface JsonLdTypeContext {
    '@id': string
    // Per-field IRI mapping. Absent when no field carries an alignment;
    // present (possibly empty) otherwise. Serializing this as a nested
    // `@context` is the JSON-LD 1.1 type-scoped context pattern.
    '@context'?: Record<string, string>
}

export interface JsonLdRecord {
    '@type': string
    // Recipe-source field name → value. JSON-LD parsers resolve each
    // field through the scoped context for `@type` when present.
    [field: string]: unknown
}

export interface JsonLdDocument {
    '@context': Record<string, JsonLdTypeContext>
    '@graph': JsonLdRecord[]
}

// Map of bare type name → resolved TypeVersion alignment metadata. The
// type-scoped context pattern uses this to keep each `@type`'s field
// vocabulary independent — two `name` fields on different types resolve
// to different IRIs through their scoped context.
export interface TypeAlignments {
    typeLevel: AlignmentUri[]
    fieldLevel: TypeFieldAlignment[]
}

// Build a JSON-LD document from a hub-stored `PackageSnapshot` plus
// the alignment metadata bundled with the type refs the recipe pins.
// `typesByName` is keyed by the *bare* type name (`Product`, not
// `@alice/Product`) — the recipe's `emit` statements use bare names and
// the records in `PackageSnapshot.records` use the same.
export function snapshotToJsonLd(
    snapshot: PackageSnapshot,
    typesByName: Map<string, TypeAlignments>,
): JsonLdDocument {
    const context: Record<string, JsonLdTypeContext> = {}
    for (const [name, alignments] of typesByName) {
        const entry = buildTypeContext(name, alignments)
        if (entry !== null) context[name] = entry
    }

    const graph: JsonLdRecord[] = []
    for (const [typeName, records] of Object.entries(snapshot.records)) {
        for (const rec of records) {
            const fields = (rec as Record<string, unknown>) ?? {}
            const out: JsonLdRecord = { '@type': typeName, ...fields }
            graph.push(out)
        }
    }

    return { '@context': context, '@graph': graph }
}

function buildTypeContext(
    name: string,
    alignments: TypeAlignments,
): JsonLdTypeContext | null {
    const fieldIris: Record<string, string> = {}
    for (const fa of alignments.fieldLevel) {
        if (fa.alignment !== null) fieldIris[fa.field] = alignmentIri(fa.alignment)
    }
    const fieldIrisEmpty = Object.keys(fieldIris).length === 0
    const typeIri = alignments.typeLevel.length > 0
        ? alignmentIri(alignments.typeLevel[0])
        : null

    if (typeIri !== null) {
        const entry: JsonLdTypeContext = { '@id': typeIri }
        if (!fieldIrisEmpty) entry['@context'] = fieldIris
        return entry
    }
    if (fieldIrisEmpty) return null
    // Field-level alignments without a type-level alignment: the type
    // identifier rides through bare so the field map still has
    // somewhere to hang off.
    return { '@id': name, '@context': fieldIris }
}

// Resolve every `type_ref` against the hub's `TypeVersion` store and
// build the bare-name → alignments map the writer needs. Unresolvable
// refs (deleted type version, KV miss) are skipped — the corresponding
// records ride through `@graph` with the bare name as `@type` and no
// context entry, which is the same shape as a recipe-local type that
// never carried an alignment.
export async function alignmentsForRefs(
    env: Env,
    refs: TypeRef[],
): Promise<Map<string, TypeAlignments>> {
    const out = new Map<string, TypeAlignments>()
    for (const ref of refs) {
        const tv = await getTypeVersion(env, ref.author, ref.name, ref.version)
        if (tv === null) continue
        out.set(ref.name, {
            typeLevel: tv.alignments,
            fieldLevel: tv.field_alignments,
        })
    }
    return out
}
