// `Accept: application/ld+json` content negotiation on the version
// artifact endpoint. The hub stores snapshots in their canonical JSON
// shape; the JSON-LD projection is computed on the fly using the
// alignment metadata bundled with the recipe's `type_refs` —
// *not* the recipe's local AST.

import { beforeEach, describe, expect, it } from 'vitest'
import {
    authedPostJson,
    fetchWorker,
    publishRequest,
    publishTypeRequest,
    resetStorage,
    userToken,
} from './_helpers'

beforeEach(resetStorage)

async function publishProductType(token: string): Promise<void> {
    await fetchWorker(
        authedPostJson(
            'https://hub/v1/types/alice/Product/versions',
            token,
            publishTypeRequest('Product', {
                source:
                    'share type Product\n'
                    + '    aligns schema.org/Product\n'
                    + '{\n'
                    + '    name: String\n'
                    + '    sku: String\n'
                    + '}\n',
                alignments: [{ ontology: 'schema.org', term: 'Product' }],
                field_alignments: [
                    {
                        field: 'name',
                        alignment: { ontology: 'schema.org', term: 'name' },
                    },
                    {
                        field: 'sku',
                        alignment: { ontology: 'schema.org', term: 'gtin' },
                    },
                ],
            }),
        ),
    )
}

async function publishPersonType(token: string): Promise<void> {
    // A Person type with a foaf alignment on the same `name` field
    // schema.org/Product uses — this exercises the type-scoped context
    // pattern that keeps both alignments distinct in `@context`.
    await fetchWorker(
        authedPostJson(
            'https://hub/v1/types/alice/Person/versions',
            token,
            publishTypeRequest('Person', {
                source:
                    'share type Person\n'
                    + '    aligns foaf/Person\n'
                    + '{\n'
                    + '    name: String\n'
                    + '}\n',
                alignments: [{ ontology: 'foaf', term: 'Person' }],
                field_alignments: [
                    {
                        field: 'name',
                        alignment: { ontology: 'foaf', term: 'name' },
                    },
                ],
            }),
        ),
    )
}

async function publishCatalogPackage(token: string): Promise<void> {
    await fetchWorker(
        authedPostJson(
            'https://hub/v1/packages/alice/catalog/versions',
            token,
            publishRequest('catalog', {
                description: 'Catalog of products and people',
                type_refs: [
                    { author: 'alice', name: 'Product', version: 1 },
                    { author: 'alice', name: 'Person', version: 1 },
                ],
                fixtures: [],
                snapshot: {
                    records: {
                        Product: [
                            { _id: 'rec-0', name: 'Widget', sku: 'W-1' },
                            { _id: 'rec-1', name: 'Gadget', sku: 'G-2' },
                        ],
                        Person: [{ _id: 'rec-2', name: 'Alice' }],
                    },
                    counts: { Product: 2, Person: 1 },
                },
            }),
        ),
    )
}

