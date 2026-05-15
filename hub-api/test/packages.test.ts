// End-to-end tests for the per-version-atomic package surface. Each
// test boots a fresh worker context with isolated KV/R2.

import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import {
    fetchJson,
    userToken,
    publishRequest,
    authedPostJson,
    get,
    resetStorage,
    setR2FallbackThreshold,
    clearR2FallbackThreshold,
    testEnv,
} from './_helpers'

beforeEach(resetStorage)
afterEach(clearR2FallbackThreshold)

describe('publish + retrieve', () => {
    it('publishes v1 of a new package', async () => {
        const token = await userToken('alice')
        const { status, body } = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/versions',
                token,
                publishRequest('zen-leaf', { description: 'A leafy thing' }),
            ),
        )
        expect(status).toBe(201)
        expect(body.author).toBe('alice')
        expect(body.slug).toBe('zen-leaf')
        expect(body.version).toBe(1)
        expect(body.latest_version).toBe(1)
    })

    it('returns the atomic version artifact with recipe + type_refs + fixtures + snapshot', async () => {
        const token = await userToken('alice')
        // Publish a type the recipe will reference, then publish the
        // recipe pinning it.
        await fetchJson(
            authedPostJson(
                'https://hub/v1/types/alice/X/versions',
                token,
                {
                    description: 'X',
                    category: 'shared-types',
                    tags: [],
                    source: 'share type X {\n    id: String\n}\n',
                    alignments: [],
                    field_alignments: [{ field: 'id', alignment: null }],
                    base_version: null,
                },
            ),
        )
        const payload = publishRequest('atom', {
            description: 'Carries everything in one artifact',
            recipe: 'recipe "atom" {}\n',
            type_refs: [{ author: 'alice', name: 'X', version: 1 }],
            fixtures: [{ name: 'captures.jsonl', content: '{"a":1}\n' }],
            snapshot: { records: { X: [{ a: 1 }] }, counts: { X: 1 } },
        })
        await fetchJson(
            authedPostJson('https://hub/v1/packages/alice/atom/versions', token, payload),
        )
        const { status, body } = await fetchJson(
            get('https://hub/v1/packages/alice/atom/versions/1'),
        )
        expect(status).toBe(200)
        expect(body.recipe).toBe(payload.recipe)
        expect(body.type_refs).toEqual(payload.type_refs)
        expect(body.fixtures).toEqual(payload.fixtures)
        expect(body.snapshot).toEqual(payload.snapshot)
        expect(body.base_version).toBeNull()
    })

    it('advances latest_version on v2 publish with matching base', async () => {
        const token = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/versions',
                token,
                publishRequest('zen-leaf', { description: 'first' }),
            ),
        )
        const v2 = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/versions',
                token,
                publishRequest('zen-leaf', { description: 'second', base_version: 1 }),
            ),
        )
        expect(v2.status).toBe(201)
        expect(v2.body.version).toBe(2)
        const detail = await fetchJson(
            get('https://hub/v1/packages/alice/zen-leaf'),
        )
        expect(detail.body.latest_version).toBe(2)
        expect(detail.body.description).toBe('second')
    })

    it('rejects stale-base publish with 409 and current latest', async () => {
        const token = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/versions',
                token,
                publishRequest('zen-leaf'),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/versions',
                token,
                publishRequest('zen-leaf', { base_version: 1 }),
            ),
        )
        const stale = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/versions',
                token,
                publishRequest('zen-leaf', { base_version: 0 }),
            ),
        )
        expect(stale.status).toBe(409)
        expect(stale.body.error.code).toBe('stale_base')
        expect(stale.body.error.latest_version).toBe(2)
        expect(stale.body.error.your_base).toBe(0)
    })

    it('rejects non-null base_version on first publish with 409', async () => {
        const token = await userToken('alice')
        const stale = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/new-pkg/versions',
                token,
                publishRequest('new-pkg', { base_version: 1 }),
            ),
        )
        expect(stale.status).toBe(409)
        expect(stale.body.error.code).toBe('stale_base')
        expect(stale.body.error.latest_version).toBe(0)
    })

    it('rejects publish under another author', async () => {
        const token = await userToken('alice')
        const denied = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/bob/foo/versions',
                token,
                publishRequest('foo'),
            ),
        )
        expect(denied.status).toBe(403)
        expect(denied.body.error.code).toBe('forbidden')
    })

    it('rejects publish whose recipe header name does not match the URL slug', async () => {
        // The recipe header name is the hub-side slug identity:
        // workspace data dirs, sidecars, and daemon state all key on
        // the header name, so a publish that stamps a different name
        // inside the body would create a round-trip mismatch on
        // sync. The hub catches it at publish time.
        const token = await userToken('alice')
        const r = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/versions',
                token,
                publishRequest('different-name'),
            ),
        )
        expect(r.status).toBe(400)
        expect(r.body.error.code).toBe('slug_mismatch')
        expect(r.body.error.message).toContain('zen-leaf')
        expect(r.body.error.message).toContain('different-name')
    })

    it('ignores caller-sent forked_from on direct publish (no lineage spoof)', async () => {
        // The wire type doesn't carry `forked_from`, but an attacker
        // can still put extra fields in the JSON body. The server
        // must drop them: lineage is server-owned, never caller-set.
        const token = await userToken('alice')
        const spoofedBody = {
            ...publishRequest('forge', { base_version: null }),
            forked_from: { author: 'torvalds', slug: 'linux', version: 1 },
        }
        const r = await fetchJson(
            authedPostJson('https://hub/v1/packages/alice/forge/versions', token, spoofedBody),
        )
        expect(r.status).toBe(201)
        const meta = await fetchJson(get('https://hub/v1/packages/alice/forge'))
        expect(meta.status).toBe(200)
        expect(meta.body.forked_from).toBeNull()
    })

    it('rejects unauthenticated publish with 401', async () => {
        const denied = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/foo/versions',
                null,
                publishRequest('foo'),
            ),
        )
        expect(denied.status).toBe(401)
    })

    it('serves /versions/latest as a convenience for the newest artifact', async () => {
        const token = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/versions',
                token,
                publishRequest('p', { description: 'first' }),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/versions',
                token,
                publishRequest('p', { description: 'second', base_version: 1 }),
            ),
        )
        const latest = await fetchJson(
            get('https://hub/v1/packages/alice/p/versions/latest'),
        )
        expect(latest.status).toBe(200)
        expect(latest.body.version).toBe(2)
    })

    it('lists the linear version history', async () => {
        const token = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/versions',
                token,
                publishRequest('p'),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/versions',
                token,
                publishRequest('p', { base_version: 1 }),
            ),
        )
        const versions = await fetchJson(
            get('https://hub/v1/packages/alice/p/versions'),
        )
        expect(versions.body.items.length).toBe(2)
        expect(versions.body.items[0].version).toBe(1)
        expect(versions.body.items[1].version).toBe(2)
    })

    it('removes singleton fixture / snapshot sub-resources (404)', async () => {
        const token = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/versions',
                token,
                publishRequest('p'),
            ),
        )
        const fix = await fetchJson(
            get('https://hub/v1/packages/alice/p/fixtures'),
        )
        expect(fix.status).toBe(404)
        const snap = await fetchJson(
            get('https://hub/v1/packages/alice/p/snapshot'),
        )
        expect(snap.status).toBe(404)
    })

    // A flat-workspace publish — the recipe lives in `<workspace>/foo.forage`
    // declaring `recipe "bar"` — produces a hub-side artifact under
    // `@alice/bar` regardless of what the file basename was. The
    // round-trip from publish → GET preserves the recipe content
    // verbatim; the slug is the header name, not any path-derived value.
    it('accepts a publish whose hub-side slug equals the recipe header name (not a folder basename)', async () => {
        const token = await userToken('alice')
        const recipeSource = 'recipe "bar" {\n  step list { source = "https://example.com" }\n}\n'
        const r = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/bar/versions',
                token,
                publishRequest('bar', { recipe: recipeSource }),
            ),
        )
        expect(r.status).toBe(201)
        expect(r.body.slug).toBe('bar')
        const fetched = await fetchJson(
            get('https://hub/v1/packages/alice/bar/versions/1'),
        )
        expect(fetched.body.recipe).toBe(recipeSource)
    })
})

