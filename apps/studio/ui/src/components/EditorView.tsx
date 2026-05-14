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

export function EditorView() {
    const paused = useStudio((s) => s.paused);
    const [inspectorWidth, dragHandlers] = useResizableWidth({
        storageKey: "studio.inspector.width",
        initial: 420,
        min: 280,
        maxVwFraction: 0.75,
    });
    return (
        // `min-w-0 overflow-hidden` on the outer column is the
        // load-bearing fix: without them, the inspector's explicit
        // width (set inline) can make the inner row wider than the
        // column itself, the column grows to fit, and the toolbar —
        // sitting above the row in the same column — gets dragged
        // along, sliding its right-aligned buttons off the visible
        // edge. Clip at the column, propagate via flex shrinking
        // below.
        <div className="flex flex-1 min-h-0 min-w-0 flex-col overflow-hidden">
            <EditorToolbar />
            <div className="flex flex-1 min-h-0 min-w-0">
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
