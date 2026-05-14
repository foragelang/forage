//! Workspace sidebar — workspace header, Runs, Dependencies, Files,
//! daemon footer. Ported from `design/Sidebar.v2.tsx` and wired to
//! the real data sources (TanStack Query for the wire shapes, the
//! Zustand store for selection).
//!
//! Reactive-UI rule: this file does not destructure useStudio; each
//! subscription is scoped to the field the rendering branch needs.

import { useEffect, useMemo, useState } from "react";
import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import {
    Braces,
    Camera,
    ChevronDown,
    ChevronRight,
    Cloud,
    File as FileIcon,
    Folder,
    FolderOpen,
    Layers,
    Play,
    Plus,
    Settings,
    Sprout,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
    Sidebar as SidebarRoot,
    SidebarContent,
    SidebarFooter,
    SidebarGroup,
    SidebarGroupLabel,
    SidebarHeader,
    SidebarMenu,
    SidebarMenuItem,
    SidebarMenuSkeleton,
} from "@/components/ui/sidebar";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

import { api, type FileNode, type Health, type Run, type WorkspaceInfo } from "@/lib/api";
import { shortenHome, slugOf } from "@/lib/path";
import { useStudio } from "@/lib/store";

// ── module-scope context-menu plumbing ──────────────────────────────
//
// Tauri's menu event for delete-recipe is fired against the active
// recipe slug. Registering inside a render-time effect collides with
// React.StrictMode's double-mount (transformCallback fires sync, the
// unlisten promise resolves async). Mirror the workaround from the
// previous sidebar: register once per module, update a pendingHandler
// slot on every mount.

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
    const confirmed = await ask(
        `Delete "${slug}"? The recipe and its fixtures will be removed permanently.`,
        {
            title: "Delete recipe",
            kind: "warning",
            okLabel: "Delete",
            cancelLabel: "Cancel",
        },
    );
    if (!confirmed) return;
    try {
        await api.deleteRecipe(slug);
        await qc.invalidateQueries({ queryKey: ["files"] });
        const active = useStudio.getState().activeFilePath;
        if (active && slugOf(active) === slug) {
            void useStudio.getState().setActiveFilePath(null);
        }
    } catch (e) {
        console.error("[sidebar] delete failed", slug, e);
    }
}

// ── root ─────────────────────────────────────────────────────────────

export function Sidebar() {
    const qc = useQueryClient();

    const workspace = useQuery({
        queryKey: ["workspace"],
        queryFn: api.currentWorkspace,
    });
    const files = useQuery({
        queryKey: ["files"],
        queryFn: api.listWorkspaceFiles,
        refetchInterval: 4_000,
    });
    const runs = useQuery({
        queryKey: ["runs"],
        queryFn: api.listRuns,
        refetchInterval: 5_000,
    });
    const daemon = useQuery({
        queryKey: ["daemon"],
        queryFn: api.daemonStatus,
        refetchInterval: 2_000,
    });

    // Register the delete-recipe menu listener once and refresh the
    // pending handler slot on every render so the latest QueryClient
    // is captured.
    useEffect(() => {
        ensureMenuListener();
        pendingHandler = (slug) => void performDelete(slug, qc);
        return () => {
            pendingHandler = null;
        };
    }, [qc]);

    const fileChildren: FileNode[] = useMemo(() => {
        const root = files.data;
        if (!root) return [];
        return root.kind === "folder" ? root.children : [root];
    }, [files.data]);

    const deps = useMemo<Dep[]>(() => {
        const ws = workspace.data;
        if (!ws) return [];
        return Object.entries(ws.deps)
            .filter((entry): entry is [string, number] => entry[1] !== undefined)
            .map(([slug, version]) => ({ slug, version }))
            .sort((a, b) => a.slug.localeCompare(b.slug));
    }, [workspace.data]);

    return (
        <SidebarRoot collapsible="icon">
            <WorkspaceHeader workspace={workspace.data ?? null} />
            <SidebarContent>
                <RunsSection runs={runs.data ?? []} loading={runs.isLoading} />
                <DepsSection deps={deps} />
                <FilesSection
                    files={fileChildren}
                    loading={files.isLoading}
                    onNewFile={async () => {
                        try {
                            const slug = await api.createRecipe();
                            await qc.invalidateQueries({ queryKey: ["files"] });
                            await useStudio
                                .getState()
                                .setActiveFilePath(`${slug}/recipe.forage`);
                        } catch (e) {
                            useStudio.getState().setRunError(String(e));
                        }
                    }}
                />
            </SidebarContent>
            <DaemonStatusFooter
                running={daemon.data?.running ?? false}
                version={daemon.data?.version ?? "?"}
                activeCount={daemon.data?.active_count ?? 0}
            />
        </SidebarRoot>
    );
}

