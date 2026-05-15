//! Recursive JSON viewer: type chips, search, expand-all. Powers the
//! Response viewer's Tree tab for JSON-formatted responses and is
//! reusable for any other in-panel JSON-tree need (preview pane,
//! diagnostic payloads, etc.).
//!
//! Rendering rules:
//! - `null` → muted "null"
//! - boolean → orange `true` / `false` chip
//! - number → yellow value + `(int)` / `(num)` chip
//! - string → green, in quotes; >120 chars get a tail toggle
//! - array → `[N]` chip; click to expand into indented children
//! - object → `{K}` chip; click to expand into key:value rows
//!
//! Tree-wide controls (Expand all / Collapse all / Search) live in
//! the header strip the response column passes in via props.

import { useMemo, useState } from "react";
import { ChevronRight, X } from "lucide-react";

import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/// Tree mode applied to every node when the user clicks Expand All
/// / Collapse All. The default `auto` mode lets each node manage its
/// own open state; the explicit modes override it for one render.
type TreeMode = "auto" | "open" | "closed";

export function JsonTree({ value }: { value: unknown }) {
    const [mode, setMode] = useState<TreeMode>("auto");
    const [search, setSearch] = useState("");
    /// Key-set the current search matches. Keys are dot-paths from the
    /// root to the matching node so the renderer can decide whether
    /// to keep each node visible. Empty search → null (no filter).
    const matchingPaths = useMemo<Set<string> | null>(() => {
        const q = search.trim().toLowerCase();
        if (!q) return null;
        const matches = new Set<string>();
        function walk(v: unknown, path: string) {
            if (v && typeof v === "object" && !Array.isArray(v)) {
                for (const key of Object.keys(v)) {
                    if (key.toLowerCase().includes(q)) {
                        matches.add(`${path}.${key}`);
                    }
                    walk((v as Record<string, unknown>)[key], `${path}.${key}`);
                }
            } else if (Array.isArray(v)) {
                v.forEach((item, idx) => walk(item, `${path}[${idx}]`));
            }
        }
        walk(value, "$");
        return matches;
    }, [value, search]);

    return (
        <div className="flex flex-col min-h-0">
            <div className="flex items-center gap-1 border-b px-2 py-1 text-xs shrink-0">
                <div className="relative flex-1">
                    <Input
                        value={search}
                        onChange={(e) => setSearch(e.target.value)}
                        placeholder="Search keys…"
                        className="h-7 text-xs pr-7"
                    />
                    {search && (
                        <button
                            type="button"
                            onClick={() => setSearch("")}
                            className="absolute right-1.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                            aria-label="Clear search"
                        >
                            <X className="size-3" />
                        </button>
                    )}
                </div>
                <Button
                    size="xs"
                    variant="ghost"
                    onClick={() => setMode("open")}
                    title="Expand all"
                >
                    Expand
                </Button>
                <Button
                    size="xs"
                    variant="ghost"
                    onClick={() => setMode("closed")}
                    title="Collapse all"
                >
                    Collapse
                </Button>
            </div>
            <div className="flex-1 overflow-y-auto p-2 font-mono text-xs select-text">
                <Node
                    label="$"
                    path="$"
                    value={value}
                    depth={0}
                    mode={mode}
                    matchingPaths={matchingPaths}
                />
            </div>
        </div>
    );
}

