//! Top-level cross-cutting effects: the engine event streams, the
//! daemon completion stream, keyboard shortcuts, and the native menu
//! event listeners. Mounted exactly once from `App.tsx` so the rest
//! of the tree doesn't need to know any of this exists.

import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";

import {
    DEBUG_PAUSED_EVENT,
    RUN_EVENT,
    type PausePayload,
    type RunEvent,
    type ScheduledRun,
} from "@/lib/api";
import { useStudio } from "@/lib/store";
import {
    createAndOpenRecipe,
    runActive,
    saveActive,
} from "@/lib/studioActions";

const DAEMON_RUN_COMPLETED_EVENT = "forage:daemon-run-completed";

export function useStudioEffects() {
    const qc = useQueryClient();

    // Engine run-event stream. The `cancelled` flag is load-bearing
    // under React.StrictMode — tauri::listen registers its callback
    // synchronously, so the cleanup must drop it after the promise
    // resolves even if we already left the effect.
    useEffect(() => {
        let cancelled = false;
        let un: (() => void) | undefined;
        listen<RunEvent>(RUN_EVENT, (e) => {
            useStudio.getState().runAppend(e.payload);
        }).then((u) => {
            if (cancelled) u();
            else un = u;
        });
        return () => {
            cancelled = true;
            un?.();
        };
    }, []);

    // Debug pause stream — fires when the engine parks at a step
    // boundary or inside a for-loop iteration.
    useEffect(() => {
        let cancelled = false;
        let un: (() => void) | undefined;
        listen<PausePayload>(DEBUG_PAUSED_EVENT, (e) => {
            useStudio.getState().debugPause(e.payload);
        }).then((u) => {
            if (cancelled) u();
            else un = u;
        });
        return () => {
            cancelled = true;
            un?.();
        };
    }, []);

    // Daemon completion — invalidate the per-run scheduled-runs query
    // so the deployment view picks up the new row.
    useEffect(() => {
        let cancelled = false;
        let un: (() => void) | undefined;
        listen<ScheduledRun>(DAEMON_RUN_COMPLETED_EVENT, (e) => {
            qc.invalidateQueries({
                queryKey: ["scheduledRuns", e.payload.run_id],
            });
            // Also nudge the runs list — health may have changed.
            qc.invalidateQueries({ queryKey: ["runs"] });
        }).then((u) => {
            if (cancelled) u();
            else un = u;
        });
        return () => {
            cancelled = true;
            un?.();
        };
    }, [qc]);

    // ⌘S → save, ⌘R → run live, ⇧⌘R → replay, ⌘N → new recipe.
    useEffect(() => {
        const handler = (e: KeyboardEvent) => {
            if (!(e.metaKey || e.ctrlKey)) return;
            if (e.key === "s") {
                e.preventDefault();
                void saveActive();
            } else if (e.key === "r" && !e.shiftKey) {
                e.preventDefault();
                void runActive(false);
            } else if (e.key === "r" && e.shiftKey) {
                e.preventDefault();
                void runActive(true);
            } else if (e.key === "n") {
                e.preventDefault();
                void createAndOpenRecipe(qc);
            }
        };
        window.addEventListener("keydown", handler);
        return () => window.removeEventListener("keydown", handler);
    }, [qc]);

    // Native menu events. The `cancelled` flag mirrors the other
    // effects above — under React.StrictMode the cleanup can fire
    // before any of the per-event `listen` promises resolve, and the
    // resolved unlisten would otherwise leak across remounts.
    useEffect(() => {
        let cancelled = false;
        const offs: (() => void)[] = [];
        const register = (name: string, handler: () => void) => {
            listen(name, handler).then((u) => {
                if (cancelled) u();
                else offs.push(u);
            });
        };
        register("menu:new_recipe", () => void createAndOpenRecipe(qc));
        register("menu:save", () => void saveActive());
        register("menu:run_live", () => void runActive(false));
        register("menu:run_replay", () => void runActive(true));
        register("menu:validate", () => void saveActive());
        // Phase 6 hooks publishing into the toolbar directly.
        register("menu:publish", () =>
            console.info("publish: Phase 6"),
        );
        return () => {
            cancelled = true;
            offs.forEach((u) => u());
        };
    }, [qc]);
}
