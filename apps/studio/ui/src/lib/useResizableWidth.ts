import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

type Options = {
    storageKey: string;
    initial: number;
    min: number;
    /** How much space the *other* side of the gutter needs to stay
     *  usable. The resized panel can never grow past `containerWidth -
     *  gutterWidth - reserveOther`, even if the user drags further or
     *  the persisted width says otherwise. */
    reserveOther: number;
    /** Width of the gutter element itself in px; subtracted from the
     *  container so the resized panel's max never overlaps it. */
    gutterWidth: number;
};

type DragHandlers = {
    onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => void;
    onPointerMove: (e: React.PointerEvent<HTMLDivElement>) => void;
    onPointerUp: (e: React.PointerEvent<HTMLDivElement>) => void;
    onDoubleClick: () => void;
};

type Result = {
    /** The width to render the resized panel at — clamped against the
     *  current container width so the panel never overflows. */
    width: number;
    /** Spread onto the container element whose width is the layout
     *  budget for (panel + gutter + other side). */
    containerRef: React.RefObject<HTMLDivElement | null>;
    /** Spread onto the gutter element. */
    dragHandlers: DragHandlers;
};

/// Drag-resize width persisted to localStorage, clamped to the actual
/// container width.
///
/// The hook observes the container's clientWidth via ResizeObserver and
/// derives a max = container - gutter - reserveOther. The persisted
/// width is what the user dragged to; the rendered width is the
/// persisted width clamped to the current max. When the window grows
/// back, the panel widens back toward the persisted intent.
export function useResizableWidth({
    storageKey,
    initial,
    min,
    reserveOther,
    gutterWidth,
}: Options): Result {
    const containerRef = useRef<HTMLDivElement | null>(null);
    const [containerWidth, setContainerWidth] = useState<number>(0);

    const [stored, setStored] = useState(() => {
        if (typeof window === "undefined") return initial;
        const raw = window.localStorage.getItem(storageKey);
        const parsed = raw ? Number(raw) : NaN;
        return Number.isFinite(parsed) && parsed >= min ? parsed : initial;
    });

    useEffect(() => {
        if (typeof window === "undefined") return;
        window.localStorage.setItem(storageKey, String(stored));
    }, [storageKey, stored]);

    useLayoutEffect(() => {
        const el = containerRef.current;
        if (!el) return;
        setContainerWidth(el.clientWidth);
        if (typeof ResizeObserver === "undefined") return;
        const ro = new ResizeObserver(([entry]) => {
            if (entry) setContainerWidth(entry.contentRect.width);
        });
        ro.observe(el);
        return () => ro.disconnect();
    }, []);

    const maxAllowed = useCallback(() => {
        // Without a measured container width yet (first paint), don't
        // overpromise — clamp to the conservative default.
        if (containerWidth <= 0) return initial;
        return Math.max(min, containerWidth - gutterWidth - reserveOther);
    }, [containerWidth, gutterWidth, reserveOther, initial, min]);

    const clamp = useCallback(
        (next: number) => {
            const max = maxAllowed();
            return Math.min(max, Math.max(min, Math.round(next)));
        },
        [maxAllowed, min],
    );

    const dragging = useRef(false);
    const startX = useRef(0);
    const startWidth = useRef(0);

    const onPointerDown = useCallback(
        (e: React.PointerEvent<HTMLDivElement>) => {
            dragging.current = true;
            startX.current = e.clientX;
            startWidth.current = stored;
            e.currentTarget.setPointerCapture(e.pointerId);
            e.preventDefault();
        },
        [stored],
    );

    const onPointerMove = useCallback(
        (e: React.PointerEvent<HTMLDivElement>) => {
            if (!dragging.current) return;
            // Gutter sits to the LEFT of the resized panel. Dragging
            // left (negative dx) grows the panel; right shrinks.
            const dx = e.clientX - startX.current;
            setStored(clamp(startWidth.current - dx));
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
    /// recovery from "I dragged way too far."
    const onDoubleClick = useCallback(() => {
        setStored(clamp(initial));
    }, [clamp, initial]);

    return {
        width: clamp(stored),
        containerRef,
        dragHandlers: { onPointerDown, onPointerMove, onPointerUp, onDoubleClick },
    };
}
