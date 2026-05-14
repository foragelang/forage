// One dynamic route per category. Read from `GET /v1/categories` so
// the build doesn't have to scan every package for its category.
//
// `requireCategories` fails the build loudly in production if the API
// is unreachable or returns no categories.
import { requireCategories } from '../.vitepress/api.mjs'

export default {
    async paths() {
        const cats = await requireCategories()
        return cats.map((category) => ({ params: { category } }))
    },
}
