// One `/discover/producers/<author>/<name>` route per hub-known type.
// `requireTypes` reads `/v1/types`; in non-strict builds an unreachable
// API yields zero routes (and a warning) rather than failing the
// build.
import { requireTypes } from '../../../.vitepress/api.mjs'

export default {
    async paths() {
        const types = await requireTypes()
        return types.map((t) => ({ params: { author: t.author, name: t.name } }))
    },
}
