<script setup>
import { ref, onMounted } from 'vue'

// Discover landing page. Lists the available type-shaped browses
// (producers / consumers per known type, plus the distinct alignment
// URIs that any hub type declares). Falls back to the keyword /
// sort-based pages for users who don't want type-shaped browsing.

const HUB_API = 'https://api.foragelang.com'

const types = ref([])
const alignmentUris = ref([])
const loading = ref(true)
const error = ref(null)

async function load() {
    loading.value = true
    error.value = null
    try {
        const r = await fetch(`${HUB_API}/v1/types?limit=100`)
        if (!r.ok) throw new Error(`HTTP ${r.status}`)
        const data = await r.json()
        const items = Array.isArray(data.items) ? data.items : []
        types.value = items

        // Walk each type's latest version to collect distinct
        // alignment URIs. Pre-1.0 volume is small; if the type count
        // grows past a few dozen, swap this for a dedicated endpoint.
        const seen = new Set()
        const collected = []
        for (const t of items) {
            const url = `${HUB_API}/v1/types/${encodeURIComponent(t.author)}/${encodeURIComponent(t.name)}/versions/latest`
            const vr = await fetch(url)
            if (!vr.ok) continue
            const version = await vr.json()
            const alignments = Array.isArray(version.alignments) ? version.alignments : []
            for (const a of alignments) {
                const key = `${a.ontology}/${a.term}`
                if (seen.has(key)) continue
                seen.add(key)
                collected.push({ ontology: a.ontology, term: a.term })
            }
        }
        alignmentUris.value = collected
    } catch (err) {
        error.value = err.message || String(err)
    } finally {
        loading.value = false
    }
}

onMounted(load)
</script>

<template>
    <div class="discover-index">
        <h1>Discover by type</h1>
        <p class="discover-index-blurb">
            Find recipes by the type they produce or consume, or browse types by the ontology they're aligned with.
        </p>

        <p v-if="loading" class="discover-index-loading">Loading…</p>
        <p v-else-if="error" class="discover-index-error">Could not load: {{ error }}</p>
        <template v-else>
            <section class="discover-index-section">
                <h2>Types</h2>
                <p v-if="types.length === 0" class="discover-index-empty">No types published yet.</p>
                <ul v-else class="discover-index-types">
                    <li v-for="t in types" :key="`${t.author}/${t.name}`">
                        <span class="discover-index-type-id">@{{ t.author }}/{{ t.name }}</span>
                        <span class="discover-index-links">
                            <a :href="`/discover/producers/${t.author}/${t.name}`">producers</a>
                            ·
                            <a :href="`/discover/consumers/${t.author}/${t.name}`">consumers</a>
                        </span>
                    </li>
                </ul>
            </section>

            <section class="discover-index-section">
                <h2>Alignment URIs</h2>
                <p v-if="alignmentUris.length === 0" class="discover-index-empty">No alignments declared yet.</p>
                <ul v-else class="discover-index-alignments">
                    <li v-for="a in alignmentUris" :key="`${a.ontology}/${a.term}`">
                        <a :href="`/discover/aligned-with/${a.ontology}/${a.term}`">{{ a.ontology }}/{{ a.term }}</a>
                    </li>
                </ul>
            </section>
        </template>

        <section class="discover-index-section">
            <h2>By keyword</h2>
            <ul class="discover-index-links-list">
                <li><a href="/discover/top-starred">Top starred</a></li>
                <li><a href="/discover/top-downloaded">Most downloaded</a></li>
            </ul>
        </section>
    </div>
</template>

<style scoped>
.discover-index {
    max-width: 1152px;
    margin: 0 auto;
    padding: 32px 24px;
}

.discover-index h1 {
    font-size: 28px;
    font-weight: 600;
    margin: 0 0 8px;
}

.discover-index-blurb {
    color: var(--vp-c-text-2);
    margin: 0 0 32px;
}

.discover-index-section {
    margin-bottom: 32px;
}

.discover-index-section h2 {
    font-size: 18px;
    font-weight: 600;
    margin: 0 0 12px;
}

.discover-index-types,
.discover-index-alignments,
.discover-index-links-list {
    list-style: none;
    padding: 0;
    margin: 0;
}

.discover-index-types li {
    display: flex;
    align-items: baseline;
    gap: 16px;
    padding: 8px 0;
    border-bottom: 1px solid var(--vp-c-divider);
}

.discover-index-type-id {
    font-family: var(--vp-font-family-mono);
    color: var(--vp-c-text-1);
}

.discover-index-links {
    font-size: 13px;
    color: var(--vp-c-text-3);
}

.discover-index-links a,
.discover-index-alignments a,
.discover-index-links-list a {
    color: var(--vp-c-brand-1);
    text-decoration: none;
}

.discover-index-links a:hover,
.discover-index-alignments a:hover,
.discover-index-links-list a:hover {
    text-decoration: underline;
}

.discover-index-alignments li,
.discover-index-links-list li {
    padding: 4px 0;
    font-family: var(--vp-font-family-mono);
}

.discover-index-loading,
.discover-index-error,
.discover-index-empty {
    color: var(--vp-c-text-2);
}
</style>
