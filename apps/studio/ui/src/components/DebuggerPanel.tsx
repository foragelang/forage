import { useId, useMemo } from "react";
import { ChevronRight, Pause, Play, Repeat, Square, X } from "lucide-react";

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

import { api, type DebugAction, type DebugScope, type PausePayload } from "@/lib/api";
import { useStudio } from "@/lib/store";

/// Bottom panel attached to the Source tab. Renders the engine's paused
/// scope and the resume controls. Mounted only when `paused !== null` —
/// the SourceTab handles that gating.
export function DebuggerPanel() {
    const {
        paused,
        debugClearPause,
        breakpoints,
        clearBreakpoints,
        pauseIterations,
        setPauseIterations,
    } = useStudio();
    const loopToggleId = useId();
    if (!paused) return null;

    const resume = async (action: DebugAction) => {
        // Optimistically clear the pause so the panel collapses and the
        // line highlight goes away immediately. The next pause event (if
        // any) reinstates it. Without this the buttons feel "stuck"
        // between click and engine wakeup.
        debugClearPause();
        try {
            await api.debugResume(action);
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
                                    onChange={(e) => setPauseIterations(e.target.checked)}
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
                            <Button size="xs" onClick={() => resume("step_over")}>
                                <Play />
                                Step over
                            </Button>
                        </TooltipTrigger>
                        <TooltipContent>
                            Run this step, pause at the next one
                        </TooltipContent>
                    </Tooltip>
                    <Tooltip>
                        <TooltipTrigger asChild>
                            <Button
                                size="xs"
                                variant="secondary"
                                onClick={() => resume("continue")}
                            >
                                Continue
                            </Button>
                        </TooltipTrigger>
                        <TooltipContent>
                            Run to next breakpoint or end of recipe
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
            <ScopeView paused={paused} />
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

function ScopeView({ paused }: { paused: PausePayload }) {
    return (
        <div className="flex-1 overflow-auto p-4 grid grid-cols-[1fr_1fr] gap-x-6 gap-y-4">
            <BindingsSection scope={paused.scope} />
            <div className="space-y-6">
                <CountsSection counts={paused.scope.emit_counts} />
                <InputsSection inputs={paused.scope.inputs} />
                <SecretsSection names={paused.scope.secrets} />
                {paused.scope.current !== null &&
                    paused.scope.current !== undefined && (
                        <Section title="$ (current)">
                            <JsonNode value={paused.scope.current} />
                        </Section>
                    )}
            </div>
        </div>
    );
}

function CountsSection({ counts }: { counts: Record<string, number> }) {
    const entries = Object.entries(counts);
    if (entries.length === 0) return null;
    return (
        <Section title="Records so far">
            <div className="space-y-1 max-w-sm">
                {entries.map(([t, n]) => (
                    <div key={t} className="flex items-baseline justify-between gap-2">
                        <span className="font-mono text-sm truncate">{t}</span>
                        <Badge variant="success" className="tabular-nums shrink-0">
                            {n}
                        </Badge>
                    </div>
                ))}
            </div>
        </Section>
    );
}

function BindingsSection({ scope }: { scope: DebugScope }) {
    // Flatten frames into a single view, marking which frame each binding
    // came from. Outer-most first matches push order; inner shadows outer
    // (which the engine honors via Scope::lookup). The UI groups by frame
    // depth so the user can see how a `for`-loop binding sits relative to
    // the top-level scope.
    const allEmpty = scope.bindings.every(
        (f) => Object.keys(f.bindings).length === 0,
    );
    return (
        <Section title="Bindings">
            {allEmpty ? (
                <div className="text-sm text-muted-foreground">
                    (no named bindings yet)
                </div>
            ) : (
                <div className="space-y-4">
                    {scope.bindings.map((frame, i) => {
                        const keys = Object.keys(frame.bindings);
                        if (keys.length === 0) return null;
                        return (
                            <div key={i}>
                                {i > 0 && <Separator className="mb-3" />}
                                <div className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold mb-2">
                                    {i === 0 ? "scope" : `frame #${i}`}
                                </div>
                                <div className="space-y-1">
                                    {keys.map((k) => (
                                        <KeyValueRow
                                            key={k}
                                            name={k}
                                            value={frame.bindings[k]}
                                        />
                                    ))}
                                </div>
                            </div>
                        );
                    })}
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
            <h3 className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold mb-3">
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

/// Compact JSON view: scalars inline, objects/arrays as Collapsible with
/// a summary line and the full value on expand.
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
