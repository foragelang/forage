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
    type PackageListing,
    type PackageMetadata,
    type PackageQuery,
    type PackageVersion,
    type PollOutcome,
    type PublishOutcome,
    type PublishPayload,
    type PublishPreview,
    type ServiceCapabilities,
    type StudioService,
    type SyncOutcomeWire,
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
import type { Run } from "@/bindings/Run";
import type { RunEvent } from "@/bindings/RunEvent";
import type { RunOutcome } from "@/bindings/RunOutcome";
import type { ScheduledRun } from "@/bindings/ScheduledRun";
import type { ValidationOutcome } from "@/bindings/ValidationOutcome";
import type { WorkspaceInfo } from "@/bindings/WorkspaceInfo";

import {
    parse_and_validate,
    parse_recipe,
    run_replay,
} from "forage-wasm";

type ListPackagesResponse = {
    items: PackageListing[];
    next_cursor: string | null;
};

type ListVersionsResponse = {
    items: ListVersionsItem[];
};

/// Lightweight event bus used to forward engine events to subscribers
/// when (eventually) the WASM bridge gains progress streaming. Today
/// `runRecipe("replay")` returns a final snapshot, not per-event
/// updates — the bus is here so the contract stays compatible.
class EventBus<T> {
    private handlers: Set<(event: T) => void> = new Set();

    subscribe(handler: (event: T) => void): Unsubscribe {
        this.handlers.add(handler);
        return () => {
            this.handlers.delete(handler);
        };
    }

    emit(event: T) {
        for (const h of this.handlers) h(event);
    }
}

/// Currently loaded version artifact — the IDE caches the latest fetch
/// so subsequent `runRecipe` calls can replay against the same recipe
/// + decls + fixtures without round-tripping to hub-api. `loadPackage`
/// from the IDE shell sets this; the methods that need it read it.
type LoadedPackage = {
    author: string;
    slug: string;
    version: PackageVersion;
};

export class HubStudioService implements StudioService {
    readonly capabilities: ServiceCapabilities = {
        workspace: false,
        deploy: false,
        liveRun: false,
        hubPackages: true,
    };

