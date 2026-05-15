//! Cross-component reactive state. Zustand for the slice that
//! TanStack Query doesn't already manage (TanStack handles workspace
//! tree, runs list, and daemon status; this store holds in-editor
//! scratch state and the path-based view routing.)

import { create } from "zustand";
import type { QueryClient } from "@tanstack/react-query";

import type { PausePayload } from "../bindings/PausePayload";
import type { ProgressUnit } from "../bindings/ProgressUnit";
import type { RecipeStatus } from "../bindings/RecipeStatus";
import type { RunEvent } from "../bindings/RunEvent";
import type { Snapshot } from "../bindings/Snapshot";
import type { ValidationOutcome } from "../bindings/ValidationOutcome";
import type { StudioService } from "./services/StudioService";
import { recipeNameOf } from "./path";
import { recipeStatusesKey } from "./queryKeys";

export type View = "editor" | "deployment" | "notebook";

/// One stage in a notebook composition. The stage's identity is a
/// recipe header name — workspace-local or hub-pulled. The frontend
/// adds a synthetic `id` so React can key list rows stably across
/// reorders / duplicates of the same recipe (a notebook can compose
/// the same recipe twice in different positions).
export type NotebookStage = {
    /// React-key stable across reorders; not persisted, not sent to
    /// the backend.
    id: string;
    /// The recipe's header name. The backend command keys every
    /// composition operation on this.
    name: string;
    /// `null` when the stage references a workspace-local recipe;
    /// `@author/name` references carry the author here so the
    /// type-shaped picker can render the citation chip.
    author: string | null;
    /// The stage's resolved output type at add-time — declared `emits`
    /// when the source has one, otherwise inferred from the body.
    /// Captured so "publish notebook" can stamp the right `emits T`
    /// on the synthesized recipe (the tail stage's output flows out
    /// of the composition). `null` for recipes whose output is
    /// ambiguous (zero or many types) — the publish path falls back
    /// to a no-`emits` composition the daemon runs ephemerally.
    outputType: string | null;
};

/// Persisted notebook scratchpad. One notebook open at a time —
/// matching the editor's single-buffer model. A future "open notebook
/// from a published composition recipe" flow would replace `stages`
/// with the recipe's parsed composition body and set `name` to its
/// header name; the same shape covers both authored-here and
/// imported-from-hub modes.
export type NotebookState = {
    /// The recipe header name a "Publish notebook" would carry. The
    /// editor view's recipe-status surface reads this to gate the
    /// publish button — names that already exist as workspace recipes
    /// can't publish without a fresh name.
    name: string;
    /// Linear chain of stages, top → bottom. Stage 1 (index 0) is the
    /// source — it consumes the notebook's own `inputs` (today
    /// always empty) and emits its declared output type; every
    /// subsequent stage receives the previous stage's emissions.
    stages: NotebookStage[];
    /// Snapshot of the most recent `notebook_run`. Cleared when the
    /// user mutates `stages` so a stale preview can't hang around
    /// against a chain it no longer reflects.
    snapshot: Snapshot | null;
    /// Free-form error from the most recent run attempt, surfaced in
    /// the notebook's banner. Cleared on any stage mutation alongside
    /// `snapshot`.
    runError: string | null;
    running: boolean;
    /// Whether the "Add stage" picker dialog is open. Modal state
    /// belongs in the store rather than each component's local state
    /// so menu / keyboard shortcuts can open the picker without
    /// climbing the React tree.
    stagePickerOpen: boolean;
    /// Whether the publish dialog is open. Lives in the store for the
    /// same reason as `stagePickerOpen`.
    publishDialogOpen: boolean;
};
export type InspectorMode = "run" | "history" | "records";

