//! Editor surface: toolbar across the top, editor pane + inspector
//! side-by-side, debugger panel mounted below when the engine is
//! paused.

import { DebuggerPanel } from "@/components/DebuggerPanel";
import { EditorPane } from "@/components/EditorPane";
import { EditorToolbar } from "@/components/EditorToolbar";
import { Inspector } from "@/components/Inspector/index";
import { useStudio } from "@/lib/store";

export function EditorView() {
    const paused = useStudio((s) => s.paused);
    return (
        <div className="flex flex-1 min-h-0 flex-col">
            <EditorToolbar />
            <div className="flex flex-1 min-h-0">
                <div className="flex flex-1 min-w-0 flex-col">
                    <div className="flex-1 min-h-0 min-w-0 flex flex-col">
                        <EditorPane />
                    </div>
                    {paused && <DebuggerPanel />}
                </div>
                <Inspector />
            </div>
        </div>
    );
}
