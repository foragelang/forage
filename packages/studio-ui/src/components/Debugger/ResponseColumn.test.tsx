/// ResponseColumn pins:
/// - empty responses → empty state copy;
/// - JSON tree renders type chips on parsed body;
/// - Override pill shows when StepResponse.format differs from the
///   content-type-detected format;
/// - Override pill stays absent when the response carries no
///   Content-Type header (the engine's JSON-first fallback set
///   `format` without any server-side hint to overrule);
/// - Maximize button click calls onMaximize;
/// - Pop-out button renders only when onPopOut is defined and
///   forwards clicks;
/// - "Load full" button surfaces on truncated bodies with an active
///   run, fetches via the service, and updates the Raw tab in place.

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import {
    act,
    cleanup,
    fireEvent,
    render,
    screen,
    waitFor,
    within,
} from "@testing-library/react";

import type { StepResponse } from "@/bindings/StepResponse";

import { TooltipProvider } from "../ui/tooltip";
import { installStudioService, useStudio } from "../../lib/store";
import { FakeStudioService, wrap as wrapWithService } from "../../test-fake-service";
import { ResponseColumn } from "./ResponseColumn";

// Monaco doesn't run under jsdom; stub it. The Tree + Headers tabs
// don't need Monaco, and the Raw tab in the port renders a plain
// `<pre>` so the stub isn't strictly required — kept for parity in
// case the Raw path evolves to use Monaco.
vi.mock("@monaco-editor/react", () => ({
    default: (props: { value?: string }) => (
        <div data-testid="monaco-stub">{props.value ?? ""}</div>
    ),
    loader: { init: vi.fn() },
}));

function wrap(node: React.ReactNode) {
    return render(<TooltipProvider delayDuration={0}>{node}</TooltipProvider>);
}

function clickTab(name: RegExp) {
    // The port's tab strip uses plain buttons (not Radix Tabs), so a
    // single click is enough to flip the active tab.
    const btn = screen
        .getAllByRole("button")
        .find((b) => name.test(b.textContent ?? ""))!;
    fireEvent.click(btn);
}

function makeResponse(over: Partial<StepResponse> = {}): StepResponse {
    return {
        status: 200,
        headers: { "content-type": "application/json" },
        body_raw: '{"items":[{"id":"a"},{"id":"b"}]}',
        body_truncated: false,
        format: "json",
        content_type_header: "application/json",
        ...over,
    };
}

const EMPTY = "No step has run yet at this pause.";

describe("ResponseColumn", () => {
    beforeEach(() => {
        // Each test asserts against a specific default tab; clear the
        // saved choice so prior tests don't leak through.
        localStorage.removeItem("forage:debugger-response-tab");
    });
    afterEach(() => {
        cleanup();
    });

    test("empty responses renders the empty state", () => {
        wrap(
            <ResponseColumn
                responses={{}}
                runId={null}
                emptyStateLabel={EMPTY}
            />,
        );
        expect(screen.getByText(EMPTY)).toBeInTheDocument();
    });

    test("JSON tree shows type chips and parsed structure", () => {
        wrap(
            <ResponseColumn
                responses={{ list: makeResponse() }}
                runId={null}
                emptyStateLabel={EMPTY}
            />,
        );
        // Default tab is Tree.
        expect(screen.getByText("items:")).toBeInTheDocument();
        expect(screen.getByText("[2]")).toBeInTheDocument();
    });

    test("Override pill shows when format differs from detected content-type", () => {
        wrap(
            <ResponseColumn
                responses={{
                    list: makeResponse({
                        format: "html",
                        content_type_header: "text/plain",
                    }),
                }}
                runId={null}
                emptyStateLabel={EMPTY}
            />,
        );
        clickTab(/Headers/);
        expect(
            screen.getByText(/Override: parsed as html/),
        ).toBeInTheDocument();
        expect(
            screen.getByText(/server said: text\/plain/),
        ).toBeInTheDocument();
    });

    test("Override pill is absent when format matches detected content-type", () => {
        wrap(
            <ResponseColumn
                responses={{ list: makeResponse() }}
                runId={null}
                emptyStateLabel={EMPTY}
            />,
        );
        clickTab(/Headers/);
        expect(screen.queryByText(/Override:/)).not.toBeInTheDocument();
    });

    test("Override pill stays absent when no Content-Type header", () => {
        // The headerless response is the JSON-first fallback's
        // domain: the engine resolves `format = "json"` because the
        // body parsed as JSON, but there's no server-side Content-
        // Type to "overrule." The pill must not light up in that case,
        // or every replay fixture without explicit headers would
        // falsely flag an override.
        wrap(
            <ResponseColumn
                responses={{
                    list: makeResponse({
                        headers: {},
                        format: "json",
                        content_type_header: null,
                    }),
                }}
                runId={null}
                emptyStateLabel={EMPTY}
            />,
        );
        clickTab(/Headers/);
        expect(screen.queryByText(/Override:/)).not.toBeInTheDocument();
    });

    test("Maximize button click calls onMaximize", () => {
        const onMax = vi.fn();
        wrap(
            <ResponseColumn
                responses={{ list: makeResponse() }}
                runId={null}
                onMaximize={onMax}
                isMaximized={false}
                emptyStateLabel={EMPTY}
            />,
        );
        fireEvent.click(
            screen.getByRole("button", { name: /Maximize/ }),
        );
        expect(onMax).toHaveBeenCalledTimes(1);
    });

    test("pop-out button is absent when onPopOut is undefined", () => {
        wrap(
            <ResponseColumn
                responses={{ list: makeResponse() }}
                runId={null}
                emptyStateLabel={EMPTY}
            />,
        );
        expect(
            screen.queryByRole("button", { name: /Pop out to window/ }),
        ).not.toBeInTheDocument();
    });

    test("pop-out button click calls onPopOut", () => {
        const onPopOut = vi.fn();
        wrap(
            <ResponseColumn
                responses={{ list: makeResponse() }}
                runId={null}
                onPopOut={onPopOut}
                emptyStateLabel={EMPTY}
            />,
        );
        fireEvent.click(
            screen.getByRole("button", { name: /Pop out to window/ }),
        );
        expect(onPopOut).toHaveBeenCalledTimes(1);
    });

    test("Headers tab shows the status chip and sorted headers", () => {
        wrap(
            <ResponseColumn
                responses={{
                    list: makeResponse({
                        status: 404,
                        headers: {
                            "X-Trace-Id": "abc",
                            "Content-Type": "application/json",
                        },
                    }),
                }}
                runId={null}
                emptyStateLabel={EMPTY}
            />,
        );
        clickTab(/Headers/);
        // Status chip
        expect(screen.getByText("404")).toBeInTheDocument();
        // Header rows render the header name (lowercased) + value;
        // x-trace-id is the case-folded form the port uses.
        const cell = screen.getByText("x-trace-id");
        const table = cell.closest("table")!;
        expect(within(table).getByText("abc")).toBeInTheDocument();
    });

    test("Raw tab renders the pretty-printed JSON body", () => {
        const service = new FakeStudioService();
        installStudioService(service, new QueryClient());
        wrapWithService(
            service,
            <ResponseColumn
                responses={{ list: makeResponse() }}
                runId={null}
                emptyStateLabel={EMPTY}
            />,
        );
        clickTab(/Raw/);
        // JSON gets pretty-printed in the Raw tab, so the indented
        // form shows up in the rendered <pre>.
        const pre = document.querySelector("pre")!;
        expect(pre.textContent).toContain('"items"');
        expect(pre.textContent).toContain('"id": "a"');
    });
});

