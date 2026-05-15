//! Editor toolbar — sidebar trigger, path crumbs, status pill, Runs
//! chip (or Configure-run shortcut), and the action buttons.
//!
//! Reactive-UI rule: every store read is a leaf — no destructuring. The
//! Runs chip subscribes through TanStack Query against `['runs']` so it
//! shares cache with the sidebar.

import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
    ChevronDown,
    ChevronRight,
    Loader2,
    Pause,
    Play,
    Save,
    Settings,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Kbd } from "@/components/ui/kbd";
import { Label } from "@/components/ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Separator } from "@/components/ui/separator";
import { SidebarTrigger } from "@/components/ui/sidebar";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

import type { Run } from "@/bindings/Run";
import { useStudioService } from "@/lib/services";
import { useRecipeNameOf } from "@/hooks/useRecipes";
import { scheduledRunsKey } from "@/lib/queryKeys";
import { useStudio } from "@/lib/store";
import { cancelActive, runActive, saveActive } from "@/lib/studioActions";

export function EditorToolbar() {
    const activeFilePath = useStudio((s) => s.activeFilePath);
    const running = useStudio((s) => s.running);
    const name = useRecipeNameOf(activeFilePath);
    const saveDisabled = !activeFilePath;
    // Run / Replay / Configure operate on a recipe-name keyed surface;
    // a header-less .forage file (declarations only) has nothing to
    // run, so the buttons are disabled even though the editor itself
    // works against the path.
    const runDisabled = !name;
    const runDisabledReason = activeFilePath && !name
        ? "This file declares no recipe."
        : undefined;
    return (
        <header className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
            <SidebarTrigger />
            <Separator orientation="vertical" className="!h-4" />
            <Crumbs path={activeFilePath} />
            <ToolbarStatus />
            <div className="ml-auto flex items-center gap-1">
                {!running && name && <RunsChipOrConfigure name={name} />}
                {(!running && name) && (
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
                            disabled={saveDisabled}
                            label="Save"
                            shortcut={["⌘", "S"]}
                            icon={<Save />}
                            variant="ghost"
                        />
                        <RunFlagsPopover disabled={runDisabled} />
                        <ToolbarButton
                            onClick={() => void runActive()}
                            disabled={runDisabled}
                            disabledReason={runDisabledReason}
                            label="Run"
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
    const name = useRecipeNameOf(path);
    if (!path) {
        return (
            <span className="font-mono text-sm text-muted-foreground select-text">
                (no file)
            </span>
        );
    }
    if (name) {
        return (
            <div className="flex items-baseline gap-1.5 text-sm select-text">
                <span className="font-mono italic text-muted-foreground">recipes</span>
                <span className="text-muted-foreground/60">/</span>
                <span className="font-mono text-foreground">{name}</span>
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
    const name = useRecipeNameOf(activeFilePath);

    const runs = useQuery({
        queryKey: ["runs"],
        queryFn: () => service.listRuns(),
    });
    const run = runs.data?.find((r) => r.recipe_name === name);
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
    let label: string;
    if (paused.kind === "step") {
        label = `step ${paused.step}`;
    } else if (paused.kind === "emit") {
        label = `emit ${paused.type_name}`;
    } else {
        // for_loop entry — total covers the empty-collection case.
        label = `for-loop $${paused.variable} · ${paused.total} item${paused.total === 1 ? "" : "s"}`;
    }
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
/// there is no `Run` for the current recipe name, we render the
/// Configure-run button instead — clicking it starts a live run, and
/// the daemon's `ensure_run` creates the entry on first success.
function RunsChipOrConfigure({ name }: { name: string }) {
    const service = useStudioService();
    const runs = useQuery({
        queryKey: ["runs"],
        queryFn: () => service.listRuns(),
    });
    const run = runs.data?.find((r) => r.recipe_name === name);
    if (!run) {
        return (
            <Tooltip>
                <TooltipTrigger asChild>
                    <Button
                        size="sm"
                        variant="ghost"
                        // "Configure run" registers the recipe with
                        // the daemon — a one-shot prod-shaped fire
                        // that creates the Run row.
                        onClick={() =>
                            void runActive({
                                sample_limit: null,
                                replay: false,
                                ephemeral: false,
                            })
                        }
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

/// Run-toolbar flag widget. Renders a small chip that summarizes the
/// current preset (dev / prod / custom) and opens a popover with three
/// toggles + a sample-size input. Picking a preset overwrites the
/// three resolved values; flipping any single toggle puts the chip in
/// "custom" mode.
function RunFlagsPopover({ disabled }: { disabled: boolean }) {
    const flags = useStudio((s) => s.runFlags);
    const setRunFlags = useStudio((s) => s.setRunFlags);
    const preset = describePreset(flags);
    return (
        <Popover>
            <PopoverTrigger asChild>
                <Button
                    size="sm"
                    variant="ghost"
                    disabled={disabled}
                    aria-label="Run flags"
                    className="gap-1.5"
                >
                    <Settings className="size-3.5" />
                    <span className="text-xs font-mono">{preset}</span>
                    <ChevronDown className="size-3" />
                </Button>
            </PopoverTrigger>
            <PopoverContent align="end" className="w-72 space-y-3">
                <div>
                    <Label className="text-xs">Preset</Label>
                    <div className="mt-1 flex gap-1">
                        <Button
                            size="sm"
                            variant={preset === "dev" ? "default" : "outline"}
                            onClick={() =>
                                setRunFlags({
                                    sample_limit: 10,
                                    replay: true,
                                    ephemeral: true,
                                })
                            }
                            className="flex-1"
                        >
                            dev
                        </Button>
                        <Button
                            size="sm"
                            variant={preset === "prod" ? "default" : "outline"}
                            onClick={() =>
                                setRunFlags({
                                    sample_limit: null,
                                    replay: false,
                                    ephemeral: false,
                                })
                            }
                            className="flex-1"
                        >
                            prod
                        </Button>
                    </div>
                </div>
                <Separator />
                <div className="space-y-2">
                    <div className="flex items-center justify-between gap-2">
                        <div className="flex flex-col">
                            <Label htmlFor="sample-limit" className="text-xs">
                                Sample limit
                            </Label>
                            <span className="text-[10px] text-muted-foreground">
                                Cap top-level for-loops; nested loops run fully.
                            </span>
                        </div>
                        <Input
                            id="sample-limit"
                            type="number"
                            min={1}
                            value={flags.sample_limit ?? ""}
                            placeholder="off"
                            onChange={(e) => {
                                const raw = e.target.value;
                                if (raw === "") {
                                    setRunFlags({ sample_limit: null });
                                    return;
                                }
                                const n = Number.parseInt(raw, 10);
                                if (Number.isNaN(n) || n < 1) return;
                                setRunFlags({ sample_limit: n });
                            }}
                            className="w-20 h-7 text-xs"
                        />
                    </div>
                    <FlagToggle
                        id="replay"
                        label="Replay"
                        hint="Use _fixtures/<recipe>.jsonl instead of live HTTP."
                        checked={flags.replay}
                        onChange={(v) => setRunFlags({ replay: v })}
                    />
                    <FlagToggle
                        id="ephemeral"
                        label="Ephemeral"
                        hint="Don't write to the daemon's persistent output store."
                        checked={flags.ephemeral}
                        onChange={(v) => setRunFlags({ ephemeral: v })}
                    />
                </div>
            </PopoverContent>
        </Popover>
    );
}

function FlagToggle(props: {
    id: string;
    label: string;
    hint: string;
    checked: boolean;
    onChange: (v: boolean) => void;
}) {
    return (
        <div className="flex items-center justify-between gap-2">
            <div className="flex flex-col">
                <Label htmlFor={props.id} className="text-xs">
                    {props.label}
                </Label>
                <span className="text-[10px] text-muted-foreground">
                    {props.hint}
                </span>
            </div>
            <button
                id={props.id}
                type="button"
                role="switch"
                aria-checked={props.checked}
                onClick={() => props.onChange(!props.checked)}
                className={cn(
                    "relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center",
                    "rounded-full transition-colors",
                    props.checked ? "bg-primary" : "bg-muted",
                )}
            >
                <span
                    className={cn(
                        "inline-block size-4 transform rounded-full bg-background",
                        "transition-transform",
                        props.checked ? "translate-x-4" : "translate-x-0.5",
                    )}
                />
            </button>
        </div>
    );
}

/// Three flag values map to a preset label: dev (all-on at default
/// values), prod (all-off), or custom (anything else). The label shows
/// in the toolbar chip so a glance tells the user which mode their
/// next run will fire in.
function describePreset(flags: {
    sample_limit: number | null;
    replay: boolean;
    ephemeral: boolean;
}): "dev" | "prod" | "custom" {
    if (flags.sample_limit === 10 && flags.replay && flags.ephemeral) {
        return "dev";
    }
    if (flags.sample_limit === null && !flags.replay && !flags.ephemeral) {
        return "prod";
    }
    return "custom";
}

function ToolbarButton(props: {
    onClick: () => void;
    disabled?: boolean;
    /// Shown in the tooltip when the button is disabled — replaces
    /// the keyboard-shortcut hint so the user sees *why* the action
    /// isn't available (e.g. "This file declares no recipe.") rather
    /// than reading the unhelpful shortcut against a greyed-out
    /// button.
    disabledReason?: string;
    label: string;
    shortcut: string[];
    icon: React.ReactNode;
    variant: "default" | "ghost";
}) {
    return (
        <Tooltip>
            <TooltipTrigger asChild>
                {/* When the underlying button is `disabled`, it stops
                    firing pointer events — which also kills the
                    Tooltip's hover trigger. Wrapping the disabled
                    button in a span keeps the tooltip reachable. */}
                <span className={props.disabled ? "inline-flex" : "contents"}>
                    <Button
                        size="sm"
                        variant={props.variant}
                        onClick={props.onClick}
                        disabled={props.disabled}
                    >
                        {props.icon}
                        {props.label}
                    </Button>
                </span>
            </TooltipTrigger>
            <TooltipContent>
                {props.disabled && props.disabledReason ? (
                    <span>{props.disabledReason}</span>
                ) : (
                    <div className="flex items-center gap-1">
                        {props.shortcut.map((k) => (
                            <Kbd key={k}>{k}</Kbd>
                        ))}
                    </div>
                )}
            </TooltipContent>
        </Tooltip>
    );
}
