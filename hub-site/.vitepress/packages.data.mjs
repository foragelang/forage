// Build-time snapshot of the most recently published packages. Used as
// the first-paint fallback for the home page when the runtime fetch
// hasn't returned yet (or the API is offline). Pages mount the live
// list on top of this snapshot in Vue.
//
// Lenient on transport failure: a temporarily offline API during
// build shouldn't crash the home page — the runtime fetch from the
// browser will repopulate. The dynamic-routes loaders take the
// strict path via `requirePackages`.
import { fetchPackages } from './api.mjs'

export default {
    async load() {
        try {
            const items = await fetchPackages({ sort: 'recent', limit: 100 })
            return { items, fetchedAt: new Date().toISOString() }
        } catch (err) {
            console.warn(`[hub-site] home-page snapshot fetch failed (continuing with empty list):`, err.message ?? err)
            return { items: [], fetchedAt: new Date().toISOString() }
        }
    },
}
