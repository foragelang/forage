//! Thin adapter that presents `forage-wasm` exports under the shape
//! `forage-ts` already uses, so the Vue web-IDE can swap one import
//! line and stay working.
//!
//! Once we're confident the wasm core matches feature-for-feature with
//! `forage-ts`, this adapter can collapse and `RecipeIDE.vue` can call
//! `forage-wasm` directly.

import init, {
    parse_recipe as wasmParseRecipe,
    parse_and_validate as wasmParseAndValidate,
    validate_recipe as wasmValidateRecipe,
    forage_version,
} from "./pkg/forage_wasm.js";

let initialized: Promise<void> | null = null;
function ensureInit(): Promise<void> {
    if (!initialized) {
        initialized = init().then(() => undefined);
    }
    return initialized;
}

export type Span = { start: number; end: number };

export type ParseError = {
    message: string;
    loc: { line: number; column: number };
    span?: Span;
};

export type ValidationIssue = {
    code: string;
    message: string;
    severity: "error" | "warning";
};

export type ParseOutcome =
    | { ok: true; recipe: unknown }
    | { ok: false; error: ParseError };

function offsetToLoc(source: string, offset: number): { line: number; column: number } {
    let line = 1;
    let lineStart = 0;
    for (let i = 0; i < Math.min(offset, source.length); i++) {
        if (source.charCodeAt(i) === 10) {
            line += 1;
            lineStart = i + 1;
        }
    }
    return { line, column: offset - lineStart + 1 };
}

export const Parser = {
    async parse(source: string): Promise<unknown> {
        await ensureInit();
        const out = wasmParseRecipe(source) as ParseOutcome;
        if (out.ok) return (out as { ok: true; recipe: unknown }).recipe;
        const err = (out as { ok: false; error: ParseError }).error;
        // Hydrate `loc` from span on the wasm side.
        const span = (err as ParseError & { span?: Span }).span;
        if (span && !err.loc) {
            err.loc = offsetToLoc(source, span.start);
        }
        throw err;
    },
};

export async function validate(recipe: unknown): Promise<ValidationIssue[]> {
    await ensureInit();
    const out = wasmValidateRecipe(JSON.stringify(recipe)) as {
        errors: { code: string; message: string }[];
        warnings: { code: string; message: string }[];
    };
    const issues: ValidationIssue[] = [];
    for (const e of out.errors) issues.push({ ...e, severity: "error" });
    for (const w of out.warnings) issues.push({ ...w, severity: "warning" });
    return issues;
}

export async function parseAndValidate(source: string): Promise<{
    recipe: unknown | null;
    issues: ValidationIssue[];
    parseError: ParseError | null;
}> {
    await ensureInit();
    const out = wasmParseAndValidate(source) as
        | { ok: true; issues: ValidationIssue[]; recipe: unknown }
        | { ok: false; error: ParseError };
    if (out.ok) {
        return { recipe: out.recipe, issues: out.issues, parseError: null };
    }
    const err = out.error;
    const span = (err as ParseError & { span?: Span }).span;
    if (span && !err.loc) {
        err.loc = offsetToLoc(source, span.start);
    }
    return { recipe: null, issues: [], parseError: err };
}

export async function forageVersion(): Promise<string> {
    await ensureInit();
    return forage_version();
}
