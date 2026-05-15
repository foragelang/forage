/// Pins the REPL section's behavior:
/// - empty state shows the hint copy and the input;
/// - submitting an expression appends a (input, result) row to the
///   transcript and pushes the raw input into per-recipe history;
/// - eval failures render in destructive-tone text without taking
///   down the panel;
/// - Up/Down arrows recall persisted inputs, and any edit cancels
///   navigation;
/// - Enter with empty input is a no-op (no history push, no
///   transcript row);
/// - A boundary separator appears between transcript rows whose
///   pause identity differs.

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

import { installStudioService, useStudio } from "../../lib/store";
import { FakeStudioService, wrap } from "../../test-fake-service";
import { ReplSection } from "./ReplSection";

const RECIPE = "trilogy";
const REPL_KEY = `forage:repl-history:${RECIPE}`;

describe("ReplSection", () => {
    let service: FakeStudioService;

    beforeEach(() => {
        localStorage.removeItem(REPL_KEY);
        service = new FakeStudioService();
        installStudioService(service, new QueryClient());
        useStudio.setState({
            activeRecipeName: RECIPE,
            replTranscript: [],
            pauseId: 1,
        });
    });

    afterEach(() => {
        cleanup();
        useStudio.setState({
            replTranscript: [],
            pauseId: 0,
        });
    });

    test("empty state shows the hint and an input", () => {
        wrap(service, <ReplSection />);
        expect(
            screen.getByText(
                /Evaluate ad-hoc expressions against the paused scope/,
            ),
        ).toBeInTheDocument();
        expect(screen.getByLabelText(/REPL input/)).toBeInTheDocument();
    });

    test("submitting an expression appends to transcript and persists input", async () => {
        service.setHandler("evalWatchExpression", async (expr: string) => {
            if (expr === "$list.items | length") return 42;
            throw new Error(`unexpected: ${expr}`);
        });
        wrap(service, <ReplSection />);

        const input = screen.getByLabelText(/REPL input/) as HTMLInputElement;
        await act(async () => {
            fireEvent.change(input, {
                target: { value: "$list.items | length" },
            });
            fireEvent.submit(input.closest("form")!);
        });

        // Raw input persisted to localStorage under the per-recipe key.
        expect(JSON.parse(localStorage.getItem(REPL_KEY)!)).toEqual([
            "$list.items | length",
        ]);

        // Transcript row renders with the input and the resolved value.
        await waitFor(() => {
            expect(screen.getByText("42")).toBeInTheDocument();
        });
        expect(
            screen.getByText("$list.items | length"),
        ).toBeInTheDocument();

        // Input field clears after submit.
        expect(input.value).toBe("");
    });

    test("eval errors render in destructive tone without crashing", async () => {
        service.setHandler("evalWatchExpression", async () => {
            throw new Error("parse: unexpected end of input");
        });
        wrap(service, <ReplSection />);

        const input = screen.getByLabelText(/REPL input/);
        await act(async () => {
            fireEvent.change(input, { target: { value: "bogus[" } });
            fireEvent.submit(input.closest("form")!);
        });

        await waitFor(() => {
            expect(
                screen.getByText(/parse: unexpected end of input/),
            ).toBeInTheDocument();
        });
        // The input source still renders so the user knows which
        // submission failed.
        expect(screen.getByText("bogus[")).toBeInTheDocument();
    });

    test("Enter on empty/whitespace input is a no-op", async () => {
        service.setHandler("evalWatchExpression", async () => {
            throw new Error("should never be called");
        });
        wrap(service, <ReplSection />);

        const input = screen.getByLabelText(/REPL input/) as HTMLInputElement;
        await act(async () => {
            fireEvent.change(input, { target: { value: "   " } });
            fireEvent.submit(input.closest("form")!);
        });

        // Nothing persisted, nothing appended.
        expect(localStorage.getItem(REPL_KEY)).toBeNull();
        expect(useStudio.getState().replTranscript).toHaveLength(0);
        // Hint still shown.
        expect(
            screen.getByText(
                /Evaluate ad-hoc expressions against the paused scope/,
            ),
        ).toBeInTheDocument();
        // No call to the evaluator either.
        expect(
            service.calls.filter((c) => c.method === "evalWatchExpression"),
        ).toHaveLength(0);
    });

    test("Up arrow recalls the previous input; Down clears past newest", () => {
        // Seed two prior inputs so we can walk both directions.
        localStorage.setItem(REPL_KEY, JSON.stringify(["$a", "$b"]));
        wrap(service, <ReplSection />);

        const input = screen.getByLabelText(/REPL input/) as HTMLInputElement;
        // First Up: newest entry.
        fireEvent.keyDown(input, { key: "ArrowUp" });
        expect(input.value).toBe("$b");
        // Second Up: older entry.
        fireEvent.keyDown(input, { key: "ArrowUp" });
        expect(input.value).toBe("$a");
        // Third Up: clamps at the oldest.
        fireEvent.keyDown(input, { key: "ArrowUp" });
        expect(input.value).toBe("$a");
        // Down: walks back toward the newest.
        fireEvent.keyDown(input, { key: "ArrowDown" });
        expect(input.value).toBe("$b");
        // Down past the newest: input clears (composing a fresh
        // entry).
        fireEvent.keyDown(input, { key: "ArrowDown" });
        expect(input.value).toBe("");
        // Down at "composing": no-op (no further newer entry).
        fireEvent.keyDown(input, { key: "ArrowDown" });
        expect(input.value).toBe("");
    });

    test("typing cancels history navigation", () => {
        localStorage.setItem(REPL_KEY, JSON.stringify(["$a", "$b"]));
        wrap(service, <ReplSection />);

        const input = screen.getByLabelText(/REPL input/) as HTMLInputElement;
        // Recall an entry.
        fireEvent.keyDown(input, { key: "ArrowUp" });
        expect(input.value).toBe("$b");
        // Now type — this should drop us back to "composing", so
        // a subsequent Up should start over at the newest entry.
        fireEvent.change(input, { target: { value: "$b.extra" } });
        fireEvent.keyDown(input, { key: "ArrowUp" });
        expect(input.value).toBe("$b");
    });

    test("Up/Down on empty history is a no-op", () => {
        wrap(service, <ReplSection />);
        const input = screen.getByLabelText(/REPL input/) as HTMLInputElement;
        fireEvent.keyDown(input, { key: "ArrowUp" });
        expect(input.value).toBe("");
        fireEvent.keyDown(input, { key: "ArrowDown" });
        expect(input.value).toBe("");
    });

    test("clear transcript button drops history and is hidden when transcript is empty", async () => {
        // Empty state: no clear button rendered.
        wrap(service, <ReplSection />);
        expect(
            screen.queryByLabelText(/Clear transcript/),
        ).not.toBeInTheDocument();
        cleanup();

        // Populated state: clear button removes the transcript entries.
        useStudio.setState({
            replTranscript: [
                { kind: "result", input: "$a", pauseId: 1, value: 1 },
            ],
        });
        wrap(service, <ReplSection />);
        const clear = screen.getByLabelText(/Clear transcript/);
        await act(async () => {
            fireEvent.click(clear);
        });
        expect(useStudio.getState().replTranscript).toHaveLength(0);
    });

    test("boundary separator appears when pause identity changes between submissions", async () => {
        // Number results to dodge JSON-quoting; the assertion is on
        // the dashed separator that the renderer inserts whenever
        // entry.pauseId differs from the prior entry's.
        let n = 0;
        service.setHandler("evalWatchExpression", async () => {
            n += 1;
            return n;
        });
        wrap(service, <ReplSection />);

        const input = screen.getByLabelText(/REPL input/) as HTMLInputElement;
        // First submission against pause id 1.
        await act(async () => {
            fireEvent.change(input, { target: { value: "$a" } });
            fireEvent.submit(input.closest("form")!);
        });
        await waitFor(() => {
            expect(screen.getByText("1")).toBeInTheDocument();
        });

        // Bump pauseId (a new pause came in), then submit again.
        act(() => {
            useStudio.setState({ pauseId: 2 });
        });
        await act(async () => {
            fireEvent.change(input, { target: { value: "$b" } });
            fireEvent.submit(input.closest("form")!);
        });
        await waitFor(() => {
            expect(screen.getByText("2")).toBeInTheDocument();
        });

        // The boundary separator is a div with dashed top border,
        // inserted only when the prior entry's pauseId differs.
        const separators = document.querySelectorAll(
            ".border-t.border-dashed",
        );
        expect(separators.length).toBeGreaterThan(0);
    });

    test("transcript survives unmount/remount (DebuggerPanel resume cycle)", async () => {
        // The whole point of hoisting transcript into the store: a
        // step-over briefly tears DebuggerPanel (and thus
        // ReplSection) down between two pauses. A component-local
        // transcript would die in that gap; the store-resident one
        // re-binds on remount.
        service.setHandler("evalWatchExpression", async () => 42);
        const { unmount } = wrap(service, <ReplSection />);
        const input = screen.getByLabelText(/REPL input/) as HTMLInputElement;
        await act(async () => {
            fireEvent.change(input, { target: { value: "$x" } });
            fireEvent.submit(input.closest("form")!);
        });
        await waitFor(() => {
            expect(screen.getByText("42")).toBeInTheDocument();
        });

        // Tear the panel down (paused = null in production); the
        // transcript stays in the store.
        unmount();
        expect(useStudio.getState().replTranscript).toHaveLength(1);

        // Remount with the next pause — the row should render again.
        wrap(service, <ReplSection />);
        expect(screen.getByText("42")).toBeInTheDocument();
        expect(screen.getByText("$x")).toBeInTheDocument();
    });
});
