//! "This run" inspector pane. Top: a summary card with status pill,
//! duration, and a progress bar. Middle: per-record-type rows with a
//! sparkline and current count + delta vs the previous run. Then
//! diagnostic cards (only when present), then the streaming activity
//! log.
//!
//! The pane is composed of small subscribers — every leaf reads the
//! exact field it needs from the store. Sparkline series come from
//! TanStack Query via the shared `scheduledRunsKey` helper so they
//! sit in the same cache buckets the deployment view uses.

import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
    AlertTriangle,
    ArrowDown,
    ArrowUp,
    CheckCircle2,
    ChevronDown,
    ChevronRight,
    Key,
    Loader2,
    Play,
    XCircle,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Sparkline } from "@/components/Sparkline";
import { StatusPill } from "@/components/StatusPill";
import type { ProgressUnit } from "@/bindings/ProgressUnit";
import type { RunEvent } from "@/bindings/RunEvent";
import type { ScheduledRun } from "@/bindings/ScheduledRun";
import { useStudioService } from "@/lib/services";
import { emitRevealLine } from "@/lib/editorCommands";
import { slugOf } from "@/lib/path";
import { scheduledRunsKey } from "@/lib/queryKeys";
import { useStudio, type EmitBurst, type LogEntry } from "@/lib/store";
import { cn } from "@/lib/utils";

export function RunPane() {
    const service = useStudioService();
    const activeFilePath = useStudio((s) => s.activeFilePath);
    const slug = activeFilePath ? slugOf(activeFilePath) : null;

    const runs = useQuery({
        queryKey: ["runs"],
        queryFn: () => service.listRuns(),
        enabled: !!slug,
    });
    const run = runs.data?.find((r) => r.recipe_slug === slug);
    const history = useQuery({
        queryKey: scheduledRunsKey(run?.id ?? "", { limit: 30 }),
        queryFn: () => service.listScheduledRuns(run!.id, { limit: 30 }),
        enabled: !!run,
    });
    const scheduledRuns = history.data ?? [];
    const latest = scheduledRuns[0] ?? null;
    const previous = scheduledRuns[1] ?? null;

    // Static analysis: the outermost-compound emit-bearing for-loop
    // is the recipe's "unit of work" (see
    // crates/forage-core/src/progress.rs). Used to scope the progress
    // bar to a single type instead of summing all emitted records,
    // and to filter the activity log so non-unit emits don't make it
    // flash through Variant / PriceObservation rows.
    const progressUnit = useQuery({
        queryKey: ["progressUnit", slug ?? ""],
        queryFn: () => service.recipeProgressUnit(slug!),
        enabled: !!slug,
    });
    // Push the inferred unit into the store so `runAppend` can filter
    // the activity log to unit-type emits only. Pre-empts the
    // first emit event in case the engine is faster than React Query.
    const setProgressUnit = useStudio((s) => s.setProgressUnit);
    useEffect(() => {
        setProgressUnit(progressUnit.data ?? null);
    }, [progressUnit.data, setProgressUnit]);

    return (
        <ScrollArea className="flex-1 min-h-0">
            <div className="flex flex-col gap-4 p-3">
                <RunSummaryCard latest={latest} unit={progressUnit.data ?? null} />
                <RecordsByType
                    history={scheduledRuns}
                    latest={latest}
                    previous={previous}
                />
                <DiagnosticCards />
                <ActivitySection />
            </div>
        </ScrollArea>
    );
}

// ── summary card ─────────────────────────────────────────────────────

