// Build-time data loader. Exposes the recipe listing to pages via
// `useData()` / `<script setup>` `import { data } from '../../.vitepress/recipes.data'`.
import { fetchRecipeList } from './api.mjs'

export default {
    async load() {
        const items = await fetchRecipeList()
        return { items, fetchedAt: new Date().toISOString() }
    },
}