describe('GET /v1/packages/:author/:slug/versions/:n with Accept: application/ld+json', () => {
    it('emits @context entries resolving aligned types to ontology IRIs', async () => {
        const token = await userToken('alice')
        await publishProductType(token)
        await publishPersonType(token)
        await publishCatalogPackage(token)

        const resp = await fetchWorker(
            new Request('https://hub/v1/packages/alice/catalog/versions/1', {
                method: 'GET',
                headers: { Accept: 'application/ld+json' },
            }),
        )
        expect(resp.status).toBe(200)
        expect(resp.headers.get('Content-Type')).toMatch(/application\/ld\+json/)
        const doc = (await resp.json()) as Record<string, unknown>

        const ctx = doc['@context'] as Record<string, Record<string, unknown>>
        expect(ctx.Product['@id']).toBe('https://schema.org/Product')
        // The per-field map keeps the two `name` fields independent —
        // Product.name resolves to schema.org/name, Person.name to
        // foaf/name.
        expect(
            (ctx.Product['@context'] as Record<string, string>).name,
        ).toBe('https://schema.org/name')
        expect(
            (ctx.Product['@context'] as Record<string, string>).sku,
        ).toBe('https://schema.org/gtin')
        expect(ctx.Person['@id']).toBe('http://xmlns.com/foaf/0.1/Person')
        expect(
            (ctx.Person['@context'] as Record<string, string>).name,
        ).toBe('http://xmlns.com/foaf/0.1/name')
    })

    it('flattens records into @graph with bare-name @type', async () => {
        const token = await userToken('alice')
        await publishProductType(token)
        await publishPersonType(token)
        await publishCatalogPackage(token)

        const resp = await fetchWorker(
            new Request('https://hub/v1/packages/alice/catalog/versions/1', {
                method: 'GET',
                headers: { Accept: 'application/ld+json' },
            }),
        )
        const doc = (await resp.json()) as Record<string, unknown>
        const graph = doc['@graph'] as Array<Record<string, unknown>>
        expect(graph).toHaveLength(3)
        const products = graph.filter((r) => r['@type'] === 'Product')
        const persons = graph.filter((r) => r['@type'] === 'Person')
        expect(products).toHaveLength(2)
        expect(persons).toHaveLength(1)
        expect(products[0].name).toBe('Widget')
        expect(products[0].sku).toBe('W-1')
        expect(persons[0].name).toBe('Alice')
    })

    it('uses alignments from published TypeVersions, not the recipe AST', async () => {
        // The recipe source carries no `aligns` clauses — the
        // alignments live on the published TypeVersion. The hub's
        // conversion must read from the TypeVersion store, not parse
        // the recipe.
        const token = await userToken('alice')
        await publishProductType(token)
        // Recipe source intentionally bare — no `aligns` clauses. The
        // hub still has to wire `@context` from the TypeVersion.
        await fetchWorker(
            authedPostJson(
                'https://hub/v1/packages/alice/catalog/versions',
                token,
                publishRequest('catalog', {
                    description: 'Recipe-local types stay bare',
                    recipe:
                        'recipe "catalog" {\n'
                        + '    step list { source = "https://example.com" }\n'
                        + '}\n',
                    type_refs: [
                        { author: 'alice', name: 'Product', version: 1 },
                    ],
                    fixtures: [],
                    snapshot: {
                        records: {
                            Product: [{ _id: 'rec-0', name: 'Widget', sku: 'W-1' }],
                        },
                        counts: { Product: 1 },
                    },
                }),
            ),
        )

        const resp = await fetchWorker(
            new Request('https://hub/v1/packages/alice/catalog/versions/1', {
                method: 'GET',
                headers: { Accept: 'application/ld+json' },
            }),
        )
        const doc = (await resp.json()) as Record<string, unknown>
        const ctx = doc['@context'] as Record<string, Record<string, unknown>>
        expect(ctx.Product['@id']).toBe('https://schema.org/Product')
        expect(
            (ctx.Product['@context'] as Record<string, string>).name,
        ).toBe('https://schema.org/name')
    })

    it('rides unaligned types through with bare @type and no @context entry', async () => {
        const token = await userToken('alice')
        // A type with no alignments — only the bare source.
        await fetchWorker(
            authedPostJson(
                'https://hub/v1/types/alice/Note/versions',
                token,
                publishTypeRequest('Note', {
                    source: 'share type Note {\n    label: String\n}\n',
                }),
            ),
        )
        await fetchWorker(
            authedPostJson(
                'https://hub/v1/packages/alice/notes/versions',
                token,
                publishRequest('notes', {
                    type_refs: [{ author: 'alice', name: 'Note', version: 1 }],
                    fixtures: [],
                    snapshot: {
                        records: { Note: [{ _id: 'rec-0', label: 'hello' }] },
                        counts: { Note: 1 },
                    },
                }),
            ),
        )

        const resp = await fetchWorker(
            new Request('https://hub/v1/packages/alice/notes/versions/1', {
                method: 'GET',
                headers: { Accept: 'application/ld+json' },
            }),
        )
        const doc = (await resp.json()) as Record<string, unknown>
        const ctx = doc['@context'] as Record<string, unknown>
        expect(ctx.Note).toBeUndefined()
        const graph = doc['@graph'] as Array<Record<string, unknown>>
        expect(graph[0]['@type']).toBe('Note')
        expect(graph[0].label).toBe('hello')
    })

    it('falls back to plain JSON when Accept does not request JSON-LD', async () => {
        const token = await userToken('alice')
        await publishProductType(token)
        await publishCatalogPackage(token)
        const resp = await fetchWorker(
            new Request('https://hub/v1/packages/alice/catalog/versions/1', {
                method: 'GET',
            }),
        )
        expect(resp.status).toBe(200)
        expect(resp.headers.get('Content-Type')).toMatch(/application\/json/)
        const body = (await resp.json()) as Record<string, unknown>
        // Plain JSON returns the full atomic artifact — recipe text,
        // type_refs, snapshot. JSON-LD's `@context` / `@graph` keys
        // aren't present.
        expect(body['@context']).toBeUndefined()
        expect(body.recipe).toBeDefined()
        expect(body.snapshot).toBeDefined()
    })

    it('404s with no_snapshot when the version has no snapshot to project', async () => {
        const token = await userToken('alice')
        await publishProductType(token)
        await fetchWorker(
            authedPostJson(
                'https://hub/v1/packages/alice/snap-less/versions',
                token,
                publishRequest('snap-less', {
                    type_refs: [
                        { author: 'alice', name: 'Product', version: 1 },
                    ],
                    fixtures: [],
                    snapshot: null,
                }),
            ),
        )
        const resp = await fetchWorker(
            new Request('https://hub/v1/packages/alice/snap-less/versions/1', {
                method: 'GET',
                headers: { Accept: 'application/ld+json' },
            }),
        )
        expect(resp.status).toBe(404)
        const body = (await resp.json()) as { error: { code: string } }
        expect(body.error.code).toBe('no_snapshot')
    })

    it('handles a wikidata Q-id alignment by lowering to the entity IRI', async () => {
        const token = await userToken('alice')
        await fetchWorker(
            authedPostJson(
                'https://hub/v1/types/alice/Beverage/versions',
                token,
                publishTypeRequest('Beverage', {
                    source: 'share type Beverage {\n    name: String\n}\n',
                    alignments: [{ ontology: 'wikidata', term: 'Q40050' }],
                    field_alignments: [],
                }),
            ),
        )
        await fetchWorker(
            authedPostJson(
                'https://hub/v1/packages/alice/drinks/versions',
                token,
                publishRequest('drinks', {
                    type_refs: [
                        { author: 'alice', name: 'Beverage', version: 1 },
                    ],
                    fixtures: [],
                    snapshot: {
                        records: { Beverage: [{ _id: 'rec-0', name: 'IPA' }] },
                        counts: { Beverage: 1 },
                    },
                }),
            ),
        )
        const resp = await fetchWorker(
            new Request('https://hub/v1/packages/alice/drinks/versions/1', {
                method: 'GET',
                headers: { Accept: 'application/ld+json' },
            }),
        )
        const doc = (await resp.json()) as Record<string, unknown>
        const ctx = doc['@context'] as Record<string, Record<string, unknown>>
        expect(ctx.Beverage['@id']).toBe(
            'http://www.wikidata.org/entity/Q40050',
        )
    })
})