const TEST_RUN_ID = "01HJK_LOAD_FULL";

describe("ResponseColumn — Load full", () => {
    let service: FakeStudioService;

    beforeEach(() => {
        localStorage.removeItem("forage:debugger-response-tab");
        service = new FakeStudioService();
        installStudioService(service, new QueryClient());
        useStudio.setState({
            activeFilePath: "demo/recipe.forage",
        });
    });
    afterEach(() => {
        cleanup();
        useStudio.setState({
            activeFilePath: null,
        });
    });

    test("button surfaces only on truncated responses with an active run", () => {
        // Truncated response + run in flight → button present.
        const { unmount } = wrapWithService(
            service,
            <ResponseColumn
                responses={{
                    list: makeResponse({
                        body_raw: "x".repeat(1024),
                        body_truncated: true,
                    }),
                }}
                runId={TEST_RUN_ID}
                emptyStateLabel={EMPTY}
            />,
        );
        clickTab(/Raw/);
        expect(
            screen.getByRole("button", { name: /Load full/ }),
        ).toBeInTheDocument();
        unmount();

        // Untruncated → no button.
        wrapWithService(
            service,
            <ResponseColumn
                responses={{ list: makeResponse({ body_truncated: false }) }}
                runId={TEST_RUN_ID}
                emptyStateLabel={EMPTY}
            />,
        );
        clickTab(/Raw/);
        expect(
            screen.queryByRole("button", { name: /Load full/ }),
        ).not.toBeInTheDocument();
    });

    test("clicking 'Load full' calls the service and swaps in the result", async () => {
        const fullBody = "{\"items\":" + JSON.stringify(Array(50).fill({ id: "x" })) + "}";
        const calls: Array<{ runId: string; step: string }> = [];
        service.setHandler(
            "loadFullStepBody",
            (runId: string, stepName: string) => {
                calls.push({ runId, step: stepName });
                return fullBody;
            },
        );

        wrapWithService(
            service,
            <ResponseColumn
                responses={{
                    list: makeResponse({
                        body_raw: "x".repeat(1024),
                        body_truncated: true,
                    }),
                }}
                runId={TEST_RUN_ID}
                emptyStateLabel={EMPTY}
            />,
        );
        clickTab(/Raw/);
        const btn = screen.getByRole("button", { name: /Load full/ });
        await act(async () => {
            fireEvent.click(btn);
        });
        await waitFor(() => {
            // Once the body is cached the button is gone; the Raw
            // tab's <pre> now carries the full payload.
            expect(
                screen.queryByRole("button", { name: /Load full/ }),
            ).not.toBeInTheDocument();
        });
        // runId from the column prop; step from the column's
        // current selection.
        expect(calls).toEqual([
            { runId: TEST_RUN_ID, step: "list" },
        ]);

        const pre = document.querySelector("pre")!;
        // JSON Raw view pretty-prints — the full body's array of 50
        // items survives, so the printed shape is visible in <pre>.
        expect(pre.textContent).toContain('"items"');
        expect(pre.textContent).toContain('"id": "x"');
    });

    test("service rejection renders the error inline and leaves the button", async () => {
        service.setHandler("loadFullStepBody", () => {
            throw new Error("disk read failed");
        });

        wrapWithService(
            service,
            <ResponseColumn
                responses={{
                    list: makeResponse({
                        body_raw: "x".repeat(1024),
                        body_truncated: true,
                    }),
                }}
                runId={TEST_RUN_ID}
                emptyStateLabel={EMPTY}
            />,
        );
        clickTab(/Raw/);
        await act(async () => {
            fireEvent.click(
                screen.getByRole("button", { name: /Load full/ }),
            );
        });
        await waitFor(() => {
            expect(screen.getByText(/disk read failed/)).toBeInTheDocument();
        });
        // The button stays so the user can retry.
        expect(
            screen.getByRole("button", { name: /Load full/ }),
        ).toBeInTheDocument();
    });
});
