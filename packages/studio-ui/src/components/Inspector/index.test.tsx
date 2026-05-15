/// Inspector tab list pins:
/// - the "Responses" tab is present;
/// - it's disabled when no responses are captured and no run is in
///   flight, so the user can't switch to an empty pane;
/// - it enables and shows the count badge once captures arrive;
/// - it enables while a run is in flight even before captures arrive.

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import { cleanup, screen } from "@testing-library/react";

import type { StepResponse } from "@/bindings/StepResponse";

import { installStudioService, useStudio } from "@/lib/store";
import { FakeStudioService, wrap } from "../../test-fake-service";
import { Inspector } from "./index";

vi.mock("@monaco-editor/react", () => ({
    default: (props: { value?: string }) => (
        <div data-testid="monaco-stub">{props.value ?? ""}</div>
    ),
    loader: { init: vi.fn() },
}));

// RunPane uses TanStack Query + the studio service for the live
// summary card; we don't need that wired for these tab-list
// assertions, but the component still imports it. Stub it.
vi.mock("./RunPane", () => ({
    RunPane: () => <div>run pane stub</div>,
}));
vi.mock("./HistoryPane", () => ({
    HistoryPane: () => <div>history pane stub</div>,
}));
vi.mock("./RecordsPane", () => ({
    RecordsPane: () => <div>records pane stub</div>,
}));
vi.mock("./RunResponsesPane", () => ({
    RunResponsesPane: () => <div>responses pane stub</div>,
}));

function makeResponse(over: Partial<StepResponse> = {}): StepResponse {
    return {
        status: 200,
        headers: {},
        body_raw: "{}",
        body_truncated: false,
        format: "json",
        content_type_header: "application/json",
        ...over,
    };
}

describe("Inspector tab list", () => {
    let service: FakeStudioService;

    beforeEach(() => {
        service = new FakeStudioService();
        installStudioService(service, new QueryClient());
        useStudio.setState({
            lastResponses: {},
            running: false,
            paused: null,
            inspectorMode: "run",
        });
    });
    afterEach(() => {
        cleanup();
    });

    test("the Responses tab is disabled when no responses are captured and no run is in flight", () => {
        wrap(service, <Inspector width={400} />);
        const trigger = screen.getByRole("tab", { name: /^Responses/ });
        expect(trigger).toBeDisabled();
    });

    test("the Responses tab enables and shows the count once captures arrive", () => {
        useStudio.setState({
            lastResponses: {
                list: makeResponse(),
                detail: makeResponse({ status: 500 }),
            },
        });
        wrap(service, <Inspector width={400} />);
        const trigger = screen.getByRole("tab", { name: /Responses \(2\)/ });
        expect(trigger).not.toBeDisabled();
    });

    test("the Responses tab enables while a run is in flight even before captures arrive", () => {
        useStudio.setState({ running: true });
        wrap(service, <Inspector width={400} />);
        const trigger = screen.getByRole("tab", { name: /^Responses/ });
        expect(trigger).not.toBeDisabled();
    });
});
