/// Pins the Watches section's behavior:
/// - empty state shows the hint copy;
/// - submitting an expression via the input dispatches `addWatch` and
///   evaluates against the active scope;
/// - eval failures render inline in red without taking down the panel;
/// - clicking the X removes the watch from the store;
/// - re-pausing re-evaluates every watch.

import {
    afterEach,
    beforeEach,
    describe,
    expect,
    test,
} from "vitest";
import { QueryClient } from "@tanstack/react-query";
import {
    act,
    cleanup,
    fireEvent,
    screen,
    waitFor,
} from "@testing-library/react";

import type { PausePayload } from "@/bindings/PausePayload";

import { installStudioService, useStudio } from "../../lib/store";
import { FakeStudioService, wrap } from "../../test-fake-service";
import { WatchesSection } from "./WatchesSection";

const RECIPE = "trilogy";

function pause(): PausePayload {
    return {
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
    };
}

describe("WatchesSection", () => {
    let service: FakeStudioService;

    beforeEach(() => {
        localStorage.clear();
        service = new FakeStudioService();
        installStudioService(service, new QueryClient());
        useStudio.setState({
            activeRecipeName: RECIPE,
            watches: [],
            paused: pause(),
        });
    });

    afterEach(() => {
        cleanup();
        useStudio.setState({
            watches: [],
            paused: null,
        });
    });

    test("empty state shows the hint", () => {
        wrap(service, <WatchesSection />);
        expect(
            screen.getByText(
                /Pin a Forage expression to evaluate on every pause/,
            ),
        ).toBeInTheDocument();
    });

    test("submitting the input adds a watch and evaluates it", async () => {
        service.setHandler("evalWatchExpression", async (expr: string) => {
            if (expr === "$list.items | length") return 42;
            throw new Error(`unexpected expr: ${expr}`);
        });
        wrap(service, <WatchesSection />);

        const input = screen.getByLabelText(/Add watch expression/);
        await act(async () => {
            fireEvent.change(input, {
                target: { value: "$list.items | length" },
            });
            fireEvent.submit(input.closest("form")!);
        });

        // Store should now reflect the added watch.
        expect(useStudio.getState().watches).toEqual([
            "$list.items | length",
        ]);

        // Eval result renders after the promise resolves.
        await waitFor(() => {
            expect(screen.getByText("42")).toBeInTheDocument();
        });
        expect(
            screen.getByText("$list.items | length"),
        ).toBeInTheDocument();
    });

    test("eval errors render inline in red text", async () => {
        useStudio.setState({
            watches: ["malformed["],
        });
        service.setHandler("evalWatchExpression", async () => {
            throw new Error("parse: unexpected end of input");
        });
        wrap(service, <WatchesSection />);

        await waitFor(() => {
            expect(
                screen.getByText(/parse: unexpected end of input/),
            ).toBeInTheDocument();
        });
        // The expression source still renders so the user knows which
        // watch is broken.
        expect(screen.getByText("malformed[")).toBeInTheDocument();
    });

    test("clicking the remove button drops the watch from the store", async () => {
        useStudio.setState({
            watches: ["$a", "$b"],
        });
        service.setHandler("evalWatchExpression", async () => null);
        wrap(service, <WatchesSection />);

        const removeButtons = await screen.findAllByLabelText(/Remove watch/);
        expect(removeButtons).toHaveLength(2);

        await act(async () => {
            fireEvent.click(removeButtons[0]!);
        });

        expect(useStudio.getState().watches).toEqual(["$b"]);
    });

    test("re-pausing re-evaluates every watch", async () => {
        useStudio.setState({
            watches: ["$count"],
        });
        let count = 0;
        service.setHandler("evalWatchExpression", async () => {
            count += 1;
            return count;
        });

        wrap(service, <WatchesSection />);
        await waitFor(() => {
            expect(screen.getByText("1")).toBeInTheDocument();
        });

        // Swap to a new pause identity — useEffect on `paused` re-fires
        // every watch.
        act(() => {
            useStudio.setState({ paused: pause() });
        });
        await waitFor(() => {
            expect(screen.getByText("2")).toBeInTheDocument();
        });
    });
});
