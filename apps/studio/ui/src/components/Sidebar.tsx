import { useEffect } from "react";
import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";

import { api } from "../lib/api";
import { useStudio } from "../lib/store";

// Module-level listener registration. React StrictMode + Vite HMR
// double-mount the Sidebar component, and `tauri::listen` registers its
// callback synchronously via transformCallback (before the unlisten
// promise resolves), so the cancelled-flag pattern can't deregister the
// orphaned one in time. Result: each engine emit fires the React
// handler twice. We side-step that by registering the listen() exactly
// once per module load, then delegating to the latest handler via a
// module-scope slot the component updates on every render.
let pendingHandler: ((slug: string) => void) | null = null;
let listenerHandle: Promise<UnlistenFn> | null = null;

function ensureMenuListener() {
    if (listenerHandle) return;
    listenerHandle = listen<string>("menu:recipe_delete", (e) => {
        pendingHandler?.(e.payload);
    });
    if (import.meta.hot) {
        import.meta.hot.dispose(async () => {
            const un = await listenerHandle;
            un?.();
            listenerHandle = null;
            pendingHandler = null;
        });
    }
}

async function performDelete(slug: string, qc: QueryClient) {
    // Tauri's WKWebView silently no-ops `window.confirm`, so we go
    // through the dialog plugin which renders a real native NSAlert.
    const confirmed = await ask(
        `Delete "${slug}"? The recipe and its fixtures will be removed permanently.`,
        {
            title: "Delete recipe",
            kind: "warning",
            okLabel: "Delete",
            cancelLabel: "Cancel",
        },
    );
    if (!confirmed) {
        console.log("[sidebar] delete cancelled", slug);
        return;
    }
    try {
        await api.deleteRecipe(slug);
        await qc.invalidateQueries({ queryKey: ["recipes"] });
        if (useStudio.getState().activeSlug === slug) {
            useStudio.getState().setActive(null);
        }
        console.log("[sidebar] deleted", slug);
    } catch (e) {
        console.error("[sidebar] delete failed", slug, e);
    }
}

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

    // Register the singleton listener (idempotent) and update the
    // module-scope handler slot with one that closes over the current
    // QueryClient. The mounted Sidebar always "wins" — if multiple
    // Sidebars ever exist, only the most recently mounted handles the
    // event, which is what you want anyway.
    useEffect(() => {
        ensureMenuListener();
        pendingHandler = (slug) => {
            console.log("[sidebar] menu:recipe_delete received", slug);
            void performDelete(slug, qc);
        };
        return () => {
            pendingHandler = null;
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
