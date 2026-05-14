// Shared API helpers used by the build-time data loader and the runtime
// home-page Vue component. Keep them tiny and parameterless so they work in
// both Node (build) and browser (runtime) contexts.

export const HUB_API = process.env.FORAGE_HUB_API || 'https://api.foragelang.com'

export async function fetchRecipeList(base = HUB_API) {
    try {
        const r = await fetch(`${base}/v1/packages?limit=100`)
        if (!r.ok) {
            console.warn(`[hub-site] /v1/packages returned ${r.status}; skipping`)
            return []
        }
        const data = await r.json()
        return Array.isArray(data.items) ? data.items : []
    } catch (err) {
        console.warn(`[hub-site] fetch ${base}/v1/packages failed:`, err?.message ?? err)
        return []
    }
}

export async function fetchRecipeDetail(slug, base = HUB_API) {
    try {
        const r = await fetch(`${base}/v1/packages/${encodeURIComponent(slug)}`)
        if (!r.ok) return null
        return await r.json()
    } catch (err) {
        console.warn(`[hub-site] fetch ${base}/v1/packages/${slug} failed:`, err?.message ?? err)
        return null
    }
}
