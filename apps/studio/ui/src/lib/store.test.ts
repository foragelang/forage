/// Tests for `runAppend`'s aggregation behavior — emit events get
/// rolled into `EmitBurst` log entries; non-emit events close the
/// current burst.

import { beforeEach, describe, expect, test } from "vitest";
import type { RunEvent } from "../bindings/RunEvent";
import { useStudio, type EmitBurst } from "./store";

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
