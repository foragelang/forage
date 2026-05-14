//! Run detail drawer — slides in from the right when a row in the
//! Deployment view's run log is clicked. Shows what that specific
//! ScheduledRun produced.

import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { Diff, Folder } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
    Sheet,
    SheetContent,
    SheetFooter,
    SheetHeader,
    SheetTitle,
} from "@/components/ui/sheet";
import {
    Tabs,
    TabsContent,
    TabsList,
    TabsTrigger,
} from "@/components/ui/tabs";
import {
    Table,
    TableBody,
    TableCell,
    TableHead,
    TableHeader,
    TableRow,
} from "@/components/ui/table";
import { StatusPill } from "@/components/StatusPill";
import { api, type Run, type ScheduledRun } from "@/lib/api";
import { useStudio } from "@/lib/store";
import { cn } from "@/lib/utils";

const PAGE_STEP = 100;

export function RunDrawer({
    run,
    scheduledRuns,
}: {
    run: Run;
    scheduledRuns: ScheduledRun[];
}) {
    const selectedId = useStudio((s) => s.selectedScheduledRunId);
    const setSelectedId = useStudio((s) => s.setSelectedScheduledRunId);
    const scheduled = scheduledRuns.find((r) => r.id === selectedId) ?? null;
    return (
        <Sheet
            open={!!scheduled}
            onOpenChange={(open) => {
                if (!open) setSelectedId(null);
            }}
        >
            <SheetContent side="right" className="sm:max-w-md w-[480px] gap-0 p-0">
                {scheduled && (
                    <DrawerBody
                        run={run}
                        scheduled={scheduled}
                        history={scheduledRuns}
                    />
                )}
            </SheetContent>
        </Sheet>
    );
}

function DrawerBody({
    run,
    scheduled,
    history,
}: {
    run: Run;
    scheduled: ScheduledRun;
    history: ScheduledRun[];
}) {
    const idx = history.findIndex((r) => r.id === scheduled.id);
    const prev = history[idx + 1] ?? null;
    const health = scheduled.outcome === "ok" ? "ok" : "fail";
    return (
        <>
            <SheetHeader className="border-b">
                <div className="flex items-center gap-2">
                    <SheetTitle className="font-mono text-sm">
                        {formatTimestamp(scheduled.at)}
                    </SheetTitle>
                    <StatusPill health={health}>
                        {scheduled.outcome === "ok"
                            ? "clean"
                            : scheduled.stall ?? "failed"}
                    </StatusPill>
                </div>
            </SheetHeader>

            <div className="flex-1 min-h-0 overflow-auto">
                <div className="px-4 py-3 grid grid-cols-3 gap-2 border-b">
                    <DrawerStat label="duration" value={`${scheduled.duration_s.toFixed(1)}s`} />
                    <DrawerStat label="trigger" value={scheduled.trigger} />
                    <DrawerStat label="output" value={shortPath(run.output)} mono />
                </div>
                <RecordsByType scheduled={scheduled} previous={prev} />
                <DrawerTabs scheduled={scheduled} />
            </div>

            <SheetFooter className="border-t flex-row p-3 gap-2">
                <Button variant="ghost" size="sm" disabled>
                    <Diff />
                    Compare to previous
                </Button>
                <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                        const parent = parentFolder(run.output);
                        shellOpen(parent).catch((e) =>
                            console.warn("open in store failed", e),
                        );
                    }}
                >
                    <Folder />
                    Open in store
                </Button>
            </SheetFooter>
        </>
    );
}

function DrawerStat({
    label,
    value,
    mono,
}: {
    label: string;
    value: string;
    mono?: boolean;
}) {
    return (
        <div className="space-y-0.5">
            <div className="text-[10px] uppercase tracking-wider text-muted-foreground">
                {label}
            </div>
            <div className={cn("text-xs truncate", mono && "font-mono")}>
                {value}
            </div>
        </div>
    );
}

