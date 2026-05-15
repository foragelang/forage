// One `/discover/consumers/<author>/<name>` route per hub-known type.
// Same enumerator as the producers side.
import { requireTypes } from '../../../.vitepress/api.mjs'

export default {
    async paths() {
        const types = await requireTypes()
        return types.map((t) => ({ params: { author: t.author, name: t.name } }))
    },
}
