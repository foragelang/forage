//! Hub-side `StudioService` implementation. Two backends:
//!
//! - `fetch` against the public hub-api at `https://api.foragelang.com`
//!   for packages / stars / fork / publish.
//! - `forage-wasm` (the Rust core compiled to WebAssembly) for parse,
//!   validate, and recipe replay against the package's captures.
//!
//! Workspace and daemon methods throw `NotSupportedByService` — the
//! hub IDE renders the shared React UI but has no local filesystem,
//! no scheduler, and no live HTTP. The UI's `capabilities` gate hides
//! those affordances before they're reachable.

import {
    NotSupportedByService,
    StaleBaseError,
    type DebugAction,
    type DeeplinkClonePayload,
    type DeviceStart,
    type ListVersionsItem,
    type NotebookSaveOutcome,
    type PackageListing,
    type RecipeSignatureWire,
    type PackageMetadata,
    type PackageQuery,
    type PackageVersion,
    type PollOutcome,
    type PublishOutcome,
    type PublishPayload,
    type PublishPreview,
    type PublishTypePayload,
    type ServiceCapabilities,
    type StudioService,
    type SyncOutcomeWire,
    type TypeVersion,
    type Unsubscribe,
} from "@/lib/services";
import type { DaemonStatus } from "@/bindings/DaemonStatus";
import type { Diagnostic } from "@/bindings/Diagnostic";
import type { FileNode } from "@/bindings/FileNode";
import type { HoverInfo } from "@/bindings/HoverInfo";
import type { LanguageDictionary } from "@/bindings/LanguageDictionary";
import type { PausePayload } from "@/bindings/PausePayload";
import type { ProgressUnit } from "@/bindings/ProgressUnit";
import type { RecentWorkspace } from "@/bindings/RecentWorkspace";
import type { RecipeOutline } from "@/bindings/RecipeOutline";
import type { RecipeStatus } from "@/bindings/RecipeStatus";
import type { Run } from "@/bindings/Run";
import type { RunEvent } from "@/bindings/RunEvent";
import type { RunOutcome } from "@/bindings/RunOutcome";
import type { RunRecipeFlags } from "@/bindings/RunRecipeFlags";
import type { ScheduledRun } from "@/bindings/ScheduledRun";
import type { ValidationOutcome } from "@/bindings/ValidationOutcome";
import type { WorkspaceInfo } from "@/bindings/WorkspaceInfo";

import {
    language_dictionary,
    parse_and_validate,
    recipe_hover,
    recipe_outline,
    recipe_progress_unit,
    run_replay,
} from "forage-wasm";

type ListPackagesResponse = {
    items: PackageListing[];
    next_cursor: string | null;
};

type ListVersionsResponse = {
    items: ListVersionsItem[];
};

/// Currently loaded version artifact — the IDE caches the latest fetch
/// so subsequent `runRecipe` calls can replay against the same recipe
/// + types + fixtures without round-tripping to hub-api. `loadPackage`
/// from the IDE shell sets this; the methods that need it read it.
///
/// `types` mirrors `version.type_refs` resolved against the type
/// resource — the shell fetches each referenced type alongside the
/// recipe so the editor can render them in the file tree and the
/// in-browser replay can fold them into the catalog.
export type LoadedPackage = {
    author: string;
    slug: string;
    version: PackageVersion;
    types: TypeVersion[];
};

export class HubStudioService implements StudioService {
    readonly capabilities: ServiceCapabilities = {
        workspace: false,
        deploy: false,
        liveRun: false,
        hubPackages: true,
    };

    private loaded: LoadedPackage | null = null;

    constructor(private readonly hubUrl: string = "https://api.foragelang.com") {}

    /// Cache the loaded version artifact for the IDE shell. The hub
    /// IDE's URL is `/edit/:author/:slug` — the shell fetches the
    /// `latest` version once and stashes it here so the service's
    /// `runRecipe` / `validateRecipe` calls find the same source.
    setLoaded(loaded: LoadedPackage | null) {
        this.loaded = loaded;
    }

    // ── Workspace: hub has none ─────────────────────────────────────

