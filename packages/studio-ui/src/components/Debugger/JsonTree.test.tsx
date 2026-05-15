/// JsonTree pins:
/// - primitives render with type chips (int/num/string/bool/null);
/// - object + array expand/collapse on click;
/// - search filter hides non-matching subtrees;
/// - expand-all button recursively opens all nodes.

import { afterEach, describe, expect, test } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import { TooltipProvider } from "../ui/tooltip";
import { JsonTree } from "./JsonTree";

function wrap(node: React.ReactNode) {
    return render(<TooltipProvider delayDuration={0}>{node}</TooltipProvider>);
}

describe("JsonTree", () => {
    afterEach(() => cleanup());

    test("renders primitive scalars with type chips", () => {
        wrap(
            <JsonTree
                value={{
                    i: 1,
                    f: 1.5,
                    s: "hello",
                    b: true,
                    n: null,
                }}
            />,
        );
        // Integers and floats both render as a value plus a type
        // chip; the int/num split distinguishes them.
        expect(screen.getByText("int")).toBeInTheDocument();
        expect(screen.getByText("num")).toBeInTheDocument();
        expect(screen.getByText(`"hello"`)).toBeInTheDocument();
        expect(screen.getByText("true")).toBeInTheDocument();
        // `null` shows up twice — once as the value, once as the type
        // chip. The pair existing at all is what the test pins.
        expect(screen.getAllByText("null").length).toBeGreaterThanOrEqual(2);
    });

    test("composite nodes toggle open on click", () => {
        wrap(<JsonTree value={{ outer: { hidden: 42 } }} />);
        // Root (depth 0) and `outer` (depth 1) open by default;
        // `hidden` is visible. Collapse `outer` and the inner key
        // disappears.
        expect(screen.getByText("hidden:")).toBeInTheDocument();
        fireEvent.click(screen.getByText("outer:"));
        expect(screen.queryByText("hidden:")).not.toBeInTheDocument();
        // Re-expand and `hidden` reappears.
        fireEvent.click(screen.getByText("outer:"));
        expect(screen.getByText("hidden:")).toBeInTheDocument();
    });

    test("array nodes show size chip and render children with bracket indices", () => {
        wrap(<JsonTree value={{ items: ["a", "b", "c"] }} />);
        // Array size summary
        expect(screen.getByText("[3]")).toBeInTheDocument();
        // Items array is at depth 1 → open by default, so children
        // render as [0]:, [1]:, [2]:.
        expect(screen.getByText("[0]:")).toBeInTheDocument();
        expect(screen.getByText("[1]:")).toBeInTheDocument();
        expect(screen.getByText("[2]:")).toBeInTheDocument();
    });

    test("search filter hides non-matching subtrees", () => {
        wrap(
            <JsonTree
                value={{ animals: { dog: 1, cat: 2 }, fruits: { apple: 3 } }}
            />,
        );
        // Type into the search box to filter the tree.
        const search = screen.getByPlaceholderText("Search keys…");
        fireEvent.change(search, { target: { value: "cat" } });
        // The matching key is visible.
        expect(screen.getByText("cat:")).toBeInTheDocument();
        // Non-matching siblings should be filtered out.
        expect(screen.queryByText("apple:")).not.toBeInTheDocument();
        expect(screen.queryByText("dog:")).not.toBeInTheDocument();
    });

    test("expand-all button opens nested objects", () => {
        wrap(<JsonTree value={{ a: { b: { c: 1 } } }} />);
        // By default deeply-nested nodes are collapsed.
        expect(screen.queryByText("c:")).not.toBeInTheDocument();
        // Click Expand to open every node.
        fireEvent.click(screen.getByTitle("Expand all"));
        expect(screen.getByText("c:")).toBeInTheDocument();
    });
});
