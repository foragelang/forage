//! Phase 4 placeholder for the deployment view. Phase 5 fleshes this
//! out with the schedule editor, trend cards, run log, and run
//! drawer (DESIGN_HANDOFF.md § "Deployment view").

import { useStudio } from "@/lib/store";

export function DeploymentView() {
    const runId = useStudio((s) => s.activeRunId);
    return (
        <div className="flex-1 min-h-0 p-6 overflow-auto">
            <h1 className="text-xl font-mono">Deployment view</h1>
            <p className="mt-2 text-sm text-muted-foreground">
                Run: {runId ?? "(none)"}
            </p>
            <p className="mt-1 text-sm text-muted-foreground">(Phase 5)</p>
        </div>
    );
}
