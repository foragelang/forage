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

describe('downloads', () => {
    it('increments the download counter', async () => {
        const a = await userToken('alice')
        await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/versions',
                a,
                publishRequest('p'),
            ),
        )
        const first = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/downloads',
                null,
                {},
            ),
        )
        expect(first.status).toBe(200)
        expect(first.body.downloads).toBe(1)
        const second = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/alice/p/downloads',
                null,
                {},
            ),
        )
        expect(second.body.downloads).toBe(2)
        const detail = await fetchJson(
            get('https://hub/v1/packages/alice/p'),
        )
        expect(detail.body.downloads).toBe(2)
    })

    it('returns 404 for a download against an unknown package', async () => {
        const r = await fetchJson(
            authedPostJson(
                'https://hub/v1/packages/nobody/p/downloads',
                null,
                {},
            ),
        )
        expect(r.status).toBe(404)
    })
})
