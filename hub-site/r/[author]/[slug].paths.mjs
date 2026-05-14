// One dynamic route per published package. The page itself renders
// client-side via the `<PackageDetail>` Vue component; this file only
// enumerates the routes so VitePress emits a static page per package.
//
// Reference: https://vitepress.dev/guide/routing#dynamic-routes
import { fetchPackages } from '../../.vitepress/api.mjs'

export default {
    async paths() {
        const list = await fetchPackages({ sort: 'recent', limit: 500 })
        return list.map((item) => ({
            params: {
                author: item.author,
                slug: item.slug,
            },
        }))
    },
}
