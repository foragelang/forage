// One dynamic route per author who has published. Profiles for users
// who have only starred but not published aren't enumerated here.
//
// Default builds warn and continue with no routes when the API is
// offline; `FORAGE_HUB_REQUIRE_API=1` in the production deploy
// pipeline fails the build loudly.
import { requirePackages } from '../.vitepress/api.mjs'

export default {
    async paths() {
        const list = await requirePackages({ sort: 'recent', limit: 500 })
        const seen = new Set()
        for (const item of list) seen.add(item.author)
        return [...seen].map((author) => ({ params: { author } }))
    },
}
