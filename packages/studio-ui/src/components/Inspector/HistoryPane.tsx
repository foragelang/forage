//! "History" inspector pane. Top: trend cards (one per emitted record
//! type) over the prior 30 scheduled runs. Below: dense table of
//! session-scoped runs (every Run live / Replay from this Studio
//! session) with mode tag, relative time, counts, status pill.
//!
//! Session runs are derived from `runLog` — each `run_started` opens a
//! row, `run_succeeded`/`run_failed` closes it, `emitted` updates the
//! running count. ScheduledRun history (from the daemon) feeds the
//! trend cards because session history is short-lived but trends are
//! cumulative across daemon runs.

import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";

import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { StatusPill } from "@/components/StatusPill";
import { TrendCard } from "@/components/TrendCard";
import type { Health } from "@/bindings/Health";
import type { ScheduledRun } from "@/bindings/ScheduledRun";
import { useStudioService } from "@/lib/services";
import { slugOf } from "@/lib/path";
import { scheduledRunsKey } from "@/lib/queryKeys";
import { useStudio, type LogEntry } from "@/lib/store";
import { cn } from "@/lib/utils";

export function HistoryPane() {
    const service = useStudioService();
    const activeFilePath = useStudio((s) => s.activeFilePath);
    const slug = activeFilePath ? slugOf(activeFilePath) : null;
    const runLog = useStudio((s) => s.runLog);
    const running = useStudio((s) => s.running);

    const runs = useQuery({
        queryKey: ["runs"],
        queryFn: () => service.listRuns(),
        enabled: !!slug,
    });
    const run = runs.data?.find((r) => r.recipe_name === slug);
    const history = useQuery({
        queryKey: scheduledRunsKey(run?.id ?? "", { limit: 30 }),
        queryFn: () => service.listScheduledRuns(run!.id, { limit: 30 }),
        enabled: !!run,
    });
    const scheduledRuns = history.data ?? [];

    const types = useMemo(() => collectTypes(scheduledRuns), [scheduledRuns]);
    const sessionRuns = useMemo(
        () => collectSessionRuns(runLog, running),
        [runLog, running],
    );

    return (
        <ScrollArea className="flex-1 min-h-0">
            <div className="flex flex-col gap-3 p-3">
                {types.length > 0 && (
                    <section className="space-y-2">
                        <SectionHead
                            title={`Trends · last ${scheduledRuns.length} runs`}
                        />
                        <div className="space-y-2">
                            {types.map((typeName) => (
                                <TrendRow
                                    key={typeName}
                                    typeName={typeName}
                                    scheduledRuns={scheduledRuns}
                                />
                            ))}
                        </div>
                    </section>
                )}
                <section className="space-y-2">
                    <SectionHead title="Session runs" />
                    {sessionRuns.length === 0 ? (
                        <div className="text-xs text-muted-foreground px-1">
                            Run live or Replay from the toolbar — your runs appear here.
                        </div>
                    ) : (
                        <div className="space-y-1">
                            {sessionRuns.map((r) => (
                                <SessionRunRow key={r.id} run={r} />
                            ))}
                        </div>
                    )}
                </section>
            </div>
        </ScrollArea>
    );
}

function TrendRow({
    typeName,
    scheduledRuns,
}: {
    typeName: string;
    scheduledRuns: ScheduledRun[];
}) {
    const series = useMemo(
        () =>
            [...scheduledRuns]
                .reverse()
                .map((r) => r.counts[typeName] ?? 0),
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
            size="compact"
        />
    );
}

// ── session run derivation ───────────────────────────────────────────

type SessionRun = {
    id: string;
    startedAt: number;
    finishedAt: number | null;
    mode: "live" | "refresh";
    counts: Record<string, number>;
    outcome: "ok" | "fail" | "running";
    error: string | null;
    durationMs: number | null;
};