type Dep = { slug: string; version: number };

// ── workspace header ─────────────────────────────────────────────────

function WorkspaceHeader({ workspace }: { workspace: WorkspaceInfo | null }) {
    const display = workspace ? shortenHome(workspace.root, workspace.home) : "";
    return (
        <SidebarHeader className="border-b">
            <Tooltip>
                <TooltipTrigger asChild>
                    <button
                        type="button"
                        // Workspace switcher is Phase-5+; the click
                        // logs a marker so the affordance never lies
                        // about doing nothing.
                        onClick={() =>
                            console.info("workspace switch: Phase 5+")
                        }
                        className={cn(
                            "flex w-full items-center gap-2 px-1.5 py-1 text-left",
                            "rounded-sm transition-colors hover:bg-sidebar-accent",
                            "group-data-[collapsible=icon]:justify-center",
                        )}
                    >
                        <span
                            className={cn(
                                "min-w-0 flex-1 truncate font-mono text-xs text-sidebar-foreground/80",
                                "group-data-[collapsible=icon]:hidden",
                            )}
                        >
                            {display}
                        </span>
                        <ChevronDown
                            className={cn(
                                "size-3 text-sidebar-foreground/50 shrink-0",
                                "group-data-[collapsible=icon]:hidden",
                            )}
                        />
                    </button>
                </TooltipTrigger>
                <TooltipContent side="right">
                    Switch workspace (coming soon)
                </TooltipContent>
            </Tooltip>
        </SidebarHeader>
    );
}

// ── runs section ─────────────────────────────────────────────────────

function RunsSection({ runs, loading }: { runs: Run[]; loading: boolean }) {
    const enabledCount = runs.filter((r) => r.enabled).length;
    return (
        <SidebarGroup className="py-1">
            <SidebarGroupLabel className="flex items-center justify-between">
                <span>Runs</span>
                <span className="font-mono text-[10px] tabular-nums text-muted-foreground">
                    {enabledCount}/{runs.length}
                </span>
            </SidebarGroupLabel>
            <SidebarMenu>
                {loading && (
                    <SidebarMenuItem>
                        <SidebarMenuSkeleton />
                    </SidebarMenuItem>
                )}
                {!loading && runs.length === 0 && (
                    <div className="px-3 py-2 text-[11px] text-muted-foreground group-data-[collapsible=icon]:hidden">
                        Runs appear here after you Run live on a recipe.
                    </div>
                )}
                {runs.map((r) => (
                    <RunRow key={r.id} run={r} />
                ))}
            </SidebarMenu>
        </SidebarGroup>
    );
}