/// An aggregated run of `Emitted` events between two non-emit
/// events. The engine fires one `RunEvent::Emitted` per record per
/// type (Product, then Variant, then PriceObservation, repeated per
/// inner iteration); keeping each one as a `runLog` entry would put
/// thousands of rows in the activity log. The store rolls them up
/// into bursts: each non-emit event (request, response, auth, run
/// state change) closes the current burst, and the next emit opens
/// a fresh one.
///
/// The renderer treats this as the unit of "what happened between
/// these two steps": the unit type drives the header row, child
/// types show as indented breakdown rows when the burst is
/// expanded. The `counts` map carries per-burst-local counts (so
/// "+87 Product" means 87 products in this burst, not since run
/// start); cumulative totals live in `runCounts`.
export type EmitBurst = {
    kind: "emit_burst";
    /// The recipe's progress unit type captured at burst start.
    /// Drives the header row; null when no unit is known (recipe
    /// has no for-loops that emit).
    unitType: string | null;
    /// Per-type emit count within this burst.
    counts: Record<string, number>;
    /// Source order of types as they first emitted in this burst.
    /// Renderer puts `unitType` first, then the remaining entries
    /// in this order.
    typeOrder: string[];
};

/// What sits in `runLog`. Raw engine events plus the synthetic
/// `EmitBurst` aggregator. The renderer is the only consumer that
/// cares about the distinction; `runAppend` is the only producer.
export type LogEntry = RunEvent | EmitBurst;

/// Per-step rollup derived from the engine event stream.
///
/// The engine emits step-tagged `RequestSent` / `ResponseReceived` events
/// and emit-tagged `Emitted` events. There is no `StepCompleted` event;
/// instead the store tracks the "current step" (last `RequestSent`) and
/// attributes emits to it. `done` flips to `true` when the run finishes
/// or the engine moves on to the next step.
///
/// Tone in the editor pill is computed in the consumer (EditorPane)
/// from this shape — keeping the derivation out of the reducer.
export type StepStat = {
    /// Number of HTTP requests the engine has fired for this step.
    requests: number;
    /// Number of records emitted while this step was current.
    emits: number;
    /// Wall-clock duration in ms from the first request to the most
    /// recent response. Null until at least one response arrives.
    duration_ms: number | null;
    /// True after the engine moves on to a different step, or the run
    /// finishes / fails. Once true, the step's pill stops updating.
    done: boolean;
    /// True when the run terminated with a failure while this step was
    /// current. Only the step the engine was on at `run_failed` time is
    /// marked failed; prior steps stay clean.
    failed: boolean;
    /// First-line marker — set when this step entry is created so the
    /// consumer can drop the stat if the step is missing from the new
    /// recipe outline.
    name: string;
};

