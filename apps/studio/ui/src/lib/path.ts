//! Workspace-relative path helpers.
//!
//! The store keeps the active file as a path (e.g.
//! `trilogy-rec/recipe.forage`). Slugs are derived from the path
//! shape — there is no separate `activeSlug` state. These helpers
//! keep that derivation in one place.

/// Recipe paths are `<slug>/recipe.forage`. Everything else returns
/// null — declarations, fixtures, manifests, etc. don't have a slug.
export function slugOf(path: string): string | null {
    const parts = path.split("/");
    return parts.length === 2 && parts[1] === "recipe.forage" ? parts[0] : null;
}

export function isRecipe(path: string): boolean {
    return slugOf(path) !== null;
}

/// A declarations file lives at the workspace root with a `.forage`
/// extension. Recipes go in `<slug>/recipe.forage`, which is excluded
/// here by the no-slash check.
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