function RunSummaryCard({
    latest,
    unit,
}: {
    latest: ScheduledRun | null;
    unit: ProgressUnit | null;
}) {
    const running = useStudio((s) => s.running);
    const paused = useStudio((s) => s.paused);
    const runStartedAt = useStudio((s) => s.runStartedAt);
    const runCounts = useStudio((s) => s.runCounts);

    // Decide which shape to render: a live in-progress run, a paused
    // run, or the last-known scheduled run (idle authoring mode).
    if (running) {
        return (
            <Card size="sm" className="gap-3 p-3">
                <div className="flex items-center justify-between">
                    <div className="text-xs uppercase tracking-wider text-muted-foreground font-semibold">
                        Current run
                    </div>
                    {paused ? (
                        <StatusPill health="drift">paused</StatusPill>
                    ) : (
                        <StatusPill health="ok">running</StatusPill>
                    )}
                </div>
                <RunProgress
                    counts={runCounts}
                    startedAt={runStartedAt}
                    baseline={baselineForUnit(latest, unit)}
                    unit={unit}
                    paused={!!paused}
                />
            </Card>
        );
    }
    if (!latest) {
        return (
            <Card size="sm" className="gap-2 p-3">
                <div className="text-xs uppercase tracking-wider text-muted-foreground font-semibold">
                    Last run
                </div>
                <div className="text-sm text-muted-foreground">
                    No runs yet — click Run live to record one.
                </div>
            </Card>
        );
    }
    const health: "ok" | "fail" =
        latest.outcome === "ok" ? "ok" : "fail";
    return (
        <Card size="sm" className="gap-3 p-3">
            <div className="flex items-center justify-between">
                <div className="text-xs uppercase tracking-wider text-muted-foreground font-semibold">
                    Last run
                </div>
                <StatusPill health={health}>
                    {latest.outcome === "ok" ? "clean" : latest.stall ?? "failed"}
                </StatusPill>
            </div>
            <div className="grid grid-cols-2 gap-2 text-sm">
                <Stat label="duration" value={`${latest.duration_s.toFixed(1)}s`} />
                <Stat label="when" value={formatRelative(latest.at)} />
                {latest.stall && (
                    <div className="col-span-2 flex items-baseline gap-2 text-xs">
                        <span className="text-muted-foreground">stall</span>
                        <span className="font-mono text-destructive">{latest.stall}</span>
                    </div>
                )}
            </div>
            <Progress
                value={100}
                indicatorClassName={
                    latest.outcome === "fail" ? "bg-destructive" : "bg-success"
                }
            />
        </Card>
    );
}

function RunProgress({
    counts,
    startedAt,
    baseline,
    unit,
    paused,
}: {
    counts: Record<string, number>;
    startedAt: number | null;
    baseline: number | null;
    unit: ProgressUnit | null;
    paused: boolean;
}) {
    // When a unit is inferred, total/baseline are scoped to its
    // primary type — matches the author's mental model of "how many
    // products are we through" rather than raw record count. Falls
    // back to summing every record type when no unit applies.
    const unitType = unit?.types[0] ?? null;
    const total = unitType
        ? (counts[unitType] ?? 0)
        : Object.values(counts).reduce((a, b) => a + b, 0);
    const pct = baseline ? Math.min(100, (total / baseline) * 100) : null;
    return (
        <div className="space-y-2">
            <div className="flex items-baseline justify-between text-xs">
                <span className="font-mono tabular-nums">
                    {total.toLocaleString()}
                    {baseline ? ` / ${baseline.toLocaleString()}` : ""}
                    {unitType ? ` ${unitType}` : " records"}
                    {pct !== null && ` · ${pct.toFixed(0)}%`}
                </span>
                <span className="text-muted-foreground">
                    {startedAt ? `${Math.floor((Date.now() - startedAt) / 1000)}s` : ""}
                </span>
            </div>
            <Progress
                value={pct ?? 0}
                indicatorClassName={paused ? "bg-warning" : "bg-success"}
            />
        </div>
    );
}

function Stat({ label, value }: { label: string; value: string }) {
    return (
        <div className="flex items-baseline gap-2 text-xs">
            <span className="text-muted-foreground w-16 shrink-0">{label}</span>
            <span className="font-mono tabular-nums text-foreground">{value}</span>
        </div>
    );
}

function baselineForUnit(
    latest: ScheduledRun | null,
    unit: ProgressUnit | null,
): number | null {
    if (!latest) return null;
    const unitType = unit?.types[0] ?? null;
    if (unitType) {
        const v = latest.counts[unitType];
        return typeof v === "number" ? v : null;
    }
    return Object.values(latest.counts).reduce(
        (a: number, b) => a + (b ?? 0),
        0,
    );
}

// ── records by type ──────────────────────────────────────────────────

