<script setup>
import { ref, onMounted, watch } from 'vue'

// Backs `/discover/aligned-with/<ontology>/<term>`. Lists every hub
// type whose latest version declares this alignment URI.
const props = defineProps({
    ontology: { type: String, required: true },
    term: { type: String, required: true },
})

const HUB_API = 'https://api.foragelang.com'

const items = ref([])
const loading = ref(true)
const error = ref(null)

async function load() {
    loading.value = true
    error.value = null
    try {
        const uri = `${encodeURIComponent(props.ontology)}/${encodeURIComponent(props.term)}`
        const r = await fetch(`${HUB_API}/v1/discover/aligned-with?term=${uri}`)
        if (!r.ok) throw new Error(`HTTP ${r.status}`)
        const data = await r.json()
        items.value = Array.isArray(data.items) ? data.items : []
    } catch (err) {
        error.value = err.message || String(err)
    } finally {
        loading.value = false
    }
}

onMounted(load)
watch(() => [props.ontology, props.term], load)
</script>

<template>
    <div class="discover-aligned">
        <header class="discover-aligned-header">
            <h1>
                Types aligned with
                <span class="discover-aligned-term">{{ ontology }}/{{ term }}</span>
            </h1>
        </header>
        <p v-if="loading" class="discover-aligned-loading">Loading…</p>
        <p v-else-if="error" class="discover-aligned-error">Could not load: {{ error }}</p>
        <p v-else-if="items.length === 0" class="discover-aligned-empty">No types declare this alignment yet.</p>
        <ul v-else class="discover-aligned-grid">
            <li v-for="t in items" :key="`${t.author}/${t.name}`" class="discover-aligned-card">
                <div class="discover-aligned-title">
                    <span class="discover-aligned-author">@{{ t.author }}/</span>{{ t.name }}
                    <span class="discover-aligned-version">v{{ t.latest_version }}</span>
                </div>
                <div v-if="t.description" class="discover-aligned-description">{{ t.description }}</div>
                <div class="discover-aligned-meta">
                    <span v-if="t.category" class="discover-aligned-category">{{ t.category }}</span>
                </div>
                <div class="discover-aligned-actions">
                    <a :href="`/discover/producers/${t.author}/${t.name}`">producers</a>
                    ·
                    <a :href="`/discover/consumers/${t.author}/${t.name}`">consumers</a>
                </div>
            </li>
        </ul>
    </div>
</template>

<style scoped>
.discover-aligned {
    max-width: 1152px;
    margin: 0 auto;
    padding: 32px 24px;
}

.discover-aligned-header h1 {
    font-size: 24px;
    font-weight: 600;
    margin: 0 0 16px;
}

.discover-aligned-term {
    color: var(--vp-c-brand-1);
    font-family: var(--vp-font-family-mono);
}

.discover-aligned-grid {
    list-style: none;
    padding: 0;
    margin: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 12px;
}

.discover-aligned-card {
    display: block;
    padding: 16px 18px;
    border: 1px solid var(--vp-c-divider);
    border-radius: 12px;
    background: var(--vp-c-bg-soft);
    text-decoration: none;
    color: inherit;
    transition: border-color 0.15s, transform 0.15s;
}

.discover-aligned-card:hover {
    border-color: var(--vp-c-brand-1);
    transform: translateY(-1px);
}

.discover-aligned-title {
    font-weight: 600;
    color: var(--vp-c-text-1);
    margin-bottom: 4px;
}

.discover-aligned-author {
    color: var(--vp-c-text-3);
    font-weight: 500;
}

.discover-aligned-version {
    font-size: 12px;
    color: var(--vp-c-text-3);
    margin-left: 8px;
    font-family: var(--vp-font-family-mono);
}

.discover-aligned-description {
    font-size: 14px;
    color: var(--vp-c-text-2);
    line-height: 1.5;
    margin-bottom: 8px;
}

.discover-aligned-meta {
    font-size: 12px;
    color: var(--vp-c-text-3);
}

.discover-aligned-category {
    background: var(--vp-c-bg);
    padding: 1px 8px;
    border-radius: 4px;
    font-family: var(--vp-font-family-mono);
}

.discover-aligned-actions {
    font-size: 12px;
    color: var(--vp-c-text-3);
    margin-top: 6px;
    text-align: center;
}

.discover-aligned-actions a {
    color: var(--vp-c-brand-1);
    text-decoration: none;
}

.discover-aligned-actions a:hover {
    text-decoration: underline;
}

.discover-aligned-loading,
.discover-aligned-error,
.discover-aligned-empty {
    color: var(--vp-c-text-2);
    text-align: center;
    margin-top: 64px;
}
</style>
