//! Bottom debugger panel — slides up from below the editor when the
//! engine is paused. Three columns:
//!
//! - Call Stack: frame switcher into the scope's stack.
//! - Scope: inputs / secrets / bindings, plus Watches and REPL.
//! - Response: per-step captured response, Tree / Raw / Headers.
//!
//! The Response column owns Maximize (collapses the other two) and
//! pop-out (opens the same view in a separate Tauri window). The
//! resume controls live in the panel header and honor the F10 / F11
//! / F5 / Shift+F5 keyboard shortcuts (wired in `useStudioEffects`).

import { useState } from "react";
import {
    ChevronRight,
    Pause,
    Play,
    Square,
    SquareDashed,
    X,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
    Collapsible,
    CollapsibleContent,
    CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Separator } from "@/components/ui/separator";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

import type { DebugScope } from "@/bindings/DebugScope";
import type { PausePayload } from "@/bindings/PausePayload";
import { useStudioService, type DebugAction } from "@/lib/services";
import { useStudio } from "@/lib/store";

import { JsonNode } from "@/components/Debugger/JsonNode";
import { ReplSection } from "@/components/Debugger/ReplSection";
import {
    ResponseColumn,
    useMaximizeResponse,
} from "@/components/Debugger/ResponseColumn";
import { WatchesSection } from "@/components/Debugger/WatchesSection";

/// Bottom panel attached to the editor pane. Renders the engine's
/// paused scope and the resume controls. Mounted only when
/// `paused !== null` — EditorView handles that gating.
export function DebuggerPanel() {
    const service = useStudioService();
    const paused = useStudio((s) => s.paused);
    const debugClearPause = useStudio((s) => s.debugClearPause);
    const breakpoints = useStudio((s) => s.breakpoints);
    const clearBreakpoints = useStudio((s) => s.clearBreakpoints);
    const runId = useStudio((s) => s.runId);
    const lastResponses = useStudio((s) => s.lastResponses);
    // Frame switcher — Call Stack column writes into this; Scope
    // reads it. Default to the innermost (last) frame, which is what
    // the engine paused on.
    const [activeFrame, setActiveFrame] = useState<number>(
        paused?.scope.bindings.length ? paused.scope.bindings.length - 1 : 0,
    );
    const { maximized, toggle: toggleMaximize } = useMaximizeResponse();

    if (!paused) return null;

    const resume = async (action: DebugAction) => {
        debugClearPause();
        try {
            await service.debugResume(action);
        } catch (e) {
            console.warn("debug resume failed", e);
        }
    };

    const responses: { [key in string]?: import("@/bindings/StepResponse").StepResponse }
        = paused.scope.step_responses && Object.keys(paused.scope.step_responses).length > 0
            ? paused.scope.step_responses
            : lastResponses;

    return (
        <div className="border-t bg-background flex flex-col min-h-0 max-h-[50vh]">
            <header className="flex items-center gap-2 border-b px-4 py-2 text-xs shrink-0">
                <Pause className="size-3.5 text-warning" />
                <PauseLabel paused={paused} />
                <div className="ml-auto flex items-center gap-2">
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
                                onClick={() => resume("step_in")}
                                aria-label="Step into"
                            >
                                <ChevronRight />
                            </Button>
                        </TooltipTrigger>
                        <TooltipContent>Step into · F11</TooltipContent>
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
                        <TooltipContent>Abort the run · Shift+F5</TooltipContent>
                    </Tooltip>
                </div>
            </header>
            <div
                className={cn(
                    "flex-1 min-h-0 grid divide-x",
                    maximized
                        ? "grid-cols-1"
                        : "grid-cols-[200px_minmax(0,1fr)_minmax(360px,1fr)]",
                )}
            >
                {!maximized && (
                    <>
                        <CallStackColumn
                            paused={paused}
                            activeFrame={activeFrame}
                            onSelectFrame={setActiveFrame}
                        />
                        <ScopeColumn paused={paused} activeFrame={activeFrame} />
                    </>
                )}
                <ResponseColumn
                    responses={responses}
                    runId={runId}
                    onMaximize={toggleMaximize}
                    isMaximized={maximized}
                    onPopOut={() => {
                        service.openResponseWindow().catch((e) =>
                            console.warn("open_response_window failed", e),
                        );
                    }}
                    emptyStateLabel="No responses captured at this pause yet."
                />
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
    if (paused.kind === "emit") {
        return (
            <>
                <span className="text-muted-foreground">paused before emit</span>
                <span className="font-mono text-warning select-text">
                    {paused.type_name}
                </span>
                <span className="text-muted-foreground tabular-nums">
                    #{paused.emit_index}
                </span>
            </>
        );
    }
    // for_loop
    return (
        <>
            <span className="text-muted-foreground">paused at for-loop</span>
            <span className="font-mono text-warning select-text">${paused.variable}</span>
            <span className="text-muted-foreground tabular-nums">
                · {paused.total} item{paused.total === 1 ? "" : "s"}
            </span>
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
                <WatchesSection />
                <ReplSection />
                <InputsSection inputs={paused.scope.inputs} />
                <SecretsSection names={paused.scope.secrets} />
                <BindingsSection frame={frame} index={activeFrame} />
                {paused.scope.current !== null
                    && paused.scope.current !== undefined && (
                        <Section title="$ current">
                            <JsonNode value={paused.scope.current} />
                        </Section>
                    )}
                <EmitCountsSection counts={paused.scope.emit_counts} />
            </div>
        </Column>
    );
}

function Column({
    title,
    meta,
    children,
}: {
    title: string;
    meta?: string;
    children: React.ReactNode;
}) {
    return (
        <div className="flex flex-col min-h-0">
            <div className="flex items-baseline gap-2 px-3 py-1.5 border-b text-[10px] uppercase tracking-wider font-semibold text-muted-foreground">
                <span>{title}</span>
                {meta !== undefined && meta !== "" && (
                    <span className="font-mono text-muted-foreground/70">{meta}</span>
                )}
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

function EmitCountsSection({ counts }: { counts: Record<string, number> }) {
    const entries = Object.entries(counts);
    if (entries.length === 0) return null;
    return (
        <Section title="Records so far">
            <div className="space-y-1">
                {entries.map(([t, n]) => (
                    <div
                        key={t}
                        className="flex items-baseline justify-between gap-2 text-sm"
                    >
                        <span className="font-mono truncate">{t}</span>
                        <span className="font-mono text-warning tabular-nums shrink-0">
                            {n}
                        </span>
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

// Retain a re-export for any caller that imported from this module
// to access the collapsible JsonNode helpers.
export { JsonNode, Collapsible, CollapsibleContent, CollapsibleTrigger };
