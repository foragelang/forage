//! Editor surface: toolbar across the top, editor pane + inspector
//! side-by-side, debugger panel mounted below when the engine is
//! paused. The divider between editor and inspector is drag-resizable
//! and the chosen width persists across sessions via localStorage.

import { DebuggerPanel } from "@/components/DebuggerPanel";
import { EditorPane } from "@/components/EditorPane";
import { EditorToolbar } from "@/components/EditorToolbar";
import { Inspector } from "@/components/Inspector/index";
import { useResizableWidth } from "@/lib/useResizableWidth";
import { useStudio } from "@/lib/store";

const GUTTER_WIDTH = 4;
/// How much horizontal space the editor column on the left of the
/// gutter needs to stay usable. The hook subtracts this from the
/// observed container width to derive the inspector's maximum, so the
/// inspector can never grow large enough to push the editor (and the
/// toolbar that lives above it) into clipping or off-screen territory.
const EDITOR_MIN_WIDTH = 320;

export function EditorView() {
    const paused = useStudio((s) => s.paused);
    const { width: inspectorWidth, containerRef, dragHandlers } =
        useResizableWidth({
            storageKey: "studio.inspector.width",
            initial: 420,
            min: 280,
            reserveOther: EDITOR_MIN_WIDTH,
            gutterWidth: GUTTER_WIDTH,
        });
    return (
        // `min-w-0 overflow-hidden` on the outer column keeps the
        // toolbar pinned to the column's allotted width — the column
        // can't grow with its content and drag the toolbar's
        // right-aligned buttons off the visible edge. The clamp in the
        // hook prevents the inspector from overflowing in the first
        // place; this is the belt to its braces.
        <div className="flex flex-1 min-h-0 min-w-0 flex-col overflow-hidden">
            <EditorToolbar />
            <div ref={containerRef} className="flex flex-1 min-h-0 min-w-0">
                <div className="flex flex-1 min-w-0 flex-col">
                    <div className="flex-1 min-h-0 min-w-0 flex flex-col">
                        <EditorPane />
                    </div>
                    {paused && <DebuggerPanel />}
                </div>
                <div
                    role="separator"
                    aria-orientation="vertical"
                    aria-label="Resize inspector"
                    title="Drag to resize · double-click to reset"
                    className="relative w-1 shrink-0 cursor-col-resize select-none touch-none group"
                    {...dragHandlers}
                >
                    {/* 1px visible line on the left edge of the 4px hit area;
                        the hit area is wide enough to grab easily while the
                        visual stays minimal. Brightens on hover/active so the
                        affordance is obvious mid-drag. */}
                    <div className="absolute inset-y-0 left-0 w-px bg-border group-hover:bg-amber-500/70 group-active:bg-amber-500 transition-colors" />
                </div>
                <Inspector width={inspectorWidth} />
            </div>
        </div>
    );
}