type StudioState = {
    // The active StudioService — set once during app boot by main.tsx.
    // Stored here (rather than read from React Context inside actions)
    // so imperative call sites — Zustand reducers, command handlers in
    // studioActions.ts, the keyboard/menu listeners in useStudioEffects
    // — can reach the service without being React components.
    //
    // `useStudioService()` (the React-side hook) still wraps the same
    // value via Context; components that subscribe via hooks use that
    // path, and never read `service` from the store directly.
    service: StudioService;

    // The React Query client, installed at boot alongside the service.
    // Store reducers use it to look up the recipe-name for a path
    // (`recipeStatusesKey`); without it, breakpoint loading on file
    // switch would have nowhere to fetch the workspace recipes.
    queryClient: QueryClient | null;

    // Top-level routing.
    view: View;
    activeFilePath: string | null;
    /// Recipe header name at `activeFilePath`, or null when the active
    /// path has no parsed recipe (header-less declarations file,
    /// fixture, snapshot, broken file). Derived from the
    /// recipe-statuses query cache at the time `setActiveFilePath` /
    /// `setActiveRecipeName` runs; components that need the live join
    /// (so a freshly-arrived cache update propagates without another
    /// path switch) keep using `useRecipeNameOf`.
    activeRecipeName: string | null;
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
    runLog: LogEntry[];
    runCounts: Record<string, number>;
    runStartedAt: number | null;
    /// Inferred from the active recipe by `infer_progress_unit`. When
    /// set, `runAppend` filters the activity log to only show emit
    /// events of `unit.types[0]` — Variant/PriceObservation emits in
    /// a Product-unit recipe still increment `runCounts` but don't
    /// clutter the activity stream. Null when no recipe is active or
    /// the recipe has no emits.
    progressUnit: ProgressUnit | null;
    /// Per-step rollup tagged by `RequestSent.step`. Cleared on
    /// `runBegin` so a previous run's pills don't bleed through.
    stepStats: Record<string, StepStat>;
    /// Name of the step that's currently emitting requests/responses.
    /// Used by `runAppend` to attribute `Emitted` events back to a step
    /// because the engine doesn't tag emits with a step name.
    currentStep: string | null;
    /// First request `RequestSent` timestamps per step, in ms since
    /// epoch. Used to compute durations on `ResponseReceived`.
    stepStartMs: Record<string, number>;
    /// Resolved toolbar run-flag state. The preset selector and the
    /// per-flag toggles in the editor toolbar bind to this; the
    /// editor "Run" button reads it at click time and ships the
    /// resolved values to the backend. Defaults match the dev preset
    /// (sample 10, replay, ephemeral) so a fresh Studio session
    /// behaves like a playground.
    runFlags: {
        sample_limit: number | null;
        replay: boolean;
        ephemeral: boolean;
    };
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

    // ── Notebook scratchpad ─────────────────────────────────────────
    notebook: NotebookState;

    // Actions.
    setView: (v: View) => void;
    setActiveFilePath: (p: string | null) => Promise<void>;
    /// Open a recipe by header name. Looks up its draft path in the
    /// recipe-statuses cache and routes through `setActiveFilePath` so
    /// every prompt-on-dirty-switch / breakpoint-load side effect
    /// runs. The optional `path` overrides the cache lookup — the
    /// sidebar's Recipes section passes both so a click works even if
    /// the cache hasn't propagated the latest entry.
    setActiveRecipeName: (name: string, path?: string) => Promise<void>;
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
    setProgressUnit: (unit: ProgressUnit | null) => void;
    debugPause: (p: PausePayload) => void;
    debugClearPause: () => void;
    toggleBreakpoint: (step: string) => void;
    clearBreakpoints: () => void;
    setPauseIterations: (enabled: boolean) => void;
    /// Update one or more run-flag fields. The toolbar's preset
    /// selector calls this with the resolved values for the selected
    /// preset; per-flag toggles call this with a single field.
    setRunFlags: (
        patch: Partial<{
            sample_limit: number | null;
            replay: boolean;
            ephemeral: boolean;
        }>,
    ) => void;

    // ── Notebook actions ────────────────────────────────────────────
    /// Rename the notebook. The new name becomes the recipe header
    /// when "Publish notebook" lands the chain as a `.forage` file.
    setNotebookName: (name: string) => void;
    /// Append `(name, author, outputType)` to the chain. `author` is
    /// `null` for workspace-local recipes; non-null for hub-pulled
    /// references. `outputType` is the stage's declared output type
    /// captured at add-time so a later "publish notebook" can stamp
    /// the recipe with the tail stage's output without a re-fetch.
    addNotebookStage: (
        name: string,
        author: string | null,
        outputType: string | null,
    ) => void;
    /// Remove the stage at `index`. No-op when the index is out of
    /// range. Removing any stage clears `snapshot` — the prior
    /// preview no longer corresponds to the new chain.
    removeNotebookStage: (index: number) => void;
    /// Swap the stages at `from` and `to`. The chain runs top to
    /// bottom, so reordering changes which recipe feeds which.
    moveNotebookStage: (from: number, to: number) => void;
    /// Start of a notebook run — flips `running` true and clears any
    /// prior snapshot/error so the inspector shows a fresh state.
    notebookRunBegin: () => void;
    /// End of a notebook run with the resulting snapshot or error.
    notebookRunFinish: (
        result: { snapshot: Snapshot } | { error: string },
    ) => void;
    /// Clear the entire notebook back to "fresh notebook" defaults.
    /// Used by "New notebook" and at workspace close.
    resetNotebook: () => void;
    openStagePicker: () => void;
    closeStagePicker: () => void;
    /// Open the publish dialog. The dialog reads the notebook's
    /// current `(name, stages)` at submit time.
    openPublishDialog: () => void;
    closePublishDialog: () => void;
};

/// Fresh-notebook shape. Used both at boot and by `resetNotebook`.
const EMPTY_NOTEBOOK: NotebookState = {
    name: "untitled-notebook",
    stages: [],
    snapshot: null,
    runError: null,
    running: false,
    stagePickerOpen: false,
    publishDialogOpen: false,
};

