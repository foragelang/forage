/// Pins the gutter-click → breakpoint flow: clicking a step's gutter
/// line toggles a breakpoint, and the binding stays live across the
/// async `recipeOutline` arrival (i.e. the click handler reads the
/// current step map rather than a snapshot captured at mount time).

import {
    afterEach,
    beforeEach,
    describe,
    expect,
    test,
    vi,
} from "vitest";
import { act, cleanup, waitFor } from "@testing-library/react";
import React from "react";

import { installStudioService, useStudio } from "../lib/store";
import { FakeStudioService, wrap } from "../test-fakes";

// ---------- Monaco mock ----------
//
// The real `@monaco-editor/react` loads the Monaco UMD bundle, which
// jsdom can't drive. We swap it for a tiny stand-in that captures the
// `onMount` callback and runs it with a fake editor + monaco namespace
// rich enough for EditorPane to wire up its decorations + click handler.
// `mountedHandlers` collects every active `editor.onMouseDown` callback
// so the test can drive a synthetic gutter click.

type MouseHandler = (e: unknown) => void;

const mockState: {
    mouseDownHandlers: MouseHandler[];
} = { mouseDownHandlers: [] };

const fakeEditor = {
    onMouseDown(h: MouseHandler) {
        mockState.mouseDownHandlers.push(h);
        return {
            dispose: () => {
                mockState.mouseDownHandlers = mockState.mouseDownHandlers.filter(
                    (x) => x !== h,
                );
            },
        };
    },
    onDidChangeCursorPosition(_h: MouseHandler) {
        return { dispose: () => {} };
    },
    getPosition() {
        return { lineNumber: 1, column: 1 };
    },
    deltaDecorations(_old: string[], _new: unknown[]) {
        return [] as string[];
    },
    revealLineInCenterIfOutsideViewport() {},
    revealLineInCenter() {},
    setPosition() {},
    focus() {},
    addContentWidget() {},
    removeContentWidget() {},
};

class FakeRange {
    constructor(
        public startLineNumber: number,
        public startColumn: number,
        public endLineNumber: number,
        public endColumn: number,
    ) {}
}

const fakeMonaco = {
    editor: {
        MouseTargetType: { GUTTER_GLYPH_MARGIN: 2 },
        setModelMarkers() {},
        getModels() {
            return [];
        },
    },
    Range: FakeRange,
};

vi.mock("@monaco-editor/react", () => ({
    default: (props: {
        beforeMount?: (m: typeof fakeMonaco) => void;
        onMount?: (e: typeof fakeEditor, m: typeof fakeMonaco) => void;
    }) => {
        React.useEffect(() => {
            props.beforeMount?.(fakeMonaco);
            props.onMount?.(fakeEditor, fakeMonaco);
            // Mount-only — Monaco's real onMount fires exactly once.
            // eslint-disable-next-line react-hooks/exhaustive-deps
        }, []);
        return null;
    },
    loader: { init: vi.fn() },
}));

// The real language registration touches `monaco.languages.register`
// and async-fetches the dictionary; none of that is relevant here.
vi.mock("@/lib/monaco-forage", () => ({
    FORAGE_LANG_ID: "forage",
    registerForageLanguage: () => {},
}));

describe("EditorPane gutter click", () => {
    let service: FakeStudioService;

    beforeEach(() => {
        mockState.mouseDownHandlers = [];
        service = new FakeStudioService();
        installStudioService(service);
        // Seed an open recipe directly so we don't trip the async
        // `setActiveFilePath` flow (loadFile / loadRecipeBreakpoints
        // would otherwise fire and need their own fake handlers).
        useStudio.setState({
            activeFilePath: "shop/recipe.forage",
            source: "step products GET https://example.test",
            breakpoints: new Set(),
            paused: null,
            stepStats: {},
            validation: null,
        });
    });

    afterEach(() => {
        cleanup();
        useStudio.setState({
            activeFilePath: null,
            source: "",
            breakpoints: new Set(),
        });
    });

    test("click on a step line toggles breakpoint after outline loads", async () => {
        service.setHandler("validateRecipe", async () => ({
            ok: true,
            diagnostics: [],
        }));
        service.setHandler("recipeOutline", async () => ({
            steps: [
                {
                    name: "products",
                    start_line: 4,
                    start_col: 0,
                    end_line: 8,
                    end_col: 0,
                },
            ],
        }));
        service.setHandler("setRecipeBreakpoints", async () => undefined);

        const { EditorPane } = await import("./EditorPane");
        wrap(service, <EditorPane />);

        // Wait for the debounced (150ms) outline RPC to land and a
        // re-render to flush the new step map.
        await waitFor(
            () => {
                const calls = service.calls.filter(
                    (c) => c.method === "recipeOutline",
                );
                expect(calls.length).toBeGreaterThan(0);
            },
            { timeout: 1000 },
        );
        // Give the .then() handler a microtask to call setSteps and let
        // React commit the resulting re-render.
        await act(async () => {
            await new Promise((r) => setTimeout(r, 50));
        });

        // The step's 0-based start_line=4 maps to Monaco's 1-based line 5.
        expect(mockState.mouseDownHandlers.length).toBeGreaterThan(0);
        const handler =
            mockState.mouseDownHandlers[mockState.mouseDownHandlers.length - 1]!;
        await act(async () => {
            handler({
                target: {
                    type: 2,
                    position: { lineNumber: 5 },
                },
            });
        });

        await waitFor(() => {
            const bpCalls = service.calls.filter(
                (c) => c.method === "setRecipeBreakpoints",
            );
            expect(bpCalls).toHaveLength(1);
            expect(bpCalls[0]!.args).toEqual(["shop", ["products"]]);
        });
        // Store should reflect the toggled breakpoint too.
        expect([...useStudio.getState().breakpoints]).toEqual(["products"]);
    });
});
