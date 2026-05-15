//! Workspace recipes — the parsed view of every `.forage` recipe
//! plus its deployment state. Components join this against the
//! active file path via `recipeNameOf` to fire recipe-scoped
//! service calls keyed by the recipe header name.

import { useQuery } from "@tanstack/react-query";

import { useStudioService } from "@/lib/services";
import { recipeNameOf } from "@/lib/path";
import { recipeStatusesKey } from "@/lib/queryKeys";

/// All recipes in the active workspace. Returns the raw query result
/// so callers can branch on loading / error if they want; most just
/// destructure `.data`.
export function useRecipes() {
    const service = useStudioService();
    return useQuery({
        queryKey: recipeStatusesKey(),
        queryFn: () => service.listRecipeStatuses(),
        // Workspace tree polls every 4s (Sidebar.tsx); recipe statuses
        // share that cadence so a freshly-added recipe surfaces in the
        // path → name join without the user waiting for an idle
        // refetch.
        refetchInterval: 4_000,
    });
}

/// Recipe header name for `path` against the workspace's current
/// recipe set, or `null` when the path doesn't host a parsed recipe.
/// Components disable recipe-scoped affordances on null and pass the
/// resolved name to the service.
export function useRecipeNameOf(path: string | null): string | null {
    const recipes = useRecipes().data;
    if (!path) return null;
    return recipeNameOf(path, recipes);
}
