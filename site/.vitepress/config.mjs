import { defineConfig } from 'vitepress'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'

const __dirname = dirname(fileURLToPath(import.meta.url))
const forageGrammar = JSON.parse(
    readFileSync(resolve(__dirname, 'languages/forage.tmLanguage.json'), 'utf8')
)

export default defineConfig({
    title: 'Forage',
    titleTemplate: ':title — Forage',
    description: 'A declarative DSL for web scraping. Recipes describe what to fetch; the engine runs the HTTP, pagination, and type-directed extraction.',
    lang: 'en-US',
    cleanUrls: true,
    lastUpdated: false,

    head: [
        ['link', { rel: 'icon', type: 'image/svg+xml', href: '/favicon.svg' }],
        ['meta', { name: 'theme-color', content: '#5c8a4f' }],
        ['meta', { property: 'og:type', content: 'website' }],
        ['meta', { property: 'og:site_name', content: 'Forage' }],
        ['meta', { name: 'twitter:card', content: 'summary' }],
    ],

    sitemap: {
        hostname: 'https://foragelang.com',
    },

    markdown: {
        languages: [
            { ...forageGrammar, aliases: ['forage'] }
        ],
        theme: {
            light: 'github-light',
            dark: 'github-dark',
        },
        lineNumbers: false,
    },

    themeConfig: {
        logo: '/favicon.svg',
        siteTitle: 'Forage',

        nav: [
            { text: 'Docs', link: '/docs/', activeMatch: '/docs/' },
            { text: 'GitHub', link: 'https://github.com/foragelang/forage' },
        ],

        sidebar: {
            '/docs/': [
                {
                    text: 'Docs',
                    items: [
                        { text: 'Overview', link: '/docs/' },
                        { text: 'Getting started', link: '/docs/getting-started' },
                    ],
                },
                {
                    text: 'The DSL',
                    items: [
                        { text: 'Syntax reference', link: '/docs/syntax' },
                        { text: 'Engines & pagination', link: '/docs/engines' },
                        { text: 'Expectations', link: '/docs/expectations' },
                    ],
                },
                {
                    text: 'Runtime',
                    items: [
                        { text: 'Diagnostics', link: '/docs/diagnostics' },
                        { text: 'Archive & replay', link: '/docs/archive-replay' },
                    ],
                },
                {
                    text: 'Tooling',
                    items: [
                        { text: 'CLI reference', link: '/docs/cli' },
                        { text: 'Toolkit (macOS app)', link: '/docs/toolkit' },
                    ],
                },
            ],
        },

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

        docFooter: {
            prev: 'Previous',
            next: 'Next',
        },
    },
})
