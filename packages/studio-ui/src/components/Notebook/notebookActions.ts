//! Imperative actions invoked from the notebook view's toolbar and
//! menu wiring. Lives next to the view rather than in
//! `lib/studioActions.ts` so the notebook surface stays cleanly
//! self-contained — the editor and deployment views don't import
//! anything from here.

import { useStudio } from "@/lib/store";

/// Capture the notebook's current shape, fire `runNotebook`, and
/// route the result back through the store. Cleared on any mutation
/// of the stage list, so a stale snapshot can't hang around.
export async function notebookRunAction(): Promise<void> {
    const state = useStudio.getState();
    const { stages, running } = state.notebook;
    if (running || stages.length === 0) return;
    const service = state.service;
    const name = state.notebook.name;
    const stageNames = stages.map((s) => s.name);
    // The toolbar's resolved flag values are shared with the editor —
    // there's a single "Run" preset across both views. The notebook
    // forces ephemeral on at the backend regardless of this value
    // (notebooks never persist to the daemon's output store), but
    // sending the resolved shape keeps log lines truthful.
    const flags = state.runFlags;
    state.notebookRunBegin();
    try {
        const r = await service.runNotebook({
            name,
            stages: stageNames,
            flags: {
                sample_limit: flags.sample_limit,
                replay: flags.replay,
                ephemeral: flags.ephemeral,
            },
        });
        if (r.ok && r.snapshot) {
            useStudio.getState().notebookRunFinish({ snapshot: r.snapshot });
        } else {
            useStudio
                .getState()
                .notebookRunFinish({ error: r.error ?? "unknown error" });
        }
    } catch (e) {
        useStudio.getState().notebookRunFinish({ error: String(e) });
    }
}

/// Save the notebook as a `.forage` recipe in the workspace. The
/// publish flow proper (description / category / tags) hangs off the
/// editor's existing publish dialog, which the user picks up after
/// the save lands them on the new recipe file.
///
/// Returns the path of the saved file so callers can route the user
/// onto it; surfaces errors back through the notebook's banner.
export async function notebookPublishAction(): Promise<string | null> {
    const state = useStudio.getState();
    const { stages } = state.notebook;
    if (stages.length === 0) return null;
    const service = state.service;
    const name = state.notebook.name;
    const stageNames = stages.map((s) => s.name);
    try {
        const outcome = await service.saveNotebook(name, stageNames);
        return outcome.path;
    } catch (e) {
        useStudio.getState().notebookRunFinish({ error: String(e) });
        return null;
    }
}