function RecordsByType({
    scheduled,
    previous,
}: {
    scheduled: ScheduledRun;
    previous: ScheduledRun | null;
}) {
    const entries = Object.entries(scheduled.counts);
    if (entries.length === 0) {
        return (
            <div className="px-4 py-3 border-b text-xs text-muted-foreground">
                No records emitted.
            </div>
        );
    }
    return (
        <section className="px-4 py-3 space-y-1.5 border-b">
            <div className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold">
                Records emitted
            </div>
            <div className="space-y-1">
                {entries.map(([t, n]) => {
                    const prev = previous?.counts[t];
                    const delta = prev !== undefined ? (n ?? 0) - prev : 0;
                    return (
                        <div key={t} className="flex items-baseline gap-2 text-xs">
                            <span className="font-mono w-40 truncate">{t}</span>
                            <span className="ml-auto font-mono tabular-nums">
                                {(n ?? 0).toLocaleString()}
                            </span>
                            {previous && (
                                <DeltaBadge delta={delta} />
                            )}
                        </div>
                    );
                })}
            </div>
        </section>
    );
}

function DeltaBadge({ delta }: { delta: number }) {
    if (delta === 0) {
        return <span className="text-muted-foreground text-xs w-12 text-right">±0</span>;
    }
    const tone = delta > 0 ? "text-success" : "text-destructive";
    return (
        <span className={cn("text-xs w-12 text-right", tone)}>
            {delta > 0 ? "+" : ""}
            {delta.toLocaleString()}
        </span>
    );
}

function DrawerTabs({ scheduled }: { scheduled: ScheduledRun }) {
    const [tab, setTab] = useState<"records" | "diagnostic" | "activity">("records");
    return (
        <section className="px-4 py-3 space-y-2">
            <Tabs value={tab} onValueChange={(v) => setTab(v as typeof tab)}>
                <TabsList variant="line" className="h-7">
                    <TabsTrigger value="records">Records</TabsTrigger>
                    <TabsTrigger value="diagnostic">
                        Diagnostic
                        {scheduled.diagnostics > 0 && (
                            <Badge variant="warning" className="ml-1 tabular-nums">
                                {scheduled.diagnostics}
                            </Badge>
                        )}
                    </TabsTrigger>
                    <TabsTrigger value="activity">Activity</TabsTrigger>
                </TabsList>
                <TabsContent value="records" className="mt-2">
                    <RecordsTab scheduled={scheduled} />
                </TabsContent>
                <TabsContent value="diagnostic" className="mt-2">
                    <DiagnosticTab scheduled={scheduled} />
                </TabsContent>
                <TabsContent value="activity" className="mt-2">
                    <ActivityTab scheduled={scheduled} />
                </TabsContent>
            </Tabs>
        </section>
    );
}

function RecordsTab({ scheduled }: { scheduled: ScheduledRun }) {
    const types = useMemo(
        () => Object.keys(scheduled.counts).sort(),
        [scheduled.counts],
    );
    const [active, setActive] = useState<string | null>(null);
    useEffect(() => {
        setActive((cur) => (cur && types.includes(cur) ? cur : (types[0] ?? null)));
    }, [types]);

    if (types.length === 0) {
        return (
            <div className="text-xs text-muted-foreground">
                No records produced. See the Activity tab for why.
            </div>
        );
    }
    return (
        <div className="space-y-2">
            <div className="flex flex-wrap gap-1">
                {types.map((t) => (
                    <button
                        key={t}
                        type="button"
                        onClick={() => setActive(t)}
                        className={cn(
                            "px-2 py-0.5 rounded text-[11px] font-mono",
                            active === t ? "bg-muted text-foreground" : "text-muted-foreground",
                        )}
                    >
                        {t} · {(scheduled.counts[t] ?? 0).toLocaleString()}
                    </button>
                ))}
            </div>
            {active && (
                <RecordsTable scheduledRunId={scheduled.id} typeName={active} />
            )}
        </div>
    );
}

