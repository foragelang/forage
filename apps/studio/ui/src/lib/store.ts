//! Cross-component reactive state. Zustand for the slice that
//! TanStack Query doesn't already manage (TanStack handles workspace
//! tree, runs list, and daemon status; this store holds in-editor
//! scratch state and the path-based view routing.)

import { create } from "zustand";
import { ask } from "@tauri-apps/plugin-dialog";

import {
    api,
    type PausePayload,
    type RunEvent,
    type Snapshot,
    type ValidationOutcome,
} from "./api";
import { slugOf } from "./path";

export type View = "editor" | "deployment";
export type InspectorMode = "run" | "history" | "records";

type StudioState = {
    // Top-level routing.
    view: View;
    activeFilePath: string | null;
    activeRunId: string | null;
    selectedScheduledRunId: string | null;
    inspectorMode: InspectorMode;

    // Editor session. Single-buffer mode: there is exactly one open
    // file at a time. Whether the buffer is dirty is derived from
    // `dirty` plus `activeFilePath`; no parallel "which file is
    // dirty" field is tracked.
    source: string;
    dirty: boolean;
    validation: ValidationOutcome | null;
    snapshot: Snapshot | null;
    runError: string | null;
    running: boolean;
    runLog: RunEvent[];
    runCounts: Record<string, number>;
    runStartedAt: number | null;
    // Breakpoints — step names with breakpoints set. Toggled by gutter
    // clicks in the editor pane; the backend reads the latest set on
    // every step pause to decide whether to actually wait.
    breakpoints: Set<string>;
    // Current pause payload when the engine is parked at a step or
    // inside a `for`-loop iteration, null otherwise.
    paused: PausePayload | null;
    // Whether the user has asked the engine to pause inside every
    // `for`-loop iteration. Reset to false at runBegin so a previous
    // run's setting doesn't carry over.
    pauseIterations: boolean;

    // Actions.
    setView: (v: View) => void;
    setActiveFilePath: (p: string | null) => Promise<void>;
    setActiveRunId: (id: string | null) => void;
    setSelectedScheduledRunId: (id: string | null) => void;
    setInspectorMode: (m: InspectorMode) => void;
    setSource: (s: string) => void;
    markClean: () => void;
    setValidation: (v: ValidationOutcome | null) => void;
    setSnapshot: (s: Snapshot | null) => void;
    setRunError: (e: string | null) => void;
    runBegin: () => void;
    runAppend: (e: RunEvent) => void;
    runFinish: () => void;
    debugPause: (p: PausePayload) => void;
    debugClearPause: () => void;
    toggleBreakpoint: (step: string) => void;
    clearBreakpoints: () => void;
    setPauseIterations: (enabled: boolean) => void;
};

