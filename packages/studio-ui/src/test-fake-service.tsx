/// Shared test scaffolding: a recording fake of `StudioService` plus a
/// `wrap` helper that mounts components inside the same provider tree
/// Studio uses at runtime (service context, react-query, tooltip).
///
/// Tests dial behavior in via `setHandler(method, value-or-fn)`. Calls
/// to methods without a configured handler reject with a descriptive
/// error so a forgotten mock surfaces loudly instead of resolving with
/// `undefined`.

import type React from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";

import { TooltipProvider } from "./components/ui/tooltip";
import {
    StudioServiceProvider,
    type ServiceCapabilities,
    type StudioService,
    type Unsubscribe,
} from "@/lib/services";

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
            return Promise.resolve(
                (handler as (...a: unknown[]) => unknown)(...args),
            );
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
    saveFile(path: string, source: string) {
        return this.call("saveFile", [path, source]);
    }

    // Recipe / authoring
    validateRecipe(source: string) { return this.call("validateRecipe", [source]); }
    recipeOutline(source: string) { return this.call("recipeOutline", [source]); }
    recipeHover(source: string, line: number, col: number) {
        return this.call("recipeHover", [source, line, col]);
    }
    recipeProgressUnit(name: string) {
        return this.call("recipeProgressUnit", [name]);
    }
    languageDictionary() { return this.call("languageDictionary", []); }
    createRecipe() { return this.call("createRecipe", []); }
    deleteRecipe(name: string) { return this.call("deleteRecipe", [name]); }
    listRecipeStatuses() { return this.call("listRecipeStatuses", []); }

    // Run
    runRecipe(name: string, flags?: unknown) {
        return this.call("runRecipe", [name, flags]);
    }
    runNotebook(args: unknown) { return this.call("runNotebook", [args]); }
    composeNotebookSource(
        name: string,
        stages: string[],
        outputType: string | null,
    ) {
        return this.call("composeNotebookSource", [name, stages, outputType]);
    }
    saveNotebook(name: string, stages: string[], outputType: string | null) {
        return this.call("saveNotebook", [name, stages, outputType]);
    }
    listWorkspaceRecipeSignatures() {
        return this.call("listWorkspaceRecipeSignatures", []);
    }
    parseRecipeSignature(source: string) {
        return this.call("parseRecipeSignature", [source]);
    }
    cancelRun() { return this.call("cancelRun", []); }
    debugResume(action: string) { return this.call("debugResume", [action]); }
    setBreakpoints(lines: number[]) { return this.call("setBreakpoints", [lines]); }
    setRecipeBreakpoints(name: string, lines: number[]) {
        return this.call("setRecipeBreakpoints", [name, lines]);
    }
    loadRecipeBreakpoints(name: string) {
        return this.call("loadRecipeBreakpoints", [name]);
    }
    evalWatchExpression(exprSource: string) {
        return this.call("evalWatchExpression", [exprSource]);
    }
    loadFullStepBody(runId: string, stepName: string) {
        return this.call("loadFullStepBody", [runId, stepName]);
    }
    openResponseWindow() { return this.call("openResponseWindow", []); }

    // Daemon
    daemonStatus() { return this.call("daemonStatus", []); }
    listRuns() { return this.call("listRuns", []); }
    getRun(id: string) { return this.call("getRun", [id]); }
    configureRun(name: string, cfg: unknown) {
        return this.call("configureRun", [name, cfg]);
    }
    removeRun(id: string) { return this.call("removeRun", [id]); }
    triggerRun(id: string) { return this.call("triggerRun", [id]); }
    listScheduledRuns(id: string, opts?: unknown) {
        return this.call("listScheduledRuns", [id, opts]);
    }
    loadRunRecords(id: string, type: string, limit: number) {
        return this.call("loadRunRecords", [id, type, limit]);
    }
    loadRunJsonld(id: string) { return this.call("loadRunJsonld", [id]); }
    validateCron(expr: string) { return this.call("validateCron", [expr]); }

    // Hub publish / auth
    publishRecipe(args: unknown) { return this.call("publishRecipe", [args]); }
    previewPublish(args: unknown) { return this.call("previewPublish", [args]); }
    syncFromHub(args: unknown) { return this.call("syncFromHub", [args]); }
    forkFromHub(args: unknown) { return this.call("forkFromHub", [args]); }
    authWhoami(hubUrl?: string) { return this.call("authWhoami", [hubUrl]); }
    authStartDeviceFlow(hubUrl?: string) {
        return this.call("authStartDeviceFlow", [hubUrl]);
    }
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
    unstarPackage(a: string, s: string) {
        return this.call("unstarPackage", [a, s]);
    }
    forkPackage(a: string, s: string, as?: string) {
        return this.call("forkPackage", [a, s, as]);
    }
    publishVersion(a: string, s: string, payload: unknown) {
        return this.call("publishVersion", [a, s, payload]);
    }
    discoverProducers(a: string, n: string) {
        return this.call("discoverProducers", [a, n]);
    }
    getTypeVersion(a: string, n: string, v: number | "latest") {
        return this.call("getTypeVersion", [a, n, v]);
    }
    publishTypeVersion(a: string, n: string, payload: unknown) {
        return this.call("publishTypeVersion", [a, n, payload]);
    }

    // Bookkeeping
    version() { return this.call("version", []); }
    showRecipeContextMenu(name: string) {
        return this.call("showRecipeContextMenu", [name]);
    }

    // Events — return immediate-no-op unsubscribes; the test surfaces
    // here don't exercise the listener side. Tests that want to fire
    // events at handlers patch the handler property out instead.
    onRunEvent(): Unsubscribe { return () => {}; }
    onDebugPaused(): Unsubscribe { return () => {}; }
    onRunBegin(): Unsubscribe { return () => {}; }
    onStepResponse(): Unsubscribe { return () => {}; }
    onDebugResumed(): Unsubscribe { return () => {}; }
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

/// Mount components inside the same provider tree Studio uses at
/// runtime (service context, react-query, tooltip). Returns the
/// react-testing-library render result; tests assert against the
/// returned DOM. The QueryClient is exposed on `result.qc` so tests
/// that need to seed query-cache entries (e.g. recipe statuses) can
/// reach into it without owning the provider boilerplate themselves.
export function wrap(service: StudioService, children: React.ReactNode) {
    const qc = new QueryClient({
        defaultOptions: { queries: { retry: false, gcTime: 0 } },
    });
    const result = render(
        <StudioServiceProvider service={service}>
            <QueryClientProvider client={qc}>
                <TooltipProvider delayDuration={200}>{children}</TooltipProvider>
            </QueryClientProvider>
        </StudioServiceProvider>,
    );
    return Object.assign(result, { qc });
}
