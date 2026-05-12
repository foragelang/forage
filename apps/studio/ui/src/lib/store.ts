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
    setActive: (slug) =>
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
            // Breakpoints intentionally NOT cleared — they're a per-recipe
            // setting from the user's perspective, but we don't yet store
            // per-recipe; clearing on switch would be more surprising than
            // leaving them as orphans (engine never reaches a step name it
            // doesn't have, so dangling entries are harmless).
        }),
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
        const next = new Set(cur);
        if (next.has(step)) next.delete(step);
        else next.add(step);
        set({ breakpoints: next });
        // Push the new set to the backend asynchronously. The host needs
        // the latest set on every step pause; fire-and-forget is fine
        // because a missed update just means the next run pauses on a
        // stale set — the user toggles again and it converges.
        api.setBreakpoints([...next]).catch((e) =>
            console.warn("set_breakpoints failed", e),
        );
    },
    clearBreakpoints: () => {
        set({ breakpoints: new Set() });
        api.setBreakpoints([]).catch((e) =>
            console.warn("set_breakpoints failed", e),
        );
    },
}));
