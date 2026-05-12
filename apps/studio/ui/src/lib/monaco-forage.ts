//! Register the Forage language with Monaco — syntax highlighting +
//! basic completion. The keyword / type / transform lists used to be
//! hand-maintained TS arrays in this file; they're now pulled from
//! `forage-core` at runtime via the `language_dictionary` Tauri
//! command so they can't drift from the parser / validator.
//!
//! The real LSP (forage-lsp) layers richer per-document intelligence
//! on top — symbols, hover, goto-def — when wired in. This file is the
//! always-on baseline: tokens + completion that works even before the
//! LSP process has handshook.

import type * as Monaco from "monaco-editor";

import { api } from "./api";

export const FORAGE_LANG_ID = "forage";

interface Dictionary {
    keywords: string[];
    typeKeywords: string[];
    transforms: string[];
}

/// Bootstrap dictionary used the moment the editor mounts, before the
/// Tauri `language_dictionary` round-trip resolves. Intentionally empty
/// — Monaco renders plain text for a few hundred ms while we fetch the
/// canonical lists, which is preferable to shipping a stale snapshot.
const EMPTY_DICTIONARY: Dictionary = { keywords: [], typeKeywords: [], transforms: [] };

let lastDictionary: Dictionary = EMPTY_DICTIONARY;
/// Resolves when the first dictionary fetch lands; subsequent calls to
/// `registerForageLanguage` await it so the language registers with the
/// canonical lists rather than the empty bootstrap.
let dictionaryReady: Promise<Dictionary> | null = null;

function loadDictionary(): Promise<Dictionary> {
    if (dictionaryReady) return dictionaryReady;
    dictionaryReady = api
        .languageDictionary()
        .then((d) => {
            const dict: Dictionary = {
                keywords: d.keywords,
                typeKeywords: d.type_keywords,
                transforms: d.transforms,
            };
            lastDictionary = dict;
            return dict;
        })
        .catch((e) => {
            console.warn("language_dictionary fetch failed, using empty", e);
            return EMPTY_DICTIONARY;
        });
    return dictionaryReady;
}

export async function registerForageLanguage(monaco: typeof Monaco) {
    if (monaco.languages.getLanguages().some((l) => l.id === FORAGE_LANG_ID)) {
        return;
    }

    monaco.languages.register({ id: FORAGE_LANG_ID, extensions: [".forage"] });

    // Register language config + a placeholder tokenizer immediately so
    // the editor renders bracket matching / auto-close from the moment
    // it mounts. Then fetch the canonical dictionary and re-register the
    // tokenizer + completion provider with the real lists.
    monaco.languages.setLanguageConfiguration(FORAGE_LANG_ID, {
        comments: { lineComment: "//", blockComment: ["/*", "*/"] },
        brackets: [
            ["{", "}"],
            ["[", "]"],
            ["(", ")"],
        ],
        autoClosingPairs: [
            { open: "{", close: "}" },
            { open: "[", close: "]" },
            { open: "(", close: ")" },
            { open: '"', close: '"', notIn: ["string", "comment"] },
        ],
    });

    applyDictionary(monaco, lastDictionary);
    const dict = await loadDictionary();
    applyDictionary(monaco, dict);
}

function applyDictionary(monaco: typeof Monaco, dict: Dictionary) {
    monaco.languages.setMonarchTokensProvider(FORAGE_LANG_ID, {
        defaultToken: "",
        tokenPostfix: ".forage",
        keywords: dict.keywords,
        typeKeywords: dict.typeKeywords,
        operators: ["←", "→", "|", "?.", "[*]", "=", ">", "<", "!"],
        symbols: /[=><!~?:&|+\-*\/\^%←→]+/,
        tokenizer: {
            root: [
                [/\/\/.*$/, "comment"],
                [/\/\*/, "comment", "@comment"],
                [
                    /[A-Z][\w$]*/,
                    {
                        cases: {
                            "@typeKeywords": "type",
                            "@default": "type.identifier",
                        },
                    },
                ],
                [
                    /[a-z_$][\w$]*/,
                    {
                        cases: {
                            "@keywords": "keyword",
                            "@default": "identifier",
                        },
                    },
                ],
                [/\$[a-zA-Z_$][\w$]*/, "variable.predefined"],
                [/\$/, "variable.predefined"],
                [/"([^"\\]|\\.)*"/, "string"],
                [/\d+\.\d+/, "number.float"],
                [/\d+-\d+-\d+/, "number"],
                [/\d+/, "number"],
                [/[{}()\[\]]/, "@brackets"],
                [/[,;.]/, "delimiter"],
                [/←|→|\|/, "operator"],
                [/@symbols/, "operator"],
            ],
            comment: [
                [/[^\/*]+/, "comment"],
                [/\*\//, "comment", "@pop"],
                [/[\/*]/, "comment"],
            ],
        },
    });

    // Stash the completion provider's disposer so re-applying replaces
    // rather than stacking duplicate keyword suggestions on top of the
    // previous registration.
    if (hoverDisposer) hoverDisposer.dispose();
    hoverDisposer = monaco.languages.registerHoverProvider(FORAGE_LANG_ID, {
        async provideHover(model, position) {
            // Drive hover through the same `forage_lsp::intel::hover_at`
            // the LSP uses. Source comes from the model so unsaved edits
            // are reflected; (line, col) is 0-based on the Rust side
            // while Monaco is 1-based.
            try {
                const info = await api.recipeHover(
                    model.getValue(),
                    position.lineNumber - 1,
                    position.column - 1,
                );
                if (!info) return null;
                return {
                    contents: [{ value: info.markdown, isTrusted: false }],
                };
            } catch (e) {
                console.warn("recipe_hover failed", e);
                return null;
            }
        },
    });

    if (completionDisposer) completionDisposer.dispose();
    completionDisposer = monaco.languages.registerCompletionItemProvider(
        FORAGE_LANG_ID,
        {
            provideCompletionItems(model, position) {
                const word = model.getWordUntilPosition(position);
                const range = {
                    startLineNumber: position.lineNumber,
                    endLineNumber: position.lineNumber,
                    startColumn: word.startColumn,
                    endColumn: word.endColumn,
                };
                const items: Monaco.languages.CompletionItem[] = [];
                for (const k of dict.keywords) {
                    items.push({
                        label: k,
                        kind: monaco.languages.CompletionItemKind.Keyword,
                        insertText: k,
                        range,
                    });
                }
                for (const t of dict.typeKeywords) {
                    items.push({
                        label: t,
                        kind: monaco.languages.CompletionItemKind.Class,
                        insertText: t,
                        range,
                    });
                }
                for (const t of dict.transforms) {
                    items.push({
                        label: t,
                        kind: monaco.languages.CompletionItemKind.Function,
                        detail: "transform",
                        insertText: t,
                        range,
                    });
                }
                return { suggestions: items };
            },
        },
    );
}

let completionDisposer: { dispose(): void } | null = null;
let hoverDisposer: { dispose(): void } | null = null;
