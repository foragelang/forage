//! Workspace sidebar — workspace header, Runs, Dependencies, Files,
//! daemon footer. Ported from `design/Sidebar.v2.tsx` and wired to
//! the real data sources (TanStack Query for the wire shapes, the
//! Zustand store for selection).
//!
//! Reactive-UI rule: this file does not destructure useStudio; each
//! subscription is scoped to the field the rendering branch needs.

import { useEffect, useMemo, useState } from "react";
import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
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
    Popover,
    PopoverContent,
    PopoverTrigger,
} from "@/components/ui/popover";
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

import type { FileNode } from "@/bindings/FileNode";
import type { Health } from "@/bindings/Health";
import type { Run } from "@/bindings/Run";
import type { WorkspaceInfo } from "@/bindings/WorkspaceInfo";
import { useStudioService, type StudioService, type Unsubscribe } from "@/lib/services";
import { shortenHome, slugOf } from "@/lib/path";
import { currentWorkspaceKey } from "@/lib/queryKeys";
import { useStudio } from "@/lib/store";
import {
    closeWorkspaceAction,
    openWorkspaceAction,
} from "@/lib/studioActions";
import { X } from "lucide-react";

// ── module-scope context-menu plumbing ──────────────────────────────
//
// The host's menu event for delete-recipe is fired against the active
// recipe slug. Registering inside a render-time effect collides with
// React.StrictMode's double-mount; register once per module, update a
// pendingHandler slot on every mount.

let pendingHandler: ((slug: string) => void) | null = null;
let listenerUnsubscribe: Unsubscribe | null = null;

function ensureMenuListener(service: StudioService) {
    if (listenerUnsubscribe) return;
    listenerUnsubscribe = service.onMenuEvent("menu:recipe_delete", (payload) => {
        if (typeof payload === "string") pendingHandler?.(payload);
    });
    if (import.meta.hot) {
        import.meta.hot.dispose(() => {
            listenerUnsubscribe?.();
            listenerUnsubscribe = null;
            pendingHandler = null;
        });
    }
}

