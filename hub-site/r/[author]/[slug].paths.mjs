// One dynamic route per published package. The page itself renders
// client-side via the `<PackageDetail>` Vue component; this file only
// enumerates the routes so VitePress emits a static page per package.
//
// Default builds (dev, fresh clone, contributor CI) warn and continue
// with no routes when the API is offline; `FORAGE_HUB_REQUIRE_API=1`
// in the production deploy pipeline fails the build loudly so the
// shipped hub doesn't 404 every direct /r/<author>/<slug> URL.
//
// Reference: https://vitepress.dev/guide/routing#dynamic-routes
import { requirePackages } from '../../.vitepress/api.mjs'

export default {
    async paths() {
        const list = await requirePackages({ sort: 'recent', limit: 500 })
        return list.map((item) => ({
            params: {
                author: item.author,
                slug: item.slug,
            },
        }))
    },
}
