import { useEffect, useMemo, useRef, useState } from "react";
import {
    ArrowDown,
    ArrowUp,
    CheckCircle2,
    ChevronsUpDown,
    Key,
    Loader2,
    Play,
    XCircle,
} from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
    Collapsible,
    CollapsibleContent,
    CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
    Table,
    TableBody,
    TableCell,
    TableHead,
    TableHeader,
    TableRow,
} from "@/components/ui/table";
import { cn } from "@/lib/utils";

import type { RunEvent } from "@/lib/api";
import { useStudio } from "@/lib/store";

export function SnapshotTab() {
    const { snapshot, runError, running, runLog, runCounts } = useStudio();
    const [activeType, setActiveType] = useState<string | null>(null);

    const byType = useMemo(() => {
        if (!snapshot) return new Map<string, any[]>();
        const m = new Map<string, any[]>();
        for (const r of snapshot.records) {
            const arr = m.get(r.typeName) ?? [];
            arr.push(r);
            m.set(r.typeName, arr);
        }
        return m;
    }, [snapshot]);

    if (running || (runLog.length > 0 && !snapshot && !runError)) {
        return <RunningView log={runLog} counts={runCounts} />;
    }

    if (runError) {
        return (
            <ScrollArea className="flex-1">
                <div className="p-6 space-y-4 max-w-3xl">
                    <Alert variant="destructive">
                        <XCircle />
                        <AlertTitle>Run failed</AlertTitle>
                        <AlertDescription>
                            <pre className="mt-2 whitespace-pre-wrap font-mono text-xs select-text">
                                {runError}
                            </pre>
                        </AlertDescription>
                    </Alert>
                    {runLog.length > 0 && (
                        <Collapsible>
                            <CollapsibleTrigger asChild>
                                <Button variant="ghost" size="sm">
                                    <ChevronsUpDown />
                                    Activity log
                                </Button>
                            </CollapsibleTrigger>
                            <CollapsibleContent className="mt-2">
                                <div className="rounded-lg border">
                                    <ActivityLog log={runLog} />
                                </div>
                            </CollapsibleContent>
                        </Collapsible>
                    )}
                </div>
            </ScrollArea>
        );
    }

    if (!snapshot) {
        return <EmptyState />;
    }

    const type = activeType ?? byType.keys().next().value ?? null;
    const records = type ? (byType.get(type) ?? []) : [];

    return (
        <div className="flex-1 flex min-h-0">
            <aside className="w-56 border-r shrink-0">
                <ScrollArea className="h-full">
                    <nav className="p-2 space-y-1">
                        {[...byType.entries()].map(([t, rs]) => (
                            <TypeListItem
                                key={t}
                                name={t}
                                count={rs.length}
                                active={type === t}
                                onClick={() => setActiveType(t)}
                            />
                        ))}
                    </nav>
                </ScrollArea>
            </aside>
            <div className="flex-1 min-h-0 flex flex-col">
                {records.length === 0 ? (
                    <div className="p-6 text-muted-foreground text-sm">
                        No records of this type.
                    </div>
                ) : (
                    <RecordsTable records={records} />
                )}
            </div>
        </div>
    );
}

function EmptyState() {
    return (
        <div className="flex-1 flex items-center justify-center p-6">
            <div className="text-center max-w-sm space-y-3">
                <div className="mx-auto size-12 rounded-full bg-muted flex items-center justify-center">
                    <Play className="size-5 text-muted-foreground" />
                </div>
                <div className="text-sm text-muted-foreground">
                    Click <span className="text-foreground font-medium">Run live</span> or{" "}
                    <span className="text-foreground font-medium">Replay</span> to populate
                    the snapshot.
                </div>
            </div>
        </div>
    );
}

function TypeListItem(props: {
    name: string;
    count: number;
    active: boolean;
    onClick: () => void;
}) {
    return (
        <button
            onClick={props.onClick}
            data-active={props.active}
            className={cn(
                "w-full flex items-baseline justify-between gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors",
                "hover:bg-muted",
                "data-[active=true]:bg-muted data-[active=true]:font-medium",
            )}
        >
            <span className="font-mono truncate">{props.name}</span>
            <Badge variant="secondary" className="tabular-nums shrink-0">
                {props.count}
            </Badge>
        </button>
    );
}

