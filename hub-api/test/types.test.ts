// End-to-end tests for the type-version surface. Mirrors the shape of
// the package tests — each test boots a fresh worker context with
// isolated KV/R2.

import { describe, it, expect, beforeEach } from 'vitest'
import {
    fetchJson,
    userToken,
    publishTypeRequest,
    authedPostJson,
    get,
    resetStorage,
} from './_helpers'

beforeEach(resetStorage)

describe('publish + retrieve type versions', () => {
    it('publishes v1 of a new type', async () => {
        const token = await userToken('alice')
        const { status, body } = await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product', { description: 'A product type' }),
            ),
        )
        expect(status).toBe(201)
        expect(body.author).toBe('alice')
        expect(body.name).toBe('Product')
        expect(body.version).toBe(1)
        expect(body.latest_version).toBe(1)
        expect(body.deduped).toBe(false)
    })

    it('returns the atomic type artifact with source + alignments', async () => {
        const token = await userToken('alice')
        const payload = publishTypeRequest('Product', {
            description: 'Carries source + alignment index data',
            source:
                'share type Product\n'
                + '    aligns schema.org/Product\n'
                + '    aligns wikidata/Q2424752\n'
                + '{\n'
                + '    name: String\n'
                + '    sku: String\n'
                + '}\n',
            alignments: [
                { ontology: 'schema.org', term: 'Product' },
                { ontology: 'wikidata', term: 'Q2424752' },
            ],
            field_alignments: [
                { field: 'name', alignment: { ontology: 'schema.org', term: 'name' } },
                { field: 'sku', alignment: { ontology: 'schema.org', term: 'gtin' } },
            ],
        })
        await fetchJson(
            authedPostJson('https://hub/v1/types/alice/Product/versions', token, payload),
        )
        const { status, body } = await fetchJson(
            get('https://hub/v1/types/alice/Product/versions/1'),
        )
        expect(status).toBe(200)
        expect(body.source).toBe(payload.source)
        expect(body.alignments).toEqual(payload.alignments)
        expect(body.field_alignments).toEqual(payload.field_alignments)
        expect(body.base_version).toBeNull()
    })

    it('advances latest_version on a v2 publish with matching base', async () => {
        const token = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product'),
            ),
        )
        const v2Source = 'share type Product {\n    id: String\n    name: String\n}\n'
        const v2 = await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product', { source: v2Source, base_version: 1 }),
            ),
        )
        expect(v2.status).toBe(201)
        expect(v2.body.version).toBe(2)
        const detail = await fetchJson(
            get('https://hub/v1/types/alice/Product'),
        )
        expect(detail.body.latest_version).toBe(2)
    })

    it('rejects stale-base publish with 409 and current latest', async () => {
        const token = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product'),
            ),
        )
        const stale = await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product', { base_version: 0 }),
            ),
        )
        expect(stale.status).toBe(409)
        expect(stale.body.error.code).toBe('stale_base')
        expect(stale.body.error.latest_version).toBe(1)
        expect(stale.body.error.your_base).toBe(0)
    })

    it('rejects non-null base_version on first publish with 409', async () => {
        const token = await userToken('alice')
        const stale = await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Brand/versions',
                token,
                publishTypeRequest('Brand', { base_version: 1 }),
            ),
        )
        expect(stale.status).toBe(409)
        expect(stale.body.error.code).toBe('stale_base')
        expect(stale.body.error.latest_version).toBe(0)
    })

    it('rejects publish whose source header name does not match the URL :name segment', async () => {
        // Hub-side identity is `@author/Name`; a publish whose body
        // declares a different name would create a round-trip mismatch
        // on sync. Catch it at publish time.
        const token = await userToken('alice')
        const r = await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product', {
                    source: 'share type Banana {\n    id: String\n}\n',
                }),
            ),
        )
        expect(r.status).toBe(400)
        expect(r.body.error.code).toBe('name_mismatch')
        expect(r.body.error.message).toContain('Product')
        expect(r.body.error.message).toContain('Banana')
    })

    it('content-hash dedups identical re-publish to the existing version', async () => {
        // The plan calls out content-hash dedup as a server-side
        // affordance: a re-publish of the same source body returns
        // the existing version number so recipe pins stay stable
        // across redundant publishes.
        const token = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product'),
            ),
        )
        const dup = await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product', { base_version: 1 }),
            ),
        )
        expect(dup.status).toBe(200)
        expect(dup.body.version).toBe(1)
        expect(dup.body.deduped).toBe(true)
        const detail = await fetchJson(get('https://hub/v1/types/alice/Product'))
        expect(detail.body.latest_version).toBe(1)
    })

    it('rejects publish under another author with 403', async () => {
        const token = await userToken('alice')
        const denied = await fetchJson(
            authedPostJson(
                'https://hub/v1/types/bob/Product/versions',
                token,
                publishTypeRequest('Product'),
            ),
        )
        expect(denied.status).toBe(403)
        expect(denied.body.error.code).toBe('forbidden')
    })

    it('rejects unauthenticated publish with 401', async () => {
        const denied = await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                null,
                publishTypeRequest('Product'),
            ),
        )
        expect(denied.status).toBe(401)
    })

    it('rejects an invalid type name with 400', async () => {
        const token = await userToken('alice')
        const r = await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/lower-case/versions',
                token,
                publishTypeRequest('Product'),
            ),
        )
        expect(r.status).toBe(400)
        expect(r.body.error.code).toBe('bad_type_name')
    })

    it('rejects malformed alignment with 400', async () => {
        const token = await userToken('alice')
        const r = await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product', {
                    alignments: [{ ontology: 'BAD/UPPER', term: 'x' }],
                }),
            ),
        )
        expect(r.status).toBe(400)
        expect(r.body.error.code).toBe('invalid')
    })

    it('serves /versions/latest', async () => {
        const token = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product'),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product', {
                    source: 'share type Product {\n    id: String\n    name: String\n}\n',
                    base_version: 1,
                }),
            ),
        )
        const latest = await fetchJson(
            get('https://hub/v1/types/alice/Product/versions/latest'),
        )
        expect(latest.status).toBe(200)
        expect(latest.body.version).toBe(2)
    })

    it('lists the linear version history', async () => {
        const token = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product'),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product', {
                    source: 'share type Product {\n    id: String\n    name: String\n}\n',
                    base_version: 1,
                }),
            ),
        )
        const versions = await fetchJson(
            get('https://hub/v1/types/alice/Product/versions'),
        )
        expect(versions.body.items.length).toBe(2)
        expect(versions.body.items[0].version).toBe(1)
        expect(versions.body.items[1].version).toBe(2)
    })
})

