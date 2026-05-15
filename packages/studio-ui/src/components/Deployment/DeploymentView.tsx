//! Deployment view — replaces the editor + inspector when a Run is
//! selected from the sidebar. Three sections:
//! - Header: title, status pill, action buttons, meta strip.
//! - Optional inline ScheduleEditor below the header.
//! - Body: TrendCards over the recent runs + a dense run log table.
//! - RunDrawer mounts on the right when a row is clicked.

import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
    Cloud,
    Loader2,
    Pause,
    PlayCircle,
    Settings,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
    Table,
    TableBody,
    TableCell,
    TableHead,
    TableHeader,
    TableRow,
} from "@/components/ui/table";
import { StatusPill } from "@/components/StatusPill";
import { TrendCard } from "@/components/TrendCard";
import type { Outcome } from "@/bindings/Outcome";
import type { Run } from "@/bindings/Run";
import type { ScheduledRun } from "@/bindings/ScheduledRun";
import { useStudioService } from "@/lib/services";
import { scheduledRunsKey } from "@/lib/queryKeys";
import { useStudio } from "@/lib/store";
import { cn } from "@/lib/utils";

import { RunDrawer } from "./RunDrawer";
import { ScheduleEditor } from "./ScheduleEditor";

type Range = "60" | "7d" | "30d" | "90d";

export function DeploymentView() {
    const service = useStudioService();
    const runId = useStudio((s) => s.activeRunId);

    const runs = useQuery({
        queryKey: ["runs"],
        queryFn: () => service.listRuns(),
    });
    const run = runs.data?.find((r) => r.id === runId) ?? null;

    const [range, setRange] = useState<Range>("60");
    const limit = rangeToLimit(range);
    const history = useQuery({
        queryKey: scheduledRunsKey(runId ?? "", { limit }),
        queryFn: () => service.listScheduledRuns(runId!, { limit }),
        enabled: !!runId,
    });
    const scheduledRuns = history.data ?? [];

    if (!runId) {
        return <EmptyState message="Pick a Run from the sidebar." />;
    }
    if (!run) {
        if (runs.isLoading) {
            return <EmptyState message="Loading…" />;
        }
        return <EmptyState message="Run not found." />;
    }

    return (
        <div className="flex-1 min-h-0 flex flex-col">
            <DepHeader run={run} scheduledRuns={scheduledRuns} />
            <ScrollArea className="flex-1 min-h-0">
                <div className="flex flex-col gap-4 p-4">
                    <Trends scheduledRuns={scheduledRuns} range={range} setRange={setRange} />
                    <RunLog scheduledRuns={scheduledRuns} />
                </div>
            </ScrollArea>
            <RunDrawer run={run} scheduledRuns={scheduledRuns} />
        </div>
    );
}

function EmptyState({ message }: { message: string }) {
    return (
        <div className="flex-1 flex items-center justify-center p-6 text-sm text-muted-foreground">
            {message}
        </div>
    );
}

// ── header ───────────────────────────────────────────────────────────

