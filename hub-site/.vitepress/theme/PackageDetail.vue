<script setup>
import { ref, onMounted } from 'vue'

const props = defineProps({
    author: { type: String, required: true },
    slug: { type: String, required: true },
})

const HUB_API = 'https://api.foragelang.com'

const meta = ref(null)
const artifact = ref(null)
const versions = ref([])
const error = ref(null)
const loading = ref(true)

onMounted(async () => {
    try {
        const [metaResp, latestResp, versionsResp] = await Promise.all([
            fetch(`${HUB_API}/v1/packages/${props.author}/${props.slug}`),
            fetch(`${HUB_API}/v1/packages/${props.author}/${props.slug}/versions/latest`),
            fetch(`${HUB_API}/v1/packages/${props.author}/${props.slug}/versions`),
        ])
        if (!metaResp.ok) throw new Error(`HTTP ${metaResp.status} on package metadata`)
        meta.value = await metaResp.json()
        if (latestResp.ok) artifact.value = await latestResp.json()
        if (versionsResp.ok) {
            const data = await versionsResp.json()
            versions.value = Array.isArray(data.items) ? data.items : []
        }
    } catch (err) {
        error.value = err.message || String(err)
    } finally {
        loading.value = false
    }
})

function formatTimestamp(ms) {
    if (!ms) return ''
    return new Date(ms).toLocaleDateString(undefined, {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
    })
}
</script>

<template>
    <div class="pkg-detail">
        <p v-if="loading" class="pkg-detail-loading">Loading…</p>
        <p v-else-if="error" class="pkg-detail-error">Could not load {{ author }}/{{ slug }}: {{ error }}</p>
        <template v-else-if="meta">
            <header class="pkg-detail-header">
                <h1>
                    <a :href="`/u/${meta.author}`" class="pkg-detail-author">{{ meta.author }}</a>/<span class="pkg-detail-slug">{{ meta.slug }}</span>
                </h1>
                <p v-if="meta.description" class="pkg-detail-description">{{ meta.description }}</p>
                <div class="pkg-detail-meta">
                    <span>v{{ meta.latest_version }} · published {{ formatTimestamp(artifact?.published_at) }}</span>
                    <span class="pkg-detail-category">
                        <a :href="`/c/${meta.category}`">{{ meta.category }}</a>
                    </span>
                    <span>★ {{ meta.stars }}</span>
                    <span>⬇ {{ meta.downloads }}</span>
                    <span v-if="meta.fork_count > 0">⑃ {{ meta.fork_count }} forks</span>
                </div>
                <p v-if="meta.forked_from" class="pkg-detail-lineage">
                    Forked from
                    <a :href="`/r/${meta.forked_from.author}/${meta.forked_from.slug}`">
                        {{ meta.forked_from.author }}/{{ meta.forked_from.slug }}
                    </a>
                    @ v{{ meta.forked_from.version }}
                </p>
                <ul v-if="meta.tags?.length" class="pkg-detail-tags">
                    <li v-for="t in meta.tags" :key="t">{{ t }}</li>
                </ul>
            </header>

            <section v-if="artifact" class="pkg-detail-section">
                <h2>Recipe</h2>
                <pre class="pkg-detail-source"><code>{{ artifact.recipe }}</code></pre>
            </section>

            <section v-if="artifact?.decls?.length" class="pkg-detail-section">
                <h2>Decls</h2>
                <details v-for="decl in artifact.decls" :key="decl.name" class="pkg-detail-file">
                    <summary>{{ decl.name }}</summary>
                    <pre><code>{{ decl.source }}</code></pre>
                </details>
            </section>

            <section v-if="artifact?.fixtures?.length" class="pkg-detail-section">
                <h2>Fixtures</h2>
                <ul class="pkg-detail-fixtures">
                    <li v-for="f in artifact.fixtures" :key="f.name">
                        <strong>{{ f.name }}</strong>
                        <span class="pkg-detail-fixture-size">{{ f.content.length.toLocaleString() }} bytes</span>
                    </li>
                </ul>
            </section>

            <section v-if="artifact?.snapshot" class="pkg-detail-section">
                <h2>Snapshot</h2>
                <ul class="pkg-detail-counts">
                    <li v-for="(n, type) in artifact.snapshot.counts" :key="type">
                        <strong>{{ type }}</strong>: {{ n.toLocaleString() }}
                    </li>
                </ul>
            </section>

            <section v-if="versions.length > 0" class="pkg-detail-section">
                <h2>Versions</h2>
                <ol class="pkg-detail-versions">
                    <li v-for="v in versions.slice().reverse()" :key="v.version">
                        v{{ v.version }} · {{ formatTimestamp(v.published_at) }} · {{ v.published_by }}
                    </li>
                </ol>
            </section>

            <section class="pkg-detail-section">
                <h2>Install</h2>
                <pre class="pkg-detail-install"><code>forage sync {{ meta.author }}/{{ meta.slug }}</code></pre>
                <p><a :href="`/edit/${meta.author}/${meta.slug}`">Open in hub IDE</a> · <a :href="`forage-studio://recipe/${meta.author}/${meta.slug}`">Open in Studio</a></p>
            </section>
        </template>
    </div>
