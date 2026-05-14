<script setup>
// Card grid for `PackageListing` items as returned by the hub-api.
defineProps({
    items: {
        type: Array,
        required: true,
    },
    showAuthor: {
        type: Boolean,
        default: true,
    },
})
</script>

<template>
    <ul class="pkg-grid">
        <li v-for="item in items" :key="`${item.author}/${item.slug}`">
            <a :href="`/r/${item.author}/${item.slug}`" class="pkg-card">
                <div class="pkg-title-row">
                    <span class="pkg-title">
                        <span v-if="showAuthor" class="pkg-author">{{ item.author }}/</span>{{ item.slug }}
                    </span>
                    <span class="pkg-version">v{{ item.latest_version }}</span>
                </div>
                <div v-if="item.description" class="pkg-description">{{ item.description }}</div>
                <div class="pkg-meta">
                    <span v-if="item.category" class="pkg-category">{{ item.category }}</span>
                    <span class="pkg-stat">★ {{ item.stars }}</span>
                    <span class="pkg-stat">⬇ {{ item.downloads }}</span>
                    <span v-if="item.fork_count > 0" class="pkg-stat">⑃ {{ item.fork_count }}</span>
                    <span
                        v-if="item.forked_from"
                        class="pkg-stat pkg-fork"
                        :title="`forked from ${item.forked_from.author}/${item.forked_from.slug}@v${item.forked_from.version}`"
                    >fork</span>
                </div>
                <div v-if="item.tags?.length" class="pkg-tags">
                    <span v-for="t in item.tags" :key="t" class="pkg-tag">{{ t }}</span>
                </div>
            </a>
        </li>
    </ul>
</template>

<style scoped>
.pkg-grid {
    list-style: none;
    padding: 0;
    margin: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 12px;
}

.pkg-card {
    display: block;
    padding: 16px 18px;
    border: 1px solid var(--vp-c-divider);
    border-radius: 12px;
    background: var(--vp-c-bg-soft);
    text-decoration: none;
    color: inherit;
    transition: border-color 0.15s, transform 0.15s;
}

.pkg-card:hover {
    border-color: var(--vp-c-brand-1);
    transform: translateY(-1px);
}

.pkg-title-row {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 4px;
}

.pkg-title {
    font-weight: 600;
    color: var(--vp-c-text-1);
}

.pkg-author {
    color: var(--vp-c-text-3);
    font-weight: 500;
}

.pkg-version {
    font-size: 12px;
    color: var(--vp-c-text-3);
    font-family: var(--vp-font-family-mono);
}

.pkg-description {
    font-size: 14px;
    color: var(--vp-c-text-2);
    line-height: 1.5;
    margin-bottom: 8px;
}

.pkg-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
    font-size: 12px;
    color: var(--vp-c-text-3);
    margin-bottom: 6px;
}

.pkg-category {
    background: var(--vp-c-bg);
    color: var(--vp-c-text-2);
    padding: 1px 8px;
    border-radius: 4px;
    font-family: var(--vp-font-family-mono);
}

.pkg-stat {
    color: var(--vp-c-text-3);
}

.pkg-fork {
    color: var(--vp-c-brand-1);
}

.pkg-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
}

.pkg-tag {
    background: var(--vp-c-brand-soft);
    color: var(--vp-c-brand-1);
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 12px;
}
</style>
