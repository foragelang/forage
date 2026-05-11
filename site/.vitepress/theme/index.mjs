import DefaultTheme from 'vitepress/theme'
import { h } from 'vue'
import HomeIntro from './HomeIntro.md'
import './custom.css'

export default {
    extends: DefaultTheme,
    Layout: () =>
        h(DefaultTheme.Layout, null, {
            'home-features-before': () =>
                h('div', { class: 'vp-doc home-intro container' }, h(HomeIntro)),
        }),
}