    currentWorkspace(): Promise<WorkspaceInfo | null> { return Promise.resolve(null); }
    openWorkspace(): Promise<WorkspaceInfo> {
        return Promise.reject(new NotSupportedByService("openWorkspace"));
    }
    newWorkspace(): Promise<WorkspaceInfo> {
        return Promise.reject(new NotSupportedByService("newWorkspace"));
    }
    closeWorkspace(): Promise<void> {
        return Promise.reject(new NotSupportedByService("closeWorkspace"));
    }
    listRecentWorkspaces(): Promise<RecentWorkspace[]> { return Promise.resolve([]); }
    listWorkspaceFiles(): Promise<FileNode> {
        // The hub IDE has a single package open at a time; we expose
        // recipe + each referenced type as a flat folder so the
        // existing sidebar's file tree still has something to render.
        // Real workspace navigation doesn't exist here.
        const loaded = this.loaded;
        if (!loaded) {
            return Promise.resolve({ kind: "folder", name: "ide", path: "", children: [] });
        }
        const recipePath = `${loaded.slug}.forage`;
        // All `.forage` files at workspace root classify as
        // `declarations` (see `FileKind` docstring). The UI joins
        // back against the parsed recipe index to tell which one is
        // the recipe.
        const children: FileNode[] = [
            {
                kind: "file",
                name: recipePath,
                path: recipePath,
                file_kind: "declarations",
            },
            ...loaded.types.map((t): FileNode => ({
                kind: "file",
                name: `${t.name}.forage`,
                path: `${t.name}.forage`,
                file_kind: "declarations",
            })),
        ];
        return Promise.resolve({
            kind: "folder",
            name: loaded.slug,
            path: loaded.slug,
            children,
        });
    }
    loadFile(path: string): Promise<string> {
        if (!this.loaded) {
            return Promise.reject(new Error(`no package loaded`));
        }
        if (path === `${this.loaded.slug}.forage`) {
            return Promise.resolve(this.loaded.version.recipe);
        }
        const type = this.loaded.types.find((t) => `${t.name}.forage` === path);
        if (!type) return Promise.reject(new Error(`no such file: ${path}`));
        return Promise.resolve(type.source);
    }
    saveFile(path: string, source: string): Promise<ValidationOutcome> {
        // Edits live in the in-memory loaded artifact until the user
        // hits Publish. Update the cached version so subsequent run /
        // validate calls see the edited source. Persistence to hub-api
        // happens through `publishVersion`.
        if (!this.loaded) {
            return Promise.reject(new Error(`no package loaded`));
        }
        if (path === `${this.loaded.slug}.forage`) {
            this.loaded.version.recipe = source;
        } else {
            const type = this.loaded.types.find((t) => `${t.name}.forage` === path);
            if (!type) return Promise.reject(new Error(`no such file: ${path}`));
            type.source = source;
        }
        return this.validateRecipe(this.loaded.version.recipe);
    }

    // ── Recipe / authoring (runs in the browser via forage-wasm) ────

    validateRecipe(source: string): Promise<ValidationOutcome> {
        // `parse_and_validate` returns `{ ok, issues, recipe }` on
        // success or `{ ok: false, error: {message, span?} }` on parse
        // failure. The IDE's Monaco markers map onto the ts-rs
        // `Diagnostic` shape (start_line/col + end_line/col), but the
        // wasm bridge currently returns byte-offset spans only —
        // surface them as line 0 col 0 so the editor lights up the
        // top, and let the LSP/future-work fill in real ranges.
        const result = parse_and_validate(source) as
            | {
                ok: true;
                issues: Array<{
                    code: string;
                    message: string;
                    severity: "error" | "warning";
                }>;
                recipe: unknown;
            }
            | {
                ok: false;
                error: { message: string; span: { start: number; end: number } | null };
            };
        const zeroRange = { start_line: 0, start_col: 0, end_line: 0, end_col: 0 };
        if (result.ok) {
            const diagnostics: Diagnostic[] = result.issues.map((i) => ({
                code: i.code,
                message: i.message,
                severity: i.severity,
                ...zeroRange,
            }));
            return Promise.resolve({
                ok: diagnostics.every((d) => d.severity !== "error"),
                diagnostics,
            });
        }
        return Promise.resolve({
            ok: false,
            diagnostics: [
                {
                    code: "ParseError",
                    message: result.error.message,
                    severity: "error",
                    ...zeroRange,
                },
            ],
        });
    }
    recipeOutline(source: string): Promise<RecipeOutline> {
        return Promise.resolve(recipe_outline(source) as RecipeOutline);
    }
    recipeHover(source: string, line: number, col: number): Promise<HoverInfo | null> {
        return Promise.resolve(recipe_hover(source, line, col) as HoverInfo | null);
    }
    recipeProgressUnit(_slug: string): Promise<ProgressUnit | null> {
        // The Tauri command keys off slug to find the source on disk;
        // in the hub IDE the loaded package's recipe is in memory, so
        // we infer from that. Returns null when no package is loaded
        // or when the recipe has no emit-bearing loop.
        if (!this.loaded) return Promise.resolve(null);
        return Promise.resolve(
            recipe_progress_unit(this.loaded.version.recipe) as ProgressUnit | null,
        );
    }
    languageDictionary(): Promise<LanguageDictionary> {
        return Promise.resolve(language_dictionary() as LanguageDictionary);
    }
    createRecipe(): Promise<string> {
        return Promise.reject(new NotSupportedByService("createRecipe"));
    }
    deleteRecipe(): Promise<void> {
        return Promise.reject(new NotSupportedByService("deleteRecipe"));
    }
    listRecipeStatuses(): Promise<RecipeStatus[]> {
        // The hub IDE renders one package at a time; there's no
        // workspace recipe list to join against a daemon, so this
        // surface is empty by construction. The UI gates draft /
        // deployed affordances on `capabilities` and won't reach here.
        return Promise.resolve([]);
    }

