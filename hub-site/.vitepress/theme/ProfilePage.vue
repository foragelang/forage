<script setup>
import { ref, onMounted } from 'vue'

const props = defineProps({
    author: { type: String, required: true },
})

const HUB_API = 'https://api.foragelang.com'

const profile = ref(null)
const packages = ref([])
const stars = ref([])
const error = ref(null)
const loading = ref(true)

onMounted(async () => {
    try {
        const [profileResp, pkgsResp, starsResp] = await Promise.all([
            fetch(`${HUB_API}/v1/users/${props.author}`),
            fetch(`${HUB_API}/v1/users/${props.author}/packages`),
            fetch(`${HUB_API}/v1/users/${props.author}/stars`),
        ])
        if (!profileResp.ok) throw new Error(`HTTP ${profileResp.status} on profile`)
        profile.value = await profileResp.json()
        if (pkgsResp.ok) {
            const data = await pkgsResp.json()
            packages.value = Array.isArray(data.items) ? data.items : []
        }
        if (starsResp.ok) {
            const data = await starsResp.json()
            stars.value = Array.isArray(data.items) ? data.items : []
        }
    } catch (err) {
        error.value = err.message || String(err)
    } finally {
        loading.value = false
    }
})
</script>

<template>
    <div class="profile">
        <p v-if="loading" class="profile-loading">Loading…</p>
        <p v-else-if="error" class="profile-error">Could not load profile: {{ error }}</p>
        <template v-else-if="profile">
            <header class="profile-header">
                <img v-if="profile.avatar_url" :src="profile.avatar_url" :alt="profile.login" class="profile-avatar">
                <div class="profile-identity">
                    <h1>{{ profile.name || profile.login }}</h1>
                    <p class="profile-handle">@{{ profile.login }}</p>
                    <p class="profile-stats">
                        {{ profile.package_count }} packages · {{ profile.star_count }} stars given
                    </p>
                </div>
            </header>

            <section v-if="packages.length > 0" class="profile-section">
                <h2>Packages</h2>
                <PackageGrid :items="packages" :show-author="false" />
            </section>

            <section v-if="stars.length > 0" class="profile-section">
                <h2>Starred</h2>
                <ul class="profile-stars">
                    <li v-for="s in stars" :key="`${s.author}/${s.slug}`">
                        <a :href="`/r/${s.author}/${s.slug}`">{{ s.author }}/{{ s.slug }}</a>
                    </li>
                </ul>
            </section>

            <p v-if="packages.length === 0 && stars.length === 0" class="profile-empty">
                Nothing to show yet.
            </p>
        </template>
    </div>
</template>

<style scoped>
.profile {
    max-width: 1152px;
    margin: 0 auto;
    padding: 32px 24px;
}

.profile-loading,
.profile-error,
.profile-empty {
    color: var(--vp-c-text-2);
    text-align: center;
    margin-top: 64px;
}

.profile-header {
    display: flex;
    align-items: center;
    gap: 24px;
    margin-bottom: 32px;
}

.profile-avatar {
    width: 96px;
    height: 96px;
    border-radius: 50%;
    border: 1px solid var(--vp-c-divider);
}

.profile-identity h1 {
    margin: 0 0 4px;
    font-size: 28px;
    font-weight: 600;
}

.profile-handle {
    margin: 0 0 4px;
    color: var(--vp-c-text-3);
    font-family: var(--vp-font-family-mono);
}

.profile-stats {
    margin: 0;
    color: var(--vp-c-text-3);
    font-size: 14px;
}

.profile-section {
    margin-bottom: 32px;
}

.profile-section h2 {
    font-size: 18px;
    font-weight: 600;
    margin: 0 0 12px;
    color: var(--vp-c-text-1);
}

.profile-stars {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
}

.profile-stars a {
    background: var(--vp-c-bg-soft);
    padding: 4px 12px;
    border-radius: 999px;
    color: var(--vp-c-text-1);
    text-decoration: none;
    font-size: 14px;
}

.profile-stars a:hover {
    background: var(--vp-c-brand-soft);
    color: var(--vp-c-brand-1);
}
</style>
