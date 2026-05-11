<script setup>
import { ref, onMounted } from 'vue'
import { data } from '../recipes.data.mjs'

// Two-track loading: prefer fresh data from the API at runtime, but fall back
// to the build-time snapshot so the page is not empty during the first paint
// (and is still useful if the API is offline).
const items = ref(data.items)
const loaded = ref(true)
const error = ref(null)

onMounted(async () => {
    try {
        const r = await fetch('https://api.foragelang.com/v1/recipes?limit=100')
        if (!r.ok) throw new Error(`HTTP ${r.status}`)
        const fresh = await r.json()
        if (Array.isArray(fresh.items)) items.value = fresh.items
    } catch (err) {
        // Build-time snapshot is still showing; signal but don't blow up.
        error.value = err.message || String(err)
    } finally {
        loaded.value = true
    }
})
</script>

<template>
    <div class="recipe-list">
        <h2>Recipes</h2>
        <p v-if="error" class="recipe-list-warn">Could not refresh from API ({{ error }}); showing the latest build-time snapshot.</p>
        <p v-if="loaded && items.length === 0" class="recipe-list-empty">
            No recipes published yet.
        </p>
        <ul v-else class="recipe-list-items">
            <li v-for="item in items" :key="item.slug">
                <a :href="`/r/${item.slug}`" class="recipe-list-item">
                    <div class="recipe-list-title">{{ item.displayName }}</div>
                    <div class="recipe-list-summary">{{ item.summary }}</div>
                    <div class="recipe-list-meta">
                        <span v-if="item.author">{{ item.author }}</span>
                        <span v-if="item.platform">{{ item.platform }}</span>
                        <span v-for="tag in item.tags" :key="tag" class="recipe-list-tag">{{ tag }}</span>
                    </div>
                </a>
            </li>
        </ul>
    </div>
</template>

<style scoped>
.recipe-list {
    max-width: 1152px;
    margin: 48px auto 0;
    padding: 0 24px;
}

@media (min-width: 640px) {
    .recipe-list {
        padding: 0 48px;
    }
}

@media (min-width: 960px) {
    .recipe-list {
        padding: 0 64px;
    }
}

.recipe-list h2 {
    margin: 0 0 16px;
    font-size: 20px;
    font-weight: 600;
    color: var(--vp-c-text-1);
}

.recipe-list-warn {
    font-size: 13px;
    color: var(--vp-c-text-3);
    margin-bottom: 12px;
}

.recipe-list-empty {
    color: var(--vp-c-text-2);
}

.recipe-list-items {
    list-style: none;
    padding: 0;
    margin: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 12px;
}

.recipe-list-item {
    display: block;
    padding: 16px 18px;
    border: 1px solid var(--vp-c-divider);
    border-radius: 12px;
    background: var(--vp-c-bg-soft);
    text-decoration: none;
    color: inherit;
    transition: border-color 0.15s, transform 0.15s;
}

.recipe-list-item:hover {
    border-color: var(--vp-c-brand-1);
    transform: translateY(-1px);
}

.recipe-list-title {
    font-weight: 600;
    color: var(--vp-c-text-1);
    margin-bottom: 4px;
}

.recipe-list-summary {
    font-size: 14px;
    color: var(--vp-c-text-2);
    line-height: 1.5;
    margin-bottom: 8px;
}

.recipe-list-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    font-size: 12px;
    color: var(--vp-c-text-3);
}

.recipe-list-tag {
    background: var(--vp-c-brand-soft);
    color: var(--vp-c-brand-1);
    padding: 1px 8px;
    border-radius: 999px;
}
</style>
