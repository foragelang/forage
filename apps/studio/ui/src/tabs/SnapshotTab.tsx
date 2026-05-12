import { useMemo, useState } from "react";

import { useStudio } from "../lib/store";

export function SnapshotTab() {
    const { snapshot, runError } = useStudio();
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

    if (runError) {
        return (
            <div className="p-6 text-red-400">
                <div className="font-medium mb-2">Run failed</div>
                <pre className="text-xs whitespace-pre-wrap bg-zinc-900 p-3 rounded">
                    {runError}
                </pre>
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

function renderCell(v: unknown): string {
    if (v === null || v === undefined) return "—";
    if (typeof v === "string") return v.length > 80 ? v.slice(0, 80) + "…" : v;
    if (typeof v === "number" || typeof v === "boolean") return String(v);
    if (Array.isArray(v)) return `[${v.length}]`;
    if (typeof v === "object") return JSON.stringify(v).slice(0, 80);
    return String(v);
}
