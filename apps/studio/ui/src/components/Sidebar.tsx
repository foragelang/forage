import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { api } from "../lib/api";
import { useStudio } from "../lib/store";

type ContextMenu = {
    slug: string;
    x: number;
    y: number;
};

export function Sidebar() {
    const qc = useQueryClient();
    const recipes = useQuery({
        queryKey: ["recipes"],
        queryFn: api.listRecipes,
        staleTime: 3_000,
    });
    const { activeSlug, setActive } = useStudio();
    const [menu, setMenu] = useState<ContextMenu | null>(null);

    const newRecipe = async () => {
        const slug = await api.createRecipe();
        await qc.invalidateQueries({ queryKey: ["recipes"] });
        setActive(slug);
    };

    const deleteRecipe = async (slug: string) => {
        const confirmed = window.confirm(
            `Delete "${slug}"? This removes the recipe and its fixtures permanently.`,
        );
        if (!confirmed) return;
        try {
            await api.deleteRecipe(slug);
            await qc.invalidateQueries({ queryKey: ["recipes"] });
            if (activeSlug === slug) setActive(null);
        } catch (e) {
            window.alert(`Delete failed: ${e}`);
        }
    };

    // Dismiss the context menu on any click/scroll/escape outside it.
    useEffect(() => {
        if (!menu) return;
        const dismiss = () => setMenu(null);
        const onKey = (e: KeyboardEvent) => {
            if (e.key === "Escape") setMenu(null);
        };
        window.addEventListener("click", dismiss);
        window.addEventListener("scroll", dismiss, true);
        window.addEventListener("keydown", onKey);
        return () => {
            window.removeEventListener("click", dismiss);
            window.removeEventListener("scroll", dismiss, true);
            window.removeEventListener("keydown", onKey);
        };
    }, [menu]);

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
                            setMenu({ slug: r.slug, x: e.clientX, y: e.clientY });
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
            {menu && (
                <div
                    // Stop propagation so clicking *inside* the menu doesn't
                    // dismiss it before the button's onClick fires.
                    onClick={(e) => e.stopPropagation()}
                    style={{ top: menu.y, left: menu.x }}
                    className="fixed z-50 min-w-[160px] bg-zinc-900 border border-zinc-700 rounded shadow-lg py-1 text-sm"
                >
                    <div className="px-3 py-1 text-xs text-zinc-500 truncate">{menu.slug}</div>
                    <button
                        onClick={() => {
                            const slug = menu.slug;
                            setMenu(null);
                            deleteRecipe(slug);
                        }}
                        className="w-full text-left px-3 py-1.5 hover:bg-red-900/40 text-red-400"
                    >
                        Delete recipe…
                    </button>
                </div>
            )}
        </aside>
    );
}
