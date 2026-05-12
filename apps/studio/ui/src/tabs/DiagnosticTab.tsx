import { useStudio } from "../lib/store";

export function DiagnosticTab() {
    const { snapshot, runError } = useStudio();
    const d = snapshot?.diagnostic;

    if (runError) {
        return (
            <div className="p-6 text-red-400">
                <div className="font-medium mb-2">Run errored before producing a diagnostic:</div>
                <pre className="text-xs whitespace-pre-wrap bg-zinc-900 p-3 rounded">
                    {runError}
                </pre>
            </div>
        );
    }
    if (!snapshot || !d) {
        return (
            <div className="p-6 text-zinc-500 text-sm">
                Diagnostic appears here after a run.
            </div>
        );
    }

    const section = (title: string, items: string[] | undefined, tone: string) =>
        items && items.length > 0 ? (
            <section className="mb-6">
                <h3 className={`text-sm font-semibold mb-2 ${tone}`}>{title}</h3>
                <ul className="space-y-1 text-xs text-zinc-300">
                    {items.map((m, i) => (
                        <li key={i} className="pl-4 border-l-2 border-zinc-700">{m}</li>
                    ))}
                </ul>
            </section>
        ) : null;

    const anything =
        d.stall_reason ||
        (d.unmet_expectations && d.unmet_expectations.length > 0) ||
        (d.unfired_capture_rules && d.unfired_capture_rules.length > 0) ||
        (d.unmatched_captures && d.unmatched_captures.length > 0) ||
        (d.unhandled_affordances && d.unhandled_affordances.length > 0);

    return (
        <div className="p-6 overflow-y-auto">
            {!anything && (
                <div className="text-emerald-400 text-sm">
                    ✓ clean run — no diagnostic items.
                </div>
            )}
            {d.stall_reason && (
                <section className="mb-6">
                    <h3 className="text-sm font-semibold text-amber-400 mb-2">Stall reason</h3>
                    <pre className="text-xs text-zinc-200 bg-zinc-900 p-3 rounded">
                        {d.stall_reason}
                    </pre>
                </section>
            )}
            {section("Unmet expectations", d.unmet_expectations, "text-red-400")}
            {section("Unfired capture rules", d.unfired_capture_rules, "text-amber-400")}
            {section("Unmatched captures", d.unmatched_captures, "text-amber-400")}
            {section("Unhandled affordances", d.unhandled_affordances, "text-amber-400")}
        </div>
    );
}
