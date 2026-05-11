<script setup>
import { ref, computed, onMounted, onBeforeUnmount, watch, shallowRef } from 'vue'
import {
    Parser,
    ParseError,
    validate,
    run as runRecipe,
    HubClient,
    DEFAULT_HUB_API,
} from '../../../forage-ts/src/index.ts'

const props = defineProps({
    slug: { type: String, default: '' },
    initialSource: { type: String, default: '' },
    apiBase: { type: String, default: DEFAULT_HUB_API },
})

const blankTemplate = `recipe "my-recipe" {
    engine http

    type Item {
        id: String
        name: String
    }

    step list {
        method "GET"
        url "https://api.example.com/items"
    }

    for $item in $list.items[*] {
        emit Item {
            id   ← $item.id | toString
            name ← $item.name
        }
    }

    expect { records.where(typeName == "Item").count >= 1 }
}
`

const source = ref(props.initialSource || blankTemplate)
const loadingRemote = ref(false)
const loadError = ref('')

const editorContainer = ref(null)
let editor = null
let monaco = null

// Validation state — recomputed on every (debounced) source change.
const parseError = ref(null)
const validationIssues = ref([])
const parsedRecipe = shallowRef(null)
const parsedInputs = computed(() => parsedRecipe.value?.inputs ?? [])

// Tabs
const activeTab = ref('validation')

// Run state
const runInputs = ref({})
const runResult = ref(null)
const running = ref(false)

// Publish state
const publish = ref({
    slug: props.slug || '',
    displayName: '',
    summary: '',
    tags: '',
    author: '',
    license: 'MIT',
})
const apiToken = ref(typeof localStorage !== 'undefined' ? (localStorage.getItem('forage_hub_token') || '') : '')
const rememberToken = ref(typeof localStorage !== 'undefined' ? !!localStorage.getItem('forage_hub_token') : false)
const publishPreview = ref('')
const publishStatus = ref('')

watch(apiToken, v => {
    if (typeof localStorage === 'undefined') return
    if (rememberToken.value && v) localStorage.setItem('forage_hub_token', v)
    else localStorage.removeItem('forage_hub_token')
})
watch(rememberToken, on => {
    if (typeof localStorage === 'undefined') return
    if (on && apiToken.value) localStorage.setItem('forage_hub_token', apiToken.value)
    else localStorage.removeItem('forage_hub_token')
})

// Snapshot diff state
const expectedSnapshot = ref(null)
const snapshotDiff = ref('')

let debounceHandle = null

async function fetchRemoteRecipe() {
    if (!props.slug) return
    loadingRemote.value = true
    try {
        const client = new HubClient({ base: props.apiBase })
        const detail = await client.get(props.slug)
        source.value = detail.body
        publish.value.slug = detail.slug
        publish.value.displayName = detail.displayName || detail.slug
        publish.value.summary = detail.summary || ''
        publish.value.tags = (detail.tags || []).join(', ')
        publish.value.author = detail.author || ''
        // Try snapshot too.
        try {
            const r = await fetch(`${props.apiBase}/v1/recipes/${encodeURIComponent(props.slug)}/snapshot`)
            if (r.ok) expectedSnapshot.value = await r.json()
        } catch {}
    } catch (e) {
        loadError.value = e?.message || String(e)
    } finally {
        loadingRemote.value = false
    }
}

function validateNow() {
    parseError.value = null
    validationIssues.value = []
    parsedRecipe.value = null
    try {
        const recipe = Parser.parse(source.value)
        parsedRecipe.value = recipe
        validationIssues.value = validate(recipe)
        // Pre-seed inputs map for the Run tab.
        const seeded = {}
        for (const i of recipe.inputs) {
            seeded[i.name] = runInputs.value[i.name] ?? ''
        }
        runInputs.value = seeded
    } catch (e) {
        if (e instanceof ParseError) {
            parseError.value = { message: e.message, loc: e.loc }
        } else {
            parseError.value = { message: e?.message || String(e), loc: { line: 1, column: 1 } }
        }
    }
    applyEditorMarkers()
}

