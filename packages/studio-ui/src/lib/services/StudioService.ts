//! Backend contract between the React UI and whatever's driving it.
//!
//! Two implementations exist today: `TauriStudioService` wraps the
//! Tauri IPC into the Rust core; `HubStudioService` talks to `hub-api`
//! over `fetch` and runs the engine in-browser via `forage-wasm`.
//! Components import `useStudioService()` and never reach for Tauri or
//! fetch directly.
//!
//! Surface design — every method is named after the product action
//! ("save this file", "run this recipe", "star this package"), not the
//! transport ("invoke save_file", "POST /v1/packages/.../stars"). The
//! impls own the plumbing.

import type { Cadence } from "../../bindings/Cadence";
import type { DaemonStatus } from "../../bindings/DaemonStatus";
import type { FileNode } from "../../bindings/FileNode";
import type { Health } from "../../bindings/Health";
import type { HoverInfo } from "../../bindings/HoverInfo";
import type { LanguageDictionary } from "../../bindings/LanguageDictionary";
import type { PausePayload } from "../../bindings/PausePayload";
import type { ProgressUnit } from "../../bindings/ProgressUnit";
import type { RecentWorkspace } from "../../bindings/RecentWorkspace";
import type { RecipeOutline } from "../../bindings/RecipeOutline";
import type { Run } from "../../bindings/Run";
import type { RunConfig } from "../../bindings/RunConfig";
import type { RunEvent } from "../../bindings/RunEvent";
import type { RunOutcome } from "../../bindings/RunOutcome";
import type { ScheduledRun } from "../../bindings/ScheduledRun";
import type { ValidationOutcome } from "../../bindings/ValidationOutcome";
import type { WorkspaceInfo } from "../../bindings/WorkspaceInfo";

// Resume action sent back to the engine when paused at a step or inside
// a for-loop iteration. Wire shape matches the Rust-side `ResumeAction`
// enum (rendered as snake_case).
export type DebugAction = "continue" | "step_over" | "stop";

// OAuth device-flow startup info; Studio uses this to bootstrap the
// publish/login flow against hub-api.
export type DeviceStart = {
    device_code: string;
    user_code: string;
    verification_url: string;
    interval: number;
    expires_in: number;
};

export type PollOutcome = {
    status: string;
    login?: string | null;
};

// --- Hub-side wire shapes ----------------------------------------------

// Minimal listing shape used by `listPackages` — mirrors hub-api's
// `PackageListing`. Defined here rather than imported from a generated
// binding because hub-api is TypeScript-only and we want the UI to own
// the type at its consumption point.
export type PackageListing = {
    author: string;
    slug: string;
    description: string;
    category: string;
    tags: string[];
    forked_from: ForkedFrom | null;
    created_at: number;
    latest_version: number;
    stars: number;
    downloads: number;
    fork_count: number;
};

export type ForkedFrom = {
    author: string;
    slug: string;
    version: number;
};

export type PackageMetadata = PackageListing & {
    owner_login: string;
};

export type PackageFile = {
    name: string;
    source: string;
};

export type PackageFixture = {
    name: string;
    content: string;
};

export type PackageSnapshot = {
    records: Record<string, unknown[]>;
    counts: Record<string, number>;
};

export type PackageVersion = {
    author: string;
    slug: string;
    version: number;
    recipe: string;
    decls: PackageFile[];
    fixtures: PackageFixture[];
    snapshot: PackageSnapshot | null;
    base_version: number | null;
    published_at: number;
    published_by: string;
};

export type ListVersionsItem = {
    version: number;
    published_at: number;
    published_by: string;
};

export type PackageQuery = {
    sort?: "top_starred" | "top_downloads" | "recent";
    category?: string;
    q?: string;
    limit?: number;
};

// Server returns `{latest_version, your_base, message}` on stale-base
// publish. Throw this typed error so the IDE can render the rebase UX
// without parsing free-form messages.
export class StaleBaseError extends Error {
    readonly kind = "stale_base" as const;
    constructor(
        readonly latestVersion: number,
        readonly yourBase: number | null,
        message: string,
    ) {
        super(message);
        this.name = "StaleBaseError";
    }
}

export type PublishPayload = {
    description: string;
    category: string;
    tags: string[];
    recipe: string;
    decls: PackageFile[];
    fixtures: PackageFixture[];
    snapshot: PackageSnapshot | null;
    base_version: number | null;
    forked_from: ForkedFrom | null;
};

// --- Service capabilities ----------------------------------------------

// The hub IDE renders a subset of Studio's affordances: no Deploy
// (the hub has no daemon to deploy to), no workspace switcher (the
// "workspace" is the package itself), no live HTTP (no network access
// for a recipe inside the worker). The UI keys off `capabilities` to
// hide unsupported actions instead of calling them and catching errors.
export type ServiceCapabilities = {
    workspace: boolean;
    deploy: boolean;
    liveRun: boolean;
    hubPackages: boolean;
};

// --- Event subscription handles ----------------------------------------

export type Unsubscribe = () => void;

// Re-export for callers that import everything via the service module.
export type {
    Cadence,
    DaemonStatus,
    FileNode,
    Health,
    HoverInfo,
    LanguageDictionary,
    PausePayload,
    ProgressUnit,
    RecentWorkspace,
    RecipeOutline,
    Run,
    RunConfig,
    RunEvent,
    RunOutcome,
    ScheduledRun,
    ValidationOutcome,
    WorkspaceInfo,
};

// --- The interface itself ----------------------------------------------

export interface StudioService {
    readonly capabilities: ServiceCapabilities;

