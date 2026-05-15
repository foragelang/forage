<script setup>
import { ref, onMounted, watch } from 'vue'

// Backs `/discover/producers/<author>/<name>` and
// `/discover/consumers/<author>/<name>`. The `kind` prop picks the
// endpoint; everything else is symmetric.
const props = defineProps({
    kind: { type: String, required: true, validator: (v) => v === 'producers' || v === 'consumers' },
    typeAuthor: { type: String, required: true },
    typeName: { type: String, required: true },
})

const HUB_API = 'https://api.foragelang.com'

const items = ref([])
const loading = ref(true)
const error = ref(null)

async function load() {
    loading.value = true
    error.value = null
    try {
        const t = `${encodeURIComponent(props.typeAuthor)}/${encodeURIComponent(props.typeName)}`
        const r = await fetch(`${HUB_API}/v1/discover/${props.kind}?type=${t}`)
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
watch(() => [props.kind, props.typeAuthor, props.typeName], load)

const verb = () => props.kind === 'producers' ? 'produce' : 'consume'
const otherKind = () => props.kind === 'producers' ? 'consumers' : 'producers'
const otherVerb = () => props.kind === 'producers' ? 'consume' : 'produce'
</script>

<template>
    <div class="discover-bytype">
        <header class="discover-bytype-header">
            <h1>
                Recipes that {{ verb() }}
                <span class="discover-bytype-type">@{{ typeAuthor }}/{{ typeName }}</span>
            </h1>
            <p>
                <a :href="`/discover/${otherKind()}/${typeAuthor}/${typeName}`">
                    Recipes that {{ otherVerb() }} this type instead
                </a>
            </p>
        </header>
        <p v-if="loading" class="discover-bytype-loading">Loading…</p>
        <p v-else-if="error" class="discover-bytype-error">Could not load: {{ error }}</p>
        <p v-else-if="items.length === 0" class="discover-bytype-empty">No recipes yet.</p>
        <PackageGrid v-else :items="items" />
    </div>
</template>

<style scoped>
.discover-bytype {
    max-width: 1152px;
    margin: 0 auto;
    padding: 32px 24px;
}

.discover-bytype-header {
    margin-bottom: 24px;
}

.discover-bytype-header h1 {
    font-size: 24px;
    font-weight: 600;
    margin: 0 0 8px;
}

.discover-bytype-type {
    color: var(--vp-c-brand-1);
    font-family: var(--vp-font-family-mono);
    text-decoration: none;
}

.discover-bytype-type:hover {
    text-decoration: underline;
}

.discover-bytype-header p {
    font-size: 14px;
    color: var(--vp-c-text-3);
    margin: 0;
}

.discover-bytype-header p a {
    color: var(--vp-c-brand-1);
    text-decoration: none;
}

.discover-bytype-header p a:hover {
    text-decoration: underline;
}

.discover-bytype-loading,
.discover-bytype-error,
.discover-bytype-empty {
    color: var(--vp-c-text-2);
    text-align: center;
    margin-top: 64px;
}
</style>
