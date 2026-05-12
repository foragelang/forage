//! Cross-component reactive state. Zustand for the slice that
//! TanStack Query doesn't already manage (TanStack handles recipe
//! list + load; this store holds the in-editor scratch state).

import { create } from "zustand";

import type { Snapshot, ValidationOutcome } from "./api";

export type Tab = "source" | "fixtures" | "snapshot" | "diagnostic" | "publish";

type StudioState = {
    activeSlug: string | null;
    tab: Tab;
    source: string;
    dirty: boolean;
    validation: ValidationOutcome | null;
    snapshot: Snapshot | null;
    runError: string | null;
    setActive: (slug: string | null) => void;
    setTab: (t: Tab) => void;
    setSource: (s: string) => void;
    markClean: () => void;
    setValidation: (v: ValidationOutcome | null) => void;
    setSnapshot: (s: Snapshot | null) => void;
    setRunError: (e: string | null) => void;
};

export const useStudio = create<StudioState>((set) => ({
    activeSlug: null,
    tab: "source",
    source: "",
    dirty: false,
    validation: null,
    snapshot: null,
    runError: null,
    setActive: (slug) =>
        set({
            activeSlug: slug,
            source: "",
            dirty: false,
            validation: null,
            snapshot: null,
            runError: null,
            tab: "source",
        }),
    setTab: (t) => set({ tab: t }),
    setSource: (s) =>
        set((state) => ({ source: s, dirty: state.source !== s })),
    markClean: () => set({ dirty: false }),
    setValidation: (v) => set({ validation: v }),
    setSnapshot: (s) => set({ snapshot: s }),
    setRunError: (e) => set({ runError: e }),
}));
