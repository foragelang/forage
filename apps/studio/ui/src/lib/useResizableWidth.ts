import { useCallback, useEffect, useRef, useState } from "react";

type Options = {
    storageKey: string;
    initial: number;
    min: number;
    /** Cap relative to the viewport width — keeps the editor side usable
     *  even on small displays. */
    maxVwFraction: number;
};

type DragHandlers = {
    onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => void;
    onPointerMove: (e: React.PointerEvent<HTMLDivElement>) => void;
    onPointerUp: (e: React.PointerEvent<HTMLDivElement>) => void;
    onDoubleClick: () => void;
};

/// Drag-resize width persisted to localStorage. Used by the editor /
/// inspector divider — the divider element captures pointer events and
/// derives the new width from the cursor delta.
///
/// `direction: "left"` means the resized element grows as the user drags
/// LEFT (which is what an Inspector pinned on the right of the layout
/// needs — drag the gutter left to widen the inspector). Reversed for
/// elements on the left edge.
export function useResizableWidth({
    storageKey,
    initial,
    min,
    maxVwFraction,
}: Options): [number, DragHandlers] {
    const [width, setWidth] = useState(() => {
        if (typeof window === "undefined") return initial;
        const raw = window.localStorage.getItem(storageKey);
        const parsed = raw ? Number(raw) : NaN;
        return Number.isFinite(parsed) && parsed >= min ? parsed : initial;
    });

    useEffect(() => {
        if (typeof window === "undefined") return;
        window.localStorage.setItem(storageKey, String(width));
    }, [storageKey, width]);

    const dragging = useRef(false);
    const startX = useRef(0);
    const startWidth = useRef(0);

    const clamp = useCallback(
        (next: number) => {
            const vw = typeof window === "undefined" ? 1920 : window.innerWidth;
            const max = Math.max(min, Math.floor(vw * maxVwFraction));
            return Math.min(max, Math.max(min, Math.round(next)));
        },
        [min, maxVwFraction],
    );

    const onPointerDown = useCallback(
        (e: React.PointerEvent<HTMLDivElement>) => {
            dragging.current = true;
            startX.current = e.clientX;
            startWidth.current = width;
            e.currentTarget.setPointerCapture(e.pointerId);
            e.preventDefault();
        },
        [width],
    );

    const onPointerMove = useCallback(
        (e: React.PointerEvent<HTMLDivElement>) => {
            if (!dragging.current) return;
            // Gutter sits to the LEFT of the resized panel. Dragging left
            // (negative dx) grows the panel; dragging right shrinks it.
            const dx = e.clientX - startX.current;
            setWidth(clamp(startWidth.current - dx));
        },
        [clamp],
    );

    const onPointerUp = useCallback(
        (e: React.PointerEvent<HTMLDivElement>) => {
            if (!dragging.current) return;
            dragging.current = false;
            e.currentTarget.releasePointerCapture(e.pointerId);
        },
        [],
    );

    /// Double-clicking the gutter resets to the initial width. Same
    /// pattern as VS Code's drag handles — gives users an obvious
    /// recovery from "I dragged way too far and now can't see anything."
    const onDoubleClick = useCallback(() => {
        setWidth(clamp(initial));
    }, [clamp, initial]);

    return [width, { onPointerDown, onPointerMove, onPointerUp, onDoubleClick }];
}
