//! Inline summary widget shared by the Scope panel's binding rows and
//! the JSON tree's leaf rendering. Scalars render inline; arrays /
//! objects render a one-line summary chip + an expand toggle whose
//! body is a `<pre>` of the pretty-printed value. Pulled out so the
//! Watch + REPL value renderers can drop it next to a label without
//! duplicating tree-rendering plumbing.

import { useMemo } from "react";
import { ChevronRight } from "lucide-react";

import {
    Collapsible,
    CollapsibleContent,
    CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";

/// Single recursion-leaf renderer for any JSON-like value. Scalars
/// inline as `text-success` / `text-warning` / etc.; containers
/// (array, object) collapse to a `[N]` / `{K keys}` chip with a
/// `<pre>` expand. Callers wrap this inside KeyValueRow, watch result
/// cells, REPL transcript entries — anywhere a JSON value needs the
/// same shape.
export function JsonNode({ value }: { value: unknown }) {
    const summary = useMemo(() => describe(value), [value]);
    if (
        value === null
        || value === undefined
        || typeof value === "string"
        || typeof value === "number"
        || typeof value === "boolean"
    ) {
        return <span className={scalarTone(value)}>{summary}</span>;
    }
    return (
        <Collapsible className="inline-block w-full">
            <CollapsibleTrigger asChild>
                <button
                    type="button"
                    className={cn(
                        "group/json inline-flex items-center gap-1 text-left",
                        "text-foreground hover:text-foreground/80",
                    )}
                >
                    <ChevronRight className="size-3 text-muted-foreground transition-transform group-data-[state=open]/json:rotate-90" />
                    <span>{summary}</span>
                </button>
            </CollapsibleTrigger>
            <CollapsibleContent>
                <pre className="mt-1 ml-3 pl-3 border-l border-border text-xs whitespace-pre-wrap overflow-x-auto max-h-96 overflow-y-auto select-text">
                    {JSON.stringify(value, null, 2)}
                </pre>
            </CollapsibleContent>
        </Collapsible>
    );
}

/// One-line description of a value. Strings are JSON-quoted and
/// truncated at 60 chars; containers carry a count chip.
export function describe(v: unknown): string {
    if (v === null) return "null";
    if (v === undefined) return "—";
    if (typeof v === "string") {
        return v.length > 60 ? JSON.stringify(v.slice(0, 60)) + "…" : JSON.stringify(v);
    }
    if (typeof v === "number" || typeof v === "boolean") return String(v);
    if (Array.isArray(v)) return `[${v.length}]`;
    if (typeof v === "object") return `{${Object.keys(v as object).length} keys}`;
    return String(v);
}

/// Tailwind class for the inline color of a scalar value. Match the
/// debugger panel's existing palette: muted for null/undefined,
/// success-green for strings, warning-amber for numbers.
export function scalarTone(v: unknown): string {
    if (v === null || v === undefined) return "text-muted-foreground";
    if (typeof v === "string") return "text-success";
    if (typeof v === "number") return "text-warning";
    if (typeof v === "boolean") return "text-foreground";
    return "text-foreground";
}
