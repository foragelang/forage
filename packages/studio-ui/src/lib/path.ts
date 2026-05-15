//! Workspace-relative path helpers.
//!
//! The store keeps the active file as a path (e.g.
//! `trilogy-rec.forage`). The recipe header name for a path is
//! whatever the parsed workspace says it is — keyed off the recipe's
//! `name`, not derived from the directory/filename — so the lookup
//! is workspace-aware. `recipeNameOf` is that join: given a file
//! path and the workspace's recipe statuses, find the recipe header
//! name whose draft sits at that path.

import type { RecipeStatus } from "@/bindings/RecipeStatus";

/// Recipe header name for the recipe whose draft sits at `path`, or
/// `null` when `path` is not a parsed recipe in `recipes` (a
/// declarations file, a fixture, a snapshot, or a broken recipe with
/// no header). Callers disable any recipe-scoped UI affordance when
/// this returns null.
export function recipeNameOf(
    path: string,
    recipes: readonly RecipeStatus[] | undefined,
): string | null {
    if (!recipes) return null;
    for (const r of recipes) {
        if (r.draft.kind === "valid" && r.draft.path === path) return r.name;
    }
    return null;
}

/// A declarations file lives at the workspace root with a `.forage`
/// extension. Recipes are detected via `recipeNameOf`, which keys off
/// the parsed workspace; this helper covers the unparsed side of the
/// classification (declarations sibling to recipes).
export function isDeclarations(path: string): boolean {
    return !path.includes("/") && path.endsWith(".forage");
}

export function parentFolder(path: string): string {
    const i = path.lastIndexOf("/");
    return i < 0 ? "" : path.slice(0, i);
}

export function fileNameOf(path: string): string {
    const i = path.lastIndexOf("/");
    return i < 0 ? path : path.slice(i + 1);
}

/// Render `abs_path` with the user's home directory replaced by `~`.
/// Falls back to the raw absolute path when `home` is null or the
/// path doesn't sit under it. Trailing slashes on `home` are
/// normalized so `/Users/dima` and `/Users/dima/` behave identically.
export function shortenHome(absPath: string, home: string | null): string {
    if (!home) return absPath;
    const h = home.endsWith("/") ? home.slice(0, -1) : home;
    if (absPath === h) return "~";
    if (absPath.startsWith(h + "/")) return "~" + absPath.slice(h.length);
    return absPath;
}
