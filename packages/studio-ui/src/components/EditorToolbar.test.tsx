/// Toolbar disable / enable tests. Save stays available on any open
/// file (editing is path-shaped); Run and the run-flags popover key
/// on a recipe header name, so a header-less .forage file disables
/// them with a "no recipe" tooltip.

import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, render, screen } from "@testing-library/react";

import { EditorToolbar } from "./EditorToolbar";
import { SidebarProvider } from "./ui/sidebar";
import { TooltipProvider } from "./ui/tooltip";
import type { RecipeStatus } from "../bindings/RecipeStatus";
import { StudioServiceProvider } from "../lib/services";
import { recipeStatusesKey } from "../lib/queryKeys";
import { installStudioService, useStudio } from "../lib/store";
import { FakeStudioService } from "../test-fake-service";

// EditorToolbar imports lucide icons; jsdom handles them fine without
// the Monaco stub the App tests need. Nothing else to mock at module
// level.

function wrap(service: FakeStudioService, qc: QueryClient) {
    return render(
        <StudioServiceProvider service={service}>
            <QueryClientProvider client={qc}>
                <TooltipProvider delayDuration={0}>
                    <SidebarProvider defaultOpen>
                        <EditorToolbar />
                    </SidebarProvider>
                </TooltipProvider>
            </QueryClientProvider>
        </StudioServiceProvider>,
    );
}

function seedRecipeStatuses(qc: QueryClient, statuses: RecipeStatus[]) {
    qc.setQueryData(recipeStatusesKey(), statuses);
}

describe("EditorToolbar enable / disable", () => {
    let service: FakeStudioService;
    let qc: QueryClient;
    beforeEach(() => {
        service = new FakeStudioService();
        qc = new QueryClient({
            defaultOptions: { queries: { retry: false, gcTime: 0 } },
        });
        installStudioService(service, qc);
        service.setHandler("listRuns", []);
        // listRecipeStatuses returns whatever the test seeded into
        // the query cache directly so we don't have to wait on an
        // async resolution to verify the disabled state.
        service.setHandler("listRecipeStatuses", () =>
            qc.getQueryData(recipeStatusesKey()) ?? [],
        );
        useStudio.setState({
            activeFilePath: null,
            activeRecipeName: null,
            running: false,
            paused: null,
            dirty: false,
            source: "",
        });
    });
    afterEach(() => {
        cleanup();
        useStudio.setState({
            activeFilePath: null,
            activeRecipeName: null,
        });
    });

    test("Run is disabled when the active path declares no recipe", async () => {
        // Header-less .forage file: workspace has the path open but
        // no recipe-statuses entry maps to it.
        seedRecipeStatuses(qc, []);
        useStudio.setState({ activeFilePath: "shared-types.forage" });

        wrap(service, qc);
        const runButton = await screen.findByRole("button", { name: "Run" });
        expect(runButton).toBeDisabled();
        const flagsButton = await screen.findByRole("button", { name: /run flags/i });
        expect(flagsButton).toBeDisabled();
        // Save stays enabled even without a recipe — every .forage
        // file is editable.
        const saveButton = await screen.findByRole("button", { name: "Save" });
        expect(saveButton).not.toBeDisabled();
    });

    test("Run is enabled when the active path hosts a parsed recipe", async () => {
        seedRecipeStatuses(qc, [
            {
                name: "trilogy",
                draft: { kind: "valid", path: "trilogy.forage" },
                deployed: { kind: "none" },
            },
        ]);
        useStudio.setState({ activeFilePath: "trilogy.forage" });

        wrap(service, qc);
        const runButton = await screen.findByRole("button", { name: "Run" });
        expect(runButton).not.toBeDisabled();
        const flagsButton = await screen.findByRole("button", { name: /run flags/i });
        expect(flagsButton).not.toBeDisabled();
    });

    test("Run stays disabled when no file is open at all", async () => {
        seedRecipeStatuses(qc, []);
        wrap(service, qc);
        const runButton = await screen.findByRole("button", { name: "Run" });
        expect(runButton).toBeDisabled();
        // With no file open, Save has nothing to write — it should
        // also be disabled.
        const saveButton = await screen.findByRole("button", { name: "Save" });
        expect(saveButton).toBeDisabled();
    });

    test("The run-flags chip surfaces the active preset label", async () => {
        seedRecipeStatuses(qc, [
            {
                name: "trilogy",
                draft: { kind: "valid", path: "trilogy.forage" },
                deployed: { kind: "none" },
            },
        ]);
        useStudio.setState({ activeFilePath: "trilogy.forage" });

        wrap(service, qc);
        // Default state matches the dev preset, so the chip shows "dev".
        const flagsButton = await screen.findByRole("button", { name: /run flags/i });
        expect(flagsButton.textContent).toMatch(/dev/);

        // Flipping the store's flags to the prod values reflects in
        // the chip label without a separate UI gesture.
        act(() => {
            useStudio.getState().setRunFlags({
                sample_limit: null,
                replay: false,
                ephemeral: false,
            });
        });
        const refreshed = await screen.findByRole("button", { name: /run flags/i });
        expect(refreshed.textContent).toMatch(/prod/);
    });
});