</template>

<style scoped>
.pkg-detail {
    max-width: 800px;
    margin: 0 auto;
    padding: 32px 24px;
}

.pkg-detail-loading,
.pkg-detail-error {
    color: var(--vp-c-text-2);
    text-align: center;
    margin-top: 64px;
}

.pkg-detail-header h1 {
    margin: 0 0 8px;
    font-size: 28px;
    font-weight: 600;
}

.pkg-detail-author {
    color: var(--vp-c-text-2);
    text-decoration: none;
}

.pkg-detail-author:hover {
    text-decoration: underline;
}

.pkg-detail-slug {
    color: var(--vp-c-text-1);
}

.pkg-detail-description {
    color: var(--vp-c-text-2);
    font-size: 16px;
    margin: 8px 0 16px;
}

.pkg-detail-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
    font-size: 13px;
    color: var(--vp-c-text-3);
    margin-bottom: 12px;
}

.pkg-detail-category a {
    background: var(--vp-c-bg-soft);
    padding: 2px 8px;
    border-radius: 4px;
    color: var(--vp-c-text-2);
    text-decoration: none;
    font-family: var(--vp-font-family-mono);
}

.pkg-detail-lineage {
    font-size: 13px;
    color: var(--vp-c-text-3);
    margin: 0 0 12px;
}

.pkg-detail-tags {
    list-style: none;
    padding: 0;
    margin: 0 0 16px;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
}

.pkg-detail-tags li {
    background: var(--vp-c-brand-soft);
    color: var(--vp-c-brand-1);
    padding: 2px 10px;
    border-radius: 999px;
    font-size: 12px;
}

.pkg-detail-section {
    margin-top: 32px;
}

.pkg-detail-section h2 {
    font-size: 18px;
    font-weight: 600;
    margin: 0 0 12px;
    color: var(--vp-c-text-1);
}

.pkg-detail-source,
.pkg-detail-install,
.pkg-detail-file pre {
    background: var(--vp-c-bg-alt);
    border: 1px solid var(--vp-c-divider);
    border-radius: 8px;
    padding: 16px;
    overflow-x: auto;
    font-family: var(--vp-font-family-mono);
    font-size: 13px;
    line-height: 1.55;
}

.pkg-detail-file summary {
    cursor: pointer;
    padding: 8px 0;
    color: var(--vp-c-text-2);
    font-family: var(--vp-font-family-mono);
}

.pkg-detail-fixtures,
.pkg-detail-counts,
.pkg-detail-versions {
    margin: 0;
    padding: 0 0 0 20px;
    color: var(--vp-c-text-2);
}

.pkg-detail-fixtures li,
.pkg-detail-counts li {
    font-size: 14px;
    margin-bottom: 4px;
}

.pkg-detail-fixture-size {
    color: var(--vp-c-text-3);
    font-size: 12px;
    margin-left: 8px;
}
</style>
