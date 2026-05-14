// One dynamic route per category. Read from `GET /v1/categories` so
// the build doesn't have to scan every package for its category.
import { fetchCategories } from '../.vitepress/api.mjs'

export default {
    async paths() {
        const cats = await fetchCategories()
        return cats.map((category) => ({ params: { category } }))
    },
}
