// VitePress dynamic-routes data loader. Fetches the registry at build and
// emits one route per recipe. Each route carries the recipe metadata + body
// via `params`, which the template at `r/[slug].md` reads.
//
// Reference: https://vitepress.dev/guide/routing#dynamic-routes
import { fetchRecipeList, fetchRecipeDetail, HUB_API } from '../.vitepress/api.mjs'

export default {
    async paths() {
        const list = await fetchRecipeList()
        const routes = []
        for (const item of list) {
            const detail = await fetchRecipeDetail(item.slug)
            if (!detail) continue
            routes.push({
                params: {
                    slug: detail.slug,
                    displayName: detail.displayName,
                    summary: detail.summary,
                    author: detail.author ?? '',
                    platform: detail.platform ?? '',
                    tags: (detail.tags ?? []).join(', '),
                    version: detail.version,
                    sha256: detail.sha256,
                    createdAt: detail.createdAt,
                    updatedAt: detail.updatedAt,
                    apiBase: HUB_API,
                },
                // VitePress concatenates `content` into the page after the
                // template markup, but we want the body in a fenced ```forage
                // block — embed it via the template itself with a `<<<` import
                // is awkward, so render it as a code block here instead.
                content: '```forage\n' + detail.body + '\n```',
            })
        }
        return routes
    },
}