    // ── Run (replay only) ───────────────────────────────────────────

    async runRecipe(_name: string, flags?: RunRecipeFlags): Promise<RunOutcome> {
        // The hub IDE has no network transport — only the in-browser
        // replay engine. `replay: false` is the only flag value that
        // can't be served here; absent / true / null all fall through
        // to fixture replay. Callers should gate on
        // `capabilities.liveRun` before requesting live.
        if (flags?.replay === false) {
            throw new NotSupportedByService("runRecipe(live)");
        }
        if (!this.loaded) {
            return {
                ok: false,
                error: "no package loaded",
                snapshot: null,
                daemon_warning: null,
            };
        }
        const captures = this.loaded.version.fixtures
            .map((f) => f.content)
            .join("\n");
        // Each loaded type rides into the catalog as a single decl-file
        // shape (`{ name, source }`); `run_replay` merges every decl
        // body's types into the recipe's catalog. The synthetic name
        // (`<Name>.forage`) is purely for diagnostic surfaces — the
        // wasm side keys off `source` content.
        const decls = this.loaded.types.map((t) => ({
            name: `${t.name}.forage`,
            source: t.source,
        }));
        try {
            // `run_replay` returns the engine's `Snapshot` serialized as a
            // JS object — same shape as `bindings/Snapshot.ts`.
            const snapshot = (await run_replay(
                this.loaded.version.recipe,
                decls,
                captures,
                // Inputs/secrets stay empty in the hub IDE: the read +
                // replay + light-authoring scope (per the roadmap) has
                // no inputs UI today; live runs (which would need
                // secrets) are gated off via `capabilities.liveRun`.
                {},
                {},
            )) as RunOutcome["snapshot"];
            return {
                ok: true,
                error: null,
                snapshot,
                // `daemon_warning` reports a daemon-bookkeeping miss
                // after a successful run. There's no daemon in the
                // hub IDE, so this is structurally always null.
                daemon_warning: null,
            };
        } catch (e) {
            return {
                ok: false,
                error: e instanceof Error ? e.message : String(e),
                snapshot: null,
                daemon_warning: null,
            };
        }
    }
    runNotebook(): Promise<RunOutcome> {
        // The hub IDE has no daemon to compose deployed recipes
        // through. Notebooks are a Studio-only surface.
        return Promise.reject(new NotSupportedByService("runNotebook"));
    }
    composeNotebookSource(
        _name: string,
        _stages: string[],
        _outputType: string | null,
    ): Promise<string> {
        return Promise.reject(new NotSupportedByService("composeNotebookSource"));
    }
    saveNotebook(
        _name: string,
        _stages: string[],
        _outputType: string | null,
    ): Promise<NotebookSaveOutcome> {
        return Promise.reject(new NotSupportedByService("saveNotebook"));
    }
    listWorkspaceRecipeSignatures(): Promise<RecipeSignatureWire[]> {
        return Promise.resolve([]);
    }
    parseRecipeSignature(): Promise<RecipeSignatureWire | null> {
        return Promise.reject(new NotSupportedByService("parseRecipeSignature"));
    }
    cancelRun(): Promise<void> { return Promise.resolve(); }
    debugResume(_action: DebugAction): Promise<void> { return Promise.resolve(); }
    setBreakpoints(_lines: number[]): Promise<void> { return Promise.resolve(); }
    setRecipeBreakpoints(_name: string, _lines: number[]): Promise<void> { return Promise.resolve(); }
    loadRecipeBreakpoints(_name: string): Promise<number[]> { return Promise.resolve([]); }
    evalWatchExpression(_exprSource: string): Promise<unknown> {
        // Hub IDE has no engine paused; nothing to evaluate against.
        return Promise.reject(new NotSupportedByService("evalWatchExpression"));
    }
    loadFullStepBody(_runId: string, _stepName: string): Promise<string> {
        return Promise.reject(new NotSupportedByService("loadFullStepBody"));
    }
    openResponseWindow(): Promise<void> { return Promise.resolve(); }

