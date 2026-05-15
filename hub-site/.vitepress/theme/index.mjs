import DefaultTheme from 'vitepress/theme'
import { h } from 'vue'
import HomeDiscover from './HomeDiscover.vue'
import PackageGrid from './PackageGrid.vue'
import PackageDetail from './PackageDetail.vue'
import PackageBrowse from './PackageBrowse.vue'
import ProfilePage from './ProfilePage.vue'
import DiscoverByType from './DiscoverByType.vue'
import DiscoverAlignedWith from './DiscoverAlignedWith.vue'
import DiscoverIndex from './DiscoverIndex.vue'
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
        app.component('DiscoverByType', DiscoverByType)
        app.component('DiscoverAlignedWith', DiscoverAlignedWith)
        app.component('DiscoverIndex', DiscoverIndex)
    },
}