function DepHeader({
    run,
    scheduledRuns,
}: {
    run: Run;
    scheduledRuns: ScheduledRun[];
}) {
    const qc = useQueryClient();
    const service = useStudioService();
    const [editing, setEditing] = useState(false);
    const latest = scheduledRuns[0] ?? null;
    const okCount = scheduledRuns.slice(0, 30).filter((r) => r.outcome === "ok")
        .length;
    const failCount = scheduledRuns.slice(0, 30).length - okCount;

    const triggerNow = useMutation({
        mutationFn: () => service.triggerRun(run.id),
        onSuccess: () => {
            // Cache buckets are keyed by `["scheduledRuns", runId, { limit }]`
            // — multiple panes hold separate buckets per limit, so a flat
            // key-prefix invalidation would miss them. Match all limits.
            qc.invalidateQueries({
                predicate: (q) =>
                    Array.isArray(q.queryKey) &&
                    q.queryKey[0] === "scheduledRuns" &&
                    q.queryKey[1] === run.id,
            });
            qc.invalidateQueries({ queryKey: ["runs"] });
        },
    });
    const togglePause = useMutation({
        mutationFn: () =>
            service.configureRun(run.recipe_name, {
                cadence: run.cadence,
                output: run.output,
                enabled: !run.enabled,
            }),
        onSuccess: () => qc.invalidateQueries({ queryKey: ["runs"] }),
    });

    return (
        <div className="border-b">
            <div className="px-4 py-3 flex items-center gap-3">
                <Cloud className="size-5 text-info" />
                <div className="font-mono text-base">{run.recipe_name}</div>
                <StatusPill health={run.health} />
                <div className="ml-auto flex items-center gap-1.5">
                    <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setEditing((v) => !v)}
                        aria-expanded={editing}
                    >
                        <Settings />
                        {editing ? "Done" : "Edit schedule"}
                    </Button>
                    <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => togglePause.mutate()}
                        disabled={togglePause.isPending}
                    >
                        {run.enabled ? (
                            <>
                                <Pause />
                                Pause
                            </>
                        ) : (
                            <>
                                <PlayCircle />
                                Resume
                            </>
                        )}
                    </Button>
                    <RunNowButton
                        onClick={() => triggerNow.mutate()}
                        running={triggerNow.isPending}
                    />
                </div>
            </div>
            <div className="px-4 pb-3 flex flex-wrap items-baseline gap-x-6 gap-y-1 text-xs">
                <MetaItem
                    label="cadence"
                    value={
                        <button
                            type="button"
                            onClick={() => setEditing((v) => !v)}
                            className="font-mono underline decoration-dotted underline-offset-2 hover:text-foreground"
                        >
                            {describeCadence(run)}
                        </button>
                    }
                />
                <MetaItem
                    label="next run"
                    value={
                        <span className="font-mono">
                            {run.next_run ? formatRelative(run.next_run) : "—"}
                        </span>
                    }
                />
                <MetaItem
                    label="last run"
                    value={
                        <span className="font-mono">
                            {latest ? formatRelative(latest.at) : "—"}
                        </span>
                    }
                />
                <MetaItem
                    label="last 30 runs"
                    value={
                        <span className="font-mono">
                            {okCount} ok · {failCount} fail
                        </span>
                    }
                />
                <MetaItem
                    label="output"
                    value={
                        <span className="font-mono text-muted-foreground truncate max-w-md">
                            {run.output}
                        </span>
                    }
                />
            </div>
            {editing && <ScheduleEditor run={run} onClose={() => setEditing(false)} />}
        </div>
    );
}

/// Run-now button with live-elapsed feedback. The mutation can take many
/// seconds (the daemon awaits the whole run); a static disabled button
/// reads as "nothing happening" — the elapsed counter and spinner give
/// the same shape of feedback the live-run path has via RunningBadge.
function RunNowButton({
    onClick,
    running,
}: {
    onClick: () => void;
    running: boolean;
}) {
    const [startedAt, setStartedAt] = useState<number | null>(null);
    const [now, setNow] = useState(Date.now());

    useEffect(() => {
        if (!running) {
            setStartedAt(null);
            return;
        }
        const start = Date.now();
        setStartedAt(start);
        setNow(start);
        const id = window.setInterval(() => setNow(Date.now()), 250);
        return () => window.clearInterval(id);
    }, [running]);

    const seconds =
        startedAt !== null ? Math.max(0, Math.floor((now - startedAt) / 1000)) : 0;

    return (
        <Button size="sm" onClick={onClick} disabled={running}>
            {running ? (
                <>
                    <Loader2 className="animate-spin" />
                    <span className="font-mono tabular-nums">running {seconds}s</span>
                </>
            ) : (
                <>
                    <PlayCircle />
                    Run now
                </>
            )}
        </Button>
    );
}

function MetaItem({
    label,
    value,
}: {
    label: string;
    value: React.ReactNode;
}) {
    return (
        <div className="flex flex-col gap-0.5">
            <span className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold">
                {label}
            </span>
            <span className="text-foreground">{value}</span>
        </div>
    );
}

