// One dynamic route per published package. The page itself renders
// client-side via the `<PackageDetail>` Vue component; this file only
// enumerates the routes so VitePress emits a static page per package.
//
// `requirePackages` fails the build loudly in production if the API
// is unreachable or returns no packages — otherwise every direct
// /r/<author>/<slug> URL would 404 after deploy. Dev builds warn and
// continue with no routes.
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