    // ── Workspace (Studio-only; hub throws) ─────────────────────────
    currentWorkspace(): Promise<WorkspaceInfo | null>;
    openWorkspace(path: string): Promise<WorkspaceInfo>;
    newWorkspace(path: string): Promise<WorkspaceInfo>;
    closeWorkspace(): Promise<void>;
    listRecentWorkspaces(): Promise<RecentWorkspace[]>;
    listWorkspaceFiles(): Promise<FileNode>;
    loadFile(path: string): Promise<string>;
    saveFile(path: string, source: string): Promise<ValidationOutcome>;

    // ── Recipe / authoring ──────────────────────────────────────────
    validateRecipe(source: string): Promise<ValidationOutcome>;
    recipeOutline(source: string): Promise<RecipeOutline>;
    recipeHover(source: string, line: number, col: number): Promise<HoverInfo | null>;
    recipeProgressUnit(slug: string): Promise<ProgressUnit | null>;
    languageDictionary(): Promise<LanguageDictionary>;
    createRecipe(): Promise<string>;
    deleteRecipe(slug: string): Promise<void>;

    // ── Run ─────────────────────────────────────────────────────────
    runRecipe(slug: string, replay: boolean): Promise<RunOutcome>;
    cancelRun(): Promise<void>;
    debugResume(action: DebugAction): Promise<void>;
    setPauseIterations(enabled: boolean): Promise<void>;
    setBreakpoints(steps: string[]): Promise<void>;
    setRecipeBreakpoints(slug: string, steps: string[]): Promise<void>;
    loadRecipeBreakpoints(slug: string): Promise<string[]>;

    // ── Daemon — Studio only ────────────────────────────────────────
    daemonStatus(): Promise<DaemonStatus>;
    listRuns(): Promise<Run[]>;
    getRun(runId: string): Promise<Run | null>;
    configureRun(slug: string, cfg: RunConfig): Promise<Run>;
    removeRun(runId: string): Promise<void>;
    triggerRun(runId: string): Promise<ScheduledRun>;
    listScheduledRuns(
        runId: string,
        opts?: { limit?: number; before?: number | null },
    ): Promise<ScheduledRun[]>;
    loadRunRecords(
        scheduledRunId: string,
        typeName: string,
        limit: number,
    ): Promise<unknown[]>;
    validateCron(expr: string): Promise<void>;

    // ── Hub publishing / auth (used by Studio's publish flow) ───────
    publishRecipe(slug: string, hubUrl?: string, dryRun?: boolean): Promise<RunOutcome>;
    authWhoami(hubUrl?: string): Promise<string | null>;
    authStartDeviceFlow(hubUrl?: string): Promise<DeviceStart>;
    authPollDevice(hubUrl: string, deviceCode: string): Promise<PollOutcome>;
    authLogout(hubUrl?: string): Promise<void>;

    // ── Hub package discovery / social ──────────────────────────────
    listPackages(query?: PackageQuery): Promise<PackageListing[]>;
    getPackage(author: string, slug: string): Promise<PackageMetadata>;
    listPackageVersions(author: string, slug: string): Promise<ListVersionsItem[]>;
    getPackageVersion(
        author: string,
        slug: string,
        version: number | "latest",
    ): Promise<PackageVersion>;
    starPackage(author: string, slug: string): Promise<void>;
    unstarPackage(author: string, slug: string): Promise<void>;
    forkPackage(author: string, slug: string, asSlug?: string): Promise<PackageMetadata>;
    publishVersion(
        author: string,
        slug: string,
        payload: PublishPayload,
    ): Promise<PackageVersion>;

    // ── Studio-specific bookkeeping ─────────────────────────────────
    version(): Promise<string>;

    // ── Event subscriptions ─────────────────────────────────────────
    onRunEvent(handler: (event: RunEvent) => void): Unsubscribe;
    onDebugPaused(handler: (payload: PausePayload) => void): Unsubscribe;
    onDaemonRunCompleted(handler: (run: ScheduledRun) => void): Unsubscribe;
    onWorkspaceOpened(handler: () => void): Unsubscribe;
    onWorkspaceClosed(handler: () => void): Unsubscribe;
    onMenuEvent(name: string, handler: (payload?: unknown) => void): Unsubscribe;

    // Show a recipe's context menu (right-click). Native menu in Studio;
    // hub IDE no-ops.
    showRecipeContextMenu(slug: string): Promise<void>;

    // ── Host dialogs ────────────────────────────────────────────────
    // Confirm dialog ("Save changes before switching?"). Resolves true
    // when the user accepted, false when they cancelled. Tauri uses
    // the native dialog; the hub IDE uses `window.confirm` or a
    // bespoke React modal.
    confirm(message: string, options?: { title?: string; okLabel?: string; cancelLabel?: string }): Promise<boolean>;
    // Open a directory picker, returning the chosen absolute path or
    // null if the user cancelled. Tauri only; hub throws.
    pickDirectory(title: string): Promise<string | null>;
    // Reveal a folder in the system file manager (Finder/Explorer).
    // Tauri only; hub throws.
    revealInFileManager(path: string): Promise<void>;
}

/// Method-not-implemented error that hub-side throws when the UI calls
/// a Studio-only method (open workspace, configure run, etc.). The UI's
/// capability gates should prevent this; throwing instead of silently
/// no-oping surfaces the bug if a gate is missed.
export class NotSupportedByService extends Error {
    constructor(method: string) {
        super(`${method} is not supported by this StudioService`);
        this.name = "NotSupportedByService";
    }
}