function RunRow({ run }: { run: Run }) {
    // Subscribe to the per-row derived boolean instead of the global
    // ids: only this row re-renders when selection moves on/off it.
    const active = useStudio(
        (s) => s.view === "deployment" && s.activeRunId === run.id,
    );
    return (
        <SidebarMenuItem
            className={cn(
                "group/run flex items-center gap-0 rounded-sm",
                "hover:bg-sidebar-accent",
                active && "bg-sidebar-accent",
            )}
        >
            <button
                type="button"
                onClick={() => {
                    useStudio.getState().setActiveRunId(run.id);
                    useStudio.getState().setView("deployment");
                }}
                className={cn(
                    "min-w-0 flex-1 flex items-center gap-2 px-2 h-7 text-left",
                    "text-sm text-sidebar-foreground",
                )}
            >
                <Cloud className="size-3.5 shrink-0 text-blue-400/70" />
                <span className="min-w-0 flex-1 truncate font-mono text-xs">
                    {run.recipe_slug}
                </span>
                <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
                    {cadenceLabel(run)}
                </span>
                <HealthDot health={run.health} />
            </button>
            <Tooltip>
                <TooltipTrigger asChild>
                    <button
                        type="button"
                        onClick={(e) => {
                            e.stopPropagation();
                            api.triggerRun(run.id)
                                .catch((err) =>
                                    console.warn("trigger_run failed", err),
                                );
                        }}
                        aria-label="Run now"
                        className={cn(
                            "flex h-7 w-6 items-center justify-center shrink-0",
                            "rounded-sm text-muted-foreground",
                            "opacity-0 group-hover/run:opacity-100",
                            "hover:bg-sidebar-accent-foreground/10 hover:text-success",
                        )}
                    >
                        <Play className="size-3 fill-current" />
                    </button>
                </TooltipTrigger>
                <TooltipContent side="right">Run now</TooltipContent>
            </Tooltip>
        </SidebarMenuItem>
    );
}

function cadenceLabel(r: Run): string {
    if (!r.enabled) return "paused";
    if (r.cadence.kind === "manual") return "manual";
    if (r.cadence.kind === "interval") {
        return `every ${r.cadence.every_n}${r.cadence.unit}`;
    }
    return r.cadence.expr;
}

function HealthDot({ health }: { health: Health }) {
    const tone =
        health === "ok"
            ? "bg-success"
            : health === "drift"
              ? "bg-warning"
              : health === "fail"
                ? "bg-destructive"
                : "bg-muted-foreground/40";
    return <span className={cn("size-1.5 shrink-0 rounded-full", tone)} />;
}

// ── deps section ─────────────────────────────────────────────────────

function DepsSection({ deps }: { deps: Dep[] }) {
    return (
        <SidebarGroup className="py-1">
            <SidebarGroupLabel className="flex items-center justify-between">
                <span>Dependencies</span>
                <span className="font-mono text-[10px] tabular-nums text-muted-foreground">
                    {deps.length}
                </span>
            </SidebarGroupLabel>
            <SidebarMenu>
                {deps.map((d) => (
                    <SidebarMenuItem key={d.slug}>
                        <div
                            className={cn(
                                "flex items-center gap-2 px-2 h-7 rounded-sm",
                                "text-sm text-sidebar-foreground",
                            )}
                        >
                            <Cloud className="size-3.5 shrink-0 text-blue-400/70" />
                            <span className="min-w-0 flex-1 truncate font-mono text-xs">
                                {d.slug}
                            </span>
                            <span className="font-mono text-[10px] text-muted-foreground">
                                v{d.version}
                            </span>
                        </div>
                    </SidebarMenuItem>
                ))}
                <SidebarMenuItem>
                    <Tooltip>
                        <TooltipTrigger asChild>
                            <button
                                type="button"
                                // Add-dependency UX is Phase 5+; the
                                // click is a logged stub so the
                                // affordance doesn't silently lie.
                                onClick={() =>
                                    console.info("add dependency: Phase 5+")
                                }
                                className={cn(
                                    "flex w-full items-center gap-2 px-2 h-6 rounded-sm",
                                    "text-xs text-muted-foreground hover:text-foreground hover:bg-sidebar-accent",
                                )}
                            >
                                <Plus className="size-3" />
                                Add dependency
                            </button>
                        </TooltipTrigger>
                        <TooltipContent side="right">
                            Add dependency (coming soon)
                        </TooltipContent>
                    </Tooltip>
                </SidebarMenuItem>
            </SidebarMenu>
        </SidebarGroup>
    );
}

