//! Tauri-backed implementation of `StudioService`. Wraps the Rust
//! core's Tauri commands.
//!
//! Hub-side package methods (listPackages, starPackage, etc.) hit
//! hub-api over `fetch` from inside Studio — the daemon-driven publish
//! flow uses a separate Tauri command (`publish_recipe`) that handles
//! its own auth, but the social/discovery surfaces talk to the same
//! HTTP endpoints the hub IDE uses.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ask, open as openDialog } from "@tauri-apps/plugin-dialog";
import { open as shellOpen } from "@tauri-apps/plugin-shell";

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
    type PausePayload,
    type PollOutcome,
    type PublishOutcome,
    type PublishPayload,
    type PublishPreview,
    type PublishTypePayload,
    type RecipeStatus,
    type RunEvent,
    type ScheduledRun,
    type ServiceCapabilities,
    type StudioService,
    type SyncOutcomeWire,
    type TypeVersion,
    type Unsubscribe,
} from "./StudioService";
import type { DaemonStatus } from "../../bindings/DaemonStatus";
import type { FileNode } from "../../bindings/FileNode";
import type { HoverInfo } from "../../bindings/HoverInfo";
import type { LanguageDictionary } from "../../bindings/LanguageDictionary";
import type { ProgressUnit } from "../../bindings/ProgressUnit";
import type { RecentWorkspace } from "../../bindings/RecentWorkspace";
import type { RecipeOutline } from "../../bindings/RecipeOutline";
import type { Run } from "../../bindings/Run";
import type { RunConfig } from "../../bindings/RunConfig";
import type { RunOutcome } from "../../bindings/RunOutcome";
import type { RunRecipeFlags } from "../../bindings/RunRecipeFlags";
import type { ValidationOutcome } from "../../bindings/ValidationOutcome";
import type { WorkspaceInfo } from "../../bindings/WorkspaceInfo";

// Tauri event channel names — must match the constants in the Rust
// commands module (`commands::RUN_EVENT`, `commands::DEBUG_PAUSED_EVENT`).
const RUN_EVENT = "forage:run-event";
const DEBUG_PAUSED_EVENT = "forage:debug-paused";
const DAEMON_RUN_COMPLETED_EVENT = "forage:daemon-run-completed";
const WORKSPACE_OPENED_EVENT = "forage:workspace-opened";
const WORKSPACE_CLOSED_EVENT = "forage:workspace-closed";
const DEEPLINK_CLONE_EVENT = "forage:deeplink-clone";

const DEFAULT_HUB = "https://api.foragelang.com";

/// Wire-shape returned by hub-api on success (`listPackages` / fork /
/// version reads). Defined inline because the IDE owns the consumption
/// point; no generated bindings cross from hub-api.
type ListPackagesResponse = {
    items: PackageListing[];
    next_cursor: string | null;
};

type ListVersionsResponse = {
    items: ListVersionsItem[];
};

/// Subscribe a Tauri listener and wrap the unlisten Promise in a sync
/// `Unsubscribe` handle. Under React.StrictMode the cleanup can fire
/// before the listener promise resolves — guarded with `cancelled` so
/// the resolved unlisten still fires.
function listenSync<P>(
    name: string,
    handler: (payload: P) => void,
): Unsubscribe {
    let cancelled = false;
    let un: (() => void) | undefined;
    listen<P>(name, (event) => handler(event.payload)).then((u) => {
        if (cancelled) u();
        else un = u;
    });
    return () => {
        cancelled = true;
        un?.();
    };
}

export class TauriStudioService implements StudioService {
    readonly capabilities: ServiceCapabilities = {
        workspace: true,
        deploy: true,
        liveRun: true,
        hubPackages: true,
    };

    constructor(private readonly hubUrl: string = DEFAULT_HUB) {}

    // ── Workspace ───────────────────────────────────────────────────

    currentWorkspace(): Promise<WorkspaceInfo | null> {
        return invoke<WorkspaceInfo | null>("current_workspace");
    }
    openWorkspace(path: string): Promise<WorkspaceInfo> {
        return invoke<WorkspaceInfo>("open_workspace", { path });
    }
    newWorkspace(path: string): Promise<WorkspaceInfo> {
        return invoke<WorkspaceInfo>("new_workspace", { path });
    }
    closeWorkspace(): Promise<void> {
        return invoke<void>("close_workspace");
    }
    listRecentWorkspaces(): Promise<RecentWorkspace[]> {
        return invoke<RecentWorkspace[]>("list_recent_workspaces");
    }
    listWorkspaceFiles(): Promise<FileNode> {
        return invoke<FileNode>("list_workspace_files");
    }
    loadFile(path: string): Promise<string> {
        return invoke<string>("load_file", { path });
    }
    saveFile(path: string, source: string): Promise<ValidationOutcome> {
        return invoke<ValidationOutcome>("save_file", { path, source });
    }

