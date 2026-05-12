//! Typed wrappers around Tauri commands.

import { invoke } from "@tauri-apps/api/core";

export type RecipeEntry = {
    slug: string;
    path: string;
    has_fixtures: boolean;
};

export type ValidationOutcome = {
    ok: boolean;
    errors: string[];
    warnings: string[];
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

export const api = {
    version: () => invoke<string>("studio_version"),
    listRecipes: () => invoke<RecipeEntry[]>("list_recipes"),
    loadRecipe: (slug: string) => invoke<string>("load_recipe", { slug }),
    saveRecipe: (slug: string, source: string) =>
        invoke<ValidationOutcome>("save_recipe", { slug, source }),
    createRecipe: () => invoke<string>("create_recipe"),
    runRecipe: (slug: string, replay: boolean) =>
        invoke<RunOutcome>("run_recipe", { slug, replay }),
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
