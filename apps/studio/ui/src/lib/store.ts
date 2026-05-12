//! Cross-component reactive state. Zustand for the slice that
//! TanStack Query doesn't already manage (TanStack handles recipe
//! list + load; this store holds the in-editor scratch state).

import { create } from "zustand";

import { api, type RunEvent, type Snapshot, type StepPause, type ValidationOutcome } from "./api";

export type Tab = "source" | "fixtures" | "snapshot" | "diagnostic" | "publish";

type StudioState = {
    activeSlug: string | null;
    tab: Tab;
    source: string;
    dirty: boolean;
    validation: ValidationOutcome | null;
    snapshot: Snapshot | null;
    runError: string | null;
    // Live-run progress. `running` is true between RunStarted and
    // RunSucceeded/RunFailed; `runLog` accumulates events for display in
    // the Snapshot tab; `runCounts` is the running per-type emit total;
    // `runStartedAt` is wall-clock ms used to drive the elapsed timer.
    running: boolean;
    runLog: RunEvent[];
    runCounts: Record<string, number>;
    runStartedAt: number | null;
    // Breakpoints — step names with breakpoints set. Toggled by gutter
    // clicks in the Source tab; the backend reads the latest set on every
    // step pause to decide whether to actually wait.
    breakpoints: Set<string>;
    // Current pause payload when the engine is parked at a step, null
    // otherwise. A pause is in-flight iff `paused !== null`.
    paused: StepPause | null;
    setActive: (slug: string | null) => void;
    setTab: (t: Tab) => void;
    setSource: (s: string) => void;
    markClean: () => void;
    setValidation: (v: ValidationOutcome | null) => void;
    setSnapshot: (s: Snapshot | null) => void;
    setRunError: (e: string | null) => void;
    runBegin: () => void;
    runAppend: (e: RunEvent) => void;
    runFinish: () => void;
    debugPause: (p: StepPause) => void;
    debugClearPause: () => void;
    toggleBreakpoint: (step: string) => void;
    clearBreakpoints: () => void;
};

export const useStudio = create<StudioState>((set, get) => ({
    activeSlug: null,
    tab: "source",
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
    setActive: (slug) => {
        set({
            activeSlug: slug,
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
            tab: "source",
            // Replace the in-memory breakpoint set with the new slug's
            // persisted set. Until the backend round-trip completes the
            // set is empty — the previous recipe's breakpoints can't
            // bleed into the new one.
            breakpoints: new Set(),
        });
        // Pull this recipe's persisted breakpoints from the library
        // sidecar and push them to the in-memory cache the engine
        // reads on pause. Fire-and-forget — if it fails, the user just
        // doesn't see saved breakpoints for that recipe until they
        // re-toggle.
        if (slug) {
            api.loadRecipeBreakpoints(slug)
                .then((steps) => {
                    set({ breakpoints: new Set(steps) });
                    return api.setBreakpoints(steps);
                })
                .catch((e) =>
                    console.warn("load_recipe_breakpoints failed", e),
                );
        } else {
            api.setBreakpoints([]).catch(() => {});
        }
    },
    setTab: (t) => set({ tab: t }),
    setSource: (s) =>
        set((state) => ({ source: s, dirty: state.source !== s })),
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
        const slug = get().activeSlug;
        const next = new Set(cur);
        if (next.has(step)) next.delete(step);
        else next.add(step);
        set({ breakpoints: next });
        // Persist + push the new set. The recipe-scoped command writes
        // through the library sidecar AND updates the engine's
        // in-memory cache, so one command covers both. When there's no
        // active recipe, fall back to the in-memory-only path — there's
        // nowhere to persist to.
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
        const slug = get().activeSlug;
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
}));
