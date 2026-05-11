<script setup>
import { ref, computed } from 'vue'
import HackerNews from './examples/hacker-news.md'
import OnThisDay from './examples/onthisday.md'
import NasaApod from './examples/nasa-apod.md'
import Earthquakes from './examples/usgs-earthquakes.md'
import GithubReleases from './examples/github-releases.md'

const examples = [
    { id: 'hn',       label: 'Hacker News',         component: HackerNews },
    { id: 'apod',     label: 'NASA APOD',           component: NasaApod },
    { id: 'quakes',   label: 'USGS earthquakes',    component: Earthquakes },
    { id: 'onthisday', label: 'Wikipedia "On this day"', component: OnThisDay },
    { id: 'releases', label: 'GitHub releases',     component: GithubReleases },
]

const active = ref(examples[0].id)
const current = computed(() => examples.find((e) => e.id === active.value))
</script>

<template>
    <div class="recipe-carousel">
        <div class="recipe-tabs" role="tablist">
            <button
                v-for="ex in examples"
                :key="ex.id"
                role="tab"
                :aria-selected="active === ex.id"
                :class="['recipe-tab', { active: active === ex.id }]"
                @click="active = ex.id"
            >{{ ex.label }}</button>
        </div>
        <div class="recipe-pane" role="tabpanel">
            <component :is="current.component" />
        </div>
    </div>
</template>