    // ── Daemon: hub has none ────────────────────────────────────────

    daemonStatus(): Promise<DaemonStatus> {
        // The hub has no daemon. Returns the full `DaemonStatus` shape
        // with `running: false`; `started_at: 0` is the documented
        // sentinel for "not running" on the Studio side.
        return Promise.resolve({
            running: false,
            version: "hub-ide",
            started_at: 0,
            active_count: 0,
        });
    }
    listRuns(): Promise<Run[]> { return Promise.resolve([]); }
    getRun(): Promise<Run | null> { return Promise.resolve(null); }
    configureRun(): Promise<Run> {
        return Promise.reject(new NotSupportedByService("configureRun"));
    }
    removeRun(): Promise<void> {
        return Promise.reject(new NotSupportedByService("removeRun"));
    }
    triggerRun(): Promise<ScheduledRun> {
        return Promise.reject(new NotSupportedByService("triggerRun"));
    }
    listScheduledRuns(): Promise<ScheduledRun[]> { return Promise.resolve([]); }
    loadRunRecords(): Promise<unknown[]> { return Promise.resolve([]); }
    loadRunJsonld(): Promise<unknown> {
        // Scheduled runs don't exist in the hub IDE; there's nothing
        // to project as a JSON-LD document. Return an empty document
        // to match the daemon's "ephemeral run" shape.
        return Promise.resolve({});
    }
    validateCron(): Promise<void> {
        return Promise.reject(new NotSupportedByService("validateCron"));
    }

    // ── Hub publish / auth (no Studio bridge — the IDE uses its own
    //    cookie-based session against hub-api) ──────────────────────

    publishRecipe(): Promise<PublishOutcome> {
        // Studio's workspace-assembling publish flow doesn't exist in
        // the hub IDE — there's no local workspace. Use
        // `publishVersion` instead, which talks straight to hub-api.
        return Promise.reject(new NotSupportedByService("publishRecipe"));
    }
    previewPublish(): Promise<PublishPreview> {
        return Promise.reject(new NotSupportedByService("previewPublish"));
    }
    syncFromHub(): Promise<SyncOutcomeWire> {
        // The hub IDE has no on-disk workspace to materialize into; a
        // user's "save" flow is `publishVersion`. CLI / Studio handle
        // sync.
        return Promise.reject(new NotSupportedByService("syncFromHub"));
    }
    forkFromHub(): Promise<SyncOutcomeWire> {
        // Use `forkPackage` for in-IDE forking — that one talks to
        // hub-api directly and returns the new package metadata.
        return Promise.reject(new NotSupportedByService("forkFromHub"));
    }
    async authWhoami(): Promise<string | null> {
        // Hub IDE sessions live in cookies — `GET /v1/oauth/whoami` is
        // the documented probe (`hub-api/src/oauth.ts:oauthWhoami`).
        // Returns `{ authenticated: true, user: { login, ... } }` when
        // signed in, `{ authenticated: false }` otherwise. Auth-keyed
        // affordances (Star, Fork, Publish) gate on the returned login.
        const resp = await fetch(`${this.hubUrl}/v1/oauth/whoami`, {
            credentials: "include",
        });
        if (!resp.ok) return null;
        const body = (await resp.json()) as
            | { authenticated: true; user: { login: string; name?: string; avatarUrl?: string } }
            | { authenticated: false };
        return body.authenticated ? body.user.login : null;
    }
    authStartDeviceFlow(): Promise<DeviceStart> {
        return Promise.reject(new NotSupportedByService("authStartDeviceFlow"));
    }
    authPollDevice(): Promise<PollOutcome> {
        return Promise.reject(new NotSupportedByService("authPollDevice"));
    }
    authLogout(): Promise<void> {
        return Promise.reject(new NotSupportedByService("authLogout"));
    }

