import { describe, it, expect, beforeEach } from 'vitest'
import {
    fetchJson,
    userToken,
    publishRequest,
    authedPostJson,
    get,
    resetStorage,
} from './_helpers'

beforeEach(resetStorage)

describe('user profile', () => {
    it('returns a profile + their packages + their stars', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p1/versions',
                a,
                publishRequest({ description: 'one' }),
            ),
        )
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p2/versions',
                a,
                publishRequest({ description: 'two' }),
            ),
        )
        const b = await userToken('bob')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/bob/q/versions',
                b,
                publishRequest({ description: 'b' }),
            ),
        )
        await fetchJson(
            authedPostJson('https://hub/v1/packages/bob/q/stars', a, {}),
        )

        const profile = await fetchJson(get('https://hub/v1/users/alice'))
        expect(profile.status).toBe(200)
        expect(profile.body.login).toBe('alice')
        expect(profile.body.package_count).toBe(2)
        expect(profile.body.star_count).toBe(1)

        const pkgs = await fetchJson(
            get('https://hub/v1/users/alice/packages'),
        )
        const slugs = new Set(pkgs.body.items.map((x: any) => x.slug))
        expect(slugs.has('p1')).toBe(true)
        expect(slugs.has('p2')).toBe(true)

        const stars = await fetchJson(
            get('https://hub/v1/users/alice/stars'),
        )
        expect(stars.body.items.length).toBe(1)
        expect(stars.body.items[0].author).toBe('bob')
        expect(stars.body.items[0].slug).toBe('q')
    })

    it('returns 404 for an unknown user with no packages and no stars', async () => {
        const r = await fetchJson(get('https://hub/v1/users/nobody'))
        expect(r.status).toBe(404)
    })

    it('paginates /users/:author/packages by cursor', async () => {
        const a = await userToken('alice')
        for (const slug of ['a', 'b', 'c']) {
            await fetchJson(
                authedPostJson(
                    `https://hub/v1/packages/alice/${slug}/versions`,
                    a,
                    publishRequest({ description: slug }),
                ),
            )
        }
        const first = await fetchJson(
            get('https://hub/v1/users/alice/packages?limit=2'),
        )
        expect(first.status).toBe(200)
        expect(first.body.items.length).toBe(2)
        expect(first.body.next_cursor).not.toBeNull()

        const second = await fetchJson(
            get(`https://hub/v1/users/alice/packages?limit=2&cursor=${encodeURIComponent(first.body.next_cursor)}`),
        )
        expect(second.body.items.length).toBe(1)
        expect(second.body.next_cursor).toBeNull()

        const allSlugs = new Set([
            ...first.body.items.map((x: any) => x.slug),
            ...second.body.items.map((x: any) => x.slug),
        ])
        expect(allSlugs).toEqual(new Set(['a', 'b', 'c']))
    })
})
