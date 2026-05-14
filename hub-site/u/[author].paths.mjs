// One dynamic route per author who has published. Profiles for users
// who have only starred but not published aren't enumerated here.
import { fetchPackages } from '../.vitepress/api.mjs'

export default {
    async paths() {
        const list = await fetchPackages({ sort: 'recent', limit: 500 })
        const seen = new Set()
        for (const item of list) seen.add(item.author)
        return [...seen].map((author) => ({ params: { author } }))
    },
}
