//! Imperative actions invoked from multiple surfaces (toolbar
//! buttons, keyboard shortcuts, menu events). Each pulls state from
//! `useStudio.getState()` at call time so callers don't need to
//! thread props through.
//!
//! Reactive-UI rule: nothing here subscribes to state — these are
//! command handlers. State subscriptions happen in components, never
//! here.

import type { QueryClient } from "@tanstack/react-query";

import type { RecipeStatus } from "../bindings/RecipeStatus";
import type { RunRecipeFlags } from "../bindings/RunRecipeFlags";
import { recipeNameOf } from "./path";
import { currentWorkspaceKey, recentWorkspacesKey, recipeStatusesKey } from "./queryKeys";
import { useStudio } from "./store";

export async function saveActive() {
    const state = useStudio.getState();
    // Capture the path/source we're writing to at call time. If the
    // user switches files between dispatch and the save's resolution,
    // the post-await state writes would otherwise land on the *new*
    // file — corrupting its validation/dirty flag.
    const { activeFilePath: path, source, service } = state;
    if (!path) return;
    try {
        const v = await service.saveFile(path, source);
        if (useStudio.getState().activeFilePath === path) {
            useStudio.getState().setValidation(v);
            useStudio.getState().markClean();
        }
    } catch (e) {
        if (useStudio.getState().activeFilePath === path) {
            useStudio.getState().setRunError(String(e));
        }
    }
}

/// Run the active recipe. With no argument, reads the resolved
/// toolbar flag state from the store; pass explicit `flags` to
/// override (the keyboard's ⇧⌘R prod-shortcut, for example).
export async function runActive(flags?: RunRecipeFlags) {
    // Capture the path so the post-await writebacks below can detect
    // a file switch and skip the writes — running state for the
    // original file would otherwise corrupt the new file's view.
    const { activeFilePath: path, running, service, queryClient } = useStudio.getState();
    if (!path) {
        console.warn("runActive: no active file");
        return;
    }
    if (running) return;
    const recipes = queryClient?.getQueryData<RecipeStatus[]>(recipeStatusesKey());
    const name = recipeNameOf(path, recipes);
    if (!name) {
        console.warn(`runActive: no recipe at path: ${path}`);
        return;
    }
    await saveActive();
    // Bail if the save's await let a file switch land — the recipe
    // we're about to run no longer matches the user's active buffer.
    if (useStudio.getState().activeFilePath !== path) return;
    useStudio.getState().runBegin();
    try {
        const resolved = flags ?? {
            // Read the toolbar's resolved values straight off the
            // store. Sending the explicit shape (rather than null)
            // keeps the backend log honest about what the user
            // picked.
            sample_limit: useStudio.getState().runFlags.sample_limit,
            replay: useStudio.getState().runFlags.replay,
            ephemeral: useStudio.getState().runFlags.ephemeral,
        };
        const r = await service.runRecipe(name, resolved);
        if (useStudio.getState().activeFilePath !== path) return;
        if (r.ok && r.snapshot) {
            useStudio.getState().setSnapshot(r.snapshot);
        } else {
            useStudio.getState().setRunError(r.error || "unknown error");
        }
    } catch (e) {
        if (useStudio.getState().activeFilePath === path) {
            useStudio.getState().setRunError(String(e));
        }
    } finally {
        if (useStudio.getState().activeFilePath === path) {
            useStudio.getState().runFinish();
        }
    }
}

export function cancelActive() {
    const { running, paused, service } = useStudio.getState();
    if (!running) return;
    // If we're paused in the debugger, the cancellation has to go
    // through the debugger too — the engine task is awaiting on the
    // resume oneshot and won't see the cancel notify until it wakes.
    if (paused) {
        service.debugResume("stop").catch((e) =>
            console.warn("debug stop failed", e),
        );
    }
    service.cancelRun().catch((e) => console.warn("cancel failed", e));
}

export async function createAndOpenRecipe(qc: QueryClient) {
    const service = useStudio.getState().service;
    try {
        // `create_recipe` returns the recipe header name. The file
        // is at `<workspace>/<name>.forage`; pass both to the store
        // so the active selection is set immediately, without
        // waiting for the recipe-statuses cache to refetch the new
        // entry.
        const name = await service.createRecipe();
        qc.invalidateQueries({ queryKey: ["files"] });
        qc.invalidateQueries({ queryKey: recipeStatusesKey() });
        await useStudio.getState().setActiveRecipeName(name, `${name}.forage`);
    } catch (e) {
        useStudio.getState().setRunError(String(e));
    }
}

/// Workspace lifecycle actions. Each invalidates the boot query so the
/// top-level App branch flips between Welcome and StudioShell, plus the
/// recents bucket so the Welcome view reflects the freshly-opened entry
/// next time the user returns.
async function pickWorkspaceDirectory(title: string): Promise<string | null> {
    return useStudio.getState().service.pickDirectory(title);
}

/// Hook errors into BOTH the store (so the UI banner renders) and the
/// console (so DevTools shows them with a stack). One-call helper to
/// avoid forgetting either path.
function surfaceError(context: string, e: unknown) {
    console.error(`[studio] ${context}:`, e);
    useStudio.getState().setRunError(`${context}: ${String(e)}`);
}

export async function openWorkspaceAction(qc: QueryClient) {
    const service = useStudio.getState().service;
    const path = await pickWorkspaceDirectory("Open workspace");
    if (!path) return;
    try {
        await service.openWorkspace(path);
        await Promise.all([
            qc.invalidateQueries({ queryKey: currentWorkspaceKey() }),
            qc.invalidateQueries({ queryKey: recentWorkspacesKey() }),
        ]);
    } catch (e) {
        surfaceError("open workspace failed", e);
    }
}

export async function newWorkspaceAction(qc: QueryClient) {
    const service = useStudio.getState().service;
    const path = await pickWorkspaceDirectory("New workspace");
    if (!path) return;
    try {
        await service.newWorkspace(path);
        await Promise.all([
            qc.invalidateQueries({ queryKey: currentWorkspaceKey() }),
            qc.invalidateQueries({ queryKey: recentWorkspacesKey() }),
        ]);
    } catch (e) {
        surfaceError("new workspace failed", e);
    }
}

export async function openRecentWorkspaceAction(qc: QueryClient, path: string) {
    const service = useStudio.getState().service;
    try {
        await service.openWorkspace(path);
        await Promise.all([
            qc.invalidateQueries({ queryKey: currentWorkspaceKey() }),
            qc.invalidateQueries({ queryKey: recentWorkspacesKey() }),
        ]);
    } catch (e) {
        surfaceError(`open recent workspace failed (${path})`, e);
    }
}

export async function closeWorkspaceAction(qc: QueryClient) {
    const service = useStudio.getState().service;
    try {
        await service.closeWorkspace();
        await qc.invalidateQueries({ queryKey: currentWorkspaceKey() });
    } catch (e) {
        surfaceError("close workspace failed", e);
    }
}