    // ── Hub packages / social ───────────────────────────────────────

    async listPackages(query?: PackageQuery): Promise<PackageListing[]> {
        const params = new URLSearchParams();
        if (query?.sort) params.set("sort", query.sort);
        if (query?.category) params.set("category", query.category);
        if (query?.q) params.set("q", query.q);
        if (query?.limit !== undefined) params.set("limit", String(query.limit));
        const qs = params.toString();
        const url = qs.length > 0 ? `${this.hubUrl}/v1/packages?${qs}` : `${this.hubUrl}/v1/packages`;
        const data = await this.fetchJson<ListPackagesResponse>(url);
        return data.items;
    }
    getPackage(author: string, slug: string): Promise<PackageMetadata> {
        return this.fetchJson<PackageMetadata>(
            `${this.hubUrl}/v1/packages/${encodeURIComponent(author)}/${encodeURIComponent(slug)}`,
        );
    }
    async listPackageVersions(author: string, slug: string): Promise<ListVersionsItem[]> {
        const data = await this.fetchJson<ListVersionsResponse>(
            `${this.hubUrl}/v1/packages/${encodeURIComponent(author)}/${encodeURIComponent(slug)}/versions`,
        );
        return data.items;
    }
    getPackageVersion(
        author: string,
        slug: string,
        version: number | "latest",
    ): Promise<PackageVersion> {
        // `version` is constrained to a number or the literal `"latest"`
        // and the server's route handler accepts both. Encoding here
        // documents the contract even though neither value contains
        // characters that need escaping today.
        return this.fetchJson<PackageVersion>(
            `${this.hubUrl}/v1/packages/${encodeURIComponent(author)}/${encodeURIComponent(slug)}/versions/${encodeURIComponent(String(version))}`,
        );
    }
    async starPackage(author: string, slug: string): Promise<void> {
        await this.fetchJson<unknown>(
            `${this.hubUrl}/v1/packages/${encodeURIComponent(author)}/${encodeURIComponent(slug)}/stars`,
            { method: "POST", credentials: "include" },
        );
    }
    async unstarPackage(author: string, slug: string): Promise<void> {
        await this.fetchJson<unknown>(
            `${this.hubUrl}/v1/packages/${encodeURIComponent(author)}/${encodeURIComponent(slug)}/stars`,
            { method: "DELETE", credentials: "include" },
        );
    }
    forkPackage(author: string, slug: string, asSlug?: string): Promise<PackageMetadata> {
        return this.fetchJson<PackageMetadata>(
            `${this.hubUrl}/v1/packages/${encodeURIComponent(author)}/${encodeURIComponent(slug)}/fork`,
            {
                method: "POST",
                credentials: "include",
                headers: { "content-type": "application/json" },
                body: JSON.stringify({ as: asSlug ?? null }),
            },
        );
    }
    publishVersion(
        author: string,
        slug: string,
        payload: PublishPayload,
    ): Promise<PackageVersion> {
        return this.fetchJson<PackageVersion>(
            `${this.hubUrl}/v1/packages/${encodeURIComponent(author)}/${encodeURIComponent(slug)}/versions`,
            {
                method: "POST",
                credentials: "include",
                headers: { "content-type": "application/json" },
                body: JSON.stringify(payload),
            },
        );
    }
    async discoverProducers(
        typeAuthor: string,
        typeName: string,
    ): Promise<PackageListing[]> {
        const t = `${encodeURIComponent(typeAuthor)}/${encodeURIComponent(typeName)}`;
        const data = await this.fetchJson<ListPackagesResponse>(
            `${this.hubUrl}/v1/discover/producers?type=${t}`,
        );
        return data.items;
    }
    getTypeVersion(
        author: string,
        name: string,
        version: number | "latest",
    ): Promise<TypeVersion> {
        return this.fetchJson<TypeVersion>(
            `${this.hubUrl}/v1/types/${encodeURIComponent(author)}/${encodeURIComponent(name)}/versions/${encodeURIComponent(String(version))}`,
        );
    }
    publishTypeVersion(
        author: string,
        name: string,
        payload: PublishTypePayload,
    ): Promise<TypeVersion> {
        return this.fetchJson<TypeVersion>(
            `${this.hubUrl}/v1/types/${encodeURIComponent(author)}/${encodeURIComponent(name)}/versions`,
            {
                method: "POST",
                credentials: "include",
                headers: { "content-type": "application/json" },
                body: JSON.stringify(payload),
            },
        );
    }