function applyEditorMarkers() {
    if (!editor || !monaco) return
    const model = editor.getModel()
    if (!model) return
    const markers = []
    if (parseError.value) {
        const loc = parseError.value.loc || { line: 1, column: 1 }
        markers.push({
            severity: monaco.MarkerSeverity.Error,
            message: parseError.value.message,
            startLineNumber: loc.line,
            startColumn: loc.column,
            endLineNumber: loc.line,
            endColumn: loc.column + 1,
        })
    }
    for (const issue of validationIssues.value) {
        markers.push({
            severity: issue.severity === 'error' ? monaco.MarkerSeverity.Error : monaco.MarkerSeverity.Warning,
            message: `${issue.message} [${issue.location}]`,
            startLineNumber: 1,
            startColumn: 1,
            endLineNumber: 1,
            endColumn: 2,
        })
    }
    monaco.editor.setModelMarkers(model, 'forage', markers)
}

function scheduleValidation() {
    if (debounceHandle) clearTimeout(debounceHandle)
    debounceHandle = setTimeout(validateNow, 250)
}

function registerForageLanguage(m) {
    if (m.languages.getLanguages().some(l => l.id === 'forage')) return
    m.languages.register({ id: 'forage', extensions: ['.forage'], aliases: ['Forage', 'forage'] })
    m.languages.setLanguageConfiguration('forage', {
        comments: { lineComment: '//', blockComment: ['/*', '*/'] },
        brackets: [['{', '}'], ['[', ']'], ['(', ')']],
        autoClosingPairs: [
            { open: '{', close: '}' },
            { open: '[', close: ']' },
            { open: '(', close: ')' },
            { open: '"', close: '"' },
        ],
        surroundingPairs: [
            { open: '{', close: '}' },
            { open: '[', close: ']' },
            { open: '(', close: ')' },
            { open: '"', close: '"' },
        ],
    })
    m.languages.setMonarchTokensProvider('forage', {
        defaultToken: '',
        tokenPostfix: '.forage',
        keywords: [
            'recipe', 'engine', 'http', 'browser', 'type', 'enum', 'input',
            'step', 'method', 'url', 'headers', 'body', 'json', 'form', 'raw',
            'auth', 'staticHeader', 'htmlPrime',
            'paginate', 'pageWithTotal', 'untilEmpty', 'cursor',
            'items', 'total', 'pageParam', 'pageSize', 'cursorPath', 'cursorParam',
            'for', 'in', 'emit', 'case', 'of', 'where', 'expect',
            'records', 'count', 'typeName',
            'true', 'false', 'null', 'import',
        ],
        primitiveTypes: ['String', 'Int', 'Double', 'Bool'],
        operators: ['←', '<-', '→', '->', '|', '?.', '?', '==', '!=', '>=', '<=', '=', '>', '<', '!'],
        symbols: /[=><!~?:&|+\-*/^%]+/,
        tokenizer: {
            root: [
                [/\/\/.*$/, 'comment'],
                [/\/\*/, 'comment', '@block_comment'],
                [/"([^"\\]|\\.)*"/, 'string'],
                [/\$[a-zA-Z_][a-zA-Z0-9_]*/, 'variable'],
                [/\$/, 'variable'],
                [/\b[0-9]+\.[0-9]+\b/, 'number.float'],
                [/\b[0-9]{4}-[0-9]{2}-[0-9]{2}\b/, 'number'],
                [/\b[0-9]+\b/, 'number'],
                [/hub:\/\/[A-Za-z0-9_\-/]+/, 'string.special'],
                [/\b[A-Z][A-Za-z0-9_]*\b/, {
                    cases: {
                        '@primitiveTypes': 'type',
                        '@default': 'type.identifier',
                    },
                }],
                [/\b[a-zA-Z_][a-zA-Z0-9_]*\b/, {
                    cases: {
                        '@keywords': 'keyword',
                        '@default': 'identifier',
                    },
                }],
                [/[{}()[\]]/, '@brackets'],
                [/←|→/, 'operator'],
                [/@symbols/, {
                    cases: {
                        '@operators': 'operator',
                        '@default': '',
                    },
                }],
            ],
            block_comment: [
                [/[^/*]+/, 'comment'],
                [/\*\//, 'comment', '@pop'],
                [/[/*]/, 'comment'],
            ],
        },
    })
}

onMounted(async () => {
    // Monaco is dynamically imported so the rest of the hub site stays lean.
    const m = await import('monaco-editor')
    monaco = m
    registerForageLanguage(m)

    editor = m.editor.create(editorContainer.value, {
        value: source.value,
        language: 'forage',
        theme: 'vs-dark',
        automaticLayout: true,
        minimap: { enabled: false },
        fontSize: 14,
        scrollBeyondLastLine: false,
        wordWrap: 'on',
    })
    editor.onDidChangeModelContent(() => {
        source.value = editor.getValue()
        scheduleValidation()
    })

    // First validation pass + optional remote fetch.
    validateNow()
    if (props.slug) {
        await fetchRemoteRecipe()
        if (editor && source.value !== editor.getValue()) {
            editor.setValue(source.value)
        }
        validateNow()
    }
})

onBeforeUnmount(() => {
    if (debounceHandle) clearTimeout(debounceHandle)
    if (editor) editor.dispose()
})

// ---- Run tab ----

async function runRecipeAgainstFixtures() {
    if (!parsedRecipe.value) return
    if (parsedRecipe.value.engineKind !== 'http') {
        runResult.value = {
            diagnostic: {
                stallReason: 'Browser-engine recipes can only run in the Forage Toolkit. Open this recipe in the Toolkit to run it locally.',
                unmetExpectations: [],
            },
            records: [],
        }
        return
    }
    running.value = true
    runResult.value = null
    snapshotDiff.value = ''
    try {
        const inputs = {}
        for (const [k, v] of Object.entries(runInputs.value)) {
            // Heuristic input coercion: numeric strings become numbers; arrays
            // typed as `[X]` need user to enter JSON. We treat the input field
            // as a JSON-when-possible coercion so users can paste objects too.
            if (v === '' || v === undefined || v === null) continue
            try { inputs[k] = JSON.parse(v) } catch { inputs[k] = v }
        }
        const result = await runRecipe(parsedRecipe.value, inputs)
        runResult.value = result
        if (expectedSnapshot.value) {
            snapshotDiff.value = diffSnapshots(expectedSnapshot.value, result)
        }
    } catch (e) {
        runResult.value = {
            diagnostic: { stallReason: `failed: ${e?.message || String(e)}`, unmetExpectations: [] },
            records: [],
        }
    } finally {
        running.value = false
    }
}

function diffSnapshots(expected, produced) {
    const expectedCounts = countByType(expected.records || [])
    const producedCounts = countByType(produced.records || [])
    const allTypes = new Set([...Object.keys(expectedCounts), ...Object.keys(producedCounts)])
    const lines = []
    for (const type of allTypes) {
        const e = expectedCounts[type] || 0
        const p = producedCounts[type] || 0
        const marker = e === p ? '=' : (p > e ? '+' : '-')
        lines.push(`${marker} ${type}: expected ${e}, produced ${p}`)
    }
    return lines.join('\n')
}

function countByType(records) {
    const out = {}
    for (const r of records) out[r.typeName] = (out[r.typeName] || 0) + 1
    return out
}

// ---- Publish tab ----

function buildPayload() {
    return {
        slug: publish.value.slug.trim(),
        displayName: publish.value.displayName.trim() || publish.value.slug.trim(),
        summary: publish.value.summary.trim(),
        author: publish.value.author.trim() || undefined,
        tags: publish.value.tags.split(',').map(s => s.trim()).filter(Boolean),
        license: publish.value.license,
        body: source.value,
    }
}

function previewPublishPayload() {
    const payload = buildPayload()
    publishPreview.value = JSON.stringify(payload, null, 2)
}

async function publishRecipe() {
    publishStatus.value = ''
    if (parseError.value) {
        publishStatus.value = 'Cannot publish: source has parse errors.'
        return
    }
    if (validationIssues.value.some(i => i.severity === 'error')) {
        publishStatus.value = 'Cannot publish: validation errors. Fix them first.'
        return
    }
    if (!apiToken.value) {
        publishStatus.value = 'Cannot publish: no API token. Paste one in the field below.'
        return
    }
    const payload = buildPayload()
    if (!payload.slug) {
        publishStatus.value = 'Slug is required.'
        return
    }
    try {
        const client = new HubClient({ base: props.apiBase, token: apiToken.value })
        const result = await client.publish(payload)
        publishStatus.value = `Published ${result.slug} v${result.version}.`
    } catch (e) {
        publishStatus.value = `Failed: ${e?.message || String(e)}`
    }
}

const errorIssues = computed(() => validationIssues.value.filter(i => i.severity === 'error'))
const warningIssues = computed(() => validationIssues.value.filter(i => i.severity === 'warning'))
const isBrowserRecipe = computed(() => parsedRecipe.value?.engineKind === 'browser')
const toolkitUrl = computed(() => `forage-toolkit://recipe/${encodeURIComponent(props.slug || publish.value.slug || 'new')}`)

function inputTypeLabel(i) {
    const t = i.type
    if (!t) return ''
    const base = (() => {
        switch (t.tag) {
            case 'string': return 'String'
            case 'int': return 'Int'
            case 'double': return 'Double'
            case 'bool': return 'Bool'
            case 'array': return '[…]'
            case 'record': return t.name
            case 'enumRef': return t.name
            default: return ''
        }
    })()
    return i.optional ? `${base}?` : base
}
</script>

<template>
    <div class="ide-shell">
        <div class="ide-editor">
            <div ref="editorContainer" class="ide-monaco"></div>
        </div>
        <div class="ide-panel">
            <div class="ide-tabs">
                <button :class="{ active: activeTab === 'validation' }" @click="activeTab = 'validation'">
                    Validation
                    <span v-if="errorIssues.length > 0" class="ide-badge ide-badge-error">{{ errorIssues.length }}</span>
                    <span v-else-if="warningIssues.length > 0" class="ide-badge ide-badge-warn">{{ warningIssues.length }}</span>
                </button>
                <button :class="{ active: activeTab === 'run' }" @click="activeTab = 'run'">Run</button>
                <button :class="{ active: activeTab === 'publish' }" @click="activeTab = 'publish'">Publish</button>
            </div>

            <section v-show="activeTab === 'validation'" class="ide-section">
                <p v-if="loadingRemote" class="ide-muted">Loading recipe…</p>
                <p v-if="loadError" class="ide-error">{{ loadError }}</p>
                <div v-if="parseError" class="ide-error">
                    <strong>Parse error</strong>
                    <div class="ide-error-msg">{{ parseError.message }}</div>
                </div>
                <div v-else-if="errorIssues.length === 0 && warningIssues.length === 0">
                    <p class="ide-ok">No issues. {{ parsedRecipe?.types.length || 0 }} types, {{ parsedRecipe?.body.length || 0 }} top-level statements.</p>
                </div>
                <ul v-else class="ide-issues">
                    <li v-for="(i, ix) in errorIssues" :key="`e${ix}`" class="ide-error">
                        <span class="ide-issue-label">error</span>
                        <span>{{ i.message }}</span>
                        <span class="ide-issue-loc">[{{ i.location }}]</span>
                    </li>
                    <li v-for="(i, ix) in warningIssues" :key="`w${ix}`" class="ide-warn">
                        <span class="ide-issue-label">warn</span>
                        <span>{{ i.message }}</span>
                        <span class="ide-issue-loc">[{{ i.location }}]</span>
                    </li>
                </ul>
            </section>

            <section v-show="activeTab === 'run'" class="ide-section">
                <div v-if="isBrowserRecipe" class="ide-info">
                    This recipe uses the browser engine. The web IDE can only run HTTP-engine recipes; open it in the Toolkit to run locally.
                    <p><a :href="toolkitUrl">Open in Toolkit</a></p>
                </div>
                <div v-else>
                    <p v-if="parsedInputs.length === 0" class="ide-muted">No inputs declared.</p>
                    <div v-else class="ide-inputs">
                        <label v-for="i in parsedInputs" :key="i.name" class="ide-input">
                            <span>{{ i.name }} <em>{{ inputTypeLabel(i) }}</em></span>
                            <input v-model="runInputs[i.name]" :placeholder="i.name" />
                        </label>
                    </div>
                    <button class="ide-button" :disabled="!!parseError || running" @click="runRecipeAgainstFixtures">
                        {{ running ? 'Running…' : 'Run against fixtures' }}
                    </button>
                    <div v-if="runResult" class="ide-result">
                        <div class="ide-result-summary">
                            <strong>{{ runResult.diagnostic.stallReason }}</strong>
                            <span class="ide-muted">— {{ runResult.records.length }} records emitted</span>
                        </div>
                        <ul v-if="runResult.diagnostic.unmetExpectations.length > 0" class="ide-issues">
                            <li v-for="(m, ix) in runResult.diagnostic.unmetExpectations" :key="ix" class="ide-warn">
                                {{ m }}
                            </li>
                        </ul>
                        <pre v-if="snapshotDiff" class="ide-diff">{{ snapshotDiff }}</pre>
                        <details v-if="runResult.records.length > 0">
                            <summary>Records ({{ runResult.records.length }})</summary>
                            <pre class="ide-records">{{ JSON.stringify(runResult.records.slice(0, 20), null, 2) }}{{ runResult.records.length > 20 ? '\n…' : '' }}</pre>
                        </details>
                    </div>
                </div>
            </section>

            <section v-show="activeTab === 'publish'" class="ide-section">
                <div class="ide-form">
                    <label><span>Slug</span><input v-model="publish.slug" placeholder="my-recipe" /></label>
                    <label><span>Display name</span><input v-model="publish.displayName" placeholder="My Recipe" /></label>
                    <label><span>Summary</span><textarea v-model="publish.summary" rows="2" placeholder="One-line description"></textarea></label>
                    <label><span>Tags (comma-separated)</span><input v-model="publish.tags" placeholder="example, demo" /></label>
                    <label><span>Author</span><input v-model="publish.author" placeholder="alice" /></label>
                    <label><span>License</span><input v-model="publish.license" placeholder="MIT" /></label>
                </div>
                <div class="ide-token-row">
                    <label class="ide-token">
                        <span>API token</span>
                        <input :type="rememberToken ? 'text' : 'password'" v-model="apiToken" placeholder="Bearer token" />
                    </label>
                    <label class="ide-remember">
                        <input type="checkbox" v-model="rememberToken" /> remember
                    </label>
                </div>
                <div class="ide-button-row">
                    <button class="ide-button" @click="previewPublishPayload">Preview payload</button>
                    <button class="ide-button ide-button-primary" :disabled="!!parseError" @click="publishRecipe">Publish</button>
                </div>
                <p v-if="publishStatus" :class="publishStatus.startsWith('Published') ? 'ide-ok' : 'ide-warn'">
                    {{ publishStatus }}
                </p>
                <pre v-if="publishPreview" class="ide-preview">{{ publishPreview }}</pre>
            </section>
        </div>
    </div>
</template>

<style scoped>
.ide-shell {
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) minmax(360px, 1fr);
    gap: 12px;
    height: calc(100vh - 96px);
    min-height: 500px;
}
.ide-editor {
    border: 1px solid var(--vp-c-divider);
    border-radius: 8px;
    overflow: hidden;
    min-height: 0;
}
.ide-monaco { width: 100%; height: 100%; }
.ide-panel {
    border: 1px solid var(--vp-c-divider);
    border-radius: 8px;
    background: var(--vp-c-bg-soft);
    overflow: auto;
    min-width: 0;
}
.ide-tabs {
    display: flex;
    border-bottom: 1px solid var(--vp-c-divider);
    position: sticky;
    top: 0;
    background: var(--vp-c-bg-soft);
    z-index: 1;
}
.ide-tabs button {
    flex: 1;
    padding: 10px 12px;
    background: none;
    border: none;
    color: var(--vp-c-text-2);
    font: inherit;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
}
.ide-tabs button.active {
    color: var(--vp-c-text-1);
    border-bottom-color: var(--vp-c-brand-1);
    font-weight: 600;
}
.ide-badge {
    display: inline-block;
    padding: 1px 6px;
    border-radius: 999px;
    font-size: 11px;
}
.ide-badge-error { background: #dc2626; color: white; }
.ide-badge-warn  { background: #d97706; color: white; }
.ide-section {
    padding: 14px 16px;
}
.ide-section p { margin: 0 0 8px; }
.ide-issues {
    list-style: none;
    padding: 0;
    margin: 0;
    font-size: 13px;
}
.ide-issues li {
    padding: 8px 10px;
    border-radius: 6px;
    background: rgba(0,0,0,0.04);
    margin-bottom: 6px;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: baseline;
}
:root.dark .ide-issues li { background: rgba(255,255,255,0.04); }
.ide-issue-label {
    text-transform: uppercase;
    font-size: 10px;
    font-weight: 700;
    padding: 1px 6px;
    border-radius: 3px;
    background: rgba(0,0,0,0.1);
}
.ide-issue-loc {
    font-family: var(--vp-font-family-mono);
    font-size: 11px;
    color: var(--vp-c-text-3);
    margin-left: auto;
}
.ide-error { color: #dc2626; }
.ide-warn  { color: #d97706; }
.ide-ok    { color: #16a34a; }
.ide-muted { color: var(--vp-c-text-3); }
.ide-info {
    padding: 10px;
    border-radius: 6px;
    background: var(--vp-c-bg-alt);
    border: 1px solid var(--vp-c-divider);
}
.ide-inputs {
    display: grid;
    gap: 8px;
    margin-bottom: 12px;
}
.ide-input, .ide-form label, .ide-token {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
}
.ide-input span em {
    color: var(--vp-c-text-3);
    font-style: normal;
    margin-left: 6px;
}
.ide-form {
    display: grid;
    gap: 10px;
    margin-bottom: 12px;
}
.ide-form input, .ide-form textarea, .ide-input input, .ide-token input {
    padding: 6px 8px;
    border: 1px solid var(--vp-c-divider);
    border-radius: 6px;
    background: var(--vp-c-bg);
    color: var(--vp-c-text-1);
    font: inherit;
}
.ide-token-row {
    display: flex;
    gap: 10px;
    align-items: flex-end;
    margin-bottom: 12px;
}
.ide-token { flex: 1; }
.ide-remember { font-size: 12px; padding-bottom: 6px; }
.ide-button {
    padding: 6px 12px;
    border-radius: 6px;
    border: 1px solid var(--vp-c-divider);
    background: var(--vp-c-bg);
    color: var(--vp-c-text-1);
    font: inherit;
    cursor: pointer;
}
.ide-button:hover { border-color: var(--vp-c-brand-1); }
.ide-button:disabled { opacity: 0.5; cursor: not-allowed; }
.ide-button-primary {
    background: var(--vp-c-brand-1);
    color: white;
    border-color: var(--vp-c-brand-1);
}
.ide-button-row {
    display: flex;
    gap: 8px;
    margin-bottom: 8px;
}
.ide-result, .ide-preview, .ide-records, .ide-diff {
    margin-top: 10px;
}
.ide-result-summary {
    font-size: 13px;
    margin-bottom: 6px;
}
.ide-preview, .ide-records, .ide-diff {
    background: var(--vp-c-bg-alt);
    border: 1px solid var(--vp-c-divider);
    border-radius: 6px;
    padding: 8px 10px;
    font-size: 12px;
    font-family: var(--vp-font-family-mono);
    overflow: auto;
    max-height: 320px;
}
.ide-error-msg {
    margin-top: 4px;
    font-family: var(--vp-font-family-mono);
    font-size: 12px;
    white-space: pre-wrap;
}
@media (max-width: 880px) {
    .ide-shell {
        grid-template-columns: 1fr;
        height: auto;
    }
    .ide-editor { min-height: 360px; }
}
</style>
