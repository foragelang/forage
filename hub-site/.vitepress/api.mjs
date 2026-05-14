// Helpers used by build-time data loaders and runtime Vue components
// against the hub-api. Tiny and parameterless; same module works in
// Node (build) and browser (runtime).

export const HUB_API = process.env.FORAGE_HUB_API || 'https://api.foragelang.com'

// `GET /v1/packages?sort=&category=&q=&limit=`. Returns an array of
// `PackageListing` objects with snake_case keys. Returns [] on any
// transport-level failure so the build doesn't fault on a temporarily
// offline API.
export async function fetchPackages({ sort, category, q, limit = 100 } = {}, base = HUB_API) {
    try {
        const params = new URLSearchParams()
        if (sort) params.set('sort', sort)
        if (category) params.set('category', category)
        if (q) params.set('q', q)
        params.set('limit', String(limit))
        const r = await fetch(`${base}/v1/packages?${params}`)
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

// `GET /v1/packages/:author/:slug` — returns `PackageMetadata`.
export async function fetchPackageDetail(author, slug, base = HUB_API) {
    try {
        const r = await fetch(`${base}/v1/packages/${author}/${slug}`)
        if (!r.ok) return null
        return await r.json()
    } catch (err) {
        console.warn(`[hub-site] fetch ${base}/v1/packages/${author}/${slug} failed:`, err?.message ?? err)
        return null
    }
}

// `GET /v1/packages/:author/:slug/versions/latest` — returns
// `PackageVersion`. Build-time consumers grab the recipe source for
// rendering on the package page.
export async function fetchLatestVersion(author, slug, base = HUB_API) {
    try {
        const r = await fetch(`${base}/v1/packages/${author}/${slug}/versions/latest`)
        if (!r.ok) return null
        return await r.json()
    } catch (err) {
        console.warn(`[hub-site] fetch ${base}/v1/packages/${author}/${slug}/versions/latest failed:`, err?.message ?? err)
        return null
    }
}

// `GET /v1/categories` — list of category names.
export async function fetchCategories(base = HUB_API) {
    try {
        const r = await fetch(`${base}/v1/categories`)
        if (!r.ok) return []
        const data = await r.json()
        return Array.isArray(data.items) ? data.items : []
    } catch (err) {
        console.warn(`[hub-site] fetch ${base}/v1/categories failed:`, err?.message ?? err)
        return []
    }
}