    // ── Bookkeeping ─────────────────────────────────────────────────

    version(): Promise<string> { return Promise.resolve("hub-ide"); }
    showRecipeContextMenu(): Promise<void> { return Promise.resolve(); }

    // ── Events ──────────────────────────────────────────────────────

    onRunEvent(_handler: (event: RunEvent) => void): Unsubscribe {
        // `runRecipe("replay")` returns a final snapshot synchronously;
        // there's no per-event stream from the wasm bridge. Hub IDE
        // subscribers see no events. When/if streaming is added, route
        // engine events through the handler here.
        return () => {};
    }
    onDebugPaused(_handler: (payload: PausePayload) => void): Unsubscribe {
        // No debugger in the hub IDE — the replay engine runs straight
        // through without pause hooks.
        return () => {};
    }
    onRunBegin(): Unsubscribe { return () => {}; }
    onStepResponse(): Unsubscribe { return () => {}; }
    onDebugResumed(): Unsubscribe { return () => {}; }
    onDaemonRunCompleted(_handler: (run: ScheduledRun) => void): Unsubscribe {
        return () => {};
    }
    onWorkspaceOpened(_handler: () => void): Unsubscribe { return () => {}; }
    onWorkspaceClosed(_handler: () => void): Unsubscribe { return () => {}; }
    onMenuEvent(_name: string, _handler: (payload?: unknown) => void): Unsubscribe {
        // No native menu on the web.
        return () => {};
    }
    onDeeplinkClone(_handler: (payload: DeeplinkClonePayload) => void): Unsubscribe {
        // No OS-scheme handler in the browser; the IDE drives package
        // selection through the URL hash itself.
        return () => {};
    }

    // ── Host dialogs ────────────────────────────────────────────────

    async confirm(
        message: string,
        options?: { title?: string; okLabel?: string; cancelLabel?: string },
    ): Promise<boolean> {
        // `window.confirm` is a native two-button dialog with no API
        // for custom labels or a title — those `options` get dropped.
        // Log the drop so a developer sees the limitation here instead
        // of debugging why their custom labels never showed up.
        if (options?.title || options?.okLabel || options?.cancelLabel) {
            console.warn(
                "HubStudioService.confirm: window.confirm() ignores title/okLabel/cancelLabel; dropping",
                options,
            );
        }
        return window.confirm(message);
    }
    pickDirectory(): Promise<string | null> {
        return Promise.reject(new NotSupportedByService("pickDirectory"));
    }
    revealInFileManager(): Promise<void> {
        return Promise.reject(new NotSupportedByService("revealInFileManager"));
    }

    // ── Internal ────────────────────────────────────────────────────

    private async fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
        const resp = await fetch(url, init);
        if (resp.status === 409) {
            const body = await resp.json().catch(() => null);
            const err = body?.error;
            // Server contract for stale-base 409: `{error: {code,
            // message, latest_version, your_base}}`. All four fields
            // are server-controlled. If `message` isn't a string the
            // body is malformed — surface the raw envelope rather than
            // masking with a synthetic default.
            if (
                err
                && typeof err.latest_version === "number"
                && typeof err.message === "string"
            ) {
                throw new StaleBaseError(
                    err.latest_version,
                    typeof err.your_base === "number" ? err.your_base : null,
                    err.message,
                );
            }
            throw new Error(`${resp.status} ${resp.statusText}: ${JSON.stringify(body)}`);
        }
        if (!resp.ok) {
            const text = await resp.text().catch(() => "");
            throw new Error(`${resp.status} ${resp.statusText}: ${text}`);
        }
        if (resp.status === 204) return undefined as T;
        return resp.json() as Promise<T>;
    }
}