async function performDelete(slug: string, qc: QueryClient, service: StudioService) {
    const confirmed = await service.confirm(
        `Delete "${slug}"? The recipe and its fixtures will be removed permanently.`,
        {
            title: "Delete recipe",
            okLabel: "Delete",
            cancelLabel: "Cancel",
        },
    );
    if (!confirmed) return;
    try {
        await service.deleteRecipe(slug);
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
    const service = useStudioService();

    const workspace = useQuery({
        queryKey: currentWorkspaceKey(),
        queryFn: () => service.currentWorkspace(),
    });
    const files = useQuery({
        queryKey: ["files"],
        queryFn: () => service.listWorkspaceFiles(),
        refetchInterval: 4_000,
    });
    const runs = useQuery({
        queryKey: ["runs"],
        queryFn: () => service.listRuns(),
        refetchInterval: 5_000,
    });
    const daemon = useQuery({
        queryKey: ["daemon"],
        queryFn: () => service.daemonStatus(),
        refetchInterval: 2_000,
    });

    // Register the delete-recipe menu listener once and refresh the
    // pending handler slot on every render so the latest QueryClient
    // is captured.
    useEffect(() => {
        ensureMenuListener(service);
        pendingHandler = (slug) => void performDelete(slug, qc, service);
        return () => {
            pendingHandler = null;
        };
    }, [qc, service]);

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
                            const slug = await service.createRecipe();
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
    const qc = useQueryClient();
    const [open, setOpen] = useState(false);

    if (!workspace) {
        // Defensive — the App-level branch swaps to Welcome when no
        // workspace is open, so we shouldn't reach this. Rendering an
        // empty header keeps the sidebar layout stable during the
        // brief moment between a `forage:workspace-closed` event and
        // the App re-rendering.
        return <SidebarHeader />;
    }

    const displayName = workspace.name
        ? workspace.name.split("/").pop() ?? workspace.name
        : workspace.root.split("/").pop() ?? workspace.root;
    const displayPath = shortenHome(workspace.root, workspace.home);

    return (
        <SidebarHeader>
            <Popover open={open} onOpenChange={setOpen}>
                <PopoverTrigger asChild>
                    <button
                        type="button"
                        className={cn(
                            "workspace-switcher-trigger",
                            "flex w-full items-center gap-2 rounded-md px-[10px] py-[7px] text-left text-[12.5px] font-medium",
                            "group-data-[collapsible=icon]:justify-center",
                        )}
                    >
                        <span className="ws-folder inline-flex">
                            <Folder className="size-[14px]" />
                        </span>
                        <span
                            className={cn(
                                "ws-name min-w-0 flex-1 truncate",
                                "group-data-[collapsible=icon]:hidden",
                            )}
                        >
                            {displayName}
                        </span>
                        <ChevronDown
                            className={cn(
                                "ws-chev size-[14px] shrink-0 transition-transform",
                                open && "rotate-180",
                                "group-data-[collapsible=icon]:hidden",
                            )}
                        />
                    </button>
                </PopoverTrigger>
                <PopoverContent
                    align="start"
                    sideOffset={6}
                    className="workspace-switcher-popover w-[228px] rounded-md p-[5px]"
                >
                    <div className="flex flex-col gap-[2px] px-[10px] pt-2 pb-[6px]">
                        <span className="ws-pop-label text-[9px] font-semibold uppercase tracking-[0.1em]">
                            Current workspace
                        </span>
                        <span
                            className="ws-pop-path overflow-hidden text-ellipsis whitespace-nowrap font-mono text-[11px]"
                            title={workspace.root}
                        >
                            {displayPath}
                        </span>
                    </div>
                    <div className="ws-pop-divider mx-1 my-1 h-px" />
                    <button
                        type="button"
                        onClick={() => {
                            setOpen(false);
                            void openWorkspaceAction(qc);
                        }}
                        className="ws-pop-item flex w-full items-center gap-[9px] rounded-[4px] px-[10px] py-[7px] text-left text-[12.5px]"
                    >
                        <Folder className="size-[14px] opacity-80" />
                        <span className="flex-1">Open Workspace…</span>
                        <span className="inline-flex gap-[2px]">
                            <span className="ws-pop-kbd inline-flex h-[18px] min-w-[18px] items-center justify-center rounded px-1 font-mono text-[10px]">
                                ⌘
                            </span>
                            <span className="ws-pop-kbd inline-flex h-[18px] min-w-[18px] items-center justify-center rounded px-1 font-mono text-[10px]">
                                O
                            </span>
                        </span>
                    </button>
                    <button
                        type="button"
                        onClick={() => {
                            setOpen(false);
                            void closeWorkspaceAction(qc);
                        }}
                        className="ws-pop-item is-danger flex w-full items-center gap-[9px] rounded-[4px] px-[10px] py-[7px] text-left text-[12.5px]"
                    >
                        <X className="size-[14px] opacity-80" />
                        <span className="flex-1">Close Workspace</span>
                        <span className="inline-flex gap-[2px]">
                            <span className="ws-pop-kbd inline-flex h-[18px] min-w-[18px] items-center justify-center rounded px-1 font-mono text-[10px]">
                                ⌘
                            </span>
                            <span className="ws-pop-kbd inline-flex h-[18px] min-w-[18px] items-center justify-center rounded px-1 font-mono text-[10px]">
                                W
                            </span>
                        </span>
                    </button>
                </PopoverContent>
            </Popover>
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
    const service = useStudioService();
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
                            service.triggerRun(run.id)
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
                                // No handler yet — the add-dependency
                                // UX isn't built. The tooltip ("Add
                                // dependency (coming soon)") tells the
                                // user; the row is here for the visual
                                // affordance only.
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
    const service = useStudioService();
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
                service.showRecipeContextMenu(slug).catch((err) =>
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
