//! Adapter — the single TS surface the Vue IDE consumes. Wraps the
//! `forage-wasm` exports with the shape the IDE needs (parse, validate,
//! parse+validate, version) plus the pure-JS hub-API client. The Rust
//! core compiled to WebAssembly is the only implementation of the
//! recipe language.

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

export class ParseError extends Error {
    loc: { line: number; column: number };
    span?: Span;
    constructor(message: string, loc: { line: number; column: number }, span?: Span) {
        super(message);
        this.name = "ParseError";
        this.loc = loc;
        this.span = span;
    }
}

export type ValidationIssue = {
    code: string;
    message: string;
    severity: "error" | "warning";
    location?: string;
};

type ParseOutcome =
    | { ok: true; recipe: unknown }
    | { ok: false; error: { message: string; span?: Span; loc?: { line: number; column: number } } };

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

function toParseError(
    err: { message: string; span?: Span; loc?: { line: number; column: number } },
    source: string,
): ParseError {
    const loc = err.loc
        ?? (err.span ? offsetToLoc(source, err.span.start) : { line: 1, column: 1 });
    return new ParseError(err.message, loc, err.span);
}

/// `Parser.parse(src)` — parses a recipe source. Throws `ParseError`
/// on failure. The class form (rather than a plain function) matches
/// the IDE's expectations and keeps `instanceof ParseError` working
/// for the catch block in `RecipeIDE.vue`.
export const Parser = {
    async parse(source: string): Promise<unknown> {
        await ensureInit();
        const out = wasmParseRecipe(source) as ParseOutcome;
        if (out.ok) return out.recipe;
        throw toParseError(out.error, source);
    },
};

export async function validate(recipe: unknown): Promise<ValidationIssue[]> {
    await ensureInit();
    const out = wasmValidateRecipe(JSON.stringify(recipe)) as {
        errors: { code: string; message: string }[];
        warnings: { code: string; message: string }[];
    };
    const issues: ValidationIssue[] = [];
    for (const e of out.errors) {
        issues.push({ ...e, severity: "error" });
    }
    for (const w of out.warnings) {
        issues.push({ ...w, severity: "warning" });
    }
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
        | { ok: false; error: { message: string; span?: Span } };
    if (out.ok) {
        return { recipe: out.recipe, issues: out.issues, parseError: null };
    }
    return { recipe: null, issues: [], parseError: toParseError(out.error, source) };
}

export async function forageVersion(): Promise<string> {
    await ensureInit();
    return forage_version();
}

/// The IDE used to run recipes against live HTTP through the TS port's
/// browser-fetch runner. The Rust HTTP engine lives in `forage-http`,
/// which doesn't compile to WASM (reqwest, tokio multi-thread, native
/// keychain), so the in-browser run path is gone. Studio remains the
/// supported way to execute a recipe end-to-end; this stub keeps the
/// IDE compiling and reports the situation to the user.
export type RunResult = {
    records: { typeName: string; fields: Record<string, unknown> }[];
    diagnostic: { stallReason: string; unmetExpectations: string[] };
};

export async function run(_recipe: unknown, _inputs: Record<string, unknown>): Promise<RunResult> {
    return {
        records: [],
        diagnostic: {
            stallReason:
                "Running recipes in the hub IDE is currently disabled. Open the recipe in Forage Studio to execute it locally.",
            unmetExpectations: [],
        },
    };
}

// ---- Hub API client ------------------------------------------------------
//
// Pure-JS client for the Forage hub API. Sits next to the wasm adapter
// so the IDE has a single import line for everything it consumes.

export const DEFAULT_HUB_API = "https://api.foragelang.com";

export interface RecipeListItem {
    slug: string;
    displayName: string;
    summary: string;
    author?: string;
    platform?: string;
    tags?: string[];
    version?: number;
    sha256?: string;
    createdAt?: string;
    updatedAt?: string;
}

export interface RecipeDetail extends RecipeListItem {
    body: string;
}

