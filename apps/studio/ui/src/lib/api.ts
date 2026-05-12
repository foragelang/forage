//! Typed wrappers around Tauri commands.

import { invoke } from "@tauri-apps/api/core";

export type RecipeEntry = {
    slug: string;
    path: string;
    has_fixtures: boolean;
};

export type ValidationOutcome = {
    ok: boolean;
    diagnostics: Diagnostic[];
};

export type Diagnostic = {
    severity: "error" | "warning";
    code: string;
    message: string;
    /** 0-based line of the span start. */
    start_line: number;
    /** 0-based column of the span start. */
    start_col: number;
    /** 0-based line of the span end (exclusive). */
    end_line: number;
    /** 0-based column of the span end (exclusive). */
    end_col: number;
};

/// Parser-emitted structural outline of a recipe. Used by Studio for
/// breakpoint glyph anchoring and reveal-on-pause without re-parsing
/// in TS.
export type RecipeOutline = {
    steps: StepLocation[];
};

export type StepLocation = {
    name: string;
    start_line: number;
    start_col: number;
    end_line: number;
    end_col: number;
};

export type Snapshot = {
    records: RecipeRecord[];
    diagnostic: DiagnosticReport;
};

export type RecipeRecord = {
    typeName: string;
    fields: Record<string, unknown>;
};

export type DiagnosticReport = {
    stall_reason?: string | null;
    unmet_expectations?: string[];
    unfired_capture_rules?: string[];
    unmatched_captures?: string[];
    unhandled_affordances?: string[];
};

export type RunOutcome = {
    ok: boolean;
    snapshot?: Snapshot | null;
    error?: string | null;
};

export type RunEvent =
    | { kind: "run_started"; recipe: string; replay: boolean }
    | { kind: "auth"; flavor: string; status: string }
    | { kind: "request_sent"; step: string; method: string; url: string; page: number }
    | {
          kind: "response_received";
          step: string;
          status: number;
          duration_ms: number;
          bytes: number;
      }
    | { kind: "emitted"; type_name: string; total: number }
    | { kind: "run_succeeded"; records: number; duration_ms: number }
    | { kind: "run_failed"; error: string; duration_ms: number };

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
/// Tauri event name the engine emits when paused at a step in debug mode.
/// Matches the Rust-side `commands::DEBUG_PAUSED_EVENT` constant.
export const DEBUG_PAUSED_EVENT = "forage:debug-paused";

export type DebugFrame = {
    bindings: Record<string, unknown>;
};

export type DebugScope = {
    bindings: DebugFrame[];
    inputs: Record<string, unknown>;
    secrets: string[];
    current: unknown | null;
    emit_counts: Record<string, number>;
};

export type StepPause = {
    step: string;
    step_index: number;
    scope: DebugScope;
};

export type DebugAction = "continue" | "step_over" | "stop";

export const api = {
    version: () => invoke<string>("studio_version"),
    listRecipes: () => invoke<RecipeEntry[]>("list_recipes"),
    loadRecipe: (slug: string) => invoke<string>("load_recipe", { slug }),
    saveRecipe: (slug: string, source: string) =>
        invoke<ValidationOutcome>("save_recipe", { slug, source }),
    validateRecipe: (source: string) =>
        invoke<ValidationOutcome>("validate_recipe", { source }),
    createRecipe: () => invoke<string>("create_recipe"),
    deleteRecipe: (slug: string) => invoke<void>("delete_recipe", { slug }),
    runRecipe: (slug: string, replay: boolean) =>
        invoke<RunOutcome>("run_recipe", { slug, replay }),
    cancelRun: () => invoke<void>("cancel_run"),
    debugResume: (action: DebugAction) =>
        invoke<void>("debug_resume", { action }),
    setBreakpoints: (steps: string[]) =>
        invoke<void>("set_breakpoints", { steps }),
    recipeOutline: (source: string) =>
        invoke<RecipeOutline>("recipe_outline", { source }),
    publishRecipe: (slug: string, hubUrl: string = HUB, dryRun = true) =>
        invoke<RunOutcome>("publish_recipe", { slug, hubUrl, dryRun }),
    authWhoami: (hubUrl: string = HUB) =>
        invoke<string | null>("auth_whoami", { hubUrl }),
    authStartDeviceFlow: (hubUrl: string = HUB) =>
        invoke<DeviceStart>("auth_start_device_flow", { hubUrl }),
    authPollDevice: (hubUrl: string = HUB, deviceCode: string) =>
        invoke<PollOutcome>("auth_poll_device", { hubUrl, deviceCode }),
    authLogout: (hubUrl: string = HUB) => invoke<void>("auth_logout", { hubUrl }),
};
