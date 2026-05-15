// Helpers used by build-time data loaders and runtime Vue components
// against the hub-api. Tiny and parameterless; same module works in
// Node (build) and browser (runtime).
//
// Build-time policy: the default is lenient. A cold build on a dev
// machine, fresh clone, or contributor CI without `api.foragelang.com`
// reachable should succeed with an empty discovery path set and log a
// warning, not fail. Production deploy CI sets
// `FORAGE_HUB_REQUIRE_API=1` to demand a populated route list and
// fail loud on API outage — otherwise the deployed hub would 404 on
// every direct package / profile / category URL.

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

// `GET /v1/types?sort=&category=&q=&limit=` — list of `TypeListing`
// rows. Used to enumerate dynamic discover routes (one
// `/discover/producers/<author>/<name>` page per hub-known type).
export async function fetchTypes(opts = {}, base = HUB_API) {
    const { sort, category, q, limit = 100 } = opts
    const params = new URLSearchParams()
    if (sort) params.set('sort', sort)
    if (category) params.set('category', category)
    if (q) params.set('q', q)
    params.set('limit', String(limit))
    const url = `${base}/v1/types?${params}`
    const r = await fetch(url)
    if (!r.ok) {
        throw new Error(`GET ${url} returned ${r.status}`)
    }
    const data = await r.json()
    return Array.isArray(data.items) ? data.items : []
}

// True iff the operator has explicitly opted into strict builds (the
// production deploy pipeline). Default is lenient because the
// majority of cold builds — dev machines, fresh clones, contributor
// CI — don't have the hub-api reachable and shouldn't be forced to
// set an env var to compile a static site.
function requireApi() {
    const v = process.env.FORAGE_HUB_REQUIRE_API
    return v === '1' || v === 'true'
}

// Fail-loud-on-strict wrapper for `fetchPackages`. Used by the
// dynamic `r/[author]/[slug]` + `u/[author]` route loaders where an
// empty result in a real deploy would 404 every direct URL.
export async function requirePackages(opts) {
    let list
    try {
        list = await fetchPackages(opts)
    } catch (err) {
        if (requireApi()) {
            throw new Error(`[hub-site] fetchPackages failed during build: ${err.message ?? err}. Unset FORAGE_HUB_REQUIRE_API to continue with no routes.`)
        }
        console.warn(`[hub-site] fetchPackages failed (continuing with no routes):`, err.message ?? err)
        return []
    }
    if (list.length === 0) {
        if (requireApi()) {
            throw new Error('[hub-site] fetchPackages returned no items during build; refusing to ship a hub with no dynamic routes. Unset FORAGE_HUB_REQUIRE_API to continue.')
        }
        console.warn('[hub-site] fetchPackages returned no items (continuing with no routes)')
        return []
    }
    return list
}

// Same shape for `fetchCategories`.
export async function requireCategories() {
    let cats
    try {
        cats = await fetchCategories()
    } catch (err) {
        if (requireApi()) {
            throw new Error(`[hub-site] fetchCategories failed during build: ${err.message ?? err}. Unset FORAGE_HUB_REQUIRE_API to continue with no routes.`)
        }
        console.warn(`[hub-site] fetchCategories failed (continuing with no routes):`, err.message ?? err)
        return []
    }
    if (cats.length === 0) {
        if (requireApi()) {
            throw new Error('[hub-site] fetchCategories returned no items during build; refusing to ship a hub with no category routes. Unset FORAGE_HUB_REQUIRE_API to continue.')
        }
        console.warn('[hub-site] fetchCategories returned no items (continuing with no routes)')
        return []
    }
    return cats
}

// Build-time enumerator for the type discovery routes. Empty (with a
// warning) when the API is unreachable; loud failure when strict mode
// is on and there are no types to enumerate.
export async function requireTypes() {
    let types
    try {
        types = await fetchTypes()
    } catch (err) {
        if (requireApi()) {
            throw new Error(`[hub-site] fetchTypes failed during build: ${err.message ?? err}. Unset FORAGE_HUB_REQUIRE_API to continue with no routes.`)
        }
        console.warn(`[hub-site] fetchTypes failed (continuing with no routes):`, err.message ?? err)
        return []
    }
    if (types.length === 0) {
        if (requireApi()) {
            throw new Error('[hub-site] fetchTypes returned no items during build; refusing to ship a hub with no type routes. Unset FORAGE_HUB_REQUIRE_API to continue.')
        }
        console.warn('[hub-site] fetchTypes returned no items (continuing with no routes)')
        return []
    }
    return types
}
