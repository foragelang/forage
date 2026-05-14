import { defineConfig } from 'vitepress'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'

const __dirname = dirname(fileURLToPath(import.meta.url))

// Reuse the grammar from the foragelang.com site. Both sites share one
// definition so syntax highlighting stays in sync.
const forageGrammar = JSON.parse(
    readFileSync(
        resolve(__dirname, '../../site/.vitepress/languages/forage.tmLanguage.json'),
        'utf8',
    ),
)

export default defineConfig({
    title: 'Forage Hub',
    titleTemplate: ':title — Forage Hub',
    description: 'A registry of declarative scraping recipes.',
    lang: 'en-US',
    cleanUrls: true,
    lastUpdated: false,

    head: [
        ['link', { rel: 'icon', type: 'image/svg+xml', href: '/favicon.svg' }],
        ['meta', { name: 'theme-color', content: '#5c8a4f' }],
        ['meta', { property: 'og:type', content: 'website' }],
        ['meta', { property: 'og:site_name', content: 'Forage Hub' }],
        ['meta', { name: 'twitter:card', content: 'summary' }],
    ],

    sitemap: {
        hostname: 'https://hub.foragelang.com',
    },

    markdown: {
        languages: [
            { ...forageGrammar, aliases: ['forage'] },
        ],
        theme: {
            light: 'github-light',
            dark: 'github-dark',
        },
        lineNumbers: false,
    },

    themeConfig: {
        logo: '/favicon.svg',
        siteTitle: 'Forage Hub',

        nav: [
            { text: 'Browse', link: '/' },
            {
                text: 'Discover',
                items: [
                    { text: 'Top starred', link: '/discover/top-starred' },
                    { text: 'Most downloaded', link: '/discover/top-downloaded' },
                ],
            },
            { text: 'Publish', link: '/publish' },
            { text: 'About', link: '/about' },
            { text: 'foragelang.com', link: 'https://foragelang.com' },
        ],

        socialLinks: [
            { icon: 'github', link: 'https://github.com/foragelang/forage' },
        ],

        footer: {
            message: 'Open source',
            copyright: 'Forage — <a href="https://github.com/foragelang/forage">github.com/foragelang/forage</a>',
        },

        search: {
            provider: 'local',
        },

        outline: {
            level: [2, 3],
            label: 'On this page',
        },
    },
})
