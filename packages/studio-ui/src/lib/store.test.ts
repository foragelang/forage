/// Tests for the studio store. Covers:
/// - `runAppend`'s aggregation (emit bursts vs. non-emit events);
/// - `breakpoint toggle` routing through the right service method;
/// - `lastResponses` capture + reset lifecycle;
/// - `watches` add / remove / persistence per active recipe;
/// - `replTranscript` survival across pause+clear-pause cycles.

import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, test } from "vitest";
import type { RunEvent } from "../bindings/RunEvent";
import type { StepResponse } from "../bindings/StepResponse";
import { FakeStudioService } from "../test-fake-service";
import { installStudioService, useStudio, type EmitBurst } from "./store";

function resetStore() {
    useStudio.setState({
        runLog: [],
        runCounts: {},
        runStartedAt: null,
        progressUnit: null,
        stepStats: {},
        currentStep: null,
        stepStartMs: {},
        running: false,
    });
}

function emit(type: string, total: number): RunEvent {
    return { kind: "emitted", type_name: type, total };
}

function request(step: string): RunEvent {
    return {
        kind: "request_sent",
        step,
        method: "POST",
        url: "https://example.test",
        page: 1,
    };
}

function response(step: string): RunEvent {
    return {
        kind: "response_received",
        step,
        status: 200,
        duration_ms: 100,
        bytes: 1024,
    };
}

function lastBurst(): EmitBurst {
    const log = useStudio.getState().runLog;
    const last = log[log.length - 1];
    if (!last || last.kind !== "emit_burst") {
        throw new Error(`expected emit_burst, got ${last?.kind ?? "empty"}`);
    }
    return last;
}