export const useStudio = create<StudioState>((set, get) => ({
    view: "editor",
    activeFilePath: null,
    activeRunId: null,
    selectedScheduledRunId: null,
    inspectorMode: "run",

    source: "",
    dirty: false,
    validation: null,
    snapshot: null,
    runError: null,
    running: false,
    runLog: [],
    runCounts: {},
    runStartedAt: null,
    breakpoints: new Set<string>(),
    paused: null,
    pauseIterations: false,

    setView: (v) => set({ view: v }),
    setActiveRunId: (id) => set({ activeRunId: id }),
    setSelectedScheduledRunId: (id) => set({ selectedScheduledRunId: id }),
    setInspectorMode: (m) => set({ inspectorMode: m }),
    setActiveFilePath: async (path) => {
        const state = get();
        // Prompt-on-switch: single-buffer model means an unsaved
        // buffer would otherwise be silently discarded when the user
        // picks a different file. The Tauri dialog plugin only offers
        // OK / Cancel — we frame it as "save first?" so cancelling
        // keeps the user on the dirty file rather than discarding it.
        if (state.dirty && state.activeFilePath && state.activeFilePath !== path) {
            const dirtyPath = state.activeFilePath;
            const dirtySource = state.source;
            const proceed = await ask(
                `Save changes to "${dirtyPath}" before switching?`,
                {
                    title: "Unsaved changes",
                    kind: "warning",
                    okLabel: "Save and switch",
                    cancelLabel: "Cancel",
                },
            );
            if (!proceed) return;
            // Inline save — keeps the store free of an import cycle
            // against `studioActions.ts`. The guard ensures we only
            // touch the store after the save if the user hasn't moved
            // on yet (a second switch could race with the dialog).
            try {
                const v = await api.saveFile(dirtyPath, dirtySource);
                if (get().activeFilePath === dirtyPath) {
                    set({ validation: v, dirty: false });
                }
            } catch (e) {
                set({ runError: String(e) });
                return;
            }
        }
        // Reset transient editor state so a previous file's source,
        // validation, run log, etc. don't bleed across.
        set({
            activeFilePath: path,
            source: "",
            dirty: false,
            validation: null,
            snapshot: null,
            runError: null,
            running: false,
            runLog: [],
            runCounts: {},
            runStartedAt: null,
            paused: null,
            breakpoints: new Set(),
        });
        // Clear engine-side breakpoints synchronously so a fast Run
        // after the switch doesn't pause on the previous recipe's
        // steps. The per-recipe set arrives via `loadRecipeBreakpoints`
        // below and overwrites this.
        api.setBreakpoints([]).catch((e) =>
            set({ runError: `set_breakpoints failed: ${String(e)}` }),
        );
        if (path === null) return;
        // Load source for any file the user picked. Errors surface in
        // the store via setRunError; no silent swallowing.
        api.loadFile(path)
            .then((s) => {
                // Guard against a faster-arriving second selection
                // landing here before this promise resolves — only
                // populate if the path is still active.
                if (get().activeFilePath === path) {
                    set({ source: s, dirty: false });
                }
            })
            .catch((e) => set({ runError: String(e) }));
        const slug = slugOf(path);
        if (slug) {
            api.loadRecipeBreakpoints(slug)
                .then((steps) => {
                    if (get().activeFilePath === path) {
                        set({ breakpoints: new Set(steps) });
                        return api.setBreakpoints(steps);
                    }
                    return undefined;
                })
                .catch((e) =>
                    set({
                        runError: `load_recipe_breakpoints failed: ${String(e)}`,
                    }),
                );
        }
    },
    setSource: (s) =>
        set((state) => ({
            source: s,
            dirty: state.source !== s,
        })),
    markClean: () => set({ dirty: false }),
    setValidation: (v) => set({ validation: v }),
    setSnapshot: (s) => set({ snapshot: s }),
    setRunError: (e) => set({ runError: e }),
    runBegin: () =>
        set({
            running: true,
            runLog: [],
            runCounts: {},
            runStartedAt: Date.now(),
            snapshot: null,
            runError: null,
            paused: null,
            pauseIterations: false,
        }),
    runAppend: (e) =>
        set((state) => {
            const next: Partial<StudioState> = {
                runLog: [...state.runLog, e],
            };
            if (e.kind === "emitted") {
                next.runCounts = { ...state.runCounts, [e.type_name]: e.total };
            }
            return next;
        }),
    runFinish: () => set({ running: false, paused: null }),
    debugPause: (p) => set({ paused: p }),
    debugClearPause: () => set({ paused: null }),
    toggleBreakpoint: (step) => {
        const cur = get().breakpoints;
        const slug = slugOf(get().activeFilePath ?? "");
        const next = new Set(cur);
        if (next.has(step)) next.delete(step);
        else next.add(step);
        set({ breakpoints: next });
        const steps = [...next];
        if (slug) {
            api.setRecipeBreakpoints(slug, steps).catch((e) =>
                console.warn("set_recipe_breakpoints failed", e),
            );
        } else {
            api.setBreakpoints(steps).catch((e) =>
                console.warn("set_breakpoints failed", e),
            );
        }
    },
    clearBreakpoints: () => {
        set({ breakpoints: new Set() });
        const slug = slugOf(get().activeFilePath ?? "");
        if (slug) {
            api.setRecipeBreakpoints(slug, []).catch((e) =>
                console.warn("set_recipe_breakpoints failed", e),
            );
        } else {
            api.setBreakpoints([]).catch((e) =>
                console.warn("set_breakpoints failed", e),
            );
        }
    },
    setPauseIterations: (enabled) => {
        set({ pauseIterations: enabled });
        api.setPauseIterations(enabled).catch((e) =>
            console.warn("set_pause_iterations failed", e),
        );
    },
}));
