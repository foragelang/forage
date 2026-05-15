//! Top-level cross-cutting effects: the engine event streams, the
//! daemon completion stream, keyboard shortcuts, and the native menu
//! event listeners. Mounted exactly once from `App.tsx` so the rest
//! of the tree doesn't need to know any of this exists.

import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";

import type { DebugAction } from "@/lib/services";
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

function isDebuggerKey(key: string): boolean {
    return key === "F5" || key === "F10" || key === "F11";
}

/// Map a keydown event to a `DebugAction`. F5 = continue (Shift+F5 =
/// stop), F10 = step over, F11 = step in. Other keys (or
/// modifier-only F5 like Cmd+F5) fall through to null and the
/// listener doesn't fire.
function debugActionFor(e: KeyboardEvent): DebugAction | null {
    if (e.key === "F5") return e.shiftKey ? "stop" : "continue";
    if (e.key === "F10") return "step_over";
    if (e.key === "F11") return "step_in";
    return null;
}

/// True when the user is focused inside a form control, contenteditable,
/// or the Monaco editor surface. Those contexts swallow the F-keys for
/// their own use (e.g. Monaco's command palette on F1); guarding here
/// keeps the global shortcut from stealing keypresses meant for the
/// editor.
function isTypingInInput(): boolean {
    const el = document.activeElement;
    if (!el) return false;
    const tag = el.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
    if ((el as HTMLElement).isContentEditable) return true;
    // Monaco's editing surface mounts a `.monaco-editor` ancestor on
    // the focused element; walk up cheaply to detect it without
    // relying on Monaco's API.
    let walker: Element | null = el;
    while (walker) {
        if (walker.classList?.contains("monaco-editor")) return true;
        walker = walker.parentElement;
    }
    return false;
}

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

    // Debug pause stream — fires when the engine parks at a
    // step / emit / for-loop pause site.
    useEffect(() => {
        const off = service.onDebugPaused((p) => {
            useStudio.getState().debugPause(p);
        });
        return off;
    }, [service]);

    // Run begin — broadcasts the freshly-minted run id. The store
    // resets `lastResponses` and `runId` at runBegin (the toolbar's
    // "Run" path) and the event fires right after to install the
    // matching id. The Inspector's Responses pane / pop-out window
    // use the id to scope the "load full" command.
    useEffect(() => {
        const off = service.onRunBegin((event) => {
            useStudio.getState().resetRunResponses();
            useStudio.getState().setRunId(event.run_id);
        });
        return off;
    }, [service]);

    // Per-step response stream — populates `lastResponses` so the
    // Inspector's Responses tab and the pop-out window can render
    // captures independent of pause state. Includes 4xx/5xx
    // responses captured before the engine aborts.
    useEffect(() => {
        const off = service.onStepResponse((event) => {
            useStudio.getState().setStepResponse(event.step, event.response);
        });
        return off;
    }, [service]);

    // Debug resumed — symmetric with onDebugPaused for the pop-out
    // window and any other surface that needs to clear pause-time UI
    // without waiting for the eventual success / failure event. The
    // main window's `DebuggerPanel` clears `paused` synchronously
    // when the user clicks Resume; the event is the safety net for
    // every other surface.
    useEffect(() => {
        const off = service.onDebugResumed(() => {
            useStudio.getState().debugClearPause();
        });
        return off;
    }, [service]);

    // F10 / F11 / F5 / Shift+F5 — debugger shortcuts. The
    // `DebuggerPanel` tooltips already advertise these; the listener
    // is mounted unconditionally (it's a global keymap) and guards
    // against firing when the user is typing into a form control or
    // the Monaco editor. `useStudio.getState()` reads paused without
    // adding a subscription — we only need it inside the handler.
    useEffect(() => {
        const handler = (e: KeyboardEvent) => {
            if (!isDebuggerKey(e.key)) return;
            if (isTypingInInput()) return;
            const paused = useStudio.getState().paused;
            if (paused === null) return;
            e.preventDefault();
            const action = debugActionFor(e);
            if (action === null) return;
            useStudio.getState().debugClearPause();
            service.debugResume(action).catch((err) =>
                console.warn("debug resume failed", err),
            );
        };
        window.addEventListener("keydown", handler);
        return () => window.removeEventListener("keydown", handler);
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
                useStudio.getState().resetNotebook();
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
