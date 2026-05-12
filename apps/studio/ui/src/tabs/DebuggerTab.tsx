import { useMemo } from "react";

import { api, type DebugAction, type DebugScope, type StepPause } from "../lib/api";
import { useStudio } from "../lib/store";

export function DebuggerTab() {
    const { debugging, paused, running, runError, debugClearPause } = useStudio();

    const resume = async (action: DebugAction) => {
        // Optimistically clear the pause so the UI returns to "running"
        // immediately — if the engine pauses again, the next event will
        // overwrite this. Without this the buttons look "stuck" between
        // click and the engine actually waking up.
        debugClearPause();
        try {
            await api.debugResume(action);
        } catch (e) {
            console.warn("debug resume failed", e);
        }
    };

    if (!debugging && !runError) {
        return (
            <div className="p-6 text-zinc-500 text-sm space-y-2">
                <div>
                    Click <span className="font-medium">Debug</span> (or press{" "}
                    <span className="font-mono">⌥⌘R</span>) to step through this recipe.
                </div>
                <div className="text-xs text-zinc-600">
                    The engine pauses before each <span className="font-mono">step</span>{" "}
                    block so you can inspect the current scope, then advance one step at a
                    time or run to the end.
                </div>
            </div>
        );
    }

    if (runError) {
        return (
            <div className="p-6 text-red-400 text-sm">
                <div className="font-medium mb-2">Debug run errored:</div>
                <pre className="text-xs whitespace-pre-wrap bg-zinc-900 p-3 rounded">
                    {runError}
                </pre>
            </div>
        );
    }

    if (!paused) {
        return (
            <div className="p-6 text-zinc-400 text-sm flex items-center gap-2">
                <span className="inline-block w-3 h-3 rounded-full border-2 border-emerald-400 border-t-transparent animate-spin" />
                {running ? "Running…" : "Waiting for engine…"}
            </div>
        );
    }

    return (
        <div className="flex-1 flex min-h-0 flex-col">
            <header className="px-6 py-3 border-b border-zinc-800 flex items-center gap-4">
                <div className="text-sm">
                    <span className="text-zinc-500">Paused before step </span>
                    <span className="font-mono text-amber-400">{paused.step}</span>
                    <span className="text-zinc-500 text-xs ml-2">
                        (step #{paused.step_index})
                    </span>
                </div>
                <div className="ml-auto flex gap-2">
                    <button
                        onClick={() => resume("step_over")}
                        className="px-3 py-1.5 text-sm bg-emerald-700 hover:bg-emerald-600 rounded font-medium"
                        title="Run this step, pause at the next one"
                    >
                        Step over
                    </button>
                    <button
                        onClick={() => resume("continue")}
                        className="px-3 py-1.5 text-sm bg-zinc-800 hover:bg-zinc-700 rounded"
                        title="Run to completion"
                    >
                        Continue
                    </button>
                    <button
                        onClick={() => resume("stop")}
                        className="px-3 py-1.5 text-sm bg-red-700 hover:bg-red-600 rounded font-medium"
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

function ScopeView({ pause }: { pause: StepPause }) {
    return (
        <div className="flex-1 overflow-auto p-6 space-y-6 text-xs">
            <CountsSection counts={pause.scope.emit_counts} />
            <BindingsSection scope={pause.scope} />
            <InputsSection inputs={pause.scope.inputs} />
            <SecretsSection names={pause.scope.secrets} />
            {pause.scope.current !== null && pause.scope.current !== undefined && (
                <Section title="$ (current)">
                    <JsonNode value={pause.scope.current} />
                </Section>
            )}
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

/// Compact JSON view: scalars inline, objects/arrays collapsed with a
/// summary plus a `<details>` for the full value. Large strings clip with
/// an ellipsis but the full text is in the DOM via title attr so the user
/// can hover/inspect.
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
