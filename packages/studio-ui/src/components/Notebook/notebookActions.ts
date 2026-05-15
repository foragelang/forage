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

/// Open the publish dialog. The dialog collects description /
/// category / tags, then runs the save → publish sequence. Save is
/// not done eagerly — it'd clutter the workspace with files the
/// user might not have wanted to commit yet.
export function notebookPublishAction(): void {
    const stages = useStudio.getState().notebook.stages;
    if (stages.length === 0) return;
    useStudio.getState().openPublishDialog();
}

/// Save + publish the notebook in one transactional flow. Called by
/// the publish dialog's submit button.
///
/// Steps: synthesize the `.forage` source, write it to the workspace,
/// then post it to the hub via the existing `publishRecipe` flow.
/// On any failure the dialog stays open so the user can correct +
/// retry; the save is idempotent (refuses to clobber) so a partial
/// failure can be retried without manual cleanup.
export async function commitNotebookPublish(args: {
    author: string;
    description: string;
    category: string;
    tags: string[];
}): Promise<{ saved: boolean; published: boolean; error?: string }> {
    const state = useStudio.getState();
    const { stages } = state.notebook;
    if (stages.length === 0) {
        return { saved: false, published: false, error: "no stages" };
    }
    const service = state.service;
    const name = state.notebook.name;
    const stageNames = stages.map((s) => s.name);
    // The composition's output type is the tail stage's output — what
    // the chain emits is what its last stage emits. Captured at add-
    // time on the stage so we can stamp `emits T` on the synthesized
    // recipe without re-fetching the stage's signature.
    const tailOutput = stages[stages.length - 1]?.outputType ?? null;
    try {
        await service.saveNotebook(name, stageNames, tailOutput);
    } catch (e) {
        return { saved: false, published: false, error: String(e) };
    }
    try {
        await service.publishRecipe({
            author: args.author,
            name,
            description: args.description,
            category: args.category,
            tags: args.tags,
        });
    } catch (e) {
        // The file is on disk but the hub publish failed. Surface
        // both facts in the error so the user knows their workspace
        // already has the new recipe.
        return {
            saved: true,
            published: false,
            error: `recipe saved to workspace but hub publish failed: ${String(e)}`,
        };
    }
    return { saved: true, published: true };
}