function RecordsTable({ records }: { records: any[] }) {
    const fields = useMemo(() => {
        const s = new Set<string>();
        for (const r of records) {
            Object.keys(r.fields ?? {}).forEach((k) => s.add(k));
        }
        return [...s];
    }, [records]);

    const truncated = records.length > 200;
    const shown = truncated ? records.slice(0, 200) : records;

    return (
        <ScrollArea className="flex-1">
            <Table>
                <TableHeader className="sticky top-0 bg-background z-10 border-b">
                    <TableRow>
                        <TableHead className="w-12 text-muted-foreground">#</TableHead>
                        {fields.map((f) => (
                            <TableHead key={f} className="font-mono text-xs">
                                {f}
                            </TableHead>
                        ))}
                    </TableRow>
                </TableHeader>
                <TableBody>
                    {shown.map((r, i) => (
                        <TableRow key={i}>
                            <TableCell className="text-muted-foreground tabular-nums">
                                {i}
                            </TableCell>
                            {fields.map((f) => (
                                <TableCell
                                    key={f}
                                    className="font-mono text-xs align-top max-w-md truncate select-text"
                                    title={String(r.fields?.[f] ?? "")}
                                >
                                    {renderCell(r.fields?.[f])}
                                </TableCell>
                            ))}
                        </TableRow>
                    ))}
                    {truncated && (
                        <TableRow>
                            <TableCell
                                colSpan={fields.length + 1}
                                className="text-muted-foreground text-center"
                            >
                                …{records.length - 200} more rows
                            </TableCell>
                        </TableRow>
                    )}
                </TableBody>
            </Table>
        </ScrollArea>
    );
}

function RunningView({
    log,
    counts,
}: {
    log: RunEvent[];
    counts: Record<string, number>;
}) {
    const countEntries = Object.entries(counts);
    return (
        <div className="flex-1 flex min-h-0">
            <aside className="w-56 border-r shrink-0">
                <ScrollArea className="h-full">
                    <div className="p-4 space-y-3">
                        <div className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold">
                            Emitting
                        </div>
                        {countEntries.length === 0 ? (
                            <div className="text-sm text-muted-foreground">(none yet)</div>
                        ) : (
                            <div className="space-y-1">
                                {countEntries.map(([t, n]) => (
                                    <div
                                        key={t}
                                        className="flex items-baseline justify-between gap-2"
                                    >
                                        <span className="text-sm font-mono truncate">
                                            {t}
                                        </span>
                                        <span className="font-mono text-sm text-success tabular-nums">
                                            {n}
                                        </span>
                                    </div>
                                ))}
                            </div>
                        )}
                    </div>
                </ScrollArea>
            </aside>
            <div className="flex-1 min-h-0">
                <ActivityLog log={log} />
            </div>
        </div>
    );
}

function ActivityLog({ log }: { log: RunEvent[] }) {
    const ref = useRef<HTMLDivElement>(null);
    useEffect(() => {
        const el = ref.current;
        if (!el) return;
        const viewport = el.querySelector(
            "[data-radix-scroll-area-viewport]",
        ) as HTMLElement | null;
        viewport?.scrollTo({ top: viewport.scrollHeight });
    }, [log.length]);
    return (
        <ScrollArea ref={ref} className="h-full">
            <div className="text-xs font-mono p-4 space-y-1 select-text">
                {log.length === 0 && (
                    <div className="text-muted-foreground flex items-center gap-2">
                        <Loader2 className="size-3 animate-spin" />
                        Waiting for engine…
                    </div>
                )}
                {log.map((e, i) => (
                    <LogLine key={i} event={e} />
                ))}
            </div>
        </ScrollArea>
    );
}

function LogLine({ event }: { event: RunEvent }) {
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
        case "emitted":
            return null;
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

function renderCell(v: unknown): string {
    if (v === null || v === undefined) return "—";
    if (typeof v === "string") return v.length > 80 ? v.slice(0, 80) + "…" : v;
    if (typeof v === "number" || typeof v === "boolean") return String(v);
    if (Array.isArray(v)) return `[${v.length}]`;
    if (typeof v === "object") return JSON.stringify(v).slice(0, 80);
    return String(v);
}
