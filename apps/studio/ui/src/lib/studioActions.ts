//! Imperative actions invoked from multiple surfaces (toolbar
//! buttons, keyboard shortcuts, menu events). Each pulls state from
//! `useStudio.getState()` at call time so callers don't need to
//! thread props through.
//!
//! Reactive-UI rule: nothing here subscribes to state — these are
//! command handlers. State subscriptions happen in components, never
//! here.

import type { QueryClient } from "@tanstack/react-query";

import { api } from "./api";
import { slugOf } from "./path";
import { useStudio } from "./store";

export async function saveActive() {
    // Capture the path/source we're writing to at call time. If the
    // user switches files between dispatch and the save's resolution,
    // the post-await state writes would otherwise land on the *new*
    // file — corrupting its validation/dirty flag.
    const { activeFilePath: path, source } = useStudio.getState();
    if (!path) return;
    try {
        const v = await api.saveFile(path, source);
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

export async function runActive(replay: boolean) {
    // Capture the path so the post-await writebacks below can detect
    // a file switch and skip the writes — running state for the
    // original file would otherwise corrupt the new file's view.
    const { activeFilePath: path, running } = useStudio.getState();
    if (!path) {
        console.warn("runActive: no active file");
        return;
    }
    if (running) return;
    const slug = slugOf(path);
    if (!slug) {
        console.warn(`runActive: not a recipe path: ${path}`);
        return;
    }
    await saveActive();
    // Bail if the save's await let a file switch land — the slug
    // we're about to run no longer matches the user's active buffer.
    if (useStudio.getState().activeFilePath !== path) return;
    useStudio.getState().runBegin();
    try {
        const r = await api.runRecipe(slug, replay);
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
    const { running, paused } = useStudio.getState();
    if (!running) return;
    // If we're paused in the debugger, the cancellation has to go
    // through the debugger too — the engine task is awaiting on the
    // resume oneshot and won't see the cancel notify until it wakes.
    if (paused) {
        api.debugResume("stop").catch((e) =>
            console.warn("debug stop failed", e),
        );
    }
    api.cancelRun().catch((e) => console.warn("cancel failed", e));
}

export async function createAndOpenRecipe(qc: QueryClient) {
    try {
        const slug = await api.createRecipe();
        qc.invalidateQueries({ queryKey: ["files"] });
        await useStudio.getState().setActiveFilePath(`${slug}/recipe.forage`);
    } catch (e) {
        useStudio.getState().setRunError(String(e));
    }
}