export interface PublishPayload {
    slug: string;
    displayName: string;
    summary: string;
    tags: string[];
    body: string;
    author?: string;
    platform?: string;
    license?: string;
    fixtures?: string;
    snapshot?: unknown;
}

export interface PublishResult {
    slug: string;
    version: number;
    sha256?: string;
    publishedAt?: string;
}

export interface HubClientOptions {
    base?: string;
    token?: string;
    fetch?: typeof fetch;
    /// When true, include credentials (cookies) on every request. Used
    /// by the web IDE so the httpOnly `forage_at` cookie from the
    /// OAuth flow authenticates publish/delete without an explicit
    /// Bearer token.
    useCredentials?: boolean;
}

export class HubClient {
    private readonly base: string;
    private readonly token: string | null;
    private readonly fetchImpl: typeof fetch;
    private readonly useCredentials: boolean;

    constructor(opts: HubClientOptions = {}) {
        this.base = opts.base ?? DEFAULT_HUB_API;
        this.token = opts.token ?? null;
        this.fetchImpl = opts.fetch ?? globalThis.fetch.bind(globalThis);
        this.useCredentials = opts.useCredentials ?? false;
    }

    private fetchInit(extra: RequestInit = {}): RequestInit {
        const init: RequestInit = { ...extra };
        if (this.useCredentials) init.credentials = "include";
        return init;
    }

    async list(): Promise<RecipeListItem[]> {
        const r = await this.fetchImpl(`${this.base}/v1/packages?limit=100`, this.fetchInit());
        if (!r.ok) throw new Error(`HTTP ${r.status} on GET /v1/packages`);
        const data = await r.json();
        return Array.isArray(data.items) ? data.items : [];
    }

    async get(slug: string, version?: number): Promise<RecipeDetail> {
        const path = encodeSlugPath(slug);
        const url = `${this.base}/v1/packages/${path}${version ? `?version=${version}` : ""}`;
        const r = await this.fetchImpl(url, this.fetchInit());
        if (!r.ok) throw new Error(`HTTP ${r.status} on GET ${url}`);
        return await r.json();
    }

    async publish(payload: PublishPayload): Promise<PublishResult> {
        if (!this.token && !this.useCredentials) {
            throw new Error("hub: publish requires an API token or signed-in session");
        }
        const headers: Record<string, string> = { "Content-Type": "application/json" };
        if (this.token) headers["Authorization"] = `Bearer ${this.token}`;
        const r = await this.fetchImpl(
            `${this.base}/v1/packages`,
            this.fetchInit({
                method: "POST",
                headers,
                body: JSON.stringify(payload),
            }),
        );
        if (!r.ok) {
            const text = await r.text();
            throw new Error(`HTTP ${r.status} on POST /v1/packages: ${text}`);
        }
        return await r.json();
    }

    async whoami(): Promise<{
        authenticated: boolean;
        user?: { login: string; name?: string; avatarUrl?: string };
    }> {
        const r = await this.fetchImpl(`${this.base}/v1/oauth/whoami`, this.fetchInit());
        if (!r.ok) return { authenticated: false };
        return await r.json();
    }

    async oauthStart(returnTo: string): Promise<{ authorizeURL: string; state: string }> {
        const r = await this.fetchImpl(
            `${this.base}/v1/oauth/start`,
            this.fetchInit({
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ returnTo }),
            }),
        );
        if (!r.ok) throw new Error(`HTTP ${r.status} on POST /v1/oauth/start`);
        return await r.json();
    }

    buildPublishRequest(payload: PublishPayload): { url: string; init: RequestInit } {
        return {
            url: `${this.base}/v1/packages`,
            init: {
                method: "POST",
                headers: {
                    Authorization: this.token ? `Bearer ${this.token}` : "",
                    "Content-Type": "application/json",
                },
                body: JSON.stringify(payload),
            },
        };
    }
}

function encodeSlugPath(slug: string): string {
    return slug.split("/").map(encodeURIComponent).join("/");
}
