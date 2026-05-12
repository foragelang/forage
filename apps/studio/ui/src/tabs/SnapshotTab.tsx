import { useEffect, useMemo, useRef, useState } from "react";

import type { RunEvent } from "../lib/api";
import { useStudio } from "../lib/store";

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

    // While a run is in flight (no snapshot yet, no error yet), show the
    // live activity log so the user can see we're not hung.
    if (running || (runLog.length > 0 && !snapshot && !runError)) {
        return <RunningView log={runLog} counts={runCounts} />;
    }

    if (runError) {
        return (
            <div className="p-6 text-red-400">
                <div className="font-medium mb-2">Run failed</div>
                <pre className="text-xs whitespace-pre-wrap bg-zinc-900 p-3 rounded">
                    {runError}
                </pre>
                {runLog.length > 0 && (
                    <details className="mt-4 text-zinc-400">
                        <summary className="cursor-pointer text-sm">Activity log</summary>
                        <ActivityLog log={runLog} />
                    </details>
                )}
            </div>
        );
    }
    if (!snapshot) {
        return (
            <div className="p-6 text-zinc-500 text-sm">
                Click <span className="font-medium">Run live</span> or{" "}
                <span className="font-medium">Replay</span> to populate the snapshot.
            </div>
        );
    }

    const type = activeType ?? byType.keys().next().value ?? null;
    const records = type ? (byType.get(type) ?? []) : [];

    return (
        <div className="flex-1 flex min-h-0">
            <aside className="w-56 border-r border-zinc-800 overflow-y-auto">
                {[...byType.entries()].map(([t, rs]) => (
                    <div
                        key={t}
                        onClick={() => setActiveType(t)}
                        className={`px-4 py-2 cursor-pointer text-sm border-b border-zinc-900 hover:bg-zinc-900 ${
                            type === t ? "bg-zinc-800" : ""
                        }`}
                    >
                        <div className="font-medium">{t}</div>
                        <div className="text-xs text-zinc-500">{rs.length} records</div>
                    </div>
                ))}
            </aside>
            <div className="flex-1 overflow-auto">
                {records.length === 0 ? (
                    <div className="p-6 text-zinc-500 text-sm">No records of this type.</div>
                ) : (
                    <RecordsTable records={records} />
                )}
            </div>
        </div>
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

    return (
        <table className="w-full text-xs">
            <thead className="sticky top-0 bg-zinc-900 z-10">
                <tr>
                    <th className="px-3 py-2 text-left text-zinc-500 font-normal w-8">#</th>
                    {fields.map((f) => (
                        <th key={f} className="px-3 py-2 text-left text-zinc-300 font-medium">
                            {f}
                        </th>
                    ))}
                </tr>
            </thead>
            <tbody>
                {records.slice(0, 200).map((r, i) => (
                    <tr key={i} className="border-b border-zinc-900 hover:bg-zinc-900/50">
                        <td className="px-3 py-2 text-zinc-500">{i}</td>
                        {fields.map((f) => (
                            <td key={f} className="px-3 py-2 text-zinc-200 align-top">
                                {renderCell(r.fields?.[f])}
                            </td>
                        ))}
                    </tr>
                ))}
                {records.length > 200 && (
                    <tr>
                        <td colSpan={fields.length + 1} className="px-3 py-2 text-zinc-500">
                            …{records.length - 200} more
                        </td>
                    </tr>
                )}
            </tbody>
        </table>
    );
}

function RunningView(props: { log: RunEvent[]; counts: Record<string, number> }) {
    const countEntries = Object.entries(props.counts);
    return (
        <div className="flex-1 flex min-h-0">
            <aside className="w-56 border-r border-zinc-800 overflow-y-auto p-4 space-y-2">
                <div className="text-xs uppercase tracking-wide text-zinc-500">
                    Emitting
                </div>
                {countEntries.length === 0 ? (
                    <div className="text-sm text-zinc-500">(none yet)</div>
                ) : (
                    countEntries.map(([t, n]) => (
                        <div key={t} className="flex items-baseline justify-between">
                            <span className="text-sm text-zinc-200">{t}</span>
                            <span className="text-sm font-mono text-emerald-400 tabular-nums">
                                {n}
                            </span>
                        </div>
                    ))
                )}
            </aside>
            <div className="flex-1 overflow-auto">
                <ActivityLog log={props.log} />
            </div>
        </div>
    );
}

function ActivityLog({ log }: { log: RunEvent[] }) {
    // Pin the scroll to the bottom as new events stream in.
    const ref = useRef<HTMLDivElement>(null);
    useEffect(() => {
        ref.current?.scrollTo({ top: ref.current.scrollHeight });
    }, [log.length]);
    return (
        <div ref={ref} className="text-xs font-mono p-4 space-y-1 h-full overflow-auto">
            {log.length === 0 && (
                <div className="text-zinc-500">Waiting for engine…</div>
            )}
            {log.map((e, i) => (
                <LogLine key={i} event={e} />
            ))}
        </div>
    );
}

function LogLine({ event }: { event: RunEvent }) {
    switch (event.kind) {
        case "run_started":
            return (
                <div className="text-zinc-300">
                    <span className="text-zinc-500">▶</span> run started{" "}
                    <span className="text-zinc-500">
                        ({event.replay ? "replay" : "live"})
                    </span>
                </div>
            );
        case "auth":
            return (
                <div className="text-zinc-300">
                    <span className="text-zinc-500">🔑</span> auth {event.flavor}:{" "}
                    <span className="text-emerald-400">{event.status}</span>
                </div>
            );
        case "request_sent":
            return (
                <div className="text-zinc-400">
                    <span className="text-zinc-500">→</span>{" "}
                    <span className="text-amber-400">{event.method}</span>{" "}
                    <span className="text-zinc-300">{event.url}</span>
                    {event.page > 1 && (
                        <span className="text-zinc-500"> (page {event.page})</span>
                    )}
                </div>
            );
        case "response_received": {
            const ok = event.status >= 200 && event.status < 400;
            return (
                <div className="text-zinc-400">
                    <span className="text-zinc-500">←</span>{" "}
                    <span className={ok ? "text-emerald-400" : "text-red-400"}>
                        {event.status}
                    </span>{" "}
                    <span className="text-zinc-500">
                        {event.duration_ms}ms · {formatBytes(event.bytes)} ·{" "}
                        {event.step}
                    </span>
                </div>
            );
        }
        case "emitted":
            // Showing every emit floods the log; skip these — the sidebar
            // already shows per-type counts ticking up.
            return null;
        case "run_succeeded":
            return (
                <div className="text-emerald-400">
                    ✓ run succeeded — {event.records} records in{" "}
                    {(event.duration_ms / 1000).toFixed(1)}s
                </div>
            );
        case "run_failed":
            return (
                <div className="text-red-400">
                    ✗ run failed in {(event.duration_ms / 1000).toFixed(1)}s:{" "}
                    {event.error}
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