    // ── Recipe / authoring ──────────────────────────────────────────

    validateRecipe(source: string): Promise<ValidationOutcome> {
        return invoke<ValidationOutcome>("validate_recipe", { source });
    }
    recipeOutline(source: string): Promise<RecipeOutline> {
        return invoke<RecipeOutline>("recipe_outline", { source });
    }
    recipeHover(source: string, line: number, col: number): Promise<HoverInfo | null> {
        return invoke<HoverInfo | null>("recipe_hover", { source, line, col });
    }
    recipeProgressUnit(name: string): Promise<ProgressUnit | null> {
        return invoke<ProgressUnit | null>("recipe_progress_unit", { name });
    }
    languageDictionary(): Promise<LanguageDictionary> {
        return invoke<LanguageDictionary>("language_dictionary");
    }
    createRecipe(): Promise<string> {
        return invoke<string>("create_recipe");
    }
    deleteRecipe(name: string): Promise<void> {
        return invoke<void>("delete_recipe", { name });
    }
    listRecipeStatuses(): Promise<RecipeStatus[]> {
        return invoke<RecipeStatus[]>("list_recipe_statuses");
    }

    // ── Run ─────────────────────────────────────────────────────────

    runRecipe(name: string, flags?: RunRecipeFlags): Promise<RunOutcome> {
        return invoke<RunOutcome>("run_recipe", { name, flags: flags ?? null });
    }
    cancelRun(): Promise<void> {
        return invoke<void>("cancel_run");
    }
    debugResume(action: DebugAction): Promise<void> {
        return invoke<void>("debug_resume", { action });
    }
    setPauseIterations(enabled: boolean): Promise<void> {
        return invoke<void>("set_pause_iterations", { enabled });
    }
    setBreakpoints(steps: string[]): Promise<void> {
        return invoke<void>("set_breakpoints", { steps });
    }
    setRecipeBreakpoints(name: string, steps: string[]): Promise<void> {
        return invoke<void>("set_recipe_breakpoints", { name, steps });
    }
    loadRecipeBreakpoints(name: string): Promise<string[]> {
        return invoke<string[]>("load_recipe_breakpoints", { name });
    }

    // ── Daemon ──────────────────────────────────────────────────────

    daemonStatus(): Promise<DaemonStatus> {
        return invoke<DaemonStatus>("daemon_status");
    }
    listRuns(): Promise<Run[]> {
        return invoke<Run[]>("list_runs");
    }
    getRun(runId: string): Promise<Run | null> {
        return invoke<Run | null>("get_run", { runId });
    }
    configureRun(name: string, cfg: RunConfig): Promise<Run> {
        return invoke<Run>("configure_run", { name, cfg });
    }
    removeRun(runId: string): Promise<void> {
        return invoke<void>("remove_run", { runId });
    }
    triggerRun(runId: string): Promise<ScheduledRun> {
        return invoke<ScheduledRun>("trigger_run", { runId });
    }
    listScheduledRuns(
        runId: string,
        opts?: { limit?: number; before?: number | null },
    ): Promise<ScheduledRun[]> {
        return invoke<ScheduledRun[]>("list_scheduled_runs", {
            runId,
            limit: opts?.limit ?? 80,
            before: opts?.before ?? null,
        });
    }
    loadRunRecords(
        scheduledRunId: string,
        typeName: string,
        limit: number,
    ): Promise<unknown[]> {
        return invoke<unknown[]>("load_run_records", {
            scheduledRunId,
            typeName,
            limit,
        });
    }
    loadRunJsonld(scheduledRunId: string): Promise<unknown> {
        return invoke<unknown>("load_run_jsonld", { scheduledRunId });
    }
    validateCron(expr: string): Promise<void> {
        return invoke<void>("validate_cron_expr", { expr });
    }

    // ── Hub publish / auth ──────────────────────────────────────────

