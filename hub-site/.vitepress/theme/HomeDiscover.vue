<script setup>
import { ref, onMounted, computed } from 'vue'
import { data as snapshot } from '../packages.data.mjs'

// Three discovery surfaces on the home page: top-starred,
// top-downloaded, recent. Categories listed as quick filters.
//
// Two-track loading: prefer fresh data from the API at runtime; fall
// back to the build-time snapshot so the page is not empty on first
// paint and stays readable when the API is offline.

const HUB_API = 'https://api.foragelang.com'

const recent = ref(snapshot.items)
const topStarred = ref([])
const topDownloaded = ref([])
const categories = ref([])
const error = ref(null)

async function fetchList(query) {
    const r = await fetch(`${HUB_API}/v1/packages?${query}&limit=12`)
    if (!r.ok) throw new Error(`HTTP ${r.status}`)
    const data = await r.json()
    return Array.isArray(data.items) ? data.items : []
}

onMounted(async () => {
    try {
        const [recentFresh, starred, downloaded, cats] = await Promise.all([
            fetchList('sort=recent'),
            fetchList('sort=stars'),
            fetchList('sort=downloads'),
            fetch(`${HUB_API}/v1/categories`)
                .then((r) => (r.ok ? r.json() : { items: [] }))
                .then((d) => (Array.isArray(d.items) ? d.items : [])),
        ])
        recent.value = recentFresh.length > 0 ? recentFresh : recent.value
        topStarred.value = starred
        topDownloaded.value = downloaded
        categories.value = cats
    } catch (err) {
        error.value = err.message || String(err)
    }
})

const hasAny = computed(
    () =>
        recent.value.length > 0
        || topStarred.value.length > 0
        || topDownloaded.value.length > 0,
)
</script>

<template>
    <div class="discover">
        <p v-if="error" class="discover-warn">Could not refresh from API ({{ error }}); showing the latest build-time snapshot.</p>
        <p v-if="!hasAny" class="discover-empty">No packages published yet.</p>

        <section v-if="topStarred.length > 0" class="discover-section">
            <header class="discover-header">
                <h2>Top starred</h2>
                <a href="/discover/top-starred">See all</a>
            </header>
            <PackageGrid :items="topStarred" />
        </section>

        <section v-if="topDownloaded.length > 0" class="discover-section">
            <header class="discover-header">
                <h2>Most downloaded</h2>
                <a href="/discover/top-downloaded">See all</a>
            </header>
            <PackageGrid :items="topDownloaded" />
        </section>

        <section v-if="recent.length > 0" class="discover-section">
            <header class="discover-header">
                <h2>Recent</h2>
            </header>
            <PackageGrid :items="recent" />
        </section>

        <section v-if="categories.length > 0" class="discover-section">
            <header class="discover-header">
                <h2>By category</h2>
            </header>
            <ul class="discover-categories">
                <li v-for="c in categories" :key="c">
                    <a :href="`/c/${c}`">{{ c }}</a>
                </li>
            </ul>
        </section>
    </div>
</template>

<style scoped>
.discover {
    max-width: 1152px;
    margin: 48px auto 0;
    padding: 0 24px;
}

@media (min-width: 640px) {
    .discover {
        padding: 0 48px;
    }
}

@media (min-width: 960px) {
    .discover {
        padding: 0 64px;
    }
}

.discover-section {
    margin-bottom: 48px;
}

.discover-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin: 0 0 16px;
}

.discover-header h2 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
    color: var(--vp-c-text-1);
}

.discover-header a {
    font-size: 13px;
    color: var(--vp-c-brand-1);
    text-decoration: none;
}

.discover-warn {
    font-size: 13px;
    color: var(--vp-c-text-3);
    margin-bottom: 12px;
}

.discover-empty {
    color: var(--vp-c-text-2);
}

.discover-categories {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
}

.discover-categories a {
    display: inline-block;
    padding: 4px 12px;
    background: var(--vp-c-brand-soft);
    color: var(--vp-c-brand-1);
    border-radius: 999px;
    text-decoration: none;
    font-size: 13px;
}

.discover-categories a:hover {
    background: var(--vp-c-brand-2);
    color: var(--vp-c-white);
}
</style>