function describeCadence(r: Run): string {
    if (!r.enabled) return "paused";
    if (r.cadence.kind === "manual") return "manual";
    if (r.cadence.kind === "interval") {
        return `every ${r.cadence.every_n}${r.cadence.unit}`;
    }
    return r.cadence.expr;
}

// ── trends ───────────────────────────────────────────────────────────

function Trends({
    scheduledRuns,
    range,
    setRange,
}: {
    scheduledRuns: ScheduledRun[];
    range: Range;
    setRange: (r: Range) => void;
}) {
    const types = useMemo(() => collectTypes(scheduledRuns), [scheduledRuns]);
    return (
        <section className="space-y-2">
            <div className="flex items-baseline justify-between">
                <span className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold">
                    Records emitted · last {scheduledRuns.length} runs
                </span>
                <RangeToggle value={range} onChange={setRange} />
            </div>
            {types.length === 0 ? (
                <div className="rounded-md border p-4 text-sm text-muted-foreground">
                    No scheduled runs in range — trigger one to populate.
                </div>
            ) : (
                <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                    {types.map((typeName) => (
                        <TrendCardRow
                            key={typeName}
                            typeName={typeName}
                            scheduledRuns={scheduledRuns}
                        />
                    ))}
                </div>
            )}
        </section>
    );
}

function TrendCardRow({
    typeName,
    scheduledRuns,
}: {
    typeName: string;
    scheduledRuns: ScheduledRun[];
}) {
    const series = useMemo(
        () =>
            [...scheduledRuns].reverse().map((r) => r.counts[typeName] ?? 0),
        [scheduledRuns, typeName],
    );
    const last = series[series.length - 1] ?? 0;
    const prev = series[series.length - 2] ?? last;
    return (
        <TrendCard
            typeName={typeName}
            series={series}
            lastValue={last}
            delta={last - prev}
            anomalies={driftIndices(series)}
            size="wide"
        />
    );
}

function RangeToggle({
    value,
    onChange,
}: {
    value: Range;
    onChange: (r: Range) => void;
}) {
    const ranges: Range[] = ["60", "7d", "30d", "90d"];
    return (
        <div className="inline-flex items-center rounded-md border bg-muted p-0.5 text-[11px]">
            {ranges.map((r) => (
                <button
                    key={r}
                    type="button"
                    onClick={() => onChange(r)}
                    className={cn(
                        "px-2 py-0.5 rounded-sm font-mono",
                        value === r
                            ? "bg-background text-foreground shadow-sm"
                            : "text-muted-foreground hover:text-foreground",
                    )}
                >
                    {r === "60" ? "60 runs" : r}
                </button>
            ))}
        </div>
    );
}

// ── run log table ────────────────────────────────────────────────────

function RunLog({ scheduledRuns }: { scheduledRuns: ScheduledRun[] }) {
    const types = useMemo(() => collectTypes(scheduledRuns), [scheduledRuns]);
    const rows = scheduledRuns.slice(0, 80);
    const more = scheduledRuns.length - rows.length;
    return (
        <section className="space-y-2">
            <div className="flex items-baseline justify-between">
                <span className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold">
                    Run log
                </span>
                <span className="text-[10px] text-muted-foreground">
                    {scheduledRuns.length} run{scheduledRuns.length === 1 ? "" : "s"}
                </span>
            </div>
            <div className="rounded-md border">
                <Table>
                    <TableHeader>
                        <TableRow>
                            <TableHead className="w-40">when</TableHead>
                            <TableHead className="w-32">status</TableHead>
                            <TableHead className="w-16 text-right">dur</TableHead>
                            {types.map((t) => (
                                <TableHead key={t} className="text-right font-mono">
                                    {t}
                                </TableHead>
                            ))}
                        </TableRow>
                    </TableHeader>
                    <TableBody>
                        {rows.map((r) => (
                            <RunLogRow key={r.id} row={r} types={types} />
                        ))}
                    </TableBody>
                </Table>
                {more > 0 && (
                    <div className="px-3 py-2 text-xs text-muted-foreground text-center">
                        … {more} more run{more === 1 ? "" : "s"}
                    </div>
                )}
                {rows.length === 0 && (
                    <div className="px-3 py-2 text-xs text-muted-foreground text-center">
                        (no runs)
                    </div>
                )}
            </div>
        </section>
    );
}

