/// Sidebar Recipes section + scaffolding tests. Pin the contract:
/// only `valid` drafts appear in Recipes; broken / missing rows never
/// surface there (they live in the Files tree only). A click on a
/// Recipes row routes through `setActiveRecipeName`, setting both the
/// path and the recipe header name in one step. `create_recipe`
/// scaffolds a flat `<workspace>/<name>.forage` and the store winds up
/// with both fields populated against the new name.

import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import { Sidebar } from "./Sidebar";
import { SidebarProvider } from "./ui/sidebar";
import { TooltipProvider } from "./ui/tooltip";
import type { RecipeStatus } from "../bindings/RecipeStatus";
import { StudioServiceProvider } from "../lib/services";
import { installStudioService, useStudio } from "../lib/store";
import { FakeStudioService } from "../test-fake-service";

function wrap(service: FakeStudioService, qc: QueryClient) {
    return render(
        <StudioServiceProvider service={service}>
            <QueryClientProvider client={qc}>
                <TooltipProvider delayDuration={0}>
                    <SidebarProvider defaultOpen>
                        <Sidebar />
                    </SidebarProvider>
                </TooltipProvider>
            </QueryClientProvider>
        </StudioServiceProvider>,
    );
}

function withDefaultHandlers(service: FakeStudioService) {
    service.setHandler("currentWorkspace", {
        root: "/tmp/ws",
        name: null,
        deps: {},
        home: null,
    });
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
}

const validStatus = (name: string, path: string): RecipeStatus => ({
    name,
    draft: { kind: "valid", path },
    deployed: { kind: "none" },
});

const brokenStatus = (name: string, path: string): RecipeStatus => ({
    name,
    draft: { kind: "broken", path, error: "parse error" },
    deployed: { kind: "none" },
});

const missingStatus = (name: string): RecipeStatus => ({
    name,
    draft: { kind: "missing" },
    deployed: { kind: "deployed", version: 1, deployed_at: 0 },
});

describe("Sidebar Recipes section", () => {
    let service: FakeStudioService;
    let qc: QueryClient;
    beforeEach(() => {
        service = new FakeStudioService();
        qc = new QueryClient({
            defaultOptions: { queries: { retry: false, gcTime: 0 } },
        });
        installStudioService(service, qc);
        useStudio.setState({
            activeFilePath: null,
            activeRecipeName: null,
            view: "editor",
            dirty: false,
            source: "",
        });
        withDefaultHandlers(service);
    });
    afterEach(() => {
        cleanup();
        // Reset the store between tests so a leaked activeRecipeName
        // doesn't bleed into the next render.
        useStudio.setState({
            activeFilePath: null,
            activeRecipeName: null,
            dirty: false,
        });
    });

    test("lists every valid recipe by header name", async () => {
        service.setHandler("listRecipeStatuses", [
            validStatus("alpha", "alpha.forage"),
            validStatus("beta", "beta.forage"),
        ]);

        wrap(service, qc);
        expect(await screen.findByText("alpha")).toBeInTheDocument();
        expect(await screen.findByText("beta")).toBeInTheDocument();
    });

    test("broken drafts stay out of the Recipes section", async () => {
        service.setHandler("listRecipeStatuses", [
            validStatus("alpha", "alpha.forage"),
            brokenStatus("broken-stem", "broken.forage"),
            missingStatus("ghost"),
        ]);

        wrap(service, qc);
        // Wait for the valid recipe to land — the query resolves
        // asynchronously, so a synchronous `getByText` after `wrap`
        // would race the suspended render.
        expect(await screen.findByText("alpha")).toBeInTheDocument();
        // Broken and missing entries never get a Recipes row. The
        // file tree (Files section) shows broken files separately.
        expect(screen.queryByText("broken-stem")).not.toBeInTheDocument();
        expect(screen.queryByText("ghost")).not.toBeInTheDocument();
    });

    test("clicking a recipe sets activeRecipeName + activeFilePath", async () => {
        service.setHandler("listRecipeStatuses", [
            validStatus("alpha", "alpha.forage"),
        ]);
        service.setHandler("loadFile", () => "recipe \"alpha\" engine http\n");
        service.setHandler("loadRecipeBreakpoints", () => []);
        service.setHandler("setBreakpoints", () => undefined);

        wrap(service, qc);
        const row = await screen.findByText("alpha");
        fireEvent.click(row.closest("button")!);

        // The store update is synchronous in `setActiveRecipeName`;
        // the async side effects continue in the background but the
        // selection fields are written immediately.
        expect(useStudio.getState().activeRecipeName).toBe("alpha");
        expect(useStudio.getState().activeFilePath).toBe("alpha.forage");
        expect(useStudio.getState().view).toBe("editor");
    });

    test("New-recipe scaffolding routes the freshly-created name through the store", async () => {
        service.setHandler("listRecipeStatuses", []);
        service.setHandler("createRecipe", () => "untitled-1");
        service.setHandler("loadFile", () => "recipe \"untitled-1\" engine http\n");
        service.setHandler("loadRecipeBreakpoints", () => []);
        service.setHandler("setBreakpoints", () => undefined);

        wrap(service, qc);
        const button = await screen.findByRole("button", { name: /new recipe/i });
        fireEvent.click(button);
        // The createRecipe + invalidate + setActiveRecipeName chain
        // spans several Promise turns. Wait until the store sees the
        // result rather than guessing how many microtasks to flush.
        await waitForCondition(
            () => useStudio.getState().activeRecipeName === "untitled-1",
            "activeRecipeName never landed",
        );

        // create_recipe scaffolds at the workspace root (flat shape);
        // the store carries both the name and the matching path.
        expect(useStudio.getState().activeRecipeName).toBe("untitled-1");
        expect(useStudio.getState().activeFilePath).toBe("untitled-1.forage");
        // And we actually invoked the backend handler.
        expect(
            service.calls.filter((c) => c.method === "createRecipe"),
        ).toHaveLength(1);
    });

    test("empty Recipes section nudges the user toward creating one", async () => {
        service.setHandler("listRecipeStatuses", []);
        wrap(service, qc);
        // The empty-state copy is split across text nodes around the
        // inline `<span>` for the `recipe "..."` fragment. Use a
        // raw-content matcher so the test pins the first visible
        // chunk; the surrounding ancestors aren't load-bearing here.
        expect(
            await screen.findByText(/No recipes/),
        ).toBeInTheDocument();
    });
});

/// Spin until `predicate()` returns true, flushing microtasks /
/// macrotasks between attempts. Default budget is 1s — well above the
/// few-microtask delay any of our state writebacks actually take.
async function waitForCondition(
    predicate: () => boolean,
    message: string,
    timeoutMs = 1_000,
): Promise<void> {
    const start = Date.now();
    while (Date.now() - start < timeoutMs) {
        if (predicate()) return;
        await new Promise((r) => setTimeout(r, 5));
    }
    throw new Error(`waitForCondition timed out: ${message}`);
}