function Node({
    label,
    path,
    value,
    depth,
    mode,
    matchingPaths,
}: {
    label: string;
    path: string;
    value: unknown;
    depth: number;
    mode: TreeMode;
    matchingPaths: Set<string> | null;
}) {
    const [openLocal, setOpenLocal] = useState<boolean>(depth < 2);
    const open = mode === "open" ? true : mode === "closed" ? false : openLocal;

    if (matchingPaths) {
        const onPath = matchingPaths.has(path);
        const hasDescendantMatch = [...matchingPaths].some((p) =>
            p.startsWith(`${path}.`) || p.startsWith(`${path}[`),
        );
        if (!onPath && !hasDescendantMatch && depth > 0) {
            return null;
        }
    }

    if (value === null) {
        return (
            <Row label={label}>
                <span className="text-muted-foreground">null</span>
                <TypeChip kind="null" />
            </Row>
        );
    }
    if (typeof value === "boolean") {
        return (
            <Row label={label}>
                <span className="text-warning">{String(value)}</span>
                <TypeChip kind="bool" />
            </Row>
        );
    }
    if (typeof value === "number") {
        const isInt = Number.isInteger(value);
        return (
            <Row label={label}>
                <span className="text-warning">{String(value)}</span>
                <TypeChip kind={isInt ? "int" : "num"} />
            </Row>
        );
    }
    if (typeof value === "string") {
        return <StringRow label={label} value={value} />;
    }
    if (Array.isArray(value)) {
        return (
            <ContainerRow
                label={label}
                summary={`[${value.length}]`}
                open={open}
                onToggle={() => setOpenLocal((o) => !o)}
                kind="array"
            >
                {open
                    && value.map((item, idx) => (
                        <Node
                            key={idx}
                            label={`[${idx}]`}
                            path={`${path}[${idx}]`}
                            value={item}
                            depth={depth + 1}
                            mode={mode}
                            matchingPaths={matchingPaths}
                        />
                    ))}
            </ContainerRow>
        );
    }
    if (typeof value === "object") {
        const obj = value as Record<string, unknown>;
        const keys = Object.keys(obj);
        return (
            <ContainerRow
                label={label}
                summary={`{${keys.length}}`}
                open={open}
                onToggle={() => setOpenLocal((o) => !o)}
                kind="object"
            >
                {open
                    && keys.map((k) => (
                        <Node
                            key={k}
                            label={k}
                            path={`${path}.${k}`}
                            value={obj[k]}
                            depth={depth + 1}
                            mode={mode}
                            matchingPaths={matchingPaths}
                        />
                    ))}
            </ContainerRow>
        );
    }
    return (
        <Row label={label}>
            <span className="text-muted-foreground">{String(value)}</span>
        </Row>
    );
}

function StringRow({ label, value }: { label: string; value: string }) {
    const [showFull, setShowFull] = useState(false);
    const isLong = value.length > 120;
    const display = !isLong || showFull ? value : value.slice(0, 120) + "…";
    return (
        <Row label={label}>
            <span className="text-success break-all">{JSON.stringify(display)}</span>
            <TypeChip kind="str" />
            {isLong && (
                <button
                    type="button"
                    onClick={() => setShowFull((s) => !s)}
                    className="text-[10px] text-muted-foreground hover:text-foreground"
                >
                    {showFull ? "shorter" : `+${value.length - 120}`}
                </button>
            )}
        </Row>
    );
}

function Row({
    label,
    children,
}: {
    label: string;
    children: React.ReactNode;
}) {
    return (
        <div className="flex items-baseline gap-2 py-0.5">
            <span className="text-muted-foreground shrink-0">{label}:</span>
            <span className="flex flex-wrap items-baseline gap-1 min-w-0">
                {children}
            </span>
        </div>
    );
}

function ContainerRow({
    label,
    summary,
    open,
    onToggle,
    kind,
    children,
}: {
    label: string;
    summary: string;
    open: boolean;
    onToggle: () => void;
    kind: "array" | "object";
    children: React.ReactNode;
}) {
    return (
        <div className="py-0.5">
            <button
                type="button"
                onClick={onToggle}
                className="flex items-center gap-1 w-full text-left"
            >
                <ChevronRight
                    className={cn(
                        "size-3 transition-transform",
                        open && "rotate-90",
                    )}
                />
                <span className="text-muted-foreground">{label}:</span>
                <span className="text-foreground">{summary}</span>
                <TypeChip kind={kind} />
            </button>
            {open && <div className="ml-4 border-l border-border pl-2">{children}</div>}
        </div>
    );
}

function TypeChip({
    kind,
}: {
    kind: "null" | "bool" | "int" | "num" | "str" | "array" | "object";
}) {
    return (
        <span className="text-[9px] uppercase tracking-wider text-muted-foreground/70 border border-border rounded px-1 py-px">
            {kind}
        </span>
    );
}