describe('listing + filtering', () => {
    it('lists published packages with metadata', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/a/versions',
                a,
                publishRequest('a', { description: 'A', category: 'dispensary' }),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/b/versions',
                a,
                publishRequest('b', { description: 'B', category: 'menu-platform' }),
            ),
        )
        const list = await fetchJson(get('https://hub/v1/packages'))
        expect(list.status).toBe(200)
        expect(list.body.items.length).toBe(2)
        const slugs = new Set(list.body.items.map((x: any) => `${x.author}/${x.slug}`))
        expect(slugs.has('alice/a')).toBe(true)
        expect(slugs.has('alice/b')).toBe(true)
    })

    it('filters by category', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/a/versions',
                a,
                publishRequest('a', { category: 'dispensary' }),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/b/versions',
                a,
                publishRequest('b', { category: 'menu-platform' }),
            ),
        )
        const filtered = await fetchJson(
            get('https://hub/v1/packages?category=dispensary'),
        )
        expect(filtered.body.items.length).toBe(1)
        expect(filtered.body.items[0].slug).toBe('a')
    })

    it('sorts by stars when requested', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/lo/versions',
                a,
                publishRequest('lo'),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/hi/versions',
                a,
                publishRequest('hi'),
            ),
        )
        const b = await userToken('bob')
        const c = await userToken('carol')
        await fetchJson(
            authedPostJson('https://hub/v1/packages/alice/hi/stars', b, {}),
        )
        await fetchJson(
            authedPostJson('https://hub/v1/packages/alice/hi/stars', c, {}),
        )
        const sorted = await fetchJson(
            get('https://hub/v1/packages?sort=stars'),
        )
        expect(sorted.body.items[0].slug).toBe('hi')
        expect(sorted.body.items[0].stars).toBe(2)
    })
})

