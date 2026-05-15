//! Editor toolbar — sidebar trigger, path crumbs, status pill, Runs
//! chip (or Configure-run shortcut), and the action buttons.
//!
//! Reactive-UI rule: every store read is a leaf — no destructuring. The
//! Runs chip subscribes through TanStack Query against `['runs']` so it
//! shares cache with the sidebar.

import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
    ChevronRight,
    Loader2,
    Pause,
    Play,
    RefreshCw,
    Save,
    Settings,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Separator } from "@/components/ui/separator";
import { SidebarTrigger } from "@/components/ui/sidebar";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

import type { Run } from "@/bindings/Run";
import { useStudioService } from "@/lib/services";
import { slugOf } from "@/lib/path";
import { scheduledRunsKey } from "@/lib/queryKeys";
import { useStudio } from "@/lib/store";
import { cancelActive, runActive, saveActive } from "@/lib/studioActions";

export function EditorToolbar() {
    const activeFilePath = useStudio((s) => s.activeFilePath);
    const running = useStudio((s) => s.running);
    const disabled = !activeFilePath;
    const slug = activeFilePath ? slugOf(activeFilePath) : null;
    return (
        <header className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
            <SidebarTrigger />
            <Separator orientation="vertical" className="!h-4" />
            <Crumbs path={activeFilePath} />
            <ToolbarStatus />
            <div className="ml-auto flex items-center gap-1">
                {!running && slug && <RunsChipOrConfigure slug={slug} />}
                {(!running && slug) && (
                    <Separator orientation="vertical" className="!h-4 mx-1" />
                )}
                {running ? (
                    <Button
                        variant="destructive"
                        size="sm"
                        onClick={cancelActive}
                    >
                        <Loader2 className="animate-spin" />
                        Cancel
                    </Button>
                ) : (
                    <>
                        <ToolbarButton
                            onClick={() => void saveActive()}
                            disabled={disabled}
                            label="Save"
                            shortcut={["⌘", "S"]}
                            icon={<Save />}
                            variant="ghost"
                        />
                        <ToolbarButton
                            onClick={() => void runActive(true)}
                            disabled={disabled}
                            label="Replay"
                            shortcut={["⇧", "⌘", "R"]}
                            icon={<RefreshCw />}
                            variant="ghost"
                        />
                        <ToolbarButton
                            onClick={() => void runActive(false)}
                            disabled={disabled}
                            label="Run live"
                            shortcut={["⌘", "R"]}
                            icon={<Play />}
                            variant="default"
                        />
                    </>
                )}
            </div>
        </header>
    );
}

function Crumbs({ path }: { path: string | null }) {
    if (!path) {
        return (
            <span className="font-mono text-sm text-muted-foreground select-text">
                (no file)
            </span>
        );
    }
    const slug = slugOf(path);
    if (slug) {
        return (
            <div className="flex items-baseline gap-1.5 text-sm select-text">
                <span className="font-mono italic text-muted-foreground">recipes</span>
                <span className="text-muted-foreground/60">/</span>
                <span className="font-mono text-foreground">{slug}</span>
            </div>
        );
    }
    return (
        <span className="font-mono text-sm text-foreground select-text">{path}</span>
    );
}

/// Status pill derived from running/paused/dirty/latest-scheduled-run.
/// Renders nothing in the idle-clean state.
function ToolbarStatus() {
    const service = useStudioService();
    const running = useStudio((s) => s.running);
    const paused = useStudio((s) => s.paused);
    const dirty = useStudio((s) => s.dirty);
    const activeFilePath = useStudio((s) => s.activeFilePath);
    const slug = activeFilePath ? slugOf(activeFilePath) : null;

    const runs = useQuery({
        queryKey: ["runs"],
        queryFn: () => service.listRuns(),
    });
    const run = runs.data?.find((r) => r.recipe_slug === slug);
    const scheduledRuns = useQuery({
        queryKey: scheduledRunsKey(run?.id ?? "", { limit: 1 }),
        queryFn: () => service.listScheduledRuns(run!.id, { limit: 1 }),
        enabled: !!run,
    });
    const latest = scheduledRuns.data?.[0];

    if (running) {
        // Running takes precedence — and a paused run is still running.
        if (paused) {
            return <PausedBadge />;
        }
        return <RunningBadge />;
    }
    if (dirty) {
        return (
            <Badge variant="warning">
                <span className="size-1.5 rounded-full bg-warning" />
                modified
            </Badge>
        );
    }
    if (latest && latest.outcome === "fail") {
        return (
            <Badge variant="destructive">
                <span className="size-1.5 rounded-full bg-destructive" />
                failed{latest.stall ? " · stalled" : ""}
            </Badge>
        );
    }
    if (run?.health === "drift") {
        return (
            <Badge variant="warning">
                <span className="size-1.5 rounded-full bg-warning" />
                drift detected
            </Badge>
        );
    }
    return null;
}