function RunLogRow({
    row,
    types,
}: {
    row: ScheduledRun;
    types: string[];
}) {
    // Leaf-read selection so changing the selected row only re-renders
    // the two rows that flip state, not every row in the log.
    const selected = useStudio((s) => s.selectedScheduledRunId === row.id);
    const setSelectedId = useStudio((s) => s.setSelectedScheduledRunId);
    const fail = row.outcome === "fail";
    return (
        <TableRow
            data-state={selected ? "selected" : undefined}
            onClick={() => setSelectedId(row.id)}
            className={cn(
                "cursor-pointer",
                fail && "bg-destructive/5 hover:bg-destructive/10",
                !fail && "hover:bg-muted/40",
            )}
        >
            <TableCell className="font-mono text-[11px]">
                <div className="flex flex-col">
                    <span>{formatRelative(row.at)}</span>
                    <span className="text-muted-foreground text-[10px]">
                        {formatHourMinute(row.at)}
                    </span>
                </div>
            </TableCell>
            <TableCell>
                <OutcomeLabel outcome={row.outcome} stall={row.stall} />
            </TableCell>
            <TableCell className="text-right font-mono tabular-nums">
                {row.duration_s.toFixed(1)}s
            </TableCell>
            {types.map((t) => (
                <TableCell
                    key={t}
                    className="text-right font-mono tabular-nums"
                >
                    {(row.counts[t] ?? 0).toLocaleString()}
                </TableCell>
            ))}
        </TableRow>
    );
}

function OutcomeLabel({
    outcome,
    stall,
}: {
    outcome: Outcome;
    stall: string | null;
}) {
    if (outcome === "ok") {
        return (
            <span className="flex items-center gap-1.5 text-xs">
                <span className="size-1.5 rounded-full bg-success" />
                <span>ok</span>
            </span>
        );
    }
    return (
        <span className="flex items-center gap-1.5 text-xs">
            <span className="size-1.5 rounded-full bg-destructive" />
            <span className="text-destructive truncate" title={stall ?? "failed"}>
                {stall ?? "failed"}
            </span>
        </span>
    );
}

// ── utilities ────────────────────────────────────────────────────────

function rangeToLimit(r: Range): number {
    if (r === "60") return 60;
    if (r === "7d") return 80;
    if (r === "30d") return 120;
    return 200;
}

function collectTypes(runs: ScheduledRun[]): string[] {
    const seen = new Set<string>();
    for (const r of runs) for (const t of Object.keys(r.counts)) seen.add(t);
    return [...seen].sort();
}

function driftIndices(series: number[]): number[] {
    if (series.length < 3) return [];
    const sorted = [...series].sort((a, b) => a - b);
    const median = sorted[Math.floor(sorted.length / 2)] ?? 0;
    if (median === 0) return [];
    const out: number[] = [];
    series.forEach((v, i) => {
        if (v <= median * 0.7) out.push(i);
    });
    return out;
}

function formatRelative(ms: number): string {
    const diff = (Date.now() - ms) / 1000;
    if (diff < 60) return "just now";
    if (diff < 3600) return `${Math.round(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.round(diff / 3600)}h ago`;
    if (diff < 86400 * 2) return "yesterday";
    if (diff < 86400 * 7) return `${Math.round(diff / 86400)}d ago`;
    return new Date(ms).toLocaleDateString(undefined, {
        month: "short",
        day: "numeric",
    });
}

function formatHourMinute(ms: number): string {
    return new Date(ms).toLocaleTimeString(undefined, {
        hour: "2-digit",
        minute: "2-digit",
        hour12: false,
    });
}
