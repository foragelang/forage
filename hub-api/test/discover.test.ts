// End-to-end tests for type-shaped discovery: producers_of(T),
// consumers_of(T), aligned_with(<ontology>/<term>). Each test boots a
// fresh worker with isolated KV; publishes the relevant type / recipe
// fixtures through the public endpoints; then hits the discover route
// and asserts the shape of the response.

import { describe, it, expect, beforeEach } from 'vitest'
import {
    fetchJson,
    userToken,
    publishRequest,
    publishTypeRequest,
    authedPostJson,
    get,
    resetStorage,
} from './_helpers'

beforeEach(resetStorage)

/// Publish `@<author>/<Name>@v1`. Returns nothing — discover indexes
/// pick up the type by the publish path's side effects.
async function publishType(
    author: string,
    name: string,
    overrides: Partial<Parameters<typeof publishTypeRequest>[1]> = {},
) {
    const token = await userToken(author)
    const { status } = await fetchJson(
        authedPostJson(
            `https://hub/v1/types/${author}/${name}/versions`,
            token,
            publishTypeRequest(name, overrides),
        ),
    )
    expect(status).toBe(201)
}

describe('producers_of(T)', () => {
    it('returns recipes whose latest version emits the type', async () => {
        await publishType('alice', 'Product')

        const token = await userToken('alice')
        const { status } = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/scrape-amazon/versions',
                token,
                publishRequest('scrape-amazon', {
                    recipe: 'recipe "scrape-amazon" {}\n',
                    type_refs: [{ author: 'alice', name: 'Product', version: 1 }],
                    input_type_refs: [],
                    output_type_refs: [{ author: 'alice', name: 'Product', version: 1 }],
                }),
            ),
        )
        expect(status).toBe(201)

        const resp = await fetchJson(
            get('https://hub/v1/discover/producers?type=alice/Product'),
        )
        expect(resp.status).toBe(200)
        expect(resp.body.items).toHaveLength(1)
        expect(resp.body.items[0].author).toBe('alice')
        expect(resp.body.items[0].slug).toBe('scrape-amazon')
        expect(resp.body.items[0].latest_version).toBe(1)
    })

    it('filters by version when ?version= is supplied', async () => {
        // Two recipes: one pinning v1, one pinning v2 of the same type.
        // The version-scoped query returns only the matching one.
        await publishType('alice', 'Product')
        // Re-publish to advance Product to v2. Tweak source so dedup
        // doesn't short-circuit.
        const aliceToken = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                aliceToken,
                publishTypeRequest('Product', {
                    base_version: 1,
                    source: 'share type Product {\n    id: String\n    name: String\n}\n',
                }),
            ),
        )

        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/legacy-amazon/versions',
                aliceToken,
                publishRequest('legacy-amazon', {
                    recipe: 'recipe "legacy-amazon" {}\n',
                    type_refs: [{ author: 'alice', name: 'Product', version: 1 }],
                    output_type_refs: [{ author: 'alice', name: 'Product', version: 1 }],
                }),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/modern-amazon/versions',
                aliceToken,
                publishRequest('modern-amazon', {
                    recipe: 'recipe "modern-amazon" {}\n',
                    type_refs: [{ author: 'alice', name: 'Product', version: 2 }],
                    output_type_refs: [{ author: 'alice', name: 'Product', version: 2 }],
                }),
            ),
        )

        const v1 = await fetchJson(
            get('https://hub/v1/discover/producers?type=alice/Product&version=1'),
        )
        expect(v1.status).toBe(200)
        const v1Slugs = v1.body.items.map((i: { slug: string }) => i.slug).sort()
        expect(v1Slugs).toEqual(['legacy-amazon'])

        const v2 = await fetchJson(
            get('https://hub/v1/discover/producers?type=alice/Product&version=2'),
        )
        expect(v2.status).toBe(200)
        const v2Slugs = v2.body.items.map((i: { slug: string }) => i.slug).sort()
        expect(v2Slugs).toEqual(['modern-amazon'])

        // Unversioned query returns both.
        const all = await fetchJson(
            get('https://hub/v1/discover/producers?type=alice/Product'),
        )
        const allSlugs = all.body.items.map((i: { slug: string }) => i.slug).sort()
        expect(allSlugs).toEqual(['legacy-amazon', 'modern-amazon'])
    })

    it('drops recipes whose republish removes the output type', async () => {
        // The index tracks the canonical view: a recipe that emits T
        // in v1 and stops in v2 disappears from producers_of(T).
        await publishType('alice', 'Product')
        const aliceToken = await userToken('alice')

        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/now-empty/versions',
                aliceToken,
                publishRequest('now-empty', {
                    recipe: 'recipe "now-empty" {}\n',
                    type_refs: [{ author: 'alice', name: 'Product', version: 1 }],
                    output_type_refs: [{ author: 'alice', name: 'Product', version: 1 }],
                }),
            ),
        )

        const before = await fetchJson(
            get('https://hub/v1/discover/producers?type=alice/Product'),
        )
        expect(before.body.items.map((i: { slug: string }) => i.slug)).toEqual(['now-empty'])

        // Republish without the Product output (e.g. recipe pivoted).
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/now-empty/versions',
                aliceToken,
                publishRequest('now-empty', {
                    recipe: 'recipe "now-empty" {}\n',
                    base_version: 1,
                    type_refs: [],
                    output_type_refs: [],
                }),
            ),
        )

        const after = await fetchJson(
            get('https://hub/v1/discover/producers?type=alice/Product'),
        )
        expect(after.body.items).toEqual([])
    })

    it('returns 400 when ?type= is missing or malformed', async () => {
        const missing = await fetchJson(get('https://hub/v1/discover/producers'))
        expect(missing.status).toBe(400)
        expect(missing.body.error.code).toBe('missing_query')

        const bad = await fetchJson(
            get('https://hub/v1/discover/producers?type=not-a-slash'),
        )
        expect(bad.status).toBe(400)
        expect(bad.body.error.code).toBe('bad_type_id')
    })
})

