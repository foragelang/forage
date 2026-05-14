// Helpers used by build-time data loaders and runtime Vue components
// against the hub-api. Tiny and parameterless; same module works in
// Node (build) and browser (runtime).
//
// Errors propagate; the swallow-on-error policy lives in the
// individual callers. The dynamic-route loaders use the strict
// `requirePackages` / `requireCategories` wrappers which fail the
// build by default; the home-page snapshot loader uses the raw
// helpers and tolerates empty/failing results.
//
// To run a permissive build locally (e.g. when the API is unreachable
// from your machine), set `FORAGE_HUB_PERMISSIVE_BUILD=1`. The
// production deploy pipeline leaves it unset so an outage at deploy
// time fails loud rather than shipping a hub with zero dynamic
// routes (and therefore 404s on every direct package / profile /
// category URL).

export const HUB_API = process.env.FORAGE_HUB_API || 'https://api.foragelang.com'

// `GET /v1/packages?sort=&category=&q=&limit=`. Returns an array of
// `PackageListing` objects with snake_case keys. Throws on transport
// failure or non-OK status — the caller decides whether that's fatal.
export async function fetchPackages({ sort, category, q, limit = 100 } = {}, base = HUB_API) {
    const params = new URLSearchParams()
    if (sort) params.set('sort', sort)
    if (category) params.set('category', category)
    if (q) params.set('q', q)
    params.set('limit', String(limit))
    const url = `${base}/v1/packages?${params}`
    const r = await fetch(url)
    if (!r.ok) {
        throw new Error(`GET ${url} returned ${r.status}`)
    }
    const data = await r.json()
    return Array.isArray(data.items) ? data.items : []
}

// `GET /v1/categories` — list of category names. Throws on transport
// failure or non-OK status.
export async function fetchCategories(base = HUB_API) {
    const url = `${base}/v1/categories`
    const r = await fetch(url)
    if (!r.ok) {
        throw new Error(`GET ${url} returned ${r.status}`)
    }
    const data = await r.json()
    return Array.isArray(data.items) ? data.items : []
}

// True iff the operator has explicitly opted into permissive builds
// (e.g. local dev with the API offline). The deploy pipeline leaves
// this unset so transport errors and empty results fail the build.
function permissive() {
    const v = process.env.FORAGE_HUB_PERMISSIVE_BUILD
    return v === '1' || v === 'true'
}

// Fail-loud wrapper for `fetchPackages`. Used by the dynamic
// `r/[author]/[slug]` + `u/[author]` route loaders where an empty
// result in a real deploy would 404 every direct URL.
export async function requirePackages(opts) {
    let list
    try {
        list = await fetchPackages(opts)
    } catch (err) {
        if (permissive()) {
            console.warn(`[hub-site] fetchPackages failed (permissive build, continuing):`, err.message ?? err)
            return []
        }
        throw new Error(`[hub-site] fetchPackages failed during build: ${err.message ?? err}. Set FORAGE_HUB_PERMISSIVE_BUILD=1 to continue with no routes.`)
    }
    if (list.length === 0) {
        if (permissive()) {
            console.warn('[hub-site] fetchPackages returned no items (permissive build, continuing)')
            return []
        }
        throw new Error('[hub-site] fetchPackages returned no items during build; refusing to ship a hub with no dynamic routes. Set FORAGE_HUB_PERMISSIVE_BUILD=1 to continue.')
    }
    return list
}

// Same shape for `fetchCategories`.
export async function requireCategories() {
    let cats
    try {
        cats = await fetchCategories()
    } catch (err) {
        if (permissive()) {
            console.warn(`[hub-site] fetchCategories failed (permissive build, continuing):`, err.message ?? err)
            return []
        }
        throw new Error(`[hub-site] fetchCategories failed during build: ${err.message ?? err}. Set FORAGE_HUB_PERMISSIVE_BUILD=1 to continue with no routes.`)
    }
    if (cats.length === 0) {
        if (permissive()) {
            console.warn('[hub-site] fetchCategories returned no items (permissive build, continuing)')
            return []
        }
        throw new Error('[hub-site] fetchCategories returned no items during build; refusing to ship a hub with no category routes. Set FORAGE_HUB_PERMISSIVE_BUILD=1 to continue.')
    }
    return cats
}
