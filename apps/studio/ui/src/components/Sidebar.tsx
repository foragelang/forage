import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { api } from "../lib/api";
import { useStudio } from "../lib/store";

export function Sidebar() {
    const qc = useQueryClient();
    const recipes = useQuery({
        queryKey: ["recipes"],
        queryFn: api.listRecipes,
        staleTime: 3_000,
    });
    const { activeSlug, setActive } = useStudio();

    const newRecipe = async () => {
        const slug = await api.createRecipe();
        await qc.invalidateQueries({ queryKey: ["recipes"] });
        setActive(slug);
    };

    // Listen for the backend's "menu:recipe_delete" — fired when the user
    // picks Delete from the native right-click menu. Mounted once: the
    // handler reads current store state via `useStudio.getState()` rather
    // than capturing closure deps, so we don't churn the listener on
    // every render. The cancelled flag guards against StrictMode's
    // double-mount → orphaned-listener race.
    useEffect(() => {
        let cancelled = false;
        let un: (() => void) | undefined;
        const handler = async (slug: string) => {
            const confirmed = window.confirm(
                `Delete "${slug}"? This removes the recipe and its fixtures permanently.`,
            );
            if (!confirmed) return;
            try {
                await api.deleteRecipe(slug);
                // Refetch the recipe list — TanStack will re-run listRecipes
                // and the sidebar rerenders with the recipe gone.
                await qc.invalidateQueries({ queryKey: ["recipes"] });
                // If the deleted recipe was open, clear the editor.
                if (useStudio.getState().activeSlug === slug) {
                    useStudio.getState().setActive(null);
                }
            } catch (e) {
                window.alert(`Delete failed: ${e}`);
            }
        };
        listen<string>("menu:recipe_delete", (e) => handler(e.payload)).then((u) => {
            if (cancelled) u();
            else un = u;
        });
        return () => {
            cancelled = true;
            un?.();
        };
    }, [qc]);

    return (
        <aside className="border-r border-zinc-800 flex flex-col bg-zinc-950">
            <header className="px-4 py-3 border-b border-zinc-800 flex items-center justify-between">
                <span className="font-semibold tracking-tight">Forage Studio</span>
                <button
                    onClick={newRecipe}
                    className="px-2 py-1 text-xs bg-emerald-700 hover:bg-emerald-600 rounded font-medium"
                >
                    + New
                </button>
            </header>
            <ul className="flex-1 overflow-y-auto">
                {(recipes.data ?? []).map((r) => (
                    <li
                        key={r.slug}
                        onClick={() => setActive(r.slug)}
                        onContextMenu={(e) => {
                            e.preventDefault();
                            invoke("show_recipe_context_menu", {
                                slug: r.slug,
                                x: e.clientX,
                                y: e.clientY,
                            }).catch((err) => console.warn("context menu failed", err));
                        }}
                        className={`px-4 py-2 cursor-pointer hover:bg-zinc-900 border-b border-zinc-900 ${
                            activeSlug === r.slug ? "bg-zinc-800" : ""
                        }`}
                    >
                        <div className="text-sm font-medium">{r.slug}</div>
                        {r.has_fixtures && (
                            <div className="text-xs text-zinc-500 mt-0.5">has fixtures</div>
                        )}
                    </li>
                ))}
                {(recipes.data ?? []).length === 0 && (
                    <li className="px-4 py-6 text-xs text-zinc-500">
                        No recipes yet. Click <span className="font-medium">+ New</span> to
                        scaffold one under <code>~/Library/Forage/Recipes/</code>.
                    </li>
                )}
            </ul>
            <footer className="border-t border-zinc-800 px-4 py-2 text-xs text-zinc-500">
                {recipes.data?.length ?? 0} recipes
            </footer>
        </aside>
    );
}