describe('consumers_of(T)', () => {
    it('returns recipes whose latest version accepts the type as input', async () => {
        await publishType('alice', 'MusicGroup')
        const token = await userToken('alice')

        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/enrich-wikidata/versions',
                token,
                publishRequest('enrich-wikidata', {
                    recipe: 'recipe "enrich-wikidata" {}\n',
                    type_refs: [{ author: 'alice', name: 'MusicGroup', version: 1 }],
                    input_type_refs: [{ author: 'alice', name: 'MusicGroup', version: 1 }],
                    output_type_refs: [{ author: 'alice', name: 'MusicGroup', version: 1 }],
                }),
            ),
        )

        const consumers = await fetchJson(
            get('https://hub/v1/discover/consumers?type=alice/MusicGroup'),
        )
        expect(consumers.status).toBe(200)
        expect(consumers.body.items).toHaveLength(1)
        expect(consumers.body.items[0].slug).toBe('enrich-wikidata')

        // Same recipe shows up in producers too — it's an enrichment.
        const producers = await fetchJson(
            get('https://hub/v1/discover/producers?type=alice/MusicGroup'),
        )
        expect(producers.body.items).toHaveLength(1)
        expect(producers.body.items[0].slug).toBe('enrich-wikidata')
    })
})

describe('type_refs partition validation', () => {
    it('rejects an output_type_refs entry whose version disagrees with type_refs', async () => {
        // The umbrella `type_refs` declaration is the source of truth
        // for the version pinned by the recipe; `input_type_refs` and
        // `output_type_refs` partition that set. A partition entry that
        // names a different version is a publish-driver bug — the hub
        // surfaces it instead of indexing inconsistent state.
        await publishType('alice', 'Product')
        const token = await userToken('alice')
        const { status, body } = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/scrape-amazon/versions',
                token,
                publishRequest('scrape-amazon', {
                    recipe: 'recipe "scrape-amazon" {}\n',
                    type_refs: [{ author: 'alice', name: 'Product', version: 5 }],
                    input_type_refs: [],
                    output_type_refs: [
                        { author: 'alice', name: 'Product', version: 7 },
                    ],
                }),
            ),
        )
        expect(status).toBe(400)
        expect(body.error.code).toBe('invalid')
        expect(body.error.message).toContain('output_type_refs')
        expect(body.error.message).toContain('alice/Product')
        expect(body.error.message).toContain('v7')
        expect(body.error.message).toContain('v5')
    })

    it('rejects an input_type_refs entry whose version disagrees with type_refs', async () => {
        await publishType('alice', 'MusicGroup')
        const token = await userToken('alice')
        const { status, body } = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/enrich-wikidata/versions',
                token,
                publishRequest('enrich-wikidata', {
                    recipe: 'recipe "enrich-wikidata" {}\n',
                    type_refs: [{ author: 'alice', name: 'MusicGroup', version: 3 }],
                    input_type_refs: [
                        { author: 'alice', name: 'MusicGroup', version: 1 },
                    ],
                    output_type_refs: [],
                }),
            ),
        )
        expect(status).toBe(400)
        expect(body.error.code).toBe('invalid')
        expect(body.error.message).toContain('input_type_refs')
        expect(body.error.message).toContain('alice/MusicGroup')
    })
})