    private loaded: LoadedPackage | null = null;
    private runEvents = new EventBus<RunEvent>();

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
        // recipe + decls as a flat folder so the existing sidebar's
        // file tree still has something to render. Real workspace
        // navigation doesn't exist here.
        const loaded = this.loaded;
        if (!loaded) {
            return Promise.resolve({ kind: "folder", name: "ide", path: "", children: [] });
        }
        const children: FileNode[] = [
            {
                kind: "file",
                name: "recipe.forage",
                path: `${loaded.slug}/recipe.forage`,
                file_kind: "recipe",
            },
            ...loaded.version.decls.map((d): FileNode => ({
                kind: "file",
                name: d.name,
                path: `${loaded.slug}/${d.name}`,
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
        if (path === `${this.loaded.slug}/recipe.forage`) {
            return Promise.resolve(this.loaded.version.recipe);
        }
        const decl = this.loaded.version.decls.find(
            (d) => `${this.loaded!.slug}/${d.name}` === path,
        );
        if (!decl) return Promise.reject(new Error(`no such file: ${path}`));
        return Promise.resolve(decl.source);
    }
    saveFile(path: string, source: string): Promise<ValidationOutcome> {
        // Edits live in the in-memory loaded artifact until the user
        // hits Publish. Update the cached version so subsequent run /
        // validate calls see the edited source. Persistence to hub-api
        // happens through `publishVersion`.
        if (!this.loaded) {
            return Promise.reject(new Error(`no package loaded`));
        }
        if (path === `${this.loaded.slug}/recipe.forage`) {
            this.loaded.version.recipe = source;
        } else {
            const decl = this.loaded.version.decls.find(
                (d) => `${this.loaded!.slug}/${d.name}` === path,
            );
            if (!decl) return Promise.reject(new Error(`no such file: ${path}`));
            decl.source = source;
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
        // The Rust-side `recipe_outline` derives step locations from
        // the AST. Until forage-wasm exports it, fall back to an empty
        // outline — the gutter just shows no step markers. Parsing
        // success / failure is independent.
        const result = parse_recipe(source) as { ok: boolean };
        return Promise.resolve({
            steps: result.ok ? [] : [],
        } as unknown as RecipeOutline);
    }
    recipeHover(_source: string, _line: number, _col: number): Promise<HoverInfo | null> {
        // Hover info lives in forage-lsp's `intel::hover_at` and isn't
        // wired through forage-wasm yet. Return null so Monaco shows no
        // hover popover; the editor still functions for typing.
        return Promise.resolve(null);
    }
    recipeProgressUnit(_slug: string): Promise<ProgressUnit | null> {
        // Progress unit inference is in forage-core::progress; not yet
        // exported through forage-wasm. Returning null disables the
        // progress bar in the run pane, which is the right behavior
        // when the inference isn't available.
        return Promise.resolve(null);
    }
    languageDictionary(): Promise<LanguageDictionary> {
        // Monaco completion. Empty dictionary → no completion
        // suggestions; the editor still highlights syntax via the
        // language grammar registered in `monaco-forage.ts`.
        return Promise.resolve({
            keywords: [],
            type_keywords: [],
            transforms: [],
        } as unknown as LanguageDictionary);
    }
    createRecipe(): Promise<string> {
        return Promise.reject(new NotSupportedByService("createRecipe"));
    }
    deleteRecipe(): Promise<void> {
        return Promise.reject(new NotSupportedByService("deleteRecipe"));
    }

    // ── Run (replay only) ───────────────────────────────────────────

    async runRecipe(_slug: string, replay: boolean): Promise<RunOutcome> {
        if (!replay) {
            // Live runs need a real network transport; the hub bundle
            // doesn't include one. Caller should gate on
            // `capabilities.liveRun` before reaching here.
            throw new NotSupportedByService("runRecipe(live)");
        }
        if (!this.loaded) {
            return {
                ok: false,
                error: "no package loaded",
                snapshot: null,
            } as unknown as RunOutcome;
        }
        const captures = this.loaded.version.fixtures
            .map((f) => f.content)
            .join("\n");
        try {
            const snapshot = await run_replay(
                this.loaded.version.recipe,
                this.loaded.version.decls,
                captures,
                {},
                {},
            );
            return {
                ok: true,
                error: null,
                snapshot,
            } as unknown as RunOutcome;
        } catch (e) {
            return {
                ok: false,
                error: e instanceof Error ? e.message : String(e),
                snapshot: null,
            } as unknown as RunOutcome;
        }
    }
    cancelRun(): Promise<void> { return Promise.resolve(); }
    debugResume(_action: DebugAction): Promise<void> { return Promise.resolve(); }
    setPauseIterations(_enabled: boolean): Promise<void> { return Promise.resolve(); }
    setBreakpoints(_steps: string[]): Promise<void> { return Promise.resolve(); }
    setRecipeBreakpoints(_slug: string, _steps: string[]): Promise<void> { return Promise.resolve(); }
    loadRecipeBreakpoints(_slug: string): Promise<string[]> { return Promise.resolve([]); }

    // ── Daemon: hub has none ────────────────────────────────────────

    daemonStatus(): Promise<DaemonStatus> {
        return Promise.resolve({
            running: false,
            version: "hub-ide",
            active_count: 0,
        } as unknown as DaemonStatus);
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
    authWhoami(): Promise<string | null> {
        // Hub IDE session lives in cookies — read it through the API
        // when implementing the auth banner; for now return null so
        // the UI doesn't claim a signed-in user.
        return Promise.resolve(null);
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
        return this.fetchJson<PackageVersion>(
            `${this.hubUrl}/v1/packages/${encodeURIComponent(author)}/${encodeURIComponent(slug)}/versions/${version}`,
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

    // ── Bookkeeping ─────────────────────────────────────────────────

    version(): Promise<string> { return Promise.resolve("hub-ide"); }
    showRecipeContextMenu(): Promise<void> { return Promise.resolve(); }

    // ── Events ──────────────────────────────────────────────────────

    onRunEvent(handler: (event: RunEvent) => void): Unsubscribe {
        return this.runEvents.subscribe(handler);
    }
    onDebugPaused(_handler: (payload: PausePayload) => void): Unsubscribe {
        // No debugger in the hub IDE — the replay engine runs straight
        // through without pause hooks.
        return () => {};
    }
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
        _options?: { title?: string; okLabel?: string; cancelLabel?: string },
    ): Promise<boolean> {
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
            if (err && typeof err.latest_version === "number") {
                throw new StaleBaseError(
                    err.latest_version,
                    typeof err.your_base === "number" ? err.your_base : null,
                    err.message ?? "stale base",
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
