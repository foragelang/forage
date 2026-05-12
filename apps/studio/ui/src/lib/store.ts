//! Cross-component reactive state. Zustand for the slice that
//! TanStack Query doesn't already manage (TanStack handles recipe
//! list + load; this store holds the in-editor scratch state).

import { create } from "zustand";

import type { RunEvent, Snapshot, StepPause, ValidationOutcome } from "./api";

export type Tab =
    | "source"
    | "fixtures"
    | "snapshot"
    | "diagnostic"
    | "debugger"
    | "publish";

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
    // Debugger state. `debugging` is true for the whole debug run;
    // `paused` is the current pause payload when the engine is waiting,
    // null otherwise. We don't drive a separate "is paused" boolean —
    // `paused !== null` is the source of truth.
    debugging: boolean;
    paused: StepPause | null;
    setActive: (slug: string | null) => void;
    setTab: (t: Tab) => void;
    setSource: (s: string) => void;
    markClean: () => void;
    setValidation: (v: ValidationOutcome | null) => void;
    setSnapshot: (s: Snapshot | null) => void;
    setRunError: (e: string | null) => void;
    runBegin: (opts?: { debug?: boolean }) => void;
    runAppend: (e: RunEvent) => void;
    runFinish: () => void;
    debugPause: (p: StepPause) => void;
    debugClearPause: () => void;
};

export const useStudio = create<StudioState>((set) => ({
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
    debugging: false,
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
            debugging: false,
            paused: null,
            tab: "source",
        }),
    setTab: (t) => set({ tab: t }),
    setSource: (s) =>
        set((state) => ({ source: s, dirty: state.source !== s })),
    markClean: () => set({ dirty: false }),
    setValidation: (v) => set({ validation: v }),
    setSnapshot: (s) => set({ snapshot: s }),
    setRunError: (e) => set({ runError: e }),
    runBegin: (opts) =>
        set({
            running: true,
            runLog: [],
            runCounts: {},
            runStartedAt: Date.now(),
            snapshot: null,
            runError: null,
            debugging: !!opts?.debug,
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
    runFinish: () => set({ running: false, debugging: false, paused: null }),
    debugPause: (p) => set({ paused: p }),
    debugClearPause: () => set({ paused: null }),
}));
