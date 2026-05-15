//! Top-level cross-cutting effects: the engine event streams, the
//! daemon completion stream, keyboard shortcuts, and the native menu
//! event listeners. Mounted exactly once from `App.tsx` so the rest
//! of the tree doesn't need to know any of this exists.

import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { useStudioService } from "@/lib/services";
import {
    currentWorkspaceKey,
    recentWorkspacesKey,
} from "@/lib/queryKeys";
import { useStudio } from "@/lib/store";
import {
    closeWorkspaceAction,
    createAndOpenRecipe,
    newWorkspaceAction,
    openWorkspaceAction,
    runActive,
    saveActive,
} from "@/lib/studioActions";
import type { WorkspaceInfo } from "@/bindings/WorkspaceInfo";

/// Read the latest `currentWorkspace` query value from the cache so
/// keyboard / menu handlers can branch on workspace presence without
/// re-subscribing. Returns `null` when no workspace is open OR the
/// query hasn't resolved yet — the menu handler treats both as "no
/// workspace", which matches the Welcome view's branch.
function readWorkspace(qc: import("@tanstack/react-query").QueryClient) {
    return qc.getQueryData<WorkspaceInfo | null>(currentWorkspaceKey()) ?? null;
}

export function useStudioEffects() {
    const qc = useQueryClient();
    const service = useStudioService();

    // Engine run-event stream. The service flattens any batching the
    // host does (Tauri coalesces engine bursts into batches of ~50ms /
    // 256 events; the hub IDE's WASM bridge emits one event at a time)
    // so consumers don't need to know about it.
    useEffect(() => {
        const append = useStudio.getState().runAppend;
        const off = service.onRunEvent(append);
        return off;
    }, [service]);

    // Debug pause stream — fires when the engine parks at a step
    // boundary or inside a for-loop iteration.
    useEffect(() => {
        const off = service.onDebugPaused((p) => {
            useStudio.getState().debugPause(p);
        });
        return off;
    }, [service]);

    // Daemon completion — invalidate every per-run scheduled-runs
    // bucket (each pane keeps its own limit, so the keys are
    // `["scheduledRuns", runId, { limit }]`) so the deployment view,
    // inspector, and toolbar pick up the new row. The hub IDE has no
    // daemon, so this never fires there — its service returns an
    // immediately-no-op Unsubscribe.
    useEffect(() => {
        if (!service.capabilities.deploy) return;
        const off = service.onDaemonRunCompleted((run) => {
            qc.invalidateQueries({
                predicate: (q) =>
                    Array.isArray(q.queryKey) &&
                    q.queryKey[0] === "scheduledRuns" &&
                    q.queryKey[1] === run.run_id,
            });
            qc.invalidateQueries({ queryKey: ["runs"] });
        });
        return off;
    }, [qc, service]);

    // ⌘S → save, ⌘R → run with current toolbar flags, ⇧⌘R → run with
    // the prod preset (live, no sampling, persisted), ⌘N → new recipe
    // (with a workspace open) or new workspace (on Welcome).
    useEffect(() => {
        const handler = (e: KeyboardEvent) => {
            if (!(e.metaKey || e.ctrlKey)) return;
            if (e.key === "s") {
                e.preventDefault();
                void saveActive();
            } else if (e.key === "r" && !e.shiftKey) {
                e.preventDefault();
                void runActive();
            } else if (e.key === "r" && e.shiftKey) {
                e.preventDefault();
                void runActive({
                    sample_limit: null,
                    replay: false,
                    ephemeral: false,
                });
            } else if (e.key === "n") {
                e.preventDefault();
                if (readWorkspace(qc)) {
                    void createAndOpenRecipe(qc);
                } else if (service.capabilities.workspace) {
                    void newWorkspaceAction(qc);
                }
            }
        };
        window.addEventListener("keydown", handler);
        return () => window.removeEventListener("keydown", handler);
    }, [qc, service]);

    // Native menu events. The service's `onMenuEvent` mirrors Tauri's
    // `listen()` API — in the hub IDE these never fire (no native
    // menu), so the registration is a no-op.
    useEffect(() => {
        const offs: (() => void)[] = [];
        const register = (name: string, handler: () => void) => {
            offs.push(service.onMenuEvent(name, handler));
        };
        register("menu:new_recipe", () => {
            if (readWorkspace(qc)) {
                void createAndOpenRecipe(qc);
            } else if (service.capabilities.workspace) {
                void newWorkspaceAction(qc);
            }
        });
        register("menu:save", () => void saveActive());
        register("menu:run_live", () =>
            void runActive({
                sample_limit: null,
                replay: false,
                ephemeral: false,
            }),
        );
        register("menu:run_replay", () =>
            void runActive({
                sample_limit: null,
                replay: true,
                ephemeral: true,
            }),
        );
        register("menu:validate", () => void saveActive());
        register("menu:open_workspace", () => void openWorkspaceAction(qc));
        register("menu:close_workspace", () => void closeWorkspaceAction(qc));
        return () => offs.forEach((u) => u());
    }, [qc, service]);

    // Workspace lifecycle events. The commands invalidate
    // `currentWorkspace` on the calling side, but the menu path and any
    // future programmatic open/close also need to flip the App's
    // top-level branch — listening here keeps that wiring in one place.
    //
    // On close we drop every workspace-scoped query (files, runs,
    // daemon, scheduledRuns, recipe statuses) so the next open
    // refetches against the new workspace's daemon rather than serving
    // stale rows.
    useEffect(() => {
        if (!service.capabilities.workspace) return;
        const offs: (() => void)[] = [];
        offs.push(
            service.onWorkspaceOpened(() => {
                qc.invalidateQueries({ queryKey: currentWorkspaceKey() });
                qc.invalidateQueries({ queryKey: recentWorkspacesKey() });
            }),
        );
        offs.push(
            service.onWorkspaceClosed(() => {
                // Drop active editor state so a fresh workspace doesn't
                // inherit the previous one's selected file or run.
                useStudio.setState({
                    activeFilePath: null,
                    activeRunId: null,
                    selectedScheduledRunId: null,
                    source: "",
                    dirty: false,
                    validation: null,
                    snapshot: null,
                    runError: null,
                    running: false,
                    runLog: [],
                    runCounts: {},
                    paused: null,
                });
                qc.invalidateQueries({ queryKey: currentWorkspaceKey() });
                qc.removeQueries({
                    predicate: (q) =>
                        Array.isArray(q.queryKey) &&
                        (q.queryKey[0] === "files" ||
                            q.queryKey[0] === "runs" ||
                            q.queryKey[0] === "daemon" ||
                            q.queryKey[0] === "scheduledRuns" ||
                            q.queryKey[0] === "workspace" ||
                            q.queryKey[0] === "recipeStatuses"),
                });
            }),
        );
        return () => offs.forEach((u) => u());
    }, [qc, service]);
}
