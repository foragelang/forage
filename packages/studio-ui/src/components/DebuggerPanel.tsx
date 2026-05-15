//! Bottom debugger panel — slides up from below the editor when the
//! engine is paused. Three columns: Call Stack · Scope · Watch.
//!
//! Watch is a placeholder: user-defined watch expressions aren't wired
//! yet (DESIGN_HANDOFF.md "open questions"). Per-type emit counters
//! render at the bottom of the Watch column so users still see
//! "records emitted so far" alongside the placeholder.

import { useId, useMemo, useState } from "react";
import {
    ChevronRight,
    Pause,
    Play,
    Plus,
    Repeat,
    Square,
    SquareDashed,
    X,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
    Collapsible,
    CollapsibleContent,
    CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

import type { DebugScope } from "@/bindings/DebugScope";
import type { PausePayload } from "@/bindings/PausePayload";
import { useStudioService, type DebugAction } from "@/lib/services";
import { useStudio } from "@/lib/store";

/// Bottom panel attached to the editor pane. Renders the engine's
/// paused scope and the resume controls. Mounted only when
/// `paused !== null` — EditorView handles that gating.
export function DebuggerPanel() {
    const service = useStudioService();
    const paused = useStudio((s) => s.paused);
    const debugClearPause = useStudio((s) => s.debugClearPause);
    const breakpoints = useStudio((s) => s.breakpoints);
    const clearBreakpoints = useStudio((s) => s.clearBreakpoints);
    const pauseIterations = useStudio((s) => s.pauseIterations);
    const setPauseIterations = useStudio((s) => s.setPauseIterations);
    const loopToggleId = useId();
    // Frame switcher — Call Stack column writes into this; Scope reads
    // it. Default to the innermost (last) frame, which is what the
    // engine paused on.
    const [activeFrame, setActiveFrame] = useState<number>(
        paused?.scope.bindings.length ? paused.scope.bindings.length - 1 : 0,
    );

    if (!paused) return null;

    const resume = async (action: DebugAction) => {
        debugClearPause();
        try {
            await service.debugResume(action);
        } catch (e) {
            console.warn("debug resume failed", e);
        }
    };

    return (
        <div className="border-t bg-background flex flex-col min-h-0 max-h-[50vh]">
            <header className="flex items-center gap-2 border-b px-4 py-2 text-xs shrink-0">
                <Pause className="size-3.5 text-warning" />
                <PauseLabel paused={paused} />
                <div className="ml-auto flex items-center gap-2">
                    <Tooltip>
                        <TooltipTrigger asChild>
                            <label
                                htmlFor={loopToggleId}
                                className="flex items-center gap-1.5 cursor-pointer select-none text-muted-foreground hover:text-foreground"
                            >
                                <Repeat className="size-3.5" />
                                <input
                                    id={loopToggleId}
                                    type="checkbox"
                                    checked={pauseIterations}
                                    onChange={(e) =>
                                        setPauseIterations(e.target.checked)
                                    }
                                    className="size-3 accent-primary"
                                />
                                <Label
                                    htmlFor={loopToggleId}
                                    className="text-xs cursor-pointer"
                                >
                                    loop iters
                                </Label>
                            </label>
                        </TooltipTrigger>
                        <TooltipContent>
                            Pause inside every for-loop iteration
                        </TooltipContent>
                    </Tooltip>
                    <Separator orientation="vertical" className="!h-4" />
                    <span className="text-muted-foreground tabular-nums">
                        {breakpoints.size} breakpoint
                        {breakpoints.size === 1 ? "" : "s"}
                    </span>
                    {breakpoints.size > 0 && (
                        <Tooltip>
                            <TooltipTrigger asChild>
                                <Button
                                    size="icon-xs"
                                    variant="ghost"
                                    onClick={clearBreakpoints}
                                    aria-label="Clear all breakpoints"
                                >
                                    <X />
                                </Button>
                            </TooltipTrigger>
                            <TooltipContent>Clear all breakpoints</TooltipContent>
                        </Tooltip>
                    )}
                    <Separator orientation="vertical" className="!h-4" />
                    <Tooltip>
                        <TooltipTrigger asChild>
                            <Button
                                size="icon-xs"
                                variant="ghost"
                                onClick={() => resume("step_over")}
                                aria-label="Step over"
                            >
                                <SquareDashed />
                            </Button>
                        </TooltipTrigger>
                        <TooltipContent>Step over · F10</TooltipContent>
                    </Tooltip>
                    <Tooltip>
                        <TooltipTrigger asChild>
                            <Button
                                size="icon-xs"
                                variant="ghost"
                                onClick={() => resume("step_over")}
                                aria-label="Step into"
                            >
                                <ChevronRight />
                            </Button>
                        </TooltipTrigger>
                        <TooltipContent>
                            Step into · F11 (uses step_over until step_in lands)
                        </TooltipContent>
                    </Tooltip>
                    <Separator orientation="vertical" className="!h-4" />
                    <Tooltip>
                        <TooltipTrigger asChild>
                            <Button size="xs" onClick={() => resume("continue")}>
                                <Play />
                                Continue
                            </Button>
                        </TooltipTrigger>
                        <TooltipContent>
                            Run to next breakpoint or end · F5
                        </TooltipContent>
                    </Tooltip>
                    <Tooltip>
                        <TooltipTrigger asChild>
                            <Button
                                size="xs"
                                variant="destructive"
                                onClick={() => resume("stop")}
                            >
                                <Square />
                                Stop
                            </Button>
                        </TooltipTrigger>
                        <TooltipContent>Abort the run</TooltipContent>
                    </Tooltip>
                </div>
            </header>
            <div className="flex-1 min-h-0 grid grid-cols-[200px_minmax(0,1fr)_240px] divide-x">
                <CallStackColumn
                    paused={paused}
                    activeFrame={activeFrame}
                    onSelectFrame={setActiveFrame}
                />
                <ScopeColumn paused={paused} activeFrame={activeFrame} />
                <WatchColumn counts={paused.scope.emit_counts} />
            </div>
        </div>
    );
}

function PauseLabel({ paused }: { paused: PausePayload }) {
    if (paused.kind === "step") {
        return (
            <>
                <span className="text-muted-foreground">paused before step</span>
                <span className="font-mono text-warning select-text">{paused.step}</span>
                <span className="text-muted-foreground tabular-nums">
                    #{paused.step_index}
                </span>
            </>
        );
    }
    return (
        <>
            <span className="text-muted-foreground">iteration</span>
            <span className="font-mono text-warning tabular-nums select-text">
                {paused.iteration + 1}/{paused.total}
            </span>
            <span className="text-muted-foreground">of</span>
            <span className="font-mono text-warning select-text">${paused.variable}</span>
        </>
    );
}

// ── columns ──────────────────────────────────────────────────────────

function CallStackColumn({
    paused,
    activeFrame,
    onSelectFrame,
}: {
    paused: PausePayload;
    activeFrame: number;
    onSelectFrame: (i: number) => void;
}) {
    const frames = paused.scope.bindings;
    return (
        <Column title="Call stack" meta={`${frames.length}`}>
            {frames.map((f, i) => {
                const active = i === activeFrame;
                const keyCount = Object.keys(f.bindings).length;
                return (
                    <button
                        type="button"
                        key={i}
                        onClick={() => onSelectFrame(i)}
                        className={cn(
                            "w-full flex items-baseline gap-2 px-3 py-1 text-xs text-left",
                            "hover:bg-muted",
                            active && "bg-accent/40 text-foreground",
                            !active && "text-muted-foreground",
                        )}
                    >
                        <span className="font-mono tabular-nums w-6 text-right text-muted-foreground">
                            #{i}
                        </span>
                        <span className="font-mono truncate flex-1">
                            {i === 0 ? "scope" : `frame ${i}`}
                        </span>
                        <span className="font-mono text-[10px] text-muted-foreground tabular-nums">
                            {keyCount}
                        </span>
                    </button>
                );
            })}
            {frames.length === 0 && (
                <div className="px-3 py-2 text-xs text-muted-foreground">
                    (no frames)
                </div>
            )}
        </Column>
    );
}

function ScopeColumn({
    paused,
    activeFrame,
}: {
    paused: PausePayload;
    activeFrame: number;
}) {
    const frame = paused.scope.bindings[activeFrame];
    return (
        <Column
            title="Scope"
            meta={
                paused.scope.bindings.length > 0
                    ? `frame #${activeFrame}`
                    : undefined
            }
        >
            <div className="flex-1 overflow-y-auto p-3 space-y-4">
                <InputsSection inputs={paused.scope.inputs} />
                <SecretsSection names={paused.scope.secrets} />
                <BindingsSection frame={frame} index={activeFrame} />
                {paused.scope.current !== null &&
                    paused.scope.current !== undefined && (
                        <Section title="$ current">
                            <JsonNode value={paused.scope.current} />
                        </Section>
                    )}
            </div>
        </Column>
    );
}

function WatchColumn({ counts }: { counts: Record<string, number> }) {
    const entries = Object.entries(counts);
    return (
        <Column
            title="Watch"
            meta=""
            trailing={
                <Tooltip>
                    <TooltipTrigger asChild>
                        <Button
                            size="icon-xs"
                            variant="ghost"
                            aria-label="Add watch (coming soon)"
                            disabled
                        >
                            <Plus />
                        </Button>
                    </TooltipTrigger>
                    <TooltipContent>Add watch — coming</TooltipContent>
                </Tooltip>
            }
        >
            <div className="flex-1 overflow-y-auto p-3 space-y-4">
                <div className="text-xs text-muted-foreground italic">
                    Add watch — coming
                </div>
                {entries.length > 0 && (
                    <Section title="Records so far">
                        <div className="space-y-1">
                            {entries.map(([t, n]) => (
                                <div
                                    key={t}
                                    className="flex items-baseline justify-between gap-2 text-sm"
                                >
                                    <span className="font-mono truncate">{t}</span>
                                    <Badge variant="success" className="tabular-nums shrink-0">
                                        {n}
                                    </Badge>
                                </div>
                            ))}
                        </div>
                    </Section>
                )}
            </div>
        </Column>
    );
}

function Column({
    title,
    meta,
    trailing,
    children,
}: {
    title: string;
    meta?: string;
    trailing?: React.ReactNode;
    children: React.ReactNode;
}) {
    return (
        <div className="flex flex-col min-h-0">
            <div className="flex items-baseline gap-2 px-3 py-1.5 border-b text-[10px] uppercase tracking-wider font-semibold text-muted-foreground">
                <span>{title}</span>
                {meta !== undefined && meta !== "" && (
                    <span className="font-mono text-muted-foreground/70">{meta}</span>
                )}
                {trailing && <span className="ml-auto">{trailing}</span>}
            </div>
            <div className="flex-1 min-h-0 flex flex-col">{children}</div>
        </div>
    );
}

// ── scope sub-sections ───────────────────────────────────────────────

function BindingsSection({
    frame,
    index,
}: {
    frame: DebugScope["bindings"][number] | undefined;
    index: number;
}) {
    if (!frame) return null;
    const keys = Object.keys(frame.bindings);
    return (
        <Section
            title={index === 0 ? "Bindings" : `Bindings (frame ${index})`}
        >
            {keys.length === 0 ? (
                <div className="text-xs text-muted-foreground">
                    (no named bindings yet)
                </div>
            ) : (
                <div className="space-y-1">
                    {keys.map((k) => (
                        <KeyValueRow
                            key={k}
                            name={k}
                            value={frame.bindings[k]}
                        />
                    ))}
                </div>
            )}
        </Section>
    );
}

function InputsSection({ inputs }: { inputs: Record<string, unknown> }) {
    const entries = Object.entries(inputs);
    if (entries.length === 0) return null;
    return (
        <Section title="Inputs">
            <div className="space-y-1">
                {entries.map(([k, v]) => (
                    <KeyValueRow key={k} name={k} value={v} />
                ))}
            </div>
        </Section>
    );
}

function SecretsSection({ names }: { names: string[] }) {
    if (names.length === 0) return null;
    return (
        <Section title="Secrets">
            <div className="space-y-1">
                {names.map((n) => (
                    <div key={n} className="flex items-baseline gap-3 font-mono text-sm">
                        <span className="text-warning">${"{"}secret.{n}{"}"}</span>
                        <span className="text-muted-foreground">= ●●●●●</span>
                    </div>
                ))}
            </div>
        </Section>
    );
}

function Section(props: { title: string; children: React.ReactNode }) {
    return (
        <section>
            <h3 className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold mb-2">
                {props.title}
            </h3>
            <div>{props.children}</div>
        </section>
    );
}

function KeyValueRow({ name, value }: { name: string; value: unknown }) {
    return (
        <div className="font-mono text-sm flex gap-3 items-start">
            <span className="text-warning shrink-0">${name}</span>
            <span className="text-muted-foreground shrink-0">=</span>
            <span className="flex-1 min-w-0 select-text">
                <JsonNode value={value} />
            </span>
        </div>
    );
}

function JsonNode({ value }: { value: unknown }) {
    const summary = useMemo(() => describe(value), [value]);
    if (
        value === null ||
        value === undefined ||
        typeof value === "string" ||
        typeof value === "number" ||
        typeof value === "boolean"
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

function describe(v: unknown): string {
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

function scalarTone(v: unknown): string {
    if (v === null || v === undefined) return "text-muted-foreground";
    if (typeof v === "string") return "text-success";
    if (typeof v === "number") return "text-warning";
    if (typeof v === "boolean") return "text-foreground";
    return "text-foreground";
}
