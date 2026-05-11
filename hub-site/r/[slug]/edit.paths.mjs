// One edit route per recipe. Shares the same listing source as `r/[slug].md`
// so the same set of recipes is reachable from both `/r/<slug>` and
// `/r/<slug>/edit`.
import { fetchRecipeList, HUB_API } from '../../.vitepress/api.mjs'

export default {
    async paths() {
        const list = await fetchRecipeList()
        return list.map(item => ({
            params: {
                slug: item.slug,
                displayName: item.displayName || item.slug,
                apiBase: HUB_API,
            },
        }))
    },
}