// ── files section (filesystem tree) ──────────────────────────────────

function FilesSection({
    files,
    loading,
    onNewFile,
}: {
    files: FileNode[];
    loading: boolean;
    onNewFile: () => void;
}) {
    const [expanded, setExpanded] = useState<Set<string>>(() => {
        const s = new Set<string>();
        for (const f of files) if (f.kind === "folder") s.add(f.path);
        return s;
    });
    // Auto-expand top-level folders as the tree arrives. Re-running on
    // every files change would clobber the user's manual collapses; we
    // only add, never remove.
    useEffect(() => {
        setExpanded((cur) => {
            let changed = false;
            const next = new Set(cur);
            for (const f of files) {
                if (f.kind === "folder" && !next.has(f.path)) {
                    next.add(f.path);
                    changed = true;
                }
            }
            return changed ? next : cur;
        });
    }, [files]);

    const toggle = (path: string) =>
        setExpanded((s) => {
            const next = new Set(s);
            if (next.has(path)) next.delete(path);
            else next.add(path);
            return next;
        });

    return (
        <SidebarGroup className="py-1">
            <SidebarGroupLabel className="flex items-center justify-between">
                <span>Files</span>
                <Tooltip>
                    <TooltipTrigger asChild>
                        <Button
                            onClick={onNewFile}
                            size="icon-sm"
                            variant="ghost"
                            aria-label="New file"
                            className="size-4"
                        >
                            <Plus className="size-3" />
                        </Button>
                    </TooltipTrigger>
                    <TooltipContent>New recipe (⌘N)</TooltipContent>
                </Tooltip>
            </SidebarGroupLabel>
            <div className="px-1">
                {loading && (
                    <SidebarMenuItem>
                        <SidebarMenuSkeleton />
                    </SidebarMenuItem>
                )}
                {!loading && (
                    <Tree
                        nodes={files}
                        depth={0}
                        expanded={expanded}
                        onToggle={toggle}
                    />
                )}
            </div>
        </SidebarGroup>
    );
}

function Tree(props: {
    nodes: FileNode[];
    depth: number;
    expanded: Set<string>;
    onToggle: (path: string) => void;
}) {
    return (
        <>
            {props.nodes.map((n) =>
                n.kind === "folder" ? (
                    <FolderRow
                        key={n.path}
                        node={n}
                        depth={props.depth}
                        expanded={props.expanded}
                        onToggle={props.onToggle}
                    />
                ) : (
                    <FileRow key={n.path} node={n} depth={props.depth} />
                ),
            )}
        </>
    );
}

function FolderRow({
    node,
    depth,
    expanded,
    onToggle,
}: {
    node: FileNode & { kind: "folder" };
    depth: number;
    expanded: Set<string>;
    onToggle: (path: string) => void;
}) {
    const indent = 4 + depth * 12;
    const open = expanded.has(node.path);
    return (
        <>
            <button
                type="button"
                onClick={() => onToggle(node.path)}
                className={cn(
                    "flex w-full items-center gap-1 h-6 pr-2 rounded-sm",
                    "text-xs text-sidebar-foreground hover:bg-sidebar-accent",
                )}
                style={{ paddingLeft: indent }}
            >
                <ChevronRight
                    className={cn(
                        "size-3 shrink-0 text-muted-foreground transition-transform",
                        open && "rotate-90",
                    )}
                />
                {open ? (
                    <FolderOpen className="size-3.5 shrink-0 text-muted-foreground" />
                ) : (
                    <Folder className="size-3.5 shrink-0 text-muted-foreground" />
                )}
                <span className="truncate font-mono">{node.name}</span>
            </button>
            {open && (
                <Tree
                    nodes={node.children}
                    depth={depth + 1}
                    expanded={expanded}
                    onToggle={onToggle}
                />
            )}
        </>
    );
}