describe("runAppend", () => {
    beforeEach(() => resetStore());

    test("first emit opens a fresh burst at count 1", () => {
        useStudio.getState().runAppend(emit("Product", 1));
        expect(useStudio.getState().runLog).toHaveLength(1);
        expect(lastBurst()).toMatchObject({
            kind: "emit_burst",
            counts: { Product: 1 },
            typeOrder: ["Product"],
        });
    });

    test("consecutive emits of same type increment within one burst", () => {
        useStudio.getState().runAppend(emit("Product", 1));
        useStudio.getState().runAppend(emit("Product", 2));
        useStudio.getState().runAppend(emit("Product", 3));
        expect(useStudio.getState().runLog).toHaveLength(1);
        expect(lastBurst().counts).toEqual({ Product: 3 });
    });

    test("emits of different types share one burst, ordered by first-seen", () => {
        useStudio.getState().runAppend(emit("Product", 1));
        useStudio.getState().runAppend(emit("Variant", 1));
        useStudio.getState().runAppend(emit("PriceObservation", 1));
        useStudio.getState().runAppend(emit("Product", 2));
        useStudio.getState().runAppend(emit("Variant", 2));
        useStudio.getState().runAppend(emit("PriceObservation", 2));
        expect(useStudio.getState().runLog).toHaveLength(1);
        const burst = lastBurst();
        expect(burst.counts).toEqual({
            Product: 2,
            Variant: 2,
            PriceObservation: 2,
        });
        expect(burst.typeOrder).toEqual([
            "Product",
            "Variant",
            "PriceObservation",
        ]);
    });

    test("non-emit event closes the burst; next emit opens a fresh one", () => {
        useStudio.getState().runAppend(emit("Product", 1));
        useStudio.getState().runAppend(emit("Product", 2));
        useStudio.getState().runAppend(request("products"));
        useStudio.getState().runAppend(response("products"));
        useStudio.getState().runAppend(emit("Product", 3));

        const log = useStudio.getState().runLog;
        expect(log).toHaveLength(4);
        expect(log[0]?.kind).toBe("emit_burst");
        expect(log[1]?.kind).toBe("request_sent");
        expect(log[2]?.kind).toBe("response_received");
        expect(log[3]?.kind).toBe("emit_burst");
        // The second burst restarts the count at 1 — the displayed
        // total is per-burst, not engine-cumulative.
        if (log[3]?.kind === "emit_burst") {
            expect(log[3].counts).toEqual({ Product: 1 });
        }
    });

    test("burst snapshots the progressUnit at open time", () => {
        useStudio.setState({
            progressUnit: { variable: "product", types: ["Product"] },
        });
        useStudio.getState().runAppend(emit("Product", 1));
        expect(lastBurst().unitType).toBe("Product");
    });

    test("changing progressUnit mid-burst doesn't reframe the current burst", () => {
        useStudio.setState({
            progressUnit: { variable: "product", types: ["Product"] },
        });
        useStudio.getState().runAppend(emit("Product", 1));
        useStudio.setState({
            progressUnit: { variable: "variant", types: ["Variant"] },
        });
        useStudio.getState().runAppend(emit("Product", 2));
        expect(lastBurst().unitType).toBe("Product");
    });

    test("runCounts tracks engine-cumulative total, not per-burst", () => {
        useStudio.getState().runAppend(emit("Product", 1));
        useStudio.getState().runAppend(emit("Product", 2));
        useStudio.getState().runAppend(request("products"));
        useStudio.getState().runAppend(emit("Product", 87));
        expect(useStudio.getState().runCounts).toEqual({ Product: 87 });
    });

    test("emit-only stream produces exactly one burst", () => {
        for (let i = 1; i <= 100; i++) {
            useStudio.getState().runAppend(emit("Product", i));
        }
        expect(useStudio.getState().runLog).toHaveLength(1);
        expect(lastBurst().counts.Product).toBe(100);
    });

    test("interleaved request/response/emit produces alternating entries", () => {
        // Mirror the engine's natural rhythm:
        //   categories step → emit batch → products step (per cat) →
        //   emit batch → … → run_succeeded.
        useStudio.getState().runAppend(request("categories"));
        useStudio.getState().runAppend(response("categories"));
        for (let i = 1; i <= 20; i++) {
            useStudio.getState().runAppend(emit("Category", i));
        }
        useStudio.getState().runAppend(request("products"));
        useStudio.getState().runAppend(response("products"));
        for (let i = 1; i <= 50; i++) {
            useStudio.getState().runAppend(emit("Product", i));
            useStudio.getState().runAppend(emit("Variant", i));
            useStudio.getState().runAppend(emit("PriceObservation", i));
        }
        const log = useStudio.getState().runLog;
        // 2 (categories req+resp) + 1 (Category burst) + 2 (products
        // req+resp) + 1 (products burst) = 6
        expect(log).toHaveLength(6);
        expect(log[2]?.kind).toBe("emit_burst");
        expect(log[5]?.kind).toBe("emit_burst");
        if (log[5]?.kind === "emit_burst") {
            expect(log[5].counts).toEqual({
                Product: 50,
                Variant: 50,
                PriceObservation: 50,
            });
        }
    });
});

describe("breakpoint toggle (line-keyed)", () => {
    let service: FakeStudioService;

    beforeEach(() => {
        service = new FakeStudioService();
        useStudio.setState({
            service,
            breakpoints: new Set(),
            activeFilePath: null,
            activeRecipeName: null,
        });
        service.setHandler("setBreakpoints", undefined);
        service.setHandler("setRecipeBreakpoints", undefined);
    });

    test("toggleBreakpoint adds a line then removes it", () => {
        useStudio.getState().toggleBreakpoint(12);
        expect(useStudio.getState().breakpoints.has(12)).toBe(true);
        useStudio.getState().toggleBreakpoint(12);
        expect(useStudio.getState().breakpoints.has(12)).toBe(false);
    });

    test("toggleBreakpoint without an active recipe calls setBreakpoints", () => {
        useStudio.getState().toggleBreakpoint(7);
        const matches = service.calls.filter((c) => c.method === "setBreakpoints");
        const last = matches[matches.length - 1];
        expect(last).toBeDefined();
        expect(last?.args[0]).toEqual([7]);
    });
});