function RecordsTable({
    scheduledRunId,
    typeName,
}: {
    scheduledRunId: string;
    typeName: string;
}) {
    const [limit, setLimit] = useState(PAGE_STEP);
    useEffect(() => setLimit(PAGE_STEP), [scheduledRunId, typeName]);
    const records = useQuery({
        queryKey: ["records", scheduledRunId, typeName, limit],
        queryFn: () => api.loadRunRecords(scheduledRunId, typeName, limit),
    });

    if (records.isLoading && !records.data) {
        return <div className="text-xs text-muted-foreground">Loading…</div>;
    }
    if (records.error) {
        return (
            <div className="text-xs text-destructive">
                Failed to load: {String(records.error)}
            </div>
        );
    }
    const rows = (records.data ?? []) as Array<Record<string, unknown>>;
    if (rows.length === 0) {
        return <div className="text-xs text-muted-foreground">No rows.</div>;
    }
    const fields: string[] = [];
    {
        const seen = new Set<string>();
        for (const r of rows) {
            for (const k of Object.keys(r)) {
                if (!seen.has(k)) {
                    seen.add(k);
                    fields.push(k);
                }
            }
        }
    }
    return (
        <div className="space-y-2">
            <ScrollArea className="max-h-72">
                <Table>
                    <TableHeader>
                        <TableRow>
                            {fields.map((f) => (
                                <TableHead key={f} className="font-mono text-[10px]">
                                    {f}
                                </TableHead>
                            ))}
                        </TableRow>
                    </TableHeader>
                    <TableBody>
                        {rows.map((r, i) => (
                            <TableRow key={i}>
                                {fields.map((f) => (
                                    <TableCell
                                        key={f}
                                        className="font-mono text-[11px] truncate max-w-[120px] select-text"
                                        title={String(r[f] ?? "")}
                                    >
                                        {renderCell(r[f])}
                                    </TableCell>
                                ))}
                            </TableRow>
                        ))}
                    </TableBody>
                </Table>
            </ScrollArea>
            {rows.length >= limit && (
                <Button
                    size="sm"
                    variant="ghost"
                    className="w-full"
                    onClick={() => setLimit(limit + PAGE_STEP)}
                >
                    Load {PAGE_STEP} more
                </Button>
            )}
        </div>
    );
}

function DiagnosticTab({ scheduled }: { scheduled: ScheduledRun }) {
    if (scheduled.diagnostics === 0 && !scheduled.stall) {
        return (
            <div className="text-xs text-muted-foreground">
                Clean run — no diagnostic items.
            </div>
        );
    }
    // Full diagnostic detail isn't carried on the ScheduledRun row;
    // only a count + stall reason. A future phase joins to the daemon's
    // diagnostic table per-run. For now, show what we have.
    return (
        <div className="space-y-2">
            {scheduled.stall && (
                <div className="rounded-md border bg-destructive/5 p-3 text-xs">
                    <div className="font-mono text-destructive text-[11px] mb-1">
                        stalled
                    </div>
                    <div>{scheduled.stall}</div>
                </div>
            )}
            <div className="text-xs text-muted-foreground">
                {scheduled.diagnostics} engine diagnostic item
                {scheduled.diagnostics === 1 ? "" : "s"} recorded. Full
                breakdown not yet surfaced from the daemon — coming in a
                later phase.
            </div>
        </div>
    );
}

function ActivityTab({ scheduled }: { scheduled: ScheduledRun }) {
    // Activity logs aren't persisted per scheduled-run yet either. Show
    // a synthesized one-liner from the outcome until the daemon stores
    // the engine event stream.
    return (
        <div className="space-y-1 text-xs font-mono text-muted-foreground">
            <div>
                <span className="text-foreground">
                    {scheduled.outcome === "ok" ? "run succeeded" : "run failed"}
                </span>
                {" · "}
                {scheduled.duration_s.toFixed(1)}s
            </div>
            <div className="text-[11px] italic">
                Per-run activity logs aren&apos;t yet persisted — only
                cumulative counts. The live editor view captures the
                full stream as a run executes.
            </div>
        </div>
    );
}

// ── helpers ──────────────────────────────────────────────────────────

function formatTimestamp(ms: number): string {
    return new Date(ms).toLocaleString(undefined, {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
    });
}

function shortPath(p: string): string {
    const parts = p.split("/");
    return parts.length > 2 ? `…/${parts.slice(-2).join("/")}` : p;
}

function parentFolder(p: string): string {
    const i = p.lastIndexOf("/");
    return i < 0 ? p : p.slice(0, i);
}

function renderCell(v: unknown): string {
    if (v === null || v === undefined) return "—";
    if (typeof v === "string") return v.length > 60 ? v.slice(0, 60) + "…" : v;
    if (typeof v === "number" || typeof v === "boolean") return String(v);
    if (Array.isArray(v)) return `[${v.length}]`;
    if (typeof v === "object") return JSON.stringify(v).slice(0, 60);
    return String(v);
}
