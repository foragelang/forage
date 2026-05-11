import DefaultTheme from 'vitepress/theme'
import { h } from 'vue'
import HomeIntro from './HomeIntro.md'
import './custom.css'

export default {
    extends: DefaultTheme,
    Layout: () =>
        h(DefaultTheme.Layout, null, {
            'home-features-before': () =>
                h('div', { class: 'home-intro' },
                    h('div', { class: 'home-intro-inner vp-doc' }, h(HomeIntro))),
        }),
}
