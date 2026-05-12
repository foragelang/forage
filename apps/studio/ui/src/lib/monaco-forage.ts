//! Register the Forage language with Monaco — syntax highlighting +
//! basic completion. The "real" LSP (forage-lsp) wires into Monaco
//! via the languageclient bridge in `lsp.ts`; this file gives us
//! useful editing immediately even if the LSP child process isn't
//! running.

import type * as Monaco from "monaco-editor";

const KEYWORDS = [
    "recipe", "engine", "http", "browser", "type", "enum", "input",
    "secret", "step", "method", "url", "headers", "body", "json", "form",
    "raw", "auth", "staticHeader", "htmlPrime", "session", "formLogin",
    "bearerLogin", "cookiePersist", "extract", "regex", "groups",
    "paginate", "pageWithTotal", "untilEmpty", "cursor", "for", "in",
    "emit", "case", "of", "expect", "observe", "browserPaginate",
    "scroll", "ageGate", "autoFill", "captures", "match", "document",
    "interactive", "bootstrapURL", "cookieDomains", "sessionExpiredPattern",
    "items", "total", "pageParam", "pageSize", "cursorPath", "cursorParam",
    "import", "as", "records", "count", "typeName", "noProgressFor",
    "maxIterations", "iterationDelay", "dob", "reloadAfter",
    "captureCookies", "maxReauthRetries", "cache", "cacheEncrypted",
    "requiresMFA", "mfaFieldName", "tokenPath", "headerName",
    "headerPrefix", "name", "value", "stepName", "nonceVar", "ajaxUrlVar",
    "warmupClicks", "dismissals", "extraLabels", "initialURL",
    "urlPattern", "iterPath", "seedFilter", "where",
];

const TYPES = ["String", "Int", "Double", "Bool"];

const BUILTIN_TRANSFORMS = [
    "toString", "lower", "upper", "trim", "capitalize", "titleCase",
    "parseInt", "parseFloat", "parseBool", "length", "dedup", "first",
    "coalesce", "default", "parseSize", "normalizeOzToGrams", "sizeValue",
    "sizeUnit", "normalizeUnitToGrams", "prevalenceNormalize",
    "parseJaneWeight", "janeWeightUnit", "janeWeightKey", "getField",
    "parseHtml", "parseJson", "select", "text", "attr", "html", "innerHtml",
];

export const FORAGE_LANG_ID = "forage";

export function registerForageLanguage(monaco: typeof Monaco) {
    if (monaco.languages.getLanguages().some((l) => l.id === FORAGE_LANG_ID)) {
        return;
    }

    monaco.languages.register({ id: FORAGE_LANG_ID, extensions: [".forage"] });

    monaco.languages.setMonarchTokensProvider(FORAGE_LANG_ID, {
        defaultToken: "",
        tokenPostfix: ".forage",
        keywords: KEYWORDS,
        typeKeywords: TYPES,
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

    // Static completion provider — keywords + transforms + primitives.
    // The LSP layer adds per-document symbols (inputs, types, etc.) on top.
    monaco.languages.registerCompletionItemProvider(FORAGE_LANG_ID, {
        provideCompletionItems(model, position) {
            const word = model.getWordUntilPosition(position);
            const range = {
                startLineNumber: position.lineNumber,
                endLineNumber: position.lineNumber,
                startColumn: word.startColumn,
                endColumn: word.endColumn,
            };
            const items: Monaco.languages.CompletionItem[] = [];
            for (const k of KEYWORDS) {
                items.push({
                    label: k,
                    kind: monaco.languages.CompletionItemKind.Keyword,
                    insertText: k,
                    range,
                });
            }
            for (const t of TYPES) {
                items.push({
                    label: t,
                    kind: monaco.languages.CompletionItemKind.Class,
                    insertText: t,
                    range,
                });
            }
            for (const t of BUILTIN_TRANSFORMS) {
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
    });
}
