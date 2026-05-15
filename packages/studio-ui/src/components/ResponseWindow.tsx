//! Pop-out Response viewer. The window mounts as a separate Tauri
//! window via `open_response_window`; its content subscribes to the
//! same Tauri event flow the main window does so the captured
//! responses + active pause carry across.
//!
//! Local store (not the main window's zustand store): the pop-out is
//! a separate webview process. Tauri broadcasts events to every
//! webview that listens, so re-using the main store via IPC would
//! double-route the same data. Instead we keep a per-window cache
//! seeded by the events directly.

import { useEffect, useState } from "react";

import { ResponseColumn } from "@/components/Debugger/ResponseColumn";
import type { PausePayload } from "@/bindings/PausePayload";
import type { StepResponse } from "@/bindings/StepResponse";
import { useStudioService } from "@/lib/services";

export function ResponseWindow() {
    const service = useStudioService();
    const [responses, setResponses] = useState<Record<string, StepResponse>>({});
    const [runId, setRunId] = useState<string | null>(null);
    const [paused, setPaused] = useState<PausePayload | null>(null);

    useEffect(() => {
        const offs: (() => void)[] = [];
        // Additive: every new run + every captured response + every
        // pause updates the local cache. The main window's
        // identical wiring (in useStudioEffects + the bottom panel)
        // doesn't conflict; both windows are pure observers of the
        // backend event stream.
        offs.push(
            service.onRunBegin((event) => {
                setResponses({});
                setRunId(event.run_id);
                setPaused(null);
            }),
        );
        offs.push(
            service.onStepResponse((event) => {
                setResponses((prev) => ({ ...prev, [event.step]: event.response }));
            }),
        );
        offs.push(service.onDebugPaused((payload) => setPaused(payload)));
        // Subtractive: explicit resume + workspace close clear the
        // pause state and run-scoped caches. Without these the pop-out
        // would keep showing the last pause's data after the user
        // continues / closes the workspace.
        offs.push(service.onDebugResumed(() => setPaused(null)));
        offs.push(
            service.onWorkspaceClosed(() => {
                setResponses({});
                setRunId(null);
                setPaused(null);
                // Close the window — the workspace is gone, the
                // Response viewer has nothing to show.
                window.close();
            }),
        );
        return () => offs.forEach((u) => u());
    }, [service]);

    const responsesToShow: { [key in string]?: StepResponse }
        = paused?.scope.step_responses && Object.keys(paused.scope.step_responses).length > 0
            ? paused.scope.step_responses
            : responses;
    return (
        <div className="h-screen w-screen flex flex-col bg-background">
            <ResponseColumn
                responses={responsesToShow}
                runId={runId}
                emptyStateLabel="No responses captured yet. Run a recipe in the main window."
            />
        </div>
    );
}