describe('listing types', () => {
    it('returns every published type', async () => {
        const token = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product'),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Variant/versions',
                token,
                publishTypeRequest('Variant'),
            ),
        )
        const list = await fetchJson(get('https://hub/v1/types'))
        expect(list.status).toBe(200)
        const names = new Set(
            list.body.items.map((x: { author: string; name: string }) => `${x.author}/${x.name}`),
        )
        expect(names.has('alice/Product')).toBe(true)
        expect(names.has('alice/Variant')).toBe(true)
    })

    it('filters by category', async () => {
        const token = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Product/versions',
                token,
                publishTypeRequest('Product', { category: 'commerce' }),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Inspection/versions',
                token,
                publishTypeRequest('Inspection', { category: 'health' }),
            ),
        )
        const filtered = await fetchJson(
            get('https://hub/v1/types?category=commerce'),
        )
        expect(filtered.body.items.length).toBe(1)
        expect(filtered.body.items[0].name).toBe('Product')
    })
})

describe('discover by extension', () => {
    it('lists every child type that extends a parent pin', async () => {
        const upstream = await userToken('upstream')
        const alice = await userToken('alice')
        const bob = await userToken('bob')

        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/upstream/JobPosting/versions',
                upstream,
                publishTypeRequest('JobPosting'),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/EnhancedJobPosting/versions',
                alice,
                publishTypeRequest('EnhancedJobPosting', {
                    source:
                        'share type EnhancedJobPosting extends @upstream/JobPosting@v1 {\n'
                        + '    salaryMin: Int?\n'
                        + '}\n',
                    extends: { author: 'upstream', name: 'JobPosting', version: 1 },
                }),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/bob/RemoteJobPosting/versions',
                bob,
                publishTypeRequest('RemoteJobPosting', {
                    source:
                        'share type RemoteJobPosting extends @upstream/JobPosting@v1 {\n'
                        + '    remoteOk: Bool\n'
                        + '}\n',
                    extends: { author: 'upstream', name: 'JobPosting', version: 1 },
                }),
            ),
        )

        const res = await fetchJson(
            get('https://hub/v1/discover/extends?type=@upstream/JobPosting@v1'),
        )
        expect(res.status).toBe(200)
        const names = res.body.items
            .map((x: { author: string; name: string }) => `${x.author}/${x.name}`)
            .sort()
        expect(names).toEqual(['alice/EnhancedJobPosting', 'bob/RemoteJobPosting'])
    })

    it('separates children by parent version', async () => {
        // A child pinned to v1 must not appear in the v2 discover
        // response, and vice versa — the index is keyed by version.
        const upstream = await userToken('upstream')
        const alice = await userToken('alice')
        const bob = await userToken('bob')

        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/upstream/JobPosting/versions',
                upstream,
                publishTypeRequest('JobPosting'),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/upstream/JobPosting/versions',
                upstream,
                publishTypeRequest('JobPosting', {
                    source: 'share type JobPosting {\n    id: String\n    title: String\n}\n',
                    base_version: 1,
                }),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/V1Ext/versions',
                alice,
                publishTypeRequest('V1Ext', {
                    source:
                        'share type V1Ext extends @upstream/JobPosting@v1 {\n'
                        + '    extra: String\n'
                        + '}\n',
                    extends: { author: 'upstream', name: 'JobPosting', version: 1 },
                }),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/bob/V2Ext/versions',
                bob,
                publishTypeRequest('V2Ext', {
                    source:
                        'share type V2Ext extends @upstream/JobPosting@v2 {\n'
                        + '    extra: String\n'
                        + '}\n',
                    extends: { author: 'upstream', name: 'JobPosting', version: 2 },
                }),
            ),
        )

        const v1 = await fetchJson(
            get('https://hub/v1/discover/extends?type=@upstream/JobPosting@v1'),
        )
        const v2 = await fetchJson(
            get('https://hub/v1/discover/extends?type=@upstream/JobPosting@v2'),
        )
        expect(v1.body.items.map((x: { name: string }) => x.name)).toEqual(['V1Ext'])
        expect(v2.body.items.map((x: { name: string }) => x.name)).toEqual(['V2Ext'])
    })

    it('republishing a child onto a new parent pin clears the old slot', async () => {
        const upstream = await userToken('upstream')
        const alice = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/upstream/JobPosting/versions',
                upstream,
                publishTypeRequest('JobPosting'),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/upstream/JobPosting/versions',
                upstream,
                publishTypeRequest('JobPosting', {
                    source: 'share type JobPosting {\n    id: String\n    title: String\n}\n',
                    base_version: 1,
                }),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Ext/versions',
                alice,
                publishTypeRequest('Ext', {
                    source:
                        'share type Ext extends @upstream/JobPosting@v1 {\n'
                        + '    extra: String\n'
                        + '}\n',
                    extends: { author: 'upstream', name: 'JobPosting', version: 1 },
                }),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/Ext/versions',
                alice,
                publishTypeRequest('Ext', {
                    source:
                        'share type Ext extends @upstream/JobPosting@v2 {\n'
                        + '    extra: String\n'
                        + '    extra2: String\n'
                        + '}\n',
                    extends: { author: 'upstream', name: 'JobPosting', version: 2 },
                    base_version: 1,
                }),
            ),
        )

        const v1 = await fetchJson(
            get('https://hub/v1/discover/extends?type=@upstream/JobPosting@v1'),
        )
        const v2 = await fetchJson(
            get('https://hub/v1/discover/extends?type=@upstream/JobPosting@v2'),
        )
        expect(v1.body.items).toEqual([])
        expect(v2.body.items.map((x: { name: string }) => x.name)).toEqual(['Ext'])
    })

    it('returns an empty list when no children pin the parent', async () => {
        const res = await fetchJson(
            get('https://hub/v1/discover/extends?type=@upstream/Missing@v1'),
        )
        expect(res.status).toBe(200)
        expect(res.body.items).toEqual([])
    })

    it('rejects a malformed query', async () => {
        const noPin = await fetchJson(
            get('https://hub/v1/discover/extends?type=@upstream/JobPosting'),
        )
        expect(noPin.status).toBe(400)
        expect(noPin.body.error.code).toBe('bad_type')

        const noV = await fetchJson(
            get('https://hub/v1/discover/extends?type=@upstream/JobPosting@1'),
        )
        expect(noV.status).toBe(400)

        const missing = await fetchJson(
            get('https://hub/v1/discover/extends'),
        )
        expect(missing.status).toBe(400)
        expect(missing.body.error.code).toBe('missing_type')
    })
})
