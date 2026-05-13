import { useEffect } from "react";
import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import { FileText, Plus } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
    Sidebar as SidebarRoot,
    SidebarContent,
    SidebarFooter,
    SidebarGroup,
    SidebarHeader,
    SidebarMenu,
    SidebarMenuButton,
    SidebarMenuItem,
    SidebarMenuSkeleton,
} from "@/components/ui/sidebar";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { api } from "@/lib/api";
import { useStudio } from "@/lib/store";

// Module-level listener registration. React StrictMode + Vite HMR
// double-mount the Sidebar, and `tauri::listen` registers its callback
// synchronously via transformCallback (before the unlisten promise
// resolves), so the cancelled-flag pattern can't deregister the
// orphaned one in time. Result: each engine emit fires the React
// handler twice. We side-step that by registering listen() exactly
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

    const items = recipes.data ?? [];

    return (
        <SidebarRoot collapsible="icon">
            <SidebarHeader className="border-b">
                <div className="flex items-center justify-between gap-2 px-1">
                    <span className="font-semibold tracking-tight text-sidebar-foreground group-data-[collapsible=icon]:hidden">
                        Forage Studio
                    </span>
                    <Tooltip>
                        <TooltipTrigger asChild>
                            <Button
                                onClick={newRecipe}
                                size="icon-sm"
                                variant="ghost"
                                aria-label="New recipe"
                            >
                                <Plus />
                            </Button>
                        </TooltipTrigger>
                        <TooltipContent side="bottom">New recipe (⌘N)</TooltipContent>
                    </Tooltip>
                </div>
            </SidebarHeader>
            <SidebarContent>
                <SidebarGroup className="p-0">
                    <SidebarMenu className="gap-0">
                        {recipes.isLoading && (
                            <>
                                <SidebarMenuItem>
                                    <SidebarMenuSkeleton />
                                </SidebarMenuItem>
                                <SidebarMenuItem>
                                    <SidebarMenuSkeleton />
                                </SidebarMenuItem>
                                <SidebarMenuItem>
                                    <SidebarMenuSkeleton />
                                </SidebarMenuItem>
                            </>
                        )}
                        {!recipes.isLoading &&
                            items.map((r) => (
                                <SidebarMenuItem
                                    key={r.slug}
                                    onContextMenu={(e) => {
                                        e.preventDefault();
                                        invoke("show_recipe_context_menu", {
                                            slug: r.slug,
                                        }).catch((err) =>
                                            console.warn("context menu failed", err),
                                        );
                                    }}
                                >
                                    <SidebarMenuButton
                                        isActive={activeSlug === r.slug}
                                        onClick={() => setActive(r.slug)}
                                        tooltip={r.slug}
                                        className="rounded-none border-b border-sidebar-border/40"
                                    >
                                        <FileText className="text-muted-foreground" />
                                        <span className="font-mono text-xs truncate">
                                            {r.slug}
                                        </span>
                                    </SidebarMenuButton>
                                </SidebarMenuItem>
                            ))}
                        {!recipes.isLoading && items.length === 0 && (
                            <div className="px-4 py-6 text-xs text-muted-foreground space-y-2 group-data-[collapsible=icon]:hidden">
                                <p>No recipes yet.</p>
                                <p>
                                    Click <span className="font-medium">+</span> to scaffold
                                    one under{" "}
                                    <code className="text-foreground">
                                        ~/Library/Forage/Recipes/
                                    </code>
                                    .
                                </p>
                            </div>
                        )}
                    </SidebarMenu>
                </SidebarGroup>
            </SidebarContent>
            <SidebarFooter className="border-t">
                <div className="px-2 py-1 text-xs text-muted-foreground tabular-nums group-data-[collapsible=icon]:hidden">
                    {items.length} {items.length === 1 ? "recipe" : "recipes"}
                </div>
            </SidebarFooter>
        </SidebarRoot>
    );
}
