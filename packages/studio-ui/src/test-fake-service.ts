/// Shared fake StudioService for vitest. Records every method
/// invocation; per-test code stamps responses by method name via
/// `setHandler`. Unset methods reject loudly so a test that forgets to
/// mock a path doesn't pass with `undefined` flowing through.

import type {
    ServiceCapabilities,
    StudioService,
    Unsubscribe,
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
    cancelRun() { return this.call("cancelRun", []); }
    debugResume(action: string) { return this.call("debugResume", [action]); }
    setPauseIterations(enabled: boolean) {
        return this.call("setPauseIterations", [enabled]);
    }
    setBreakpoints(steps: string[]) { return this.call("setBreakpoints", [steps]); }
    setRecipeBreakpoints(name: string, steps: string[]) {
        return this.call("setRecipeBreakpoints", [name, steps]);
    }
    loadRecipeBreakpoints(name: string) {
        return this.call("loadRecipeBreakpoints", [name]);
    }

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
    // here don't exercise the listener side.
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
