import DefaultTheme from 'vitepress/theme'
import { h } from 'vue'
import HomeDiscover from './HomeDiscover.vue'
import PackageGrid from './PackageGrid.vue'
import PackageDetail from './PackageDetail.vue'
import PackageBrowse from './PackageBrowse.vue'
import ProfilePage from './ProfilePage.vue'
import './custom.css'

export default {
    extends: DefaultTheme,
    Layout: () =>
        h(DefaultTheme.Layout, null, {
            'home-features-after': () => h(HomeDiscover),
        }),
    enhanceApp({ app }) {
        app.component('PackageGrid', PackageGrid)
        app.component('PackageDetail', PackageDetail)
        app.component('PackageBrowse', PackageBrowse)
        app.component('ProfilePage', ProfilePage)
    },
}
