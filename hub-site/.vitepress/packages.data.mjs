// Build-time snapshot of the most recently published packages. Used as
// the first-paint fallback for the home page when the runtime fetch
// hasn't returned yet (or the API is offline). Pages mount the live
// list on top of this snapshot in Vue.
import { fetchPackages } from './api.mjs'

export default {
    async load() {
        const items = await fetchPackages({ sort: 'recent', limit: 100 })
        return { items, fetchedAt: new Date().toISOString() }
    },
}
