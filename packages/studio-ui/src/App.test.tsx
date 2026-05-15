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
import { StudioServiceProvider, type StudioService } from "./lib/services";
import { installStudioService } from "./lib/store";
import { FakeStudioService } from "./test-fake-service";

// Stub Monaco editor — `EditorView` pulls in `@monaco-editor/react`,
// which doesn't run under jsdom. The Welcome branch doesn't touch it,
// but the App module imports it through its child tree.
vi.mock("@monaco-editor/react", () => ({
    default: () => null,
    loader: { init: vi.fn() },
}));

function wrap(service: StudioService, qc: QueryClient, children: React.ReactNode) {
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
    let qc: QueryClient;
    beforeEach(() => {
        service = new FakeStudioService();
        qc = new QueryClient({
            defaultOptions: { queries: { retry: false, gcTime: 0 } },
        });
        // The store reads the QueryClient for path → recipe-name
        // lookups; tests use the same client as the React tree so
        // store reads see whatever the components render against.
        installStudioService(service, qc);
    });
    afterEach(() => cleanup());

    test("currentWorkspace null branch renders Welcome", async () => {
        service.setHandler("currentWorkspace", null);
        service.setHandler("listRecentWorkspaces", []);
        service.setHandler("version", "0.0.0");

        const App = await importApp();
        wrap(service, qc, <App />);

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
        wrap(service, qc, <App />);

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
        wrap(service, qc, <App />);

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
        wrap(service, qc, <App />);

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