/// Walk the engine event stream from oldest to newest, opening a new
/// SessionRun on every `run_started` and closing it on success/fail.
/// Emit events live inside `emit_burst` aggregator entries (see
/// `runAppend` in `lib/store.ts`); their per-burst counts sum into
/// the active session's running totals. Reverse-sort so the freshest
/// run is on top.
function collectSessionRuns(log: LogEntry[], running: boolean): SessionRun[] {
    const out: SessionRun[] = [];
    let current: SessionRun | null = null;
    let id = 0;
    for (const e of log) {
        switch (e.kind) {
            case "run_started":
                current = {
                    id: `session-${id++}`,
                    startedAt: Date.now(),
                    finishedAt: null,
                    mode: e.replay ? "refresh" : "live",
                    counts: {},
                    outcome: "running",
                    error: null,
                    durationMs: null,
                };
                out.push(current);
                break;
            case "emit_burst":
                if (current) {
                    for (const [type, n] of Object.entries(e.counts)) {
                        current.counts[type] = (current.counts[type] ?? 0) + n;
                    }
                }
                break;
            case "run_succeeded":
                if (current) {
                    current.outcome = "ok";
                    current.finishedAt = Date.now();
                    current.durationMs = e.duration_ms;
                    current = null;
                }
                break;
            case "run_failed":
                if (current) {
                    current.outcome = "fail";
                    current.finishedAt = Date.now();
                    current.durationMs = e.duration_ms;
                    current.error = e.error;
                    current = null;
                }
                break;
        }
    }
    if (current && !running) {
        // Engine event stream is closed but no terminal event arrived
        // (cancelled, dropped). Mark it as failed so it doesn't look
        // perpetually-running.
        current.outcome = "fail";
        current.error = "cancelled";
    }
    return out.reverse();
}

function SessionRunRow({ run }: { run: SessionRun }) {
    const health: Health =
        run.outcome === "running"
            ? "unknown"
            : run.outcome === "ok"
              ? "ok"
              : "fail";
    return (
        <div className="rounded-md border bg-muted/20 p-2 space-y-1">
            <div className="flex items-center gap-2 text-xs">
                <ModeTag mode={run.mode} />
                <span className="font-mono text-muted-foreground tabular-nums">
                    {formatRelative(run.startedAt)}
                </span>
                <span className="ml-auto">
                    {run.outcome === "running" ? (
                        <Badge variant="warning" className="font-mono">
                            running
                        </Badge>
                    ) : (
                        <StatusPill health={health}>
                            {run.outcome === "ok"
                                ? "clean"
                                : run.error ?? "failed"}
                        </StatusPill>
                    )}
                </span>
            </div>
            <div className="flex flex-wrap items-baseline gap-3 text-[11px] font-mono text-muted-foreground">
                {Object.entries(run.counts).map(([t, n]) => (
                    <span key={t} className="flex items-baseline gap-1">
                        <span>{t}</span>
                        <span className="text-foreground tabular-nums">
                            {n.toLocaleString()}
                        </span>
                    </span>
                ))}
                {run.durationMs !== null && (
                    <span className="ml-auto tabular-nums">
                        {(run.durationMs / 1000).toFixed(1)}s
                    </span>
                )}
            </div>
        </div>
    );
}

function ModeTag({ mode }: { mode: "live" | "refresh" }) {
    return (
        <Badge
            variant="info"
            className={cn("font-mono uppercase tracking-wider text-[10px]")}
        >
            {mode}
        </Badge>
    );
}

// ── utilities ────────────────────────────────────────────────────────

function collectTypes(runs: ScheduledRun[]): string[] {
    const s = new Set<string>();
    for (const r of runs) {
        for (const t of Object.keys(r.counts)) s.add(t);
    }
    return [...s].sort();
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

function SectionHead({ title }: { title: string }) {
    return (
        <div className="px-1">
            <span className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold">
                {title}
            </span>
        </div>
    );
}