function PausedBadge() {
    const paused = useStudio((s) => s.paused);
    if (!paused) return null;
    const label =
        paused.kind === "step"
            ? `step ${paused.step}`
            : `iter ${paused.iteration + 1}/${paused.total} of $${paused.variable}`;
    return (
        <Badge variant="warning" className="font-mono tabular-nums">
            <Pause />
            paused at {label}
        </Badge>
    );
}

function RunningBadge() {
    const startedAt = useStudio((s) => s.runStartedAt);
    const [now, setNow] = useState(Date.now());
    useEffect(() => {
        const id = window.setInterval(() => setNow(Date.now()), 250);
        return () => window.clearInterval(id);
    }, []);
    if (!startedAt) return null;
    const seconds = Math.max(0, Math.floor((now - startedAt) / 1000));
    return (
        <Badge variant="success" className="font-mono tabular-nums">
            <span className="size-1.5 rounded-full bg-success" />
            running {seconds}s
        </Badge>
    );
}

/// The Runs chip is the inline shortcut to the Deployment view. If
/// there is no `Run` for the current recipe slug, we render the
/// Configure-run button instead — clicking it starts a live run, and
/// the daemon's `ensure_run` creates the entry on first success.
function RunsChipOrConfigure({ slug }: { slug: string }) {
    const service = useStudioService();
    const runs = useQuery({
        queryKey: ["runs"],
        queryFn: () => service.listRuns(),
    });
    const run = runs.data?.find((r) => r.recipe_slug === slug);
    if (!run) {
        return (
            <Tooltip>
                <TooltipTrigger asChild>
                    <Button
                        size="sm"
                        variant="ghost"
                        onClick={() => void runActive(false)}
                    >
                        <Settings />
                        Configure run
                    </Button>
                </TooltipTrigger>
                <TooltipContent>
                    Run live to register this recipe with the daemon
                </TooltipContent>
            </Tooltip>
        );
    }
    return <RunsChip run={run} />;
}

function RunsChip({ run }: { run: Run }) {
    const cadenceLabel = describeCadence(run);
    return (
        <Tooltip>
            <TooltipTrigger asChild>
                <button
                    type="button"
                    onClick={() => {
                        useStudio.getState().setActiveRunId(run.id);
                        useStudio.getState().setView("deployment");
                    }}
                    className={cn(
                        "flex h-7 items-center gap-1.5 rounded-md border border-border",
                        "px-2 text-xs hover:bg-muted transition-colors",
                    )}
                >
                    <HealthDot health={run.health} />
                    <span className="font-mono text-muted-foreground tabular-nums">
                        {cadenceLabel}
                    </span>
                    <ChevronRight className="size-3 text-muted-foreground" />
                    <span>Runs</span>
                </button>
            </TooltipTrigger>
            <TooltipContent>Open the deployment view</TooltipContent>
        </Tooltip>
    );
}

function HealthDot({ health }: { health: Run["health"] }) {
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

function describeCadence(r: Run): string {
    if (!r.enabled) return "paused";
    if (r.cadence.kind === "manual") return "manual";
    if (r.cadence.kind === "interval") {
        return `every ${r.cadence.every_n}${r.cadence.unit}`;
    }
    return r.cadence.expr;
}

function ToolbarButton(props: {
    onClick: () => void;
    disabled?: boolean;
    label: string;
    shortcut: string[];
    icon: React.ReactNode;
    variant: "default" | "ghost";
}) {
    return (
        <Tooltip>
            <TooltipTrigger asChild>
                <Button
                    size="sm"
                    variant={props.variant}
                    onClick={props.onClick}
                    disabled={props.disabled}
                >
                    {props.icon}
                    {props.label}
                </Button>
            </TooltipTrigger>
            <TooltipContent>
                <div className="flex items-center gap-1">
                    {props.shortcut.map((k) => (
                        <Kbd key={k}>{k}</Kbd>
                    ))}
                </div>
            </TooltipContent>
        </Tooltip>
    );
}