function RecordsByType({
    history,
    latest,
    previous,
}: {
    history: ScheduledRun[];
    latest: ScheduledRun | null;
    previous: ScheduledRun | null;
}) {
    const runCounts = useStudio((s) => s.runCounts);
    const running = useStudio((s) => s.running);

    // Decide which "current counts" we show: rolling live counts when
    // running, otherwise the latest scheduled run's counts.
    const current = running ? runCounts : (latest?.counts ?? {});
    const previousCounts = previous?.counts ?? null;

    // Per-type sparkline series (oldest → newest, so the rightmost
    // sparkline dot is the most recent observation).
    const seriesByType = useMemo(() => {
        const m = new Map<string, number[]>();
        for (const t of Object.keys(current)) m.set(t, []);
        const ordered = [...history].reverse();
        for (const r of ordered) {
            for (const t of m.keys()) {
                m.get(t)!.push(r.counts[t] ?? 0);
            }
        }
        return m;
    }, [history, current]);

    const entries = Object.entries(current);
    if (entries.length === 0) {
        return null;
    }
    return (
        <section className="space-y-2">
            <SectionHead title="Records by type" />
            <div className="space-y-1.5">
                {entries.map(([typeName, count]) => {
                    const series = seriesByType.get(typeName) ?? [];
                    const prev = previousCounts?.[typeName];
                    const delta = prev !== undefined ? (count ?? 0) - prev : 0;
                    const anomalies = anomalyIndices(series);
                    return (
                        <RecordTypeRow
                            key={typeName}
                            typeName={typeName}
                            series={series}
                            count={count ?? 0}
                            delta={delta}
                            anomalies={anomalies}
                        />
                    );
                })}
            </div>
        </section>
    );
}

function RecordTypeRow({
    typeName,
    series,
    count,
    delta,
    anomalies,
}: {
    typeName: string;
    series: number[];
    count: number;
    delta: number;
    anomalies: number[];
}) {
    const tone =
        count === 0
            ? "text-destructive"
            : delta < -5
              ? "text-warning"
              : "text-success";
    return (
        <div className="flex items-center gap-2 text-xs">
            <span className="font-mono w-32 truncate">{typeName}</span>
            <Sparkline
                values={series}
                width={90}
                height={20}
                anomalies={anomalies}
                className={tone}
            />
            <span className="ml-auto font-mono tabular-nums">{count.toLocaleString()}</span>
            <DeltaBadge delta={delta} />
        </div>
    );
}

function DeltaBadge({ delta }: { delta: number }) {
    if (delta === 0) {
        return <span className="text-muted-foreground text-xs w-10 text-right">±0</span>;
    }
    const positive = delta > 0;
    const tone = positive ? "text-success" : "text-destructive";
    return (
        <span className={cn("text-xs w-10 text-right", tone)}>
            {positive ? "+" : ""}
            {delta.toLocaleString()}
        </span>
    );
}

