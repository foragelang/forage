/// Top-level App branch tests. The App's only job is choosing between
/// Welcome (no workspace) and StudioShell (workspace open); these
/// tests pin that contract.
///
/// Tauri's `invoke` is mocked by route so each test controls what the
/// `current_workspace` query returns. Other side-channel APIs (event
/// listen, dialog) are mocked to no-ops.

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

// All Tauri side-effect channels stub to no-ops; tests override
// `invoke` per case to return query data.
vi.mock("@tauri-apps/api/event", () => ({
    listen: vi.fn(async () => () => {}),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
    ask: vi.fn(async () => false),
    open: vi.fn(async () => null),
}));

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
    invoke: (...args: unknown[]) => invokeMock(...args),
}));

// Stub Monaco editor — `EditorView` pulls in `@monaco-editor/react`,
// which doesn't run under jsdom. The Welcome branch doesn't touch it,
// but the App module imports it through its child tree.
vi.mock("@monaco-editor/react", () => ({
    default: () => null,
    loader: { init: vi.fn() },
}));

// The App's child tree imports a few heavy modules (Monaco-backed
// editor, sidebar tree). Each test re-imports App freshly after the
// mocks are installed so the mock graph wins.
async function importApp() {
    const mod = await import("./App");
    return mod.App;
}

function wrap(children: React.ReactNode) {
    const qc = new QueryClient({
        defaultOptions: { queries: { retry: false, gcTime: 0 } },
    });
    return render(
        <QueryClientProvider client={qc}>
            <TooltipProvider delayDuration={200}>{children}</TooltipProvider>
        </QueryClientProvider>,
    );
}

describe("App top-level branch", () => {
    beforeEach(() => {
        invokeMock.mockReset();
    });
    afterEach(() => cleanup());

    test("currentWorkspace null branch renders Welcome", async () => {
        invokeMock.mockImplementation(async (cmd: string) => {
            if (cmd === "current_workspace") return null;
            if (cmd === "list_recent_workspaces") return [];
            if (cmd === "studio_version") return "0.0.0";
            return null;
        });

        const App = await importApp();
        wrap(<App />);

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
        invokeMock.mockImplementation(async (cmd: string) => {
            if (cmd === "current_workspace") return ws;
            if (cmd === "list_workspace_files")
                return { kind: "folder", name: "ws", path: "", children: [] };
            if (cmd === "list_runs") return [];
            if (cmd === "daemon_status")
                return { running: true, version: "0.0.0", active_count: 0 };
            if (cmd === "studio_version") return "0.0.0";
            return null;
        });

        const App = await importApp();
        wrap(<App />);

        // The Welcome tagline is the strongest negative signal — if
        // it's missing, the App branched to StudioShell.
        await screen.findByText("Files");
        expect(
            screen.queryByText(/Author recipes\. Manage runs/),
        ).not.toBeInTheDocument();
    });

    test("recent workspaces empty state hides the section label", async () => {
        invokeMock.mockImplementation(async (cmd: string) => {
            if (cmd === "current_workspace") return null;
            if (cmd === "list_recent_workspaces") return [];
            if (cmd === "studio_version") return "0.0.0";
            return null;
        });

        const App = await importApp();
        wrap(<App />);

        await screen.findByText("Open workspace");
        // The section header is uppercased in the rendered DOM
        // ("RECENT WORKSPACES") via Tailwind's `uppercase` utility.
        // queryByText returns the inner text pre-transform.
        expect(screen.queryByText("Recent workspaces")).toBeNull();
    });

    test("recent row click invokes open_workspace with the row's path", async () => {
        const recent: RecentWorkspace = {
            path: "/Users/dima/Library/Forage/Recipes",
            name: "Recipes",
            opened_at: Date.now() - 5_000,
            recipe_count: 3,
        };
        invokeMock.mockImplementation(async (cmd: string) => {
            if (cmd === "current_workspace") return null;
            if (cmd === "list_recent_workspaces") return [recent];
            if (cmd === "studio_version") return "0.0.0";
            if (cmd === "open_workspace") return null;
            return null;
        });

        const App = await importApp();
        wrap(<App />);

        const row = await screen.findByText("Recipes");
        row.closest("button")!.click();

        // The click goes through `openRecentWorkspaceAction`, which
        // invokes `open_workspace` with `{ path }`.
        const openCalls = invokeMock.mock.calls.filter(
            (c) => c[0] === "open_workspace",
        );
        expect(openCalls).toHaveLength(1);
        expect(openCalls[0][1]).toEqual({ path: recent.path });
    });
});
