import DefaultTheme from 'vitepress/theme'
import { h } from 'vue'
import HomeDiscover from './HomeDiscover.vue'
import './custom.css'

export default {
    extends: DefaultTheme,
    Layout: () =>
        h(DefaultTheme.Layout, null, {
            'home-features-after': () => h(HomeDiscover),
        }),
}
