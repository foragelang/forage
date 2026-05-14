// Star + unstar + reverse-index behavior.

import { describe, it, expect, beforeEach } from 'vitest'
import {
    fetchJson,
    userToken,
    publishRequest,
    authedPostJson,
    authedDelete,
    get,
    resetStorage,
} from './_helpers'

beforeEach(resetStorage)

describe('stars', () => {
    it('starring increments the package star counter', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/versions',
                a,
                publishRequest(),
            ),
        )
        const b = await userToken('bob')
        const star = await fetchJson(
            authedPostJson('https://hub/v1/packages/alice/p/stars', b, {}),
        )
        expect(star.status).toBe(201)
        expect(star.body.stars).toBe(1)
        const detail = await fetchJson(
            get('https://hub/v1/packages/alice/p'),
        )
        expect(detail.body.stars).toBe(1)
    })

    it('starring is idempotent (second post returns 200, count unchanged)', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/versions',
                a,
                publishRequest(),
            ),
        )
        const b = await userToken('bob')
        const first = await fetchJson(
            authedPostJson('https://hub/v1/packages/alice/p/stars', b, {}),
        )
        const second = await fetchJson(
            authedPostJson('https://hub/v1/packages/alice/p/stars', b, {}),
        )
        expect(first.status).toBe(201)
        expect(second.status).toBe(200)
        expect(second.body.already_starred).toBe(true)
        expect(second.body.stars).toBe(1)
    })

    it('unstarring decrements the counter and clears reverse index', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/versions',
                a,
                publishRequest(),
            ),
        )
        const b = await userToken('bob')
        await fetchJson(
            authedPostJson('https://hub/v1/packages/alice/p/stars', b, {}),
        )
        const removed = await fetchJson(
            authedDelete('https://hub/v1/packages/alice/p/stars', b),
        )
        expect(removed.status).toBe(200)
        expect(removed.body.stars).toBe(0)

        const reverse = await fetchJson(
            get('https://hub/v1/users/bob/stars'),
        )
        expect(reverse.body.items.length).toBe(0)
    })

    it('lists who starred a package', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/versions',
                a,
                publishRequest(),
            ),
        )
        const b = await userToken('bob')
        const c = await userToken('carol')
        await fetchJson(
            authedPostJson('https://hub/v1/packages/alice/p/stars', b, {}),
        )
        await fetchJson(
            authedPostJson('https://hub/v1/packages/alice/p/stars', c, {}),
        )
        const stars = await fetchJson(
            get('https://hub/v1/packages/alice/p/stars'),
        )
        const users = new Set(stars.body.items.map((s: any) => s.user))
        expect(users.has('bob')).toBe(true)
        expect(users.has('carol')).toBe(true)
    })

    it('rejects unauthenticated star with 401', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/versions',
                a,
                publishRequest(),
            ),
        )
        const denied = await fetchJson(
            authedPostJson('https://hub/v1/packages/alice/p/stars', null, {}),
        )
        expect(denied.status).toBe(401)
    })

    it('surfaces a user\'s starred packages on their profile', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/versions',
                a,
                publishRequest(),
            ),
        )
        const b = await userToken('bob')
        await fetchJson(
            authedPostJson('https://hub/v1/packages/alice/p/stars', b, {}),
        )
        const profile = await fetchJson(get('https://hub/v1/users/bob/stars'))
        expect(profile.body.items.length).toBe(1)
        expect(profile.body.items[0].author).toBe('alice')
        expect(profile.body.items[0].slug).toBe('p')
    })
})
