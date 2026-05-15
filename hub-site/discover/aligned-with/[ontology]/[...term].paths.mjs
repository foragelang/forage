// One `/discover/aligned-with/<ontology>/<term>` route per
// (ontology, term) pair declared on any hub type's latest version.
// Build-time enumeration walks the type registry and reads each
// type's latest-version alignment list.
//
// The hub does not maintain a separate `/v1/alignments` resource —
// the canonical list is whatever's been published. Pre-1.0 type
// counts are small enough that walking every type at build is fine.
import { HUB_API, requireTypes } from '../../../.vitepress/api.mjs'

export default {
    async paths() {
        const types = await requireTypes()
        const seen = new Set()
        const pairs = []
        for (const t of types) {
            const url = `${HUB_API}/v1/types/${encodeURIComponent(t.author)}/${encodeURIComponent(t.name)}/versions/latest`
            let version
            try {
                const r = await fetch(url)
                if (!r.ok) continue
                version = await r.json()
            } catch {
                continue
            }
            const alignments = Array.isArray(version.alignments) ? version.alignments : []
            for (const a of alignments) {
                if (typeof a.ontology !== 'string' || typeof a.term !== 'string') continue
                const key = `${a.ontology}/${a.term}`
                if (seen.has(key)) continue
                seen.add(key)
                pairs.push({ params: { ontology: a.ontology, term: a.term } })
            }
        }
        return pairs
    },
}