describe('R2 fallback for oversized version artifacts', () => {
    it('routes a publish past the threshold through R2 and reads back transparently', async () => {
        // Drop the threshold to 100 bytes so the canonical
        // publishRequest (~240 bytes serialized) lands above it.
        setR2FallbackThreshold(100)

        const token = await userToken('alice')
        const publish = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/big/versions',
                token,
                publishRequest('big', { description: 'goes to R2' }),
            ),
        )
        expect(publish.status).toBe(201)

        // The KV slot must hold a pointer, not the inline JSON.
        const kvSlot = await testEnv.METADATA.get('ver:alice:big:1')
        expect(kvSlot).not.toBeNull()
        const parsed = JSON.parse(kvSlot as string)
        expect(parsed).toHaveProperty('r2_key')
        expect(parsed.r2_key).toBe('versions/alice/big/1.json')

        // The R2 object exists and round-trips through the GET handler.
        const r2Obj = await testEnv.BLOBS.get('versions/alice/big/1.json')
        expect(r2Obj).not.toBeNull()

        const fetched = await fetchJson(
            get('https://hub/v1/packages/alice/big/versions/1'),
        )
        expect(fetched.status).toBe(200)
        expect(fetched.body.recipe.length).toBeGreaterThan(0)
        expect(fetched.body.author).toBe('alice')
        expect(fetched.body.slug).toBe('big')
    })
})

describe('categories', () => {
    it('lists every category that has at least one package', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/a/versions',
                a,
                publishRequest('a', { category: 'dispensary' }),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/b/versions',
                a,
                publishRequest('b', { category: 'menu-platform' }),
            ),
        )
        const cats = await fetchJson(get('https://hub/v1/categories'))
        expect(cats.body.items).toContain('dispensary')
        expect(cats.body.items).toContain('menu-platform')
    })

    it('drops empty categories from /v1/categories after re-categorize', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/a/versions',
                a,
                publishRequest('a', { category: 'dispensary' }),
            ),
        )
        // Re-publish v2 under a different category. `dispensary` now
        // has zero packages and must drop out of the category list.
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/a/versions',
                a,
                publishRequest('a', { category: 'menu-platform', base_version: 1 }),
            ),
        )
        const cats = await fetchJson(get('https://hub/v1/categories'))
        expect(cats.body.items).toContain('menu-platform')
        expect(cats.body.items).not.toContain('dispensary')
    })
})