/// Stable per-stage React key. The notebook can compose the same
/// recipe twice; identifying stages by `name` alone would make those
/// rows share a key and confuse list reconciliation on drag-reorder.
let nextStageId = 0;
function freshStageId(): string {
    nextStageId += 1;
    return `stage-${nextStageId}`;
}

/// Look up the recipe header name for a workspace-relative `path`
/// against the recipe-statuses query cache. Returns null when the
/// cache hasn't populated yet, the path isn't a parsed recipe, or no
/// QueryClient is installed. Callers that need the name (breakpoint
/// load on file switch, etc.) treat a null result as "this path
/// isn't recipe-scoped" and skip the recipe-keyed call.
function recipeNameForPath(state: StudioState, path: string): string | null {
    const recipes = state.queryClient?.getQueryData<RecipeStatus[]>(recipeStatusesKey());
    return recipeNameOf(path, recipes);
}

/// Reverse of `recipeNameForPath`: look up the on-disk draft path for
/// a recipe by header name. Used by `setActiveRecipeName` when the
/// caller didn't pass a path explicitly — only `valid` drafts have a
/// path (broken / missing entries return null).
function pathForRecipeName(state: StudioState, name: string): string | null {
    const recipes = state.queryClient?.getQueryData<RecipeStatus[]>(recipeStatusesKey());
    if (!recipes) return null;
    for (const r of recipes) {
        if (r.name === name && r.draft.kind === "valid") return r.draft.path;
    }
    return null;
}

/// Shared path-switch reducer routed through by both
/// `setActiveFilePath` and `setActiveRecipeName`. The explicit
/// `forcedRecipeName` lets `setActiveRecipeName` win the race when
/// the recipe-statuses cache hasn't yet populated the entry the
/// caller is selecting against; passing `null` falls back to the
/// cache lookup. Returns once the synchronous state writes are done;
/// async side effects (loadFile, loadRecipeBreakpoints) continue in
/// the background.
async function switchActiveTarget(
    get: () => StudioState,
    set: (partial: Partial<StudioState>) => void,
    path: string | null,
    forcedRecipeName: string | null,
): Promise<void> {
    const state = get();
    const service = state.service;
    // Prompt-on-switch: single-buffer model means an unsaved buffer
    // would otherwise be silently discarded when the user picks a
    // different file. The host's confirm dialog only offers OK /
    // Cancel — we frame it as "save first?" so cancelling keeps the
    // user on the dirty file rather than discarding it.
    if (state.dirty && state.activeFilePath && state.activeFilePath !== path) {
        const dirtyPath = state.activeFilePath;
        const dirtySource = state.source;
        const proceed = await service.confirm(
            `Save changes to "${dirtyPath}" before switching?`,
            {
                title: "Unsaved changes",
                okLabel: "Save and switch",
                cancelLabel: "Cancel",
            },
        );
        if (!proceed) return;
        try {
            const v = await service.saveFile(dirtyPath, dirtySource);
            if (get().activeFilePath === dirtyPath) {
                set({ validation: v, dirty: false });
            }
        } catch (e) {
            set({ runError: String(e) });
            return;
        }
    }
    const name = forcedRecipeName ?? (path ? recipeNameForPath(state, path) : null);
    set({
        activeFilePath: path,
        activeRecipeName: name,
        source: "",
        dirty: false,
        validation: null,
        snapshot: null,
        runError: null,
        running: false,
        runLog: [],
        runCounts: {},
        runStartedAt: null,
        progressUnit: null,
        stepStats: {},
        currentStep: null,
        stepStartMs: {},
        paused: null,
        breakpoints: new Set(),
    });
    // Clear engine-side breakpoints synchronously so a fast Run after
    // the switch doesn't pause on the previous recipe's steps. The
    // per-recipe set arrives via `loadRecipeBreakpoints` below and
    // overwrites this.
    service.setBreakpoints([]).catch((e) =>
        set({ runError: `set_breakpoints failed: ${String(e)}` }),
    );
    if (path === null) return;
    service.loadFile(path)
        .then((s) => {
            if (get().activeFilePath === path) {
                set({ source: s, dirty: false });
            }
        })
        .catch((e) => set({ runError: String(e) }));
    if (name) {
        service.loadRecipeBreakpoints(name)
            .then((steps) => {
                if (get().activeFilePath === path) {
                    set({ breakpoints: new Set(steps) });
                    return service.setBreakpoints(steps);
                }
                return undefined;
            })
            .catch((e) =>
                set({
                    runError: `load_recipe_breakpoints failed: ${String(e)}`,
                }),
            );
    }
}

