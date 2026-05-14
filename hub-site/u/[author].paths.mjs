// One dynamic route per author who has published. Profiles for users
// who have only starred but not published aren't enumerated here.
//
// `requirePackages` fails the build loudly in production if the API
// is unreachable or returns no packages.
import { requirePackages } from '../.vitepress/api.mjs'

export default {
    async paths() {
        const list = await requirePackages({ sort: 'recent', limit: 500 })
        const seen = new Set()
        for (const item of list) seen.add(item.author)
        return [...seen].map((author) => ({ params: { author } }))
    },
}
