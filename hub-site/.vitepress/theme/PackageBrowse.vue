<script setup>
import { ref, onMounted, watch } from 'vue'

const props = defineProps({
    // Fixed sort key; if set, the page is a "top X" surface and the
    // sort cannot be changed by the user.
    sort: { type: String, default: null },
    // Fixed category filter; if set, the page is a category page.
    category: { type: String, default: null },
    title: { type: String, default: '' },
})

const HUB_API = 'https://api.foragelang.com'

const items = ref([])
const loading = ref(true)
const error = ref(null)

async function load() {
    loading.value = true
    error.value = null
    try {
        const params = new URLSearchParams()
        if (props.sort) params.set('sort', props.sort)
        if (props.category) params.set('category', props.category)
        params.set('limit', '100')
        const r = await fetch(`${HUB_API}/v1/packages?${params}`)
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
watch(() => [props.sort, props.category], load)
</script>

<template>
    <div class="pkg-browse">
        <h1>{{ title }}</h1>
        <p v-if="loading" class="pkg-browse-loading">Loading…</p>
        <p v-else-if="error" class="pkg-browse-error">Could not load packages: {{ error }}</p>
        <p v-else-if="items.length === 0" class="pkg-browse-empty">Nothing to show here yet.</p>
        <PackageGrid v-else :items="items" />
    </div>
</template>

<style scoped>
.pkg-browse {
    max-width: 1152px;
    margin: 0 auto;
    padding: 32px 24px;
}

.pkg-browse h1 {
    font-size: 28px;
    font-weight: 600;
    margin: 0 0 24px;
}

.pkg-browse-loading,
.pkg-browse-error,
.pkg-browse-empty {
    color: var(--vp-c-text-2);
    text-align: center;
    margin-top: 64px;
}
</style>
