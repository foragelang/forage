/// End-to-end notebook actions: store mutations + run/save service
/// calls. Tests describe timeless invariants — what flows where, what
/// the synthesized publish payload carries.

import { beforeEach, describe, expect, test } from "vitest";

import { FakeStudioService } from "@/test-fake-service";
import { installStudioService, useStudio } from "@/lib/store";
import { QueryClient } from "@tanstack/react-query";

import {
    commitNotebookPublish,
    notebookRunAction,
} from "./notebookActions";

function installFakeService(): FakeStudioService {
    const service = new FakeStudioService();
    installStudioService(service, new QueryClient());
    useStudio.getState().resetNotebook();
    return service;
}

describe("notebookRunAction", () => {
    let service: FakeStudioService;
    beforeEach(() => {
        service = installFakeService();
    });

    test("threads stage names and run flags to the service", async () => {
        useStudio.getState().setNotebookName("nb-1");
        useStudio.getState().addNotebookStage("scrape", null, "Item");
        useStudio.getState().addNotebookStage("enrich", null, "Item");
        useStudio.getState().setRunFlags({ sample_limit: 5, replay: false, ephemeral: true });

        service.setHandler("runNotebook", () => ({
            ok: true,
            snapshot: {
                records: [
                    { _id: "rec-0", typeName: "Item", fields: { id: "x" } },
                ],
                recordTypes: [],
            },
            error: null,
            daemon_warning: null,
        }));

        await notebookRunAction();

        const calls = service.calls.filter((c) => c.method === "runNotebook");
        expect(calls).toHaveLength(1);
        const args = calls[0]!.args[0] as {
            name: string;
            stages: string[];
            flags: { sample_limit: number | null; replay: boolean; ephemeral: boolean };
        };
        expect(args.name).toBe("nb-1");
        expect(args.stages).toEqual(["scrape", "enrich"]);
        expect(args.flags).toEqual({
            sample_limit: 5,
            replay: false,
            ephemeral: true,
        });

        const snap = useStudio.getState().notebook.snapshot;
        expect(snap?.records).toHaveLength(1);
        expect(useStudio.getState().notebook.runError).toBeNull();
        expect(useStudio.getState().notebook.running).toBe(false);
    });

    test("surfaces engine errors to the notebook banner", async () => {
        useStudio.getState().addNotebookStage("scrape", null, "Item");
        service.setHandler("runNotebook", () => ({
            ok: false,
            snapshot: null,
            error: "engine: blew up",
            daemon_warning: null,
        }));

        await notebookRunAction();

        expect(useStudio.getState().notebook.runError).toBe("engine: blew up");
        expect(useStudio.getState().notebook.snapshot).toBeNull();
    });

    test("noop when chain is empty", async () => {
        await notebookRunAction();
        const calls = service.calls.filter((c) => c.method === "runNotebook");
        expect(calls).toHaveLength(0);
    });
});

describe("commitNotebookPublish", () => {
    let service: FakeStudioService;
    beforeEach(() => {
        service = installFakeService();
    });

    test("save → publish carries the tail stage's output type", async () => {
        useStudio.getState().setNotebookName("nb-pub");
        useStudio.getState().addNotebookStage("scrape", null, "Item");
        useStudio.getState().addNotebookStage("enrich", null, "Item");

        service.setHandler("saveNotebook", () => ({
            path: "/ws/nb-pub.forage",
            source: "recipe \"nb-pub\"\n",
        }));
        service.setHandler("publishRecipe", () => ({
            author: "me",
            slug: "nb-pub",
            version: 1,
        }));

        const result = await commitNotebookPublish({
            author: "me",
            description: "test",
            category: "examples",
            tags: ["one", "two"],
        });
        expect(result).toEqual({ saved: true, published: true });

        const saves = service.calls.filter((c) => c.method === "saveNotebook");
        expect(saves).toHaveLength(1);
        expect(saves[0]!.args).toEqual([
            "nb-pub",
            ["scrape", "enrich"],
            "Item",
        ]);

        const publishes = service.calls.filter(
            (c) => c.method === "publishRecipe",
        );
        expect(publishes).toHaveLength(1);
        const arg = publishes[0]!.args[0] as { name: string; tags: string[] };
        expect(arg.name).toBe("nb-pub");
        expect(arg.tags).toEqual(["one", "two"]);
    });

    test("partial failure preserves the save and surfaces the publish error", async () => {
        useStudio.getState().addNotebookStage("scrape", null, "Item");

        service.setHandler("saveNotebook", () => ({
            path: "/ws/nb-pub.forage",
            source: "recipe \"nb-pub\"\n",
        }));
        service.setHandler("publishRecipe", () => {
            throw new Error("hub down");
        });

        const result = await commitNotebookPublish({
            author: "me",
            description: "test",
            category: "examples",
            tags: [],
        });
        expect(result.saved).toBe(true);
        expect(result.published).toBe(false);
        expect(result.error).toContain("hub publish failed");
    });

    test("save failure leaves both flags false", async () => {
        useStudio.getState().addNotebookStage("scrape", null, "Item");
        service.setHandler("saveNotebook", () => {
            throw new Error("filename already exists");
        });
        const result = await commitNotebookPublish({
            author: "me",
            description: "test",
            category: "examples",
            tags: [],
        });
        expect(result.saved).toBe(false);
        expect(result.published).toBe(false);
        expect(result.error).toContain("filename already exists");
    });
});