describe('aligned_with(<ontology>/<term>)', () => {
    it('returns types whose latest version declares the alignment', async () => {
        await publishType('alice', 'Product', {
            alignments: [{ ontology: 'schema.org', term: 'Product' }],
        })
        await publishType('alice', 'Person', {
            alignments: [{ ontology: 'schema.org', term: 'Person' }],
        })

        const resp = await fetchJson(
            get('https://hub/v1/discover/aligned-with?term=schema.org/Product'),
        )
        expect(resp.status).toBe(200)
        expect(resp.body.items).toHaveLength(1)
        expect(resp.body.items[0].author).toBe('alice')
        expect(resp.body.items[0].name).toBe('Product')
    })

    it('serves opaque ontology prefixes unchanged', async () => {
        // The hub indexes alignment URIs verbatim; an unknown prefix
        // works the same as a curated one. Mirrors the "alignment
        // ontology registry: open" decision in the program plan.
        await publishType('alice', 'Widget', {
            alignments: [{ ontology: 'some.unknown', term: 'Term' }],
        })

        const resp = await fetchJson(
            get('https://hub/v1/discover/aligned-with?term=some.unknown/Term'),
        )
        expect(resp.status).toBe(200)
        expect(resp.body.items).toHaveLength(1)
        expect(resp.body.items[0].name).toBe('Widget')
    })

    it('drops a type when its republish removes the alignment', async () => {
        await publishType('alice', 'Product', {
            alignments: [{ ontology: 'schema.org', term: 'Product' }],
        })
        const before = await fetchJson(
            get('https://hub/v1/discover/aligned-with?term=schema.org/Product'),
        )
        expect(before.body.items.map((i: { name: string }) => i.name)).toEqual(['Product'])

        // Republish without the alignment.
        const aliceToken = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                aliceToken,
                publishTypeRequest('Product', {
                    base_version: 1,
                    alignments: [],
                    source: 'share type Product {\n    id: String\n    label: String\n}\n',
                }),
            ),
        )

        const after = await fetchJson(
            get('https://hub/v1/discover/aligned-with?term=schema.org/Product'),
        )
        expect(after.body.items).toEqual([])
    })

    it('returns 400 when ?term= is missing or malformed', async () => {
        const missing = await fetchJson(get('https://hub/v1/discover/aligned-with'))
        expect(missing.status).toBe(400)
        expect(missing.body.error.code).toBe('missing_query')

        const bad = await fetchJson(
            get('https://hub/v1/discover/aligned-with?term=no-slash'),
        )
        expect(bad.status).toBe(400)
        expect(bad.body.error.code).toBe('bad_term')
    })
})

describe('fork carries producer / consumer index entries', () => {
    it("a fork's v1 inherits the upstream's role partitions for the new identity", async () => {
        await publishType('alice', 'Product')
        const aliceToken = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/scrape-amazon/versions',
                aliceToken,
                publishRequest('scrape-amazon', {
                    recipe: 'recipe "scrape-amazon" {}\n',
                    type_refs: [{ author: 'alice', name: 'Product', version: 1 }],
                    output_type_refs: [{ author: 'alice', name: 'Product', version: 1 }],
                }),
            ),
        )

        const bobToken = await userToken('bob')
        const fork = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/scrape-amazon/fork',
                bobToken,
                { as: 'scrape-amazon-bob' },
            ),
        )
        expect(fork.status).toBe(201)

        const producers = await fetchJson(
            get('https://hub/v1/discover/producers?type=alice/Product'),
        )
        const slugs = producers.body.items
            .map((i: { author: string; slug: string }) => `${i.author}/${i.slug}`)
            .sort()
        expect(slugs).toEqual(['alice/scrape-amazon', 'bob/scrape-amazon-bob'])
    })
})
