//! Shared test scaffolding: a recording fake of `StudioService` plus a
//! `wrap` helper that mounts components inside the same provider tree
//! Studio uses at runtime (service context, react-query, tooltip).
//!
//! Tests dial behavior in via `setHandler(method, value-or-fn)`. Calls
//! to methods without a configured handler reject with a descriptive
//! error so a forgotten mock surfaces loudly instead of resolving with
//! `undefined`.

import type React from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";

import { TooltipProvider } from "./components/ui/tooltip";
import {
    StudioServiceProvider,
    type ServiceCapabilities,
    type StudioService,
    type Unsubscribe,
} from "./lib/services";

export class FakeStudioService implements StudioService {
    readonly capabilities: ServiceCapabilities = {
        workspace: true,
        deploy: true,
        liveRun: true,
        hubPackages: true,
    };
    readonly calls: Array<{ method: string; args: unknown[] }> = [];
    readonly handlers: Record<string, unknown> = {};

    private call(method: string, args: unknown[]): Promise<any> {
        this.calls.push({ method, args });
        const handler = this.handlers[method];
        if (handler === undefined) {
            return Promise.reject(
                new Error(`FakeStudioService: ${method} not configured`),
            );
        }
        if (typeof handler === "function") {
            return Promise.resolve((handler as (...a: unknown[]) => unknown)(...args));
        }
        return Promise.resolve(handler);
    }

    setHandler(method: string, handler: unknown) {
        this.handlers[method] = handler;
    }

    // Workspace
    currentWorkspace() { return this.call("currentWorkspace", []); }
    openWorkspace(path: string) { return this.call("openWorkspace", [path]); }
    newWorkspace(path: string) { return this.call("newWorkspace", [path]); }
    closeWorkspace() { return this.call("closeWorkspace", []); }
    listRecentWorkspaces() { return this.call("listRecentWorkspaces", []); }
    listWorkspaceFiles() { return this.call("listWorkspaceFiles", []); }
    loadFile(path: string) { return this.call("loadFile", [path]); }
    saveFile(path: string, source: string) { return this.call("saveFile", [path, source]); }

    // Recipe / authoring
    validateRecipe(source: string) { return this.call("validateRecipe", [source]); }
    recipeOutline(source: string) { return this.call("recipeOutline", [source]); }
    recipeHover(source: string, line: number, col: number) {
        return this.call("recipeHover", [source, line, col]);
    }
    recipeProgressUnit(slug: string) { return this.call("recipeProgressUnit", [slug]); }
    languageDictionary() { return this.call("languageDictionary", []); }
    createRecipe() { return this.call("createRecipe", []); }
    deleteRecipe(slug: string) { return this.call("deleteRecipe", [slug]); }

    // Run
    runRecipe(slug: string, replay: boolean) { return this.call("runRecipe", [slug, replay]); }
    cancelRun() { return this.call("cancelRun", []); }
    debugResume(action: string) { return this.call("debugResume", [action]); }
    setPauseIterations(enabled: boolean) { return this.call("setPauseIterations", [enabled]); }
    setBreakpoints(steps: string[]) { return this.call("setBreakpoints", [steps]); }
    setRecipeBreakpoints(slug: string, steps: string[]) {
        return this.call("setRecipeBreakpoints", [slug, steps]);
    }
    loadRecipeBreakpoints(slug: string) {
        return this.call("loadRecipeBreakpoints", [slug]);
    }

    // Daemon
    daemonStatus() { return this.call("daemonStatus", []); }
    listRuns() { return this.call("listRuns", []); }
    getRun(id: string) { return this.call("getRun", [id]); }
    configureRun(slug: string, cfg: unknown) { return this.call("configureRun", [slug, cfg]); }
    removeRun(id: string) { return this.call("removeRun", [id]); }
    triggerRun(id: string) { return this.call("triggerRun", [id]); }
    listScheduledRuns(id: string, opts?: unknown) {
        return this.call("listScheduledRuns", [id, opts]);
    }
    loadRunRecords(id: string, type: string, limit: number) {
        return this.call("loadRunRecords", [id, type, limit]);
    }
    validateCron(expr: string) { return this.call("validateCron", [expr]); }

    // Hub publish / auth (Studio side)
    publishRecipe(args: unknown) { return this.call("publishRecipe", [args]); }
    previewPublish(args: unknown) { return this.call("previewPublish", [args]); }
    syncFromHub(args: unknown) { return this.call("syncFromHub", [args]); }
    forkFromHub(args: unknown) { return this.call("forkFromHub", [args]); }
    authWhoami(hubUrl?: string) { return this.call("authWhoami", [hubUrl]); }
    authStartDeviceFlow(hubUrl?: string) { return this.call("authStartDeviceFlow", [hubUrl]); }
    authPollDevice(hubUrl: string, deviceCode: string) {
        return this.call("authPollDevice", [hubUrl, deviceCode]);
    }
    authLogout(hubUrl?: string) { return this.call("authLogout", [hubUrl]); }

    // Hub package discovery / social
    listPackages(query?: unknown) { return this.call("listPackages", [query]); }
    getPackage(a: string, s: string) { return this.call("getPackage", [a, s]); }
    listPackageVersions(a: string, s: string) {
        return this.call("listPackageVersions", [a, s]);
    }
    getPackageVersion(a: string, s: string, v: number | "latest") {
        return this.call("getPackageVersion", [a, s, v]);
    }
    starPackage(a: string, s: string) { return this.call("starPackage", [a, s]); }
    unstarPackage(a: string, s: string) { return this.call("unstarPackage", [a, s]); }
    forkPackage(a: string, s: string, as?: string) { return this.call("forkPackage", [a, s, as]); }
    publishVersion(a: string, s: string, payload: unknown) {
        return this.call("publishVersion", [a, s, payload]);
    }

    // Bookkeeping
    version() { return this.call("version", []); }
    showRecipeContextMenu(slug: string) {
        return this.call("showRecipeContextMenu", [slug]);
    }

    // Events — return immediately-no-op unsubscribes; the test
    // surfaces don't exercise them.
    onRunEvent(): Unsubscribe { return () => {}; }
    onDebugPaused(): Unsubscribe { return () => {}; }
    onDaemonRunCompleted(): Unsubscribe { return () => {}; }
    onWorkspaceOpened(): Unsubscribe { return () => {}; }
    onWorkspaceClosed(): Unsubscribe { return () => {}; }
    onMenuEvent(): Unsubscribe { return () => {}; }
    onDeeplinkClone(): Unsubscribe { return () => {}; }

    // Host dialogs
    async confirm(): Promise<boolean> { return false; }
    async pickDirectory(): Promise<string | null> { return null; }
    async revealInFileManager(): Promise<void> { return undefined; }
}

export function wrap(service: StudioService, children: React.ReactNode) {
    const qc = new QueryClient({
        defaultOptions: { queries: { retry: false, gcTime: 0 } },
    });
    return render(
        <StudioServiceProvider service={service}>
            <QueryClientProvider client={qc}>
                <TooltipProvider delayDuration={200}>{children}</TooltipProvider>
            </QueryClientProvider>
        </StudioServiceProvider>,
    );
}
