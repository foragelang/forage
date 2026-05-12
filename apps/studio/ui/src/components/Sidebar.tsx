import { useQuery, useQueryClient } from "@tanstack/react-query";

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
