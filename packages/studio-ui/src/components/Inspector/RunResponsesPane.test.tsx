/// RunResponsesPane pins:
/// - empty `lastResponses` + no pause renders the run-not-started copy;
/// - `lastResponses` drives the ResponseColumn when not paused;
/// - paused scope's `step_responses` wins over `lastResponses` when both
///   are present (so the pane stays consistent with DebuggerPanel).

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import { cleanup, screen } from "@testing-library/react";

import type { StepResponse } from "@/bindings/StepResponse";

import { installStudioService, useStudio } from "@/lib/store";
import { FakeStudioService, wrap } from "../../test-fake-service";
import { RunResponsesPane } from "./RunResponsesPane";

vi.mock("@monaco-editor/react", () => ({
    default: (props: { value?: string }) => (
        <div data-testid="monaco-stub">{props.value ?? ""}</div>
    ),
    loader: { init: vi.fn() },
}));

function makeResponse(over: Partial<StepResponse> = {}): StepResponse {
    return {
        status: 200,
        headers: { "content-type": "application/json" },
        body_raw: '{"items":[{"id":"a"}]}',
        body_truncated: false,
        format: "json",
        content_type_header: "application/json",
        ...over,
    };
}

describe("RunResponsesPane", () => {
    let service: FakeStudioService;

    beforeEach(() => {
        service = new FakeStudioService();
        installStudioService(service, new QueryClient());
        useStudio.setState({ lastResponses: {}, paused: null, runId: null });
        localStorage.removeItem("forage:debugger-response-tab");
    });
    afterEach(() => {
        cleanup();
        useStudio.setState({ lastResponses: {}, paused: null, runId: null });
    });

    test("renders run-not-started copy with empty state", () => {
        wrap(service, <RunResponsesPane />);
        expect(
            screen.getByText(/Run a recipe to see captured step responses/),
        ).toBeInTheDocument();
    });

    test("renders ResponseColumn against lastResponses when not paused", () => {
        useStudio.setState({
            lastResponses: { list: makeResponse() },
            paused: null,
        });
        wrap(service, <RunResponsesPane />);
        // The Tree tab is the default; JSON parsing of the body yields
        // an `items` row the column renders as an expandable entry.
        // The port's JsonTree formats labels with a trailing colon.
        expect(screen.getByText("items:")).toBeInTheDocument();
    });

    test("paused scope's step_responses wins over lastResponses", () => {
        // Both maps populated. The paused.scope wins so the pane
        // stays in sync with what the DebuggerPanel's own Response
        // column is showing.
        useStudio.setState({
            lastResponses: {
                list: makeResponse({ body_raw: '{"from":"lastResponses"}' }),
            },
            paused: {
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
                    step_responses: {
                        list: makeResponse({
                            body_raw: '{"from":"paused"}',
                        }),
                    },
                },
            },
        });
        wrap(service, <RunResponsesPane />);
        // The Tree tab renders the top-level key — `from:` — and its
        // JSON-string value `"paused"` appears in the rendered row.
        expect(screen.getByText("from:")).toBeInTheDocument();
        expect(screen.getByText(`"paused"`)).toBeInTheDocument();
    });
});