    publishRecipe(args: {
        author: string;
        name: string;
        description: string;
        category: string;
        tags: string[];
        hubUrl?: string;
    }): Promise<PublishOutcome> {
        return invoke<PublishOutcome>("publish_recipe", {
            author: args.author,
            name: args.name,
            description: args.description,
            category: args.category,
            tags: args.tags,
            hubUrl: args.hubUrl ?? this.hubUrl,
        });
    }
    previewPublish(args: {
        name: string;
        description: string;
        category: string;
        tags: string[];
    }): Promise<PublishPreview> {
        return invoke<PublishPreview>("preview_publish", {
            name: args.name,
            description: args.description,
            category: args.category,
            tags: args.tags,
        });
    }
    syncFromHub(args: {
        author: string;
        slug: string;
        version?: number | null;
        hubUrl?: string;
    }): Promise<SyncOutcomeWire> {
        return invoke<SyncOutcomeWire>("sync_from_hub", {
            author: args.author,
            slug: args.slug,
            version: args.version ?? null,
            hubUrl: args.hubUrl ?? this.hubUrl,
        });
    }
    forkFromHub(args: {
        upstreamAuthor: string;
        upstreamSlug: string;
        as?: string | null;
        hubUrl?: string;
    }): Promise<SyncOutcomeWire> {
        return invoke<SyncOutcomeWire>("fork_from_hub", {
            upstreamAuthor: args.upstreamAuthor,
            upstreamSlug: args.upstreamSlug,
            as: args.as ?? null,
            hubUrl: args.hubUrl ?? this.hubUrl,
        });
    }
    authWhoami(hubUrl?: string): Promise<string | null> {
        return invoke<string | null>("auth_whoami", { hubUrl: hubUrl ?? this.hubUrl });
    }
    authStartDeviceFlow(hubUrl?: string): Promise<DeviceStart> {
        return invoke<DeviceStart>("auth_start_device_flow", {
            hubUrl: hubUrl ?? this.hubUrl,
        });
    }
    authPollDevice(hubUrl: string, deviceCode: string): Promise<PollOutcome> {
        return invoke<PollOutcome>("auth_poll_device", { hubUrl, deviceCode });
    }
    authLogout(hubUrl?: string): Promise<void> {
        return invoke<void>("auth_logout", { hubUrl: hubUrl ?? this.hubUrl });
    }

    // ── Hub package discovery / social (fetch hub-api directly) ─────

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

    version(): Promise<string> {
        return invoke<string>("studio_version");
    }
    showRecipeContextMenu(name: string): Promise<void> {
        return invoke<void>("show_recipe_context_menu", { name });
    }

    // ── Event subscriptions ─────────────────────────────────────────

    onRunEvent(handler: (event: RunEvent) => void): Unsubscribe {
        // The Rust side coalesces RunEvents into batches; flatten back
        // to per-event handler calls so consumers don't care about the
        // batching.
        return listenSync<RunEvent[]>(RUN_EVENT, (batch) => {
            for (const ev of batch) handler(ev);
        });
    }
    onDebugPaused(handler: (payload: PausePayload) => void): Unsubscribe {
        return listenSync<PausePayload>(DEBUG_PAUSED_EVENT, handler);
    }
    onDaemonRunCompleted(handler: (run: ScheduledRun) => void): Unsubscribe {
        return listenSync<ScheduledRun>(DAEMON_RUN_COMPLETED_EVENT, handler);
    }
    onWorkspaceOpened(handler: () => void): Unsubscribe {
        return listenSync<unknown>(WORKSPACE_OPENED_EVENT, () => handler());
    }
    onWorkspaceClosed(handler: () => void): Unsubscribe {
        return listenSync<unknown>(WORKSPACE_CLOSED_EVENT, () => handler());
    }
    onMenuEvent(name: string, handler: (payload?: unknown) => void): Unsubscribe {
        return listenSync<unknown>(name, handler);
    }
    onDeeplinkClone(handler: (payload: DeeplinkClonePayload) => void): Unsubscribe {
        return listenSync<DeeplinkClonePayload>(DEEPLINK_CLONE_EVENT, handler);
    }

    // ── Host dialogs ────────────────────────────────────────────────

    async confirm(
        message: string,
        options?: { title?: string; okLabel?: string; cancelLabel?: string },
    ): Promise<boolean> {
        return ask(message, {
            title: options?.title ?? "Confirm",
            kind: "warning",
            okLabel: options?.okLabel ?? "OK",
            cancelLabel: options?.cancelLabel ?? "Cancel",
        });
    }
    async pickDirectory(title: string): Promise<string | null> {
        const picked = await openDialog({
            directory: true,
            multiple: false,
            title,
        });
        return typeof picked === "string" ? picked : null;
    }
    async revealInFileManager(path: string): Promise<void> {
        await shellOpen(path);
    }

    // ── Internal: hub-api fetch wrapper ─────────────────────────────

    private async fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
        const resp = await fetch(url, init);
        if (resp.status === 409) {
            // The 409 envelope on publish: {error: {code, message,
            // latest_version, your_base}}. Surface as a typed exception
            // so the rebase UX can pull the integer fields out. If
            // `message` is missing the body is malformed — surface the
            // raw envelope rather than mask it with a synthetic
            // default.
            const body = await resp.json().catch(() => null);
            const err = body?.error;
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

// Re-export the not-supported marker for callers that want to handle
// it specially in a UI fallback.
export { NotSupportedByService };
