//! Inspector "Responses" tab: per-step response viewer that survives
//! between pauses. Reads `paused?.scope.step_responses` when the
//! engine is paused (so the displayed state matches the pause-time
//! capture), and falls back to `lastResponses` otherwise — the
//! streamed-during-run state. Empty state for a fresh workspace
//! prompts the user to run a recipe.

import { useStudio } from "@/lib/store";
import { ResponseColumn } from "@/components/Debugger/ResponseColumn";
import { useStudioService } from "@/lib/services";

export function RunResponsesPane() {
    const paused = useStudio((s) => s.paused);
    const lastResponses = useStudio((s) => s.lastResponses);
    const runId = useStudio((s) => s.runId);
    const service = useStudioService();
    const responses: { [key in string]?: import("@/bindings/StepResponse").StepResponse }
        = paused?.scope.step_responses && Object.keys(paused.scope.step_responses).length > 0
            ? paused.scope.step_responses
            : lastResponses;
    const empty = Object.keys(responses).length === 0;
    return (
        <div className="flex-1 min-h-0 flex flex-col">
            <ResponseColumn
                responses={responses}
                runId={runId}
                onPopOut={() => {
                    service.openResponseWindow().catch((e) =>
                        console.warn("open_response_window failed", e),
                    );
                }}
                emptyStateLabel={
                    empty
                        ? "Run a recipe to see captured step responses."
                        : "No responses captured at this pause yet."
                }
            />
        </div>
    );
}