function makeStepResponse(over: Partial<StepResponse> = {}): StepResponse {
    return {
        status: 200,
        headers: { "content-type": "application/json" },
        body_raw: "{}",
        body_truncated: false,
        format: "json",
        content_type_header: "application/json",
        ...over,
    };
}

describe("lastResponses", () => {
    beforeEach(() => {
        useStudio.setState({
            lastResponses: {},
            running: false,
            paused: null,
            runId: null,
        });
    });

    test("setStepResponse stores the entry by step name", () => {
        const resp = makeStepResponse({ status: 200 });
        useStudio.getState().setStepResponse("list", resp);
        expect(useStudio.getState().lastResponses).toEqual({ list: resp });
    });

    test("a second setStepResponse adds rather than replaces other steps", () => {
        useStudio.getState().setStepResponse("a", makeStepResponse({ status: 200 }));
        useStudio.getState().setStepResponse("b", makeStepResponse({ status: 500 }));
        const got = useStudio.getState().lastResponses;
        expect(Object.keys(got)).toEqual(["a", "b"]);
        expect(got.a?.status).toBe(200);
        expect(got.b?.status).toBe(500);
    });

    test("re-recording the same step replaces its entry", () => {
        useStudio.getState().setStepResponse("list", makeStepResponse({ status: 200 }));
        useStudio.getState().setStepResponse("list", makeStepResponse({ status: 500 }));
        expect(useStudio.getState().lastResponses.list?.status).toBe(500);
    });

    test("captures survive a pause + clear-pause cycle", () => {
        // Recording happens independent of pause state; the pause
        // payload only carries the scope snapshot. Pausing then
        // clearing must not wipe lastResponses.
        useStudio.getState().setStepResponse("list", makeStepResponse());
        useStudio.getState().debugPause({
            kind: "step",
            step: "list",
            step_index: 0,
            start_line: 0,
            scope: {
                bindings: [],
                inputs: {},
                secrets: [],
                current: null,
                emit_counts: {},
                step_responses: {},
            },
        });
        useStudio.getState().debugClearPause();
        expect(useStudio.getState().lastResponses.list).toBeDefined();
    });

    test("runBegin clears lastResponses + runId", () => {
        useStudio.setState({ runId: "previous-run" });
        useStudio.getState().setStepResponse("list", makeStepResponse());
        expect(useStudio.getState().lastResponses.list).toBeDefined();
        useStudio.getState().runBegin();
        expect(useStudio.getState().lastResponses).toEqual({});
        expect(useStudio.getState().runId).toBeNull();
    });

    test("resetRunResponses drops captures + runId", () => {
        useStudio.setState({ runId: "rid" });
        useStudio.getState().setStepResponse("list", makeStepResponse());
        useStudio.getState().resetRunResponses();
        expect(useStudio.getState().lastResponses).toEqual({});
        expect(useStudio.getState().runId).toBeNull();
    });
});

const WATCH_KEY = (name: string) => `forage:watch-expressions:${name}`;

