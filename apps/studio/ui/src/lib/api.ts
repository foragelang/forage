//! Typed wrappers around Tauri commands.
//!
//! Cross-wire payload types are *generated* from Rust by ts-rs (see the
//! sibling `bindings/` directory). Do not redefine them here — extend
//! the Rust definition and run `cargo test` to refresh the .ts file.
//! This file only owns the `invoke()` shims, command names, event
//! constants, and TS-only conveniences.

import { invoke } from "@tauri-apps/api/core";

import type { Cadence } from "../bindings/Cadence";
import type { DaemonStatus } from "../bindings/DaemonStatus";
import type { DebugFrame } from "../bindings/DebugFrame";
import type { DebugScope } from "../bindings/DebugScope";
import type { Diagnostic } from "../bindings/Diagnostic";
import type { DiagnosticReport } from "../bindings/DiagnosticReport";
import type { FileNode } from "../bindings/FileNode";
import type { Health } from "../bindings/Health";
import type { HoverInfo } from "../bindings/HoverInfo";
import type { IterationPause } from "../bindings/IterationPause";
import type { LanguageDictionary } from "../bindings/LanguageDictionary";
import type { Outcome } from "../bindings/Outcome";
import type { PausePayload } from "../bindings/PausePayload";
import type { RecipeRecord } from "../bindings/RecipeRecord";
import type { ProgressUnit } from "../bindings/ProgressUnit";
import type { RecipeOutline } from "../bindings/RecipeOutline";
import type { ResumeAction } from "../bindings/ResumeAction";
import type { Run } from "../bindings/Run";
import type { RunConfig } from "../bindings/RunConfig";
import type { RunEvent } from "../bindings/RunEvent";
import type { RunOutcome } from "../bindings/RunOutcome";
import type { ScheduledRun } from "../bindings/ScheduledRun";
import type { Snapshot } from "../bindings/Snapshot";
import type { StepLocation } from "../bindings/StepLocation";
import type { StepPause } from "../bindings/StepPause";
import type { TimeUnit } from "../bindings/TimeUnit";
import type { Trigger } from "../bindings/Trigger";
import type { ValidationOutcome } from "../bindings/ValidationOutcome";
import type { WorkspaceInfo } from "../bindings/WorkspaceInfo";

// Re-export for the rest of the UI. Importing from "lib/api" rather than
// directly from "bindings/…" keeps the call sites stable if bindings move.
export type {
    Cadence,
    DaemonStatus,
    DebugFrame,
    DebugScope,
    Diagnostic,
    DiagnosticReport,
    FileNode,
    Health,
    HoverInfo,
    IterationPause,
    LanguageDictionary,
    Outcome,
    PausePayload,
    ProgressUnit,
    RecipeOutline,
    RecipeRecord,
    ResumeAction,
    Run,
    RunConfig,
    RunEvent,
    RunOutcome,
    ScheduledRun,
    Snapshot,
    StepLocation,
    StepPause,
    TimeUnit,
    Trigger,
    ValidationOutcome,
    WorkspaceInfo,
};

// `DebugAction` is a Studio-only TS alias for the resume verbs sent
// back to the engine. The Rust side is `ResumeAction` (a Rust enum); on
// the wire we serialize one of three strings, so the frontend works
// with the string union directly rather than mirroring the enum shape.
export type DebugAction = "continue" | "step_over" | "stop";

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

const HUB = "https://api.foragelang.com";

/// Tauri event name the engine emits run progress through. Matches the
/// Rust-side `commands::RUN_EVENT` constant.
export const RUN_EVENT = "forage:run-event";
/// Tauri event name the engine emits when paused — at a step boundary
/// or inside a for-loop iteration. Matches `commands::DEBUG_PAUSED_EVENT`.
export const DEBUG_PAUSED_EVENT = "forage:debug-paused";

export const api = {
    version: () => invoke<string>("studio_version"),
    currentWorkspace: () => invoke<WorkspaceInfo>("current_workspace"),
    listWorkspaceFiles: () => invoke<FileNode>("list_workspace_files"),
    loadFile: (path: string) => invoke<string>("load_file", { path }),
    saveFile: (path: string, source: string) =>
        invoke<ValidationOutcome>("save_file", { path, source }),
    validateRecipe: (source: string) =>
        invoke<ValidationOutcome>("validate_recipe", { source }),
    recipeProgressUnit: (slug: string) =>
        invoke<ProgressUnit | null>("recipe_progress_unit", { slug }),
    createRecipe: () => invoke<string>("create_recipe"),
    deleteRecipe: (slug: string) => invoke<void>("delete_recipe", { slug }),
    runRecipe: (slug: string, replay: boolean) =>
        invoke<RunOutcome>("run_recipe", { slug, replay }),
    cancelRun: () => invoke<void>("cancel_run"),
    debugResume: (action: DebugAction) =>
        invoke<void>("debug_resume", { action }),
    setPauseIterations: (enabled: boolean) =>
        invoke<void>("set_pause_iterations", { enabled }),
    setBreakpoints: (steps: string[]) =>
        invoke<void>("set_breakpoints", { steps }),
    setRecipeBreakpoints: (slug: string, steps: string[]) =>
        invoke<void>("set_recipe_breakpoints", { slug, steps }),
    loadRecipeBreakpoints: (slug: string) =>
        invoke<string[]>("load_recipe_breakpoints", { slug }),
    recipeOutline: (source: string) =>
        invoke<RecipeOutline>("recipe_outline", { source }),
    recipeHover: (source: string, line: number, col: number) =>
        invoke<HoverInfo | null>("recipe_hover", { source, line, col }),
    languageDictionary: () =>
        invoke<LanguageDictionary>("language_dictionary"),
    publishRecipe: (slug: string, hubUrl: string = HUB, dryRun = true) =>
        invoke<RunOutcome>("publish_recipe", { slug, hubUrl, dryRun }),
    authWhoami: (hubUrl: string = HUB) =>
        invoke<string | null>("auth_whoami", { hubUrl }),
    authStartDeviceFlow: (hubUrl: string = HUB) =>
        invoke<DeviceStart>("auth_start_device_flow", { hubUrl }),
    authPollDevice: (hubUrl: string = HUB, deviceCode: string) =>
        invoke<PollOutcome>("auth_poll_device", { hubUrl, deviceCode }),
    authLogout: (hubUrl: string = HUB) => invoke<void>("auth_logout", { hubUrl }),

    // Daemon — Run scheduling + history.
    daemonStatus: () => invoke<DaemonStatus>("daemon_status"),
    listRuns: () => invoke<Run[]>("list_runs"),
    getRun: (runId: string) => invoke<Run | null>("get_run", { runId }),
    configureRun: (slug: string, cfg: RunConfig) =>
        invoke<Run>("configure_run", { slug, cfg }),
    removeRun: (runId: string) => invoke<void>("remove_run", { runId }),
    triggerRun: (runId: string) =>
        invoke<ScheduledRun>("trigger_run", { runId }),
    listScheduledRuns: (
        runId: string,
        opts?: { limit?: number; before?: number | null },
    ) =>
        invoke<ScheduledRun[]>("list_scheduled_runs", {
            runId,
            limit: opts?.limit ?? 80,
            before: opts?.before ?? null,
        }),
    loadRunRecords: (scheduledRunId: string, typeName: string, limit: number) =>
        invoke<unknown[]>("load_run_records", {
            scheduledRunId,
            typeName,
            limit,
        }),
    validateCron: (expr: string) =>
        invoke<void>("validate_cron_expr", { expr }),
};
