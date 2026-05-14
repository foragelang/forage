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
})
