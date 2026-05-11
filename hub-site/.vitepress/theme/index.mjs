import DefaultTheme from 'vitepress/theme'
import { h } from 'vue'
import RecipeList from './RecipeList.vue'
import RecipeIDE from './components/RecipeIDE.vue'
import './custom.css'

export default {
    extends: DefaultTheme,
    Layout: () =>
        h(DefaultTheme.Layout, null, {
            'home-features-after': () => h(RecipeList),
        }),
    enhanceApp({ app }) {
        app.component('RecipeIDE', RecipeIDE)
    },
}
