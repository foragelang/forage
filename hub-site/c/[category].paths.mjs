// One dynamic route per category. Read from `GET /v1/categories` so
// the build doesn't have to scan every package for its category.
//
// Default builds warn and continue with no routes when the API is
// offline; `FORAGE_HUB_REQUIRE_API=1` in the production deploy
// pipeline fails the build loudly.
import { requireCategories } from '../.vitepress/api.mjs'

export default {
    async paths() {
        const cats = await requireCategories()
        return cats.map((category) => ({ params: { category } }))
    },
}
