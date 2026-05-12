import { useMemo } from "react";

import {
    api,
    type DebugAction,
    type DebugScope,
    type PausePayload,
} from "../lib/api";
import { useStudio } from "../lib/store";

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
        <div className="border-t border-zinc-800 bg-zinc-950 flex flex-col min-h-0 max-h-[50vh]">
            <header className="px-4 py-2 border-b border-zinc-800 flex items-center gap-3 text-xs flex-shrink-0">
                <span className="text-amber-400 font-mono">⏸ paused</span>
                <PauseLabel paused={paused} />
                <div className="ml-auto flex items-center gap-2">
                    <label
                        className="flex items-center gap-1 text-zinc-500 mr-3 cursor-pointer select-none"
                        title="Pause inside every for-loop iteration"
                    >
                        <input
                            type="checkbox"
                            checked={pauseIterations}
                            onChange={(e) => setPauseIterations(e.target.checked)}
                        />
                        loop iters
                    </label>
                    <span className="text-zinc-600 mr-2">
                        {breakpoints.size} breakpoint
                        {breakpoints.size === 1 ? "" : "s"}
                    </span>
                    {breakpoints.size > 0 && (
                        <button
                            onClick={clearBreakpoints}
                            className="px-2 py-1 text-zinc-400 hover:text-zinc-200"
                            title="Remove all breakpoints"
                        >
                            clear
                        </button>
                    )}
                    <button
                        onClick={() => resume("step_over")}
                        className="px-3 py-1 bg-emerald-700 hover:bg-emerald-600 rounded font-medium"
                        title="Run this step, pause at the next one"
                    >
                        Step over
                    </button>
                    <button
                        onClick={() => resume("continue")}
                        className="px-3 py-1 bg-zinc-800 hover:bg-zinc-700 rounded"
                        title="Run to next breakpoint or end of recipe"
                    >
                        Continue
                    </button>
                    <button
                        onClick={() => resume("stop")}
                        className="px-3 py-1 bg-red-700 hover:bg-red-600 rounded font-medium"
                        title="Abort the run"
                    >
                        Stop
                    </button>
                </div>
            </header>
            <ScopeView pause={paused} />
        </div>
    );
}

function PauseLabel({ paused }: { paused: PausePayload }) {
    if (paused.kind === "step") {
        return (
            <>
                <span className="text-zinc-500">before step</span>
                <span className="font-mono text-zinc-200">{paused.step}</span>
                <span className="text-zinc-600">#{paused.step_index}</span>
            </>
        );
    }
    return (
        <>
            <span className="text-zinc-500">iteration</span>
            <span className="font-mono text-zinc-200">
                {paused.iteration + 1}/{paused.total}
            </span>
            <span className="text-zinc-500">of</span>
            <span className="font-mono text-amber-400">${paused.variable}</span>
        </>
    );
}

function ScopeView({ pause }: { pause: PausePayload }) {
    return (
        <div className="flex-1 overflow-auto p-4 text-xs grid grid-cols-[1fr_1fr] gap-x-6 gap-y-4">
            <BindingsSection scope={pause.scope} />
            <div className="space-y-4">
                <CountsSection counts={pause.scope.emit_counts} />
                <InputsSection inputs={pause.scope.inputs} />
                <SecretsSection names={pause.scope.secrets} />
                {pause.scope.current !== null &&
                    pause.scope.current !== undefined && (
                        <Section title="$ (current)">
                            <JsonNode value={pause.scope.current} />
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
            <div className="space-y-1 font-mono">
                {entries.map(([t, n]) => (
                    <div key={t} className="flex justify-between max-w-xs">
                        <span className="text-zinc-300">{t}</span>
                        <span className="text-emerald-400 tabular-nums">{n}</span>
                    </div>
                ))}
            </div>
        </Section>
    );
}

function BindingsSection({ scope }: { scope: DebugScope }) {
    const allEmpty = scope.bindings.every(
        (f) => Object.keys(f.bindings).length === 0,
    );
    return (
        <Section title="Bindings">
            {allEmpty ? (
                <div className="text-zinc-500">(no named bindings yet)</div>
            ) : (
                scope.bindings.map((frame, i) => {
                    const keys = Object.keys(frame.bindings);
                    if (keys.length === 0) return null;
                    return (
                        <div key={i} className="mb-3">
                            <div className="text-zinc-500 text-[10px] uppercase tracking-wide mb-1">
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
                })
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
            <div className="space-y-1 text-zinc-400">
                {names.map((n) => (
                    <div key={n} className="font-mono">
                        ${"{"}secret.{n}
                        {"}"} <span className="text-zinc-600">= ●●●●●</span>
                    </div>
                ))}
            </div>
        </Section>
    );
}

function Section(props: { title: string; children: React.ReactNode }) {
    return (
        <section>
            <h3 className="text-[11px] uppercase tracking-wider text-zinc-500 font-semibold mb-2">
                {props.title}
            </h3>
            <div>{props.children}</div>
        </section>
    );
}

function KeyValueRow({ name, value }: { name: string; value: unknown }) {
    return (
        <div className="font-mono flex gap-3 items-start">
            <span className="text-amber-400 shrink-0">${name}</span>
            <span className="text-zinc-500 shrink-0">=</span>
            <span className="flex-1 min-w-0">
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
        <details className="inline-block">
            <summary className="cursor-pointer text-zinc-300 list-none marker:hidden">
                <span className="text-zinc-500">▶</span> {summary}
            </summary>
            <pre className="mt-1 pl-4 border-l border-zinc-800 text-zinc-300 whitespace-pre-wrap overflow-x-auto max-h-96 overflow-y-auto">
                {JSON.stringify(value, null, 2)}
            </pre>
        </details>
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
    if (v === null || v === undefined) return "text-zinc-500";
    if (typeof v === "string") return "text-emerald-300";
    if (typeof v === "number") return "text-sky-300";
    if (typeof v === "boolean") return "text-violet-300";
    return "text-zinc-300";
}
