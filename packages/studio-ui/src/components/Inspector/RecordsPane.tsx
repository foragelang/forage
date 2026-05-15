//! "Records" inspector pane. A table of records emitted by the most
//! recent scheduled run. Type selector sits up top; pagination is a
//! "Load 100 more" button at the bottom.

import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
    Tabs,
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
import { useStudioService } from "@/lib/services";
import { slugOf } from "@/lib/path";
import { scheduledRunsKey } from "@/lib/queryKeys";
import { useStudio } from "@/lib/store";

const PAGE_STEP = 100;

export function RecordsPane() {
    const service = useStudioService();
    const activeFilePath = useStudio((s) => s.activeFilePath);
    const slug = activeFilePath ? slugOf(activeFilePath) : null;

    const runs = useQuery({
        queryKey: ["runs"],
        queryFn: () => service.listRuns(),
        enabled: !!slug,
    });
    const run = runs.data?.find((r) => r.recipe_name === slug);
    const history = useQuery({
        queryKey: scheduledRunsKey(run?.id ?? "", { limit: 1 }),
        queryFn: () => service.listScheduledRuns(run!.id, { limit: 1 }),
        enabled: !!run,
    });
    const latest = history.data?.[0] ?? null;

    const types = useMemo(
        () => (latest ? Object.keys(latest.counts).sort() : []),
        [latest],
    );
    const [activeType, setActiveType] = useState<string | null>(null);
    useEffect(() => {
        // First load or recipe switch: pick the first type.
        setActiveType((prev) => {
            if (prev && types.includes(prev)) return prev;
            return types[0] ?? null;
        });
    }, [types]);

    if (!latest) {
        return (
            <ScrollArea className="flex-1 min-h-0">
                <div className="p-6 text-sm text-muted-foreground">
                    No scheduled runs yet — run live to populate this view.
                </div>
            </ScrollArea>
        );
    }

    return (
        <div className="flex-1 min-h-0 flex flex-col min-w-0">
            {/* overflow-x-auto so the type-tab row scrolls horizontally
                when there are more types than fit at the current
                inspector width. The user can also drag the inspector
                wider via the gutter on its left edge. */}
            <div className="border-b p-2 overflow-x-auto">
                <Tabs
                    value={activeType ?? ""}
                    onValueChange={(v) => setActiveType(v)}
                >
                    <TabsList variant="line" className="h-7 w-max">
                        {types.map((t) => (
                            <TabsTrigger key={t} value={t}>
                                <span className="font-mono">{t}</span>
                                <Badge variant="secondary" className="ml-1 tabular-nums">
                                    {(latest.counts[t] ?? 0).toLocaleString()}
                                </Badge>
                            </TabsTrigger>
                        ))}
                    </TabsList>
                </Tabs>
            </div>
            {activeType && (
                <RecordsTable
                    scheduledRunId={latest.id}
                    typeName={activeType}
                />
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
    const service = useStudioService();
    const [limit, setLimit] = useState(PAGE_STEP);
    // Reset the limit when the user switches type — otherwise paging
    // state from one type leaks into the next.
    useEffect(() => {
        setLimit(PAGE_STEP);
    }, [typeName, scheduledRunId]);

    const records = useQuery({
        queryKey: ["records", scheduledRunId, typeName, limit],
        queryFn: () => service.loadRunRecords(scheduledRunId, typeName, limit),
    });

    const rows = (records.data ?? []) as Array<Record<string, unknown>>;
    // Each row is the deserialized JSON of one stored record. The
    // output store's `_scheduled_run_id` / `_emitted_at` bookkeeping
    // columns are stripped in the daemon, so whatever's left is recipe
    // fields. Compute the column union here — must run before any
    // early return so the hook order stays stable across renders.
    const fields = useMemo(() => {
        const s = new Set<string>();
        for (const r of rows) {
            for (const k of Object.keys(r)) s.add(k);
        }
        return [...s];
    }, [rows]);

    if (records.isLoading && !records.data) {
        return (
            <div className="p-4 text-xs text-muted-foreground">Loading…</div>
        );
    }
    if (records.error) {
        return (
            <div className="p-4 text-xs text-destructive">
                Failed to load records: {String(records.error)}
            </div>
        );
    }

    return (
        <div className="flex-1 min-h-0 flex flex-col">
            <ScrollArea className="flex-1 min-h-0">
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
                        {rows.map((r, i) => (
                            <TableRow key={i}>
                                <TableCell className="text-muted-foreground tabular-nums">
                                    {i}
                                </TableCell>
                                {fields.map((f) => (
                                    <TableCell
                                        key={f}
                                        className="font-mono text-xs align-top max-w-md truncate select-text"
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
                <div className="border-t p-2 flex justify-center">
                    <Button
                        size="sm"
                        variant="ghost"
                        onClick={() => setLimit(limit + PAGE_STEP)}
                    >
                        Load {PAGE_STEP} more
                    </Button>
                </div>
            )}
        </div>
    );
}

function renderCell(v: unknown): string {
    if (v === null || v === undefined) return "—";
    if (typeof v === "string") return v.length > 80 ? v.slice(0, 80) + "…" : v;
    if (typeof v === "number" || typeof v === "boolean") return String(v);
    if (Array.isArray(v)) return `[${v.length}]`;
    if (typeof v === "object") {
        const ref = refValue(v);
        if (ref) return `→ ${ref.type}(${ref.id})`;
        return JSON.stringify(v).slice(0, 80);
    }
    return String(v);
}

/// A `Ref<T>` field serializes as `{_ref: string, _type: string}` per
/// `EvalValue::Ref::into_json`. Detect that shape so the table cell
/// renders the typed pointer instead of the raw JSON blob — keeps the
/// distinction between "this is a typed parent link" and "this is a
/// nested arbitrary object" obvious at a glance.
function refValue(v: object): { id: string; type: string } | null {
    const o = v as Record<string, unknown>;
    if (typeof o._ref === "string" && typeof o._type === "string") {
        return { id: o._ref, type: o._type };
    }
    return null;
}