function FileRow({
    node,
    depth,
}: {
    node: FileNode & { kind: "file" };
    depth: number;
}) {
    // Subscribe to per-leaf-derived booleans so flipping the active
    // file (or dirtying the buffer) only re-renders the two rows
    // whose answer changed, not every sibling in the tree.
    const isActive = useStudio((s) => s.activeFilePath === node.path);
    const isDirty = useStudio(
        (s) => s.dirty && s.activeFilePath === node.path,
    );
    const slug = slugOf(node.path);
    const indent = 4 + depth * 12;
    return (
        <button
            type="button"
            onClick={() => {
                useStudio.getState().setView("editor");
                // setActiveFilePath is async (prompts on dirty switch);
                // fire-and-forget here — the store handles all the
                // state writeback internally.
                void useStudio.getState().setActiveFilePath(node.path);
            }}
            onContextMenu={(e) => {
                // Only recipe rows have a backing slug; declarations
                // and fixtures have no per-row context menu yet.
                if (!slug) return;
                e.preventDefault();
                invoke("show_recipe_context_menu", { slug }).catch((err) =>
                    console.warn("context menu failed", err),
                );
            }}
            className={cn(
                "flex w-full items-center gap-1 h-6 pr-2 rounded-sm",
                "text-xs text-sidebar-foreground hover:bg-sidebar-accent",
                isActive && "bg-sidebar-accent text-sidebar-foreground font-medium",
            )}
            style={{ paddingLeft: indent }}
        >
            <span className="size-3 shrink-0" />
            <FileKindIcon node={node} />
            <span className="min-w-0 flex-1 truncate font-mono text-left">
                {node.name}
            </span>
            {isDirty && (
                <span
                    className="size-1.5 shrink-0 rounded-full bg-warning"
                    title="Unsaved changes"
                />
            )}
        </button>
    );
}

function FileKindIcon({ node }: { node: FileNode & { kind: "file" } }) {
    if (node.name === "forage.toml") {
        return <Settings className="size-3.5 shrink-0 text-muted-foreground" />;
    }
    switch (node.file_kind) {
        case "recipe":
            return <Sprout className="size-3.5 shrink-0 text-success" />;
        case "declarations":
            return <Layers className="size-3.5 shrink-0 text-warning" />;
        case "fixture":
            return <Braces className="size-3.5 shrink-0 text-chart-3" />;
        case "snapshot":
            return <Camera className="size-3.5 shrink-0 text-chart-1" />;
        case "manifest":
            return <Settings className="size-3.5 shrink-0 text-muted-foreground" />;
        default:
            return <FileIcon className="size-3.5 shrink-0 text-muted-foreground" />;
    }
}

// ── daemon footer ────────────────────────────────────────────────────

function DaemonStatusFooter({
    running,
    version,
    activeCount,
}: {
    running: boolean;
    version: string;
    activeCount: number;
}) {
    return (
        <SidebarFooter className="border-t">
            <div
                className={cn(
                    "flex items-center gap-2 px-2 py-1.5 text-xs text-muted-foreground",
                    "group-data-[collapsible=icon]:justify-center",
                )}
            >
                <span
                    className={cn(
                        "size-2 shrink-0 rounded-full",
                        running
                            ? "bg-success ring-2 ring-success/20"
                            : "bg-muted-foreground/30",
                    )}
                />
                <span className="text-sidebar-foreground font-medium group-data-[collapsible=icon]:hidden">
                    Daemon {running ? "running" : "stopped"}
                </span>
                <span className="ml-auto font-mono text-[10px] group-data-[collapsible=icon]:hidden">
                    {activeCount} active · v{version}
                </span>
            </div>
        </SidebarFooter>
    );
}