describe("watches", () => {
    beforeEach(() => {
        localStorage.clear();
        useStudio.setState({
            activeRecipeName: "trilogy",
            watches: [],
        });
    });

    test("setWatches persists to per-recipe localStorage", () => {
        useStudio.getState().setWatches(["$list.items | length", "$i.id"]);
        expect(useStudio.getState().watches).toEqual([
            "$list.items | length",
            "$i.id",
        ]);
        const raw = localStorage.getItem(WATCH_KEY("trilogy"));
        expect(raw).not.toBeNull();
        expect(JSON.parse(raw!)).toEqual([
            "$list.items | length",
            "$i.id",
        ]);
    });

    test("setWatches with an empty list still persists (clears the sidecar)", () => {
        useStudio.getState().setWatches(["$a"]);
        useStudio.getState().setWatches([]);
        expect(useStudio.getState().watches).toEqual([]);
        expect(JSON.parse(localStorage.getItem(WATCH_KEY("trilogy"))!)).toEqual(
            [],
        );
    });

    test("recipes have isolated sidecars", () => {
        useStudio.getState().setWatches(["$a"]);
        useStudio.setState({ activeRecipeName: "zen" });
        useStudio.getState().setWatches(["$z"]);
        // Each sidecar carries only its own recipe's list — the prior
        // recipe's entries don't bleed across.
        expect(JSON.parse(localStorage.getItem(WATCH_KEY("trilogy"))!)).toEqual(
            ["$a"],
        );
        expect(JSON.parse(localStorage.getItem(WATCH_KEY("zen"))!)).toEqual([
            "$z",
        ]);
    });

    test("setWatches with no active recipe doesn't persist", () => {
        useStudio.setState({ activeRecipeName: null });
        useStudio.getState().setWatches(["$x"]);
        // Nothing in localStorage — the persistor needs a recipe name
        // to key the sidecar by, and a null recipe means we can't
        // safely associate the list.
        expect(localStorage.length).toBe(0);
        // The in-memory list still updated so the UI renders the
        // additions for the current session.
        expect(useStudio.getState().watches).toEqual(["$x"]);
    });
});

describe("replTranscript", () => {
    beforeEach(() => {
        const service = new FakeStudioService();
        installStudioService(service, new QueryClient());
        useStudio.setState({
            replTranscript: [],
            paused: null,
            pauseId: 0,
        });
    });

    test("appendReplEntry pushes rows in order", () => {
        useStudio.getState().appendReplEntry({
            kind: "result",
            input: "$a",
            pauseId: 1,
            value: 1,
        });
        useStudio.getState().appendReplEntry({
            kind: "error",
            input: "$b",
            pauseId: 1,
            message: "boom",
        });
        const rows = useStudio.getState().replTranscript;
        expect(rows).toHaveLength(2);
        expect(rows[0]?.input).toBe("$a");
        expect(rows[1]?.input).toBe("$b");
    });

    test("clearReplTranscript drops every entry", () => {
        useStudio.getState().appendReplEntry({
            kind: "result",
            input: "$a",
            pauseId: 1,
            value: 1,
        });
        useStudio.getState().clearReplTranscript();
        expect(useStudio.getState().replTranscript).toEqual([]);
    });

    test("transcript survives a pause+clear-pause cycle", () => {
        // The whole reason transcript lives in the store rather than
        // component-local state: DebuggerPanel unmounts when paused
        // goes null, and component-local state would die with it.
        // Pausing then clearing the pause must NOT drop the rows.
        useStudio.getState().appendReplEntry({
            kind: "result",
            input: "$a",
            pauseId: 1,
            value: 1,
        });
        useStudio.getState().debugPause({
            kind: "step",
            step: "list",
            step_index: 0,
            start_line: 0,
            scope: {
                bindings: [],
                inputs: {},
                secrets: [],
                current: null,
                emit_counts: {},
                step_responses: {},
            },
        });
        useStudio.getState().debugClearPause();
        expect(useStudio.getState().replTranscript).toHaveLength(1);
    });

    test("debugPause bumps pauseId so the transcript can boundary-mark", () => {
        // The renderer keys the dashed separator off pauseId
        // differences between rows; each pause must produce a fresh
        // id so the boundary lands.
        const initial = useStudio.getState().pauseId;
        useStudio.getState().debugPause({
            kind: "step",
            step: "list",
            step_index: 0,
            start_line: 0,
            scope: {
                bindings: [],
                inputs: {},
                secrets: [],
                current: null,
                emit_counts: {},
                step_responses: {},
            },
        });
        expect(useStudio.getState().pauseId).toBe(initial + 1);
    });
});
