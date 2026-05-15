/// Top-level App branch tests. The App's only job is choosing between
/// Welcome (no workspace) and StudioShell (workspace open); these
/// tests pin that contract.
///
/// Tests inject a fake `StudioService` and assert against method calls
/// instead of mocking the Tauri IPC bridge directly. The fake interface
/// is the same contract Studio's TauriStudioService and the hub IDE's
/// HubStudioService implement.

import {
    afterEach,
    beforeEach,
    describe,
    expect,
    test,
    vi,
} from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen } from "@testing-library/react";

import { TooltipProvider } from "./components/ui/tooltip";
import type { RecentWorkspace } from "./bindings/RecentWorkspace";
import type { WorkspaceInfo } from "./bindings/WorkspaceInfo";
import {
    StudioServiceProvider,
    type ServiceCapabilities,
    type StudioService,
    type Unsubscribe,
} from "./lib/services";
import { installStudioService } from "./lib/store";

// Stub Monaco editor — `EditorView` pulls in `@monaco-editor/react`,
// which doesn't run under jsdom. The Welcome branch doesn't touch it,
// but the App module imports it through its child tree.
vi.mock("@monaco-editor/react", () => ({
    default: () => null,
    loader: { init: vi.fn() },
}));

/// Records every method invocation; per-test code sets the result the
/// fake returns by name. Unset methods reject with a descriptive error
/// so a test that forgets to mock a path surfaces the gap loudly
/// rather than letting `undefined` flow through.
class FakeStudioService implements StudioService {
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
    publishRecipe(slug: string, hubUrl?: string, dryRun?: boolean) {
        return this.call("publishRecipe", [slug, hubUrl, dryRun]);
    }
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

    // Host dialogs
    async confirm(): Promise<boolean> { return false; }
    async pickDirectory(): Promise<string | null> { return null; }
}

function wrap(service: StudioService, children: React.ReactNode) {
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

async function importApp() {
    const mod = await import("./App");
    return mod.App;
}

describe("App top-level branch", () => {
    let service: FakeStudioService;
    beforeEach(() => {
        service = new FakeStudioService();
        installStudioService(service);
    });
    afterEach(() => cleanup());

    test("currentWorkspace null branch renders Welcome", async () => {
        service.setHandler("currentWorkspace", null);
        service.setHandler("listRecentWorkspaces", []);
        service.setHandler("version", "0.0.0");

        const App = await importApp();
        wrap(service, <App />);

        // Welcome's header text and the two action buttons are
        // distinctive enough to assert that branch landed.
        expect(
            await screen.findByText(/Author recipes\. Manage runs/),
        ).toBeInTheDocument();
        expect(screen.getByText("Open workspace")).toBeInTheDocument();
        expect(screen.getByText("New workspace")).toBeInTheDocument();
    });

    test("currentWorkspace populated branch renders StudioShell", async () => {
        const ws: WorkspaceInfo = {
            root: "/tmp/ws",
            name: "dima/ws",
            deps: {},
            home: "/Users/dima",
        };
        service.setHandler("currentWorkspace", ws);
        service.setHandler("listWorkspaceFiles", {
            kind: "folder",
            name: "ws",
            path: "",
            children: [],
        });
        service.setHandler("listRuns", []);
        service.setHandler("daemonStatus", {
            running: true,
            version: "0.0.0",
            active_count: 0,
        });
        service.setHandler("version", "0.0.0");

        const App = await importApp();
        wrap(service, <App />);

        // The Welcome tagline is the strongest negative signal — if
        // it's missing, the App branched to StudioShell.
        await screen.findByText("Files");
        expect(
            screen.queryByText(/Author recipes\. Manage runs/),
        ).not.toBeInTheDocument();
    });

    test("recent workspaces empty state hides the section label", async () => {
        service.setHandler("currentWorkspace", null);
        service.setHandler("listRecentWorkspaces", []);
        service.setHandler("version", "0.0.0");

        const App = await importApp();
        wrap(service, <App />);

        await screen.findByText("Open workspace");
        // The section header is uppercased in the rendered DOM
        // ("RECENT WORKSPACES") via Tailwind's `uppercase` utility.
        // queryByText returns the inner text pre-transform.
        expect(screen.queryByText("Recent workspaces")).toBeNull();
    });

    test("recent row click opens the workspace at the row's path", async () => {
        const recent: RecentWorkspace = {
            path: "/Users/dima/Library/Forage/Recipes",
            name: "Recipes",
            opened_at: Date.now() - 5_000,
            recipe_count: 3,
        };
        service.setHandler("currentWorkspace", null);
        service.setHandler("listRecentWorkspaces", [recent]);
        service.setHandler("openWorkspace", () => null);
        service.setHandler("version", "0.0.0");

        const App = await importApp();
        wrap(service, <App />);

        const row = await screen.findByText("Recipes");
        row.closest("button")!.click();

        // The click goes through `openRecentWorkspaceAction`, which
        // calls `openWorkspace` with the row's path.
        await new Promise((r) => setTimeout(r, 0));
        const openCalls = service.calls.filter((c) => c.method === "openWorkspace");
        expect(openCalls).toHaveLength(1);
        expect(openCalls[0]!.args).toEqual([recent.path]);
    });
});