function anomalyIndices(series: number[]): number[] {
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

// ── diagnostics ──────────────────────────────────────────────────────

function DiagnosticCards() {
    const snapshot = useStudio((s) => s.snapshot);
    const diagnostics = snapshot?.diagnostic;
    const items = useMemo(() => collectDiagnostics(diagnostics), [diagnostics]);
    if (items.length === 0) return null;
    return (
        <section className="space-y-2">
            <SectionHead
                title="Diagnostic"
                meta={`${items.length} item${items.length === 1 ? "" : "s"}`}
            />
            <div className="space-y-2">
                {items.map((item, i) => (
                    <DiagCard key={i} item={item} />
                ))}
            </div>
        </section>
    );
}

type DiagItem = {
    kind: string;
    line: number | null;
    msg: string;
    hint: string | null;
    severity: "warn" | "fail";
};

function DiagCard({ item }: { item: DiagItem }) {
    const Icon = item.severity === "fail" ? XCircle : AlertTriangle;
    const toneFg =
        item.severity === "fail" ? "text-destructive" : "text-warning";
    return (
        <Card size="sm" className="gap-1 p-3">
            <div className="flex items-center gap-2">
                <Icon className={cn("size-3.5 shrink-0", toneFg)} />
                <span className={cn("font-mono text-[11px] tracking-tight", toneFg)}>
                    {item.kind}
                </span>
                {item.line !== null && (
                    <button
                        type="button"
                        onClick={() => emitRevealLine(item.line! + 1)}
                        aria-label={`Jump to recipe line ${item.line + 1}`}
                        className="ml-auto"
                    >
                        <Badge variant="outline" className="font-mono">
                            recipe:{item.line + 1}
                        </Badge>
                    </button>
                )}
            </div>
            <div className="text-xs text-foreground">{item.msg}</div>
            {item.hint && (
                <div className="text-[11px] italic text-muted-foreground">
                    {item.hint}
                </div>
            )}
        </Card>
    );
}

function collectDiagnostics(
    d: import("@/bindings/DiagnosticReport").DiagnosticReport | undefined,
): DiagItem[] {
    if (!d) return [];
    const out: DiagItem[] = [];
    for (const r of d.unfired_capture_rules) {
        out.push({
            kind: "unfired_capture",
            line: r.line ?? null,
            msg: r.message,
            hint: null,
            severity: "warn",
        });
    }
    for (const r of d.unmatched_captures) {
        out.push({
            kind: "unmatched_capture",
            line: r.line ?? null,
            msg: r.message,
            hint: null,
            severity: "warn",
        });
    }
    for (const r of d.unmet_expectations) {
        out.push({
            kind: "unmet_expect",
            line: r.line ?? null,
            msg: r.message,
            hint: null,
            severity: "fail",
        });
    }
    for (const r of d.unhandled_affordances) {
        out.push({
            kind: "unhandled_affordance",
            line: r.line ?? null,
            msg: r.message,
            hint: null,
            severity: "warn",
        });
    }
    if (d.stall_reason) {
        out.push({
            kind: "stalled",
            line: d.stall_reason.line ?? null,
            msg: d.stall_reason.message,
            hint: null,
            severity: "fail",
        });
    }
    return out;
}

// ── activity log ─────────────────────────────────────────────────────

function ActivitySection() {
    const runLog = useStudio((s) => s.runLog);
    const running = useStudio((s) => s.running);
    return (
        <section className="space-y-2">
            <SectionHead
                title="Activity"
                meta={running ? "streaming…" : undefined}
            />
            <div className="rounded-md border bg-muted/30">
                <ActivityLog log={runLog} running={running} />
            </div>
        </section>
    );
}

function ActivityLog({ log, running }: { log: LogEntry[]; running: boolean }) {
    const ref = useRef<HTMLDivElement>(null);
    useEffect(() => {
        if (!running) return;
        const el = ref.current;
        if (!el) return;
        el.scrollTop = el.scrollHeight;
    }, [log.length, running]);
    if (log.length === 0) {
        return (
            <div className="px-3 py-2 text-xs text-muted-foreground flex items-center gap-2">
                {running && <Loader2 className="size-3 animate-spin" />}
                {running ? "Waiting for engine…" : "(no activity)"}
            </div>
        );
    }
    return (
        <div
            ref={ref}
            className="max-h-72 overflow-y-auto px-3 py-2 text-xs font-mono space-y-0.5 select-text"
        >
            {log.map((e, i) => (
                <LogLine key={i} entry={e} />
            ))}
        </div>
    );
}

function LogLine({ entry }: { entry: LogEntry }) {
    if (entry.kind === "emit_burst") return <EmitBurstLine burst={entry} />;
    if (entry.kind === "emitted") {
        // Should be unreachable — `runAppend` aggregates every
        // `Emitted` event into an `EmitBurst` entry, so this branch
        // exists only to narrow the type and surface a bug if the
        // invariant ever breaks.
        console.warn(
            "Emitted event reached LogLine; expected to be aggregated into a burst",
            entry,
        );
        return null;
    }
    return <RunEventLine event={entry} />;
}

/// Header row for the unit type + expandable child rows for every
/// other type that emitted in the same burst. Collapsed by default
/// so a long run reads as one row per `step → response → burst`
/// section.
function EmitBurstLine({ burst }: { burst: EmitBurst }) {
    const [expanded, setExpanded] = useState(false);
    // Pick the header type: prefer the recipe's unit type when it
    // actually emitted in this burst; otherwise fall back to the
    // first emitted type so we don't render an empty header.
    const headerType =
        burst.unitType && burst.counts[burst.unitType] !== undefined
            ? burst.unitType
            : (burst.typeOrder[0] ?? null);
    const headerCount = headerType ? (burst.counts[headerType] ?? 0) : 0;
    const children = burst.typeOrder.filter((t) => t !== headerType);
    const hasChildren = children.length > 0;
    return (
        <div>
            <button
                type="button"
                onClick={() => hasChildren && setExpanded((v) => !v)}
                className={
                    "flex items-center gap-2 text-success w-full text-left " +
                    (hasChildren ? "cursor-pointer" : "cursor-default")
                }
                disabled={!hasChildren}
            >
                {hasChildren ? (
                    expanded ? (
                        <ChevronDown className="size-3 text-muted-foreground" />
                    ) : (
                        <ChevronRight className="size-3 text-muted-foreground" />
                    )
                ) : (
                    <span className="size-3 flex items-center justify-center">
                        +
                    </span>
                )}
                <span>
                    emit{" "}
                    <strong className="tabular-nums">{headerCount}</strong>{" "}
                    {headerType ?? "—"}
                    {hasChildren && !expanded && (
                        <span className="text-muted-foreground">
                            {" "}
                            · +{children.length}{" "}
                            {children.length === 1 ? "type" : "types"}
                        </span>
                    )}
                </span>
            </button>
            {expanded && hasChildren && (
                <div className="ml-5 space-y-0.5">
                    {children.map((t) => (
                        <div
                            key={t}
                            className="flex items-center gap-2 text-success/80"
                        >
                            <span className="size-3 flex items-center justify-center text-muted-foreground">
                                ↳
                            </span>
                            <span>
                                <strong className="tabular-nums">
                                    {burst.counts[t] ?? 0}
                                </strong>{" "}
                                {t}
                            </span>
                        </div>
                    ))}
                </div>
            )}
        </div>
    );
}

function RunEventLine({
    event,
}: {
    event: Exclude<RunEvent, { kind: "emitted" }>;
}) {
    switch (event.kind) {
        case "run_started":
            return (
                <div className="flex items-center gap-2 text-foreground">
                    <Play className="size-3 text-muted-foreground" />
                    <span>
                        run started{" "}
                        <span className="text-muted-foreground">
                            ({event.replay ? "replay" : "live"})
                        </span>
                    </span>
                </div>
            );
        case "auth":
            return (
                <div className="flex items-center gap-2 text-foreground">
                    <Key className="size-3 text-muted-foreground" />
                    <span>
                        auth {event.flavor}:{" "}
                        <span className="text-success">{event.status}</span>
                    </span>
                </div>
            );
        case "request_sent":
            return (
                <div className="flex items-center gap-2 text-muted-foreground">
                    <ArrowUp className="size-3" />
                    <span className="text-warning">{event.method}</span>
                    <span className="text-foreground truncate">{event.url}</span>
                    {event.page > 1 && (
                        <span className="text-muted-foreground">(page {event.page})</span>
                    )}
                </div>
            );
        case "response_received": {
            const ok = event.status >= 200 && event.status < 400;
            return (
                <div className="flex items-center gap-2 text-muted-foreground">
                    <ArrowDown className="size-3" />
                    <span className={ok ? "text-success" : "text-destructive"}>
                        {event.status}
                    </span>
                    <span className="text-muted-foreground">
                        {event.duration_ms}ms · {formatBytes(event.bytes)} · {event.step}
                    </span>
                </div>
            );
        }
        case "run_succeeded":
            return (
                <div className="flex items-center gap-2 text-success">
                    <CheckCircle2 className="size-3" />
                    <span>
                        run succeeded — {event.records} records in{" "}
                        {(event.duration_ms / 1000).toFixed(1)}s
                    </span>
                </div>
            );
        case "run_failed":
            return (
                <div className="flex items-center gap-2 text-destructive">
                    <XCircle className="size-3" />
                    <span>
                        run failed in {(event.duration_ms / 1000).toFixed(1)}s: {event.error}
                    </span>
                </div>
            );
    }
}

function formatBytes(n: number): string {
    if (n < 1024) return `${n}B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
    return `${(n / 1024 / 1024).toFixed(1)}MB`;
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

function SectionHead({ title, meta }: { title: string; meta?: string }) {
    return (
        <div className="flex items-baseline justify-between px-1">
            <span className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold">
                {title}
            </span>
            {meta && (
                <span className="text-[10px] text-muted-foreground">{meta}</span>
            )}
        </div>
    );
}
