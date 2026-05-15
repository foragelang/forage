// Fork lineage + counter side-effects.

import { describe, it, expect, beforeEach } from 'vitest'
import {
    fetchJson,
    userToken,
    publishRequest,
    forkRequest,
    authedPostJson,
    get,
    resetStorage,
} from './_helpers'

beforeEach(resetStorage)

describe('forks', () => {
    it('creates a fork with one-shot lineage and bumps upstream counters', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/versions',
                a,
                publishRequest('zen-leaf', { description: 'upstream', category: 'dispensary' }),
            ),
        )
        const b = await userToken('bob')
        const forked = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/fork',
                b,
                forkRequest(),
            ),
        )
        expect(forked.status).toBe(201)
        expect(forked.body.author).toBe('bob')
        expect(forked.body.slug).toBe('zen-leaf')
        expect(forked.body.forked_from).toEqual({
            author: 'alice',
            slug: 'zen-leaf',
            version: 1,
        })
        expect(forked.body.latest_version).toBe(1)

        const upstream = await fetchJson(
            get('https://hub/v1/packages/alice/zen-leaf'),
        )
        expect(upstream.body.fork_count).toBe(1)
        expect(upstream.body.downloads).toBe(1)

        const forkVersion = await fetchJson(
            get('https://hub/v1/packages/bob/zen-leaf/versions/1'),
        )
        expect(forkVersion.body.author).toBe('bob')
        expect(forkVersion.body.recipe.length).toBeGreaterThan(0)
    })

    it('renames forks with the `as` field', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/versions',
                a,
                publishRequest('zen-leaf'),
            ),
        )
        const b = await userToken('bob')
        const forked = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/fork',
                b,
                forkRequest('my-leaf'),
            ),
        )
        expect(forked.status).toBe(201)
        expect(forked.body.slug).toBe('my-leaf')
    })

    it('rejects forks that would clobber an existing package', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/versions',
                a,
                publishRequest('zen-leaf'),
            ),
        )
        const b = await userToken('bob')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/bob/zen-leaf/versions',
                b,
                publishRequest('zen-leaf'),
            ),
        )
        const collision = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/fork',
                b,
                forkRequest(),
            ),
        )
        expect(collision.status).toBe(409)
        expect(collision.body.error.code).toBe('already_exists')
    })

    it('normalizes mixed-case caller login when stamping the fork owner', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/versions',
                a,
                publishRequest('zen-leaf'),
            ),
        )
        // Mint a token for `John-Doe`. GitHub returns logins with
        // their preserved case, but the system stores everything
        // under the lowercase form.
        const mixed = await userToken('John-Doe')
        const forked = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/zen-leaf/fork',
                mixed,
                forkRequest(),
            ),
        )
        expect(forked.status).toBe(201)
        expect(forked.body.author).toBe('john-doe')

        // Direct lookup at the lowercase URL must hit the stored
        // record — the regression this guards against landed at
        // pkg:John-Doe:zen-leaf and was unreachable.
        const direct = await fetchJson(
            get('https://hub/v1/packages/john-doe/zen-leaf'),
        )
        expect(direct.status).toBe(200)
        expect(direct.body.author).toBe('john-doe')
        expect(direct.body.owner_login).toBe('john-doe')

        // Profile lookup also normalizes the URL segment so a request
        // at `/v1/users/John-Doe` resolves to the same data as
        // `/v1/users/john-doe`.
        const profile = await fetchJson(get('https://hub/v1/users/John-Doe'))
        expect(profile.status).toBe(200)
        expect(profile.body.login).toBe('john-doe')
        expect(profile.body.package_count).toBe(1)
    })

    it('does not auto-track lineage on subsequent publishes against the fork', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/versions',
                a,
                publishRequest('p', { description: 'v1' }),
            ),
        )
        const b = await userToken('bob')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/fork',
                b,
                forkRequest(),
            ),
        )
        // Publish v2 against the fork. Lineage pointer must NOT
        // change to track the upstream after the initial fork.
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/bob/p/versions',
                b,
                publishRequest('p', { description: 'fork v2', base_version: 1 }),
            ),
        )
        const fork = await fetchJson(get('https://hub/v1/packages/bob/p'))
        expect(fork.body.forked_from).toEqual({
            author: 'alice',
            slug: 'p',
            version: 1,
        })
        expect(fork.body.latest_version).toBe(2)
    })
})
