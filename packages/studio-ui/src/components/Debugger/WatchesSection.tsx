//! Watch expressions section — pinned `extraction` expressions that
//! re-evaluate against the paused scope on every pause and on every
//! list mutation. Lives at the top of the Scope column inside the
//! bottom debugger panel.
//!
//! Persisted per recipe in localStorage; the store carries the
//! current list so the UI re-renders on add / remove.

import { useEffect, useState } from "react";
import { Plus, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useStudioService } from "@/lib/services";
import { useStudio } from "@/lib/store";

import { JsonNode } from "./JsonNode";

/// Per-watch evaluation result keyed by the expression source so the
/// row can render the value or the error inline. The map is local to
/// the section because watches are per-recipe and the data flow is
/// strictly evaluate-on-pause; hoisting it to the store would
/// re-trigger renders elsewhere on every value change.
type ResultMap = Record<
    string,
    | { kind: "value"; value: unknown }
    | { kind: "error"; message: string }
    | undefined
>;

export function WatchesSection() {
    const service = useStudioService();
    const watches = useStudio((s) => s.watches);
    const setWatches = useStudio((s) => s.setWatches);
    const paused = useStudio((s) => s.paused);
    const [input, setInput] = useState("");
    const [results, setResults] = useState<ResultMap>({});

    // Re-evaluate every watch on every pause + whenever the list
    // changes. The evaluator command takes the paused scope from the
    // session under the hood; firing it outside a pause throws the
    // "not paused" error which the row renders as a faint hint.
    useEffect(() => {
        if (!paused) return;
        let cancelled = false;
        const next: ResultMap = {};
        Promise.all(
            watches.map(async (expr) => {
                try {
                    const v = await service.evalWatchExpression(expr);
                    next[expr] = { kind: "value", value: v };
                } catch (e) {
                    next[expr] = { kind: "error", message: String(e) };
                }
            }),
        ).then(() => {
            if (!cancelled) setResults(next);
        });
        return () => {
            cancelled = true;
        };
    }, [watches, paused, service]);

    function addWatch() {
        const expr = input.trim();
        if (!expr) return;
        if (watches.includes(expr)) {
            setInput("");
            return;
        }
        setWatches([...watches, expr]);
        setInput("");
    }

    function removeWatch(expr: string) {
        setWatches(watches.filter((w) => w !== expr));
        setResults((prev) => {
            const next = { ...prev };
            delete next[expr];
            return next;
        });
    }

    return (
        <section>
            <h3 className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold mb-2">
                Watches
            </h3>
            <form
                onSubmit={(e) => {
                    e.preventDefault();
                    addWatch();
                }}
                className="flex gap-1 mb-2"
            >
                <Input
                    value={input}
                    onChange={(e) => setInput(e.target.value)}
                    placeholder="$list.items | length"
                    className="h-7 text-xs font-mono flex-1"
                    aria-label="Add watch expression"
                />
                <Button
                    type="submit"
                    size="icon-xs"
                    variant="ghost"
                    aria-label="Add watch"
                    disabled={input.trim() === ""}
                >
                    <Plus />
                </Button>
            </form>
            {watches.length === 0 && (
                <div className="text-xs text-muted-foreground italic">
                    Pin a Forage expression to evaluate on every pause.
                </div>
            )}
            <div className="space-y-1">
                {watches.map((expr) => (
                    <WatchRow
                        key={expr}
                        expr={expr}
                        result={results[expr]}
                        onRemove={() => removeWatch(expr)}
                    />
                ))}
            </div>
        </section>
    );
}

function WatchRow({
    expr,
    result,
    onRemove,
}: {
    expr: string;
    result: ResultMap[string];
    onRemove: () => void;
}) {
    return (
        <div className="flex items-baseline gap-2 font-mono text-sm">
            <span className="text-warning shrink-0 break-all max-w-[40%]">{expr}</span>
            <span className="text-muted-foreground shrink-0">=</span>
            <span className="flex-1 min-w-0 select-text">
                {result === undefined && (
                    <span className="text-muted-foreground italic">…</span>
                )}
                {result?.kind === "value" && <JsonNode value={result.value} />}
                {result?.kind === "error" && (
                    <span className="text-destructive break-all">{result.message}</span>
                )}
            </span>
            <button
                type="button"
                onClick={onRemove}
                className="text-muted-foreground hover:text-destructive shrink-0"
                aria-label={`Remove watch ${expr}`}
            >
                <X className="size-3" />
            </button>
        </div>
    );
}