/// Placeholder service used before `installStudioService` runs. Every
/// method throws; tests and the boot path must replace it. Keeps the
/// store typing honest — no `Optional<StudioService>` to thread.
const UNINSTALLED_SERVICE: StudioService = new Proxy({} as StudioService, {
    get(_target, prop) {
        if (prop === "capabilities") {
            return { workspace: false, deploy: false, liveRun: false, hubPackages: false };
        }
        return () => {
            throw new Error(
                `StudioService not installed (called .${String(prop)}). Wrap the app in installStudioService(service).`,
            );
        };
    },
});

export const useStudio = create<StudioState>((set, get) => ({
    service: UNINSTALLED_SERVICE,
    queryClient: null,
    view: "editor",
    activeFilePath: null,
    activeRecipeName: null,
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
    progressUnit: null,
    stepStats: {},
    currentStep: null,
    stepStartMs: {},
    breakpoints: new Set<string>(),
    paused: null,
    pauseIterations: false,
    runFlags: {
        sample_limit: 10,
        replay: true,
        ephemeral: true,
    },
    notebook: EMPTY_NOTEBOOK,

    setView: (v) => set({ view: v }),
    setActiveRunId: (id) => set({ activeRunId: id }),
    setSelectedScheduledRunId: (id) => set({ selectedScheduledRunId: id }),
    setInspectorMode: (m) => set({ inspectorMode: m }),
    setActiveFilePath: async (path) => {
        await switchActiveTarget(get, set, path, null);
    },
    setActiveRecipeName: async (name, path) => {
        // When the sidebar's Recipes section calls this, it already
        // has the recipe's draft path in hand; pass it through so the
        // editor opens even if the recipe-statuses cache hasn't
        // surfaced this entry yet. Fall back to the cache when no
        // explicit path was provided.
        const resolved = path ?? pathForRecipeName(get(), name);
        if (!resolved) {
            set({
                runError: `setActiveRecipeName: no path for recipe "${name}"`,
            });
            return;
        }
        await switchActiveTarget(get, set, resolved, name);
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
            stepStats: {},
            currentStep: null,
            stepStartMs: {},
            snapshot: null,
            runError: null,
            paused: null,
            pauseIterations: false,
        }),
    runAppend: (e) =>
        set((state) => {
            // Activity-log shape: emit events roll up into burst
            // entries; non-emit events end the current burst and get
            // pushed as themselves. A burst entry carries per-type
            // counts (Product 87, Variant 87, PriceObservation 87 in
            // the zen-leaf-elkridge case), so the renderer can show
            // the unit type as the header and children as an
            // expandable breakdown.
            const log = state.runLog;
            const last = log[log.length - 1];

            let nextLog: LogEntry[];
            if (e.kind === "emitted") {
                if (last?.kind === "emit_burst") {
                    // Aggregate into the running burst.
                    const prev = last.counts[e.type_name] ?? 0;
                    const burst: EmitBurst = {
                        kind: "emit_burst",
                        unitType: last.unitType,
                        counts: {
                            ...last.counts,
                            [e.type_name]: prev + 1,
                        },
                        typeOrder: last.typeOrder.includes(e.type_name)
                            ? last.typeOrder
                            : [...last.typeOrder, e.type_name],
                    };
                    nextLog = [...log.slice(0, -1), burst];
                } else {
                    // Open a fresh burst. Unit type is captured at
                    // burst-start so a mid-run recipe edit can't
                    // change the framing of an in-progress burst.
                    const burst: EmitBurst = {
                        kind: "emit_burst",
                        unitType: state.progressUnit?.types[0] ?? null,
                        counts: { [e.type_name]: 1 },
                        typeOrder: [e.type_name],
                    };
                    nextLog = [...log, burst];
                }
            } else {
                nextLog = [...log, e];
            }
            const next: Partial<StudioState> = { runLog: nextLog };
            // Derive per-step stats from the event stream. The engine
            // doesn't emit a `StepCompleted` variant; instead we treat
            // the last `RequestSent` as the "current step" and credit
            // emits to it. This is best-effort but matches the way the
            // engine actually drives runs: steps are sequential, and
            // emits inside `for $i in $step[*] { emit … }` always follow
            // their step's responses.
            switch (e.kind) {
                case "request_sent": {
                    const prev = state.stepStats[e.step];
                    const nowMs = Date.now();
                    const startMs = state.stepStartMs[e.step] ?? nowMs;
                    next.stepStats = {
                        ...state.stepStats,
                        [e.step]: {
                            name: e.step,
                            requests: (prev?.requests ?? 0) + 1,
                            emits: prev?.emits ?? 0,
                            duration_ms: prev?.duration_ms ?? null,
                            done: false,
                            failed: false,
                        },
                    };
                    next.stepStartMs = {
                        ...state.stepStartMs,
                        [e.step]: startMs,
                    };
                    // Stepping onto a new step closes out the previous.
                    if (
                        state.currentStep !== null &&
                        state.currentStep !== e.step &&
                        state.stepStats[state.currentStep]
                    ) {
                        next.stepStats[state.currentStep] = {
                            ...state.stepStats[state.currentStep]!,
                            done: true,
                        };
                    }
                    next.currentStep = e.step;
                    break;
                }
                case "response_received": {
                    const start = state.stepStartMs[e.step];
                    const prev = state.stepStats[e.step];
                    if (prev) {
                        next.stepStats = {
                            ...state.stepStats,
                            [e.step]: {
                                ...prev,
                                duration_ms:
                                    start !== undefined
                                        ? Date.now() - start
                                        : prev.duration_ms,
                            },
                        };
                    }
                    break;
                }
                case "emitted": {
                    next.runCounts = {
                        ...state.runCounts,
                        [e.type_name]: e.total,
                    };
                    const cur = state.currentStep;
                    if (cur && state.stepStats[cur]) {
                        next.stepStats = {
                            ...(next.stepStats ?? state.stepStats),
                            [cur]: {
                                ...(next.stepStats?.[cur] ?? state.stepStats[cur]!),
                                emits:
                                    (next.stepStats?.[cur]?.emits ??
                                        state.stepStats[cur]!.emits) + 1,
                            },
                        };
                    }
                    break;
                }
                case "run_succeeded": {
                    // Freeze the final step.
                    if (state.currentStep && state.stepStats[state.currentStep]) {
                        next.stepStats = {
                            ...state.stepStats,
                            [state.currentStep]: {
                                ...state.stepStats[state.currentStep]!,
                                done: true,
                            },
                        };
                    }
                    break;
                }
                case "run_failed": {
                    // Freeze the final step and mark it failed. Only the
                    // step the engine was on when the run failed gets the
                    // red pill — prior steps did complete successfully.
                    if (state.currentStep && state.stepStats[state.currentStep]) {
                        next.stepStats = {
                            ...state.stepStats,
                            [state.currentStep]: {
                                ...state.stepStats[state.currentStep]!,
                                done: true,
                                failed: true,
                            },
                        };
                    }
                    break;
                }
            }
            return next;
        }),
    runFinish: () =>
        set((state) => {
            // Mark every step done — defensive in case `runFinish` is
            // called without a preceding `run_succeeded`/`run_failed`
            // event (e.g. a Tauri command error before the engine fires).
            const stepStats = { ...state.stepStats };
            for (const k of Object.keys(stepStats)) {
                stepStats[k] = { ...stepStats[k]!, done: true };
            }
            return { running: false, paused: null, stepStats };
        }),
    setProgressUnit: (unit) => set({ progressUnit: unit }),
    debugPause: (p) => set({ paused: p }),
    debugClearPause: () => set({ paused: null }),
    toggleBreakpoint: (step) => {
        const state = get();
        const { service, activeFilePath } = state;
        const cur = state.breakpoints;
        const name = activeFilePath ? recipeNameForPath(state, activeFilePath) : null;
        const next = new Set(cur);
        if (next.has(step)) next.delete(step);
        else next.add(step);
        set({ breakpoints: next });
        const steps = [...next];
        if (name) {
            service.setRecipeBreakpoints(name, steps).catch((e) =>
                console.warn("set_recipe_breakpoints failed", e),
            );
        } else {
            service.setBreakpoints(steps).catch((e) =>
                console.warn("set_breakpoints failed", e),
            );
        }
    },
    clearBreakpoints: () => {
        const state = get();
        const { service, activeFilePath } = state;
        set({ breakpoints: new Set() });
        const name = activeFilePath ? recipeNameForPath(state, activeFilePath) : null;
        if (name) {
            service.setRecipeBreakpoints(name, []).catch((e) =>
                console.warn("set_recipe_breakpoints failed", e),
            );
        } else {
            service.setBreakpoints([]).catch((e) =>
                console.warn("set_breakpoints failed", e),
            );
        }
    },
    setPauseIterations: (enabled) => {
        const service = get().service;
        set({ pauseIterations: enabled });
        service.setPauseIterations(enabled).catch((e) =>
            console.warn("set_pause_iterations failed", e),
        );
    },
    setRunFlags: (patch) =>
        set((state) => ({
            runFlags: { ...state.runFlags, ...patch },
        })),

    // ── Notebook actions ────────────────────────────────────────────
    setNotebookName: (name) =>
        set((state) => ({ notebook: { ...state.notebook, name } })),
    addNotebookStage: (name, author, outputType) =>
        set((state) => ({
            notebook: {
                ...state.notebook,
                stages: [
                    ...state.notebook.stages,
                    { id: freshStageId(), name, author, outputType },
                ],
                snapshot: null,
                runError: null,
            },
        })),
    removeNotebookStage: (index) =>
        set((state) => {
            if (index < 0 || index >= state.notebook.stages.length) {
                return {};
            }
            const next = state.notebook.stages.slice();
            next.splice(index, 1);
            return {
                notebook: {
                    ...state.notebook,
                    stages: next,
                    snapshot: null,
                    runError: null,
                },
            };
        }),
    moveNotebookStage: (from, to) =>
        set((state) => {
            const len = state.notebook.stages.length;
            if (
                from === to ||
                from < 0 ||
                from >= len ||
                to < 0 ||
                to >= len
            ) {
                return {};
            }
            const next = state.notebook.stages.slice();
            const [moved] = next.splice(from, 1);
            if (!moved) return {};
            next.splice(to, 0, moved);
            return {
                notebook: {
                    ...state.notebook,
                    stages: next,
                    snapshot: null,
                    runError: null,
                },
            };
        }),
    notebookRunBegin: () =>
        set((state) => ({
            notebook: {
                ...state.notebook,
                running: true,
                snapshot: null,
                runError: null,
            },
        })),
    notebookRunFinish: (result) =>
        set((state) => ({
            notebook: {
                ...state.notebook,
                running: false,
                snapshot: "snapshot" in result ? result.snapshot : null,
                runError: "error" in result ? result.error : null,
            },
        })),
    resetNotebook: () => set({ notebook: EMPTY_NOTEBOOK }),
    openStagePicker: () =>
        set((state) => ({
            notebook: { ...state.notebook, stagePickerOpen: true },
        })),
    closeStagePicker: () =>
        set((state) => ({
            notebook: { ...state.notebook, stagePickerOpen: false },
        })),
    openPublishDialog: () =>
        set((state) => ({
            notebook: { ...state.notebook, publishDialogOpen: true },
        })),
    closePublishDialog: () =>
        set((state) => ({
            notebook: { ...state.notebook, publishDialogOpen: false },
        })),
}));

/// Install a concrete service + QueryClient into the global store.
/// Both the Tauri main.tsx and the hub IDE's main.tsx call this
/// before mounting the React tree. Idempotent: re-installing
/// replaces the prior pair. The QueryClient gives store reducers
/// access to the workspace-recipes cache for path → recipe-name
/// lookups; without it, breakpoint loading on file switch can't
/// resolve which recipe to fetch.
export function installStudioService(
    service: StudioService,
    queryClient: QueryClient,
): void {
    useStudio.setState({ service, queryClient });
}
