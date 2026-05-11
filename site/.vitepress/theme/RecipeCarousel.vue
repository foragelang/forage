<script setup>
import { ref, computed } from 'vue'
import HackerNews from './examples/hacker-news.md'
import OnThisDay from './examples/onthisday.md'
import NasaApod from './examples/nasa-apod.md'
import Earthquakes from './examples/usgs-earthquakes.md'
import GithubReleases from './examples/github-releases.md'

const examples = [
    { id: 'hn',       label: 'Hacker News',         component: HackerNews,
      summary: 'Top 30 front-page stories from the Algolia search API. One step, no auth, no pagination — the smallest end-to-end recipe.' },
    { id: 'apod',     label: 'NASA APOD',           component: NasaApod,
      summary: 'NASA\'s Astronomy Picture of the Day archive for a date range. One entry per day with title, image URL, prose, and copyright.' },
    { id: 'quakes',   label: 'USGS earthquakes',    component: Earthquakes,
      summary: 'Every M4.5+ earthquake from the past week. GeoJSON feed updated every five minutes; magnitude, place, depth, time, URL.' },
    { id: 'onthisday', label: 'Wikipedia "On this day"', component: OnThisDay,
      summary: 'Wikimedia\'s REST feed of events/births/deaths for any MM/DD. Year + text + a canonical title from the linked Wikipedia page.' },
    { id: 'releases', label: 'GitHub releases',     component: GithubReleases,
      summary: 'Latest 15 releases for any public GitHub repo. One paginated GET, ordered newest-first.' },
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
        <p class="recipe-summary">{{ current.summary }}</p>
        <div class="recipe-pane" role="tabpanel">
            <component :is="current.component" />
        </div>
    </div>
</template>
