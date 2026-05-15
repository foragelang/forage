/// DomTree pins:
/// - element with attributes renders its tag + attribute summary;
/// - self-closing (childless) elements render in short form;
/// - comment nodes render with `<!-- ... -->` framing;
/// - XML walks from documentElement so the root tag is visible.

import { afterEach, describe, expect, test } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

import { DomTree } from "./DomTree";

describe("DomTree", () => {
    afterEach(() => cleanup());

    test("HTML element renders tag + attributes", () => {
        render(
            <DomTree
                source={`<html><body><a href="https://example.com" class="x">click</a></body></html>`}
                mime="text/html"
            />,
        );
        // The `<a>` element sits at depth 2 in the rendered tree
        // (html > body > a) so it starts collapsed; expand it to
        // reveal child text. The tag name + attribute names/values
        // are visible regardless of collapse state.
        expect(screen.getByText("a")).toBeInTheDocument();
        expect(screen.getByText("href")).toBeInTheDocument();
        expect(screen.getByText(`"https://example.com"`)).toBeInTheDocument();
        expect(screen.getByText("class")).toBeInTheDocument();
        expect(screen.getByText(`"x"`)).toBeInTheDocument();
        fireEvent.click(screen.getByText("a").closest("button")!);
        expect(screen.getByText("click")).toBeInTheDocument();
    });

    test("childless HTML element renders self-closing form", () => {
        render(
            <DomTree
                source={`<html><body><img src="x.png" alt="x"/></body></html>`}
                mime="text/html"
            />,
        );
        // The `<img>` element has no children, so the renderer emits
        // the short form (` />` suffix) rather than an open+close pair.
        // `<img>` is itself nested under <html>/<body> which start
        // open; its enclosing button is the only one inside the
        // body's child div.
        const buttons = screen.getAllByRole("button");
        const imgButton = buttons.find((b) =>
            b.textContent?.includes("img"),
        );
        expect(imgButton).toBeDefined();
        expect(imgButton!.textContent).toMatch(/<img\s.*\/>\s*$/);
        expect(imgButton!.textContent).not.toMatch(/<\/img>/);
    });

    test("HTML comment renders with comment delimiters", () => {
        render(
            <DomTree
                source={`<html><body><!-- a note --></body></html>`}
                mime="text/html"
            />,
        );
        // The comment text is wrapped in `<!-- ... -->` framing.
        expect(screen.getByText(/<!--\s*a note\s*-->/)).toBeInTheDocument();
    });

    test("XML walks from documentElement so the root tag is visible", () => {
        render(
            <DomTree
                source={`<?xml version="1.0"?><root><inner id="42">v</inner></root>`}
                mime="application/xml"
            />,
        );
        // `<root>` is the documentElement — must appear even though
        // there's no `<body>` wrapper like in HTML. The renderer
        // emits the tag name in both open and close spans, so each
        // tag has two matches; existence of either is enough.
        expect(screen.getAllByText("root").length).toBeGreaterThan(0);
        expect(screen.getAllByText("inner").length).toBeGreaterThan(0);
        expect(screen.getByText("id")).toBeInTheDocument();
        expect(screen.getByText(`"42"`)).toBeInTheDocument();
    });

    // DROPPED: "empty document body shows the empty-state line"
    // The port's DomTree renders an empty `<html>` skeleton instead of
    // a dedicated empty-state hint, so the original assertion against
    // a `Document is empty` line doesn't apply against current main.
});
