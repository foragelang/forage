import Editor, { type Monaco } from "@monaco-editor/react";
import type * as MonacoNs from "monaco-editor";
import { useEffect, useMemo, useRef, useState } from "react";

import { api, type StepLocation } from "../lib/api";
import { FORAGE_LANG_ID, registerForageLanguage } from "../lib/monaco-forage";
import { useStudio } from "../lib/store";
import { DebuggerPanel } from "../components/DebuggerPanel";

/// Validate the source on every change, debounced. Result lands in the
/// Studio store via `setValidation` so SourceTab's marker effect picks
/// it up and re-paints squigglies without waiting for a save.
function useLiveValidation(source: string, slug: string | null, delayMs = 250) {
    const setValidation = useStudio((s) => s.setValidation);
    useEffect(() => {
        if (!slug) return;
        let cancelled = false;
        const id = window.setTimeout(() => {
            api.validateRecipe(source)
                .then((v) => {
                    if (!cancelled) setValidation(v);
                })
                .catch((e) => console.warn("validate_recipe failed", e));
        }, delayMs);
        return () => {
            cancelled = true;
            window.clearTimeout(id);
        };
    }, [source, slug, delayMs, setValidation]);
}

type Editor = MonacoNs.editor.IStandaloneCodeEditor;

/// Subscribe to the parser-driven outline of the current source. Debounced
/// so we don't fire a Tauri command on every keystroke. The backend
/// returns an empty outline when the source doesn't parse, which is
/// fine — the gutter just shows nothing until the syntax is valid.
function useRecipeOutline(source: string, delayMs = 150): StepLocation[] {
    const [steps, setSteps] = useState<StepLocation[]>([]);
    useEffect(() => {
        let cancelled = false;
        const id = window.setTimeout(() => {
            api.recipeOutline(source)
                .then((o) => {
                    if (!cancelled) setSteps(o.steps);
                })
                .catch((e) => console.warn("recipe_outline failed", e));
        }, delayMs);
        return () => {
            cancelled = true;
            window.clearTimeout(id);
        };
    }, [source, delayMs]);
    return steps;
}

export function SourceTab() {
    const {
        activeSlug,
        source,
        setSource,
        validation,
        breakpoints,
        paused,
        toggleBreakpoint,
    } = useStudio();
    const monacoRef = useRef<Monaco | null>(null);
    const editorRef = useRef<Editor | null>(null);
    const decorationsRef = useRef<string[]>([]);

    useLiveValidation(source, activeSlug);
    const stepLocations = useRecipeOutline(source);

    // Map for the gutter click handler: clicked line → step name (if any).
    // Monaco line numbers are 1-based; the outline lines are 0-based.
    const stepByLine = useMemo(() => {
        const m = new Map<number, string>();
        for (const s of stepLocations) m.set(s.start_line + 1, s.name);
        return m;
    }, [stepLocations]);

    // Step name → 1-based Monaco line (for decorations + reveal).
    const stepNameToLine = useMemo(() => {
        const m = new Map<string, number>();
        for (const s of stepLocations) m.set(s.name, s.start_line + 1);
        return m;
    }, [stepLocations]);

    // Push gutter decorations + paused-line highlight whenever any input
    // changes. deltaDecorations replaces the previous set in one shot so
    // we don't have to track stale IDs across renders.
    useEffect(() => {
        const ed = editorRef.current;
        const monaco = monacoRef.current;
        if (!ed || !monaco) return;
        const decos: MonacoNs.editor.IModelDeltaDecoration[] = [];
        for (const [name, line] of stepNameToLine) {
            if (breakpoints.has(name)) {
                decos.push({
                    range: new monaco.Range(line, 1, line, 1),
                    options: {
                        isWholeLine: false,
                        glyphMarginClassName: "forage-bp-glyph",
                        glyphMarginHoverMessage: { value: `Breakpoint on \`${name}\`` },
                    },
                });
            }
        }
        if (paused?.kind === "step") {
            // Iteration pauses don't anchor on a step — they happen
            // inside a for-loop body, so we leave the editor decoration
            // alone. The DebuggerPanel header tells the user where we
            // are; line-highlighting would need expression-level spans.
            const line = stepNameToLine.get(paused.step);
            if (line) {
                decos.push({
                    range: new monaco.Range(line, 1, line, 1),
                    options: {
                        isWholeLine: true,
                        className: "forage-paused-line",
                        glyphMarginClassName: "forage-paused-glyph",
                    },
                });
            }
        }
        decorationsRef.current = ed.deltaDecorations(
            decorationsRef.current,
            decos,
        );
    }, [stepNameToLine, breakpoints, paused]);

    // Reveal the paused line so the user doesn't have to scroll to find
    // where the engine stopped. Only fire on the rising edge of `paused`
    // — otherwise we'd fight the user every time decorations re-render.
    const pausedStep = paused?.kind === "step" ? paused.step : null;
    useEffect(() => {
        const ed = editorRef.current;
        if (!ed || !pausedStep) return;
        const line = stepNameToLine.get(pausedStep);
        if (line) {
            ed.revealLineInCenterIfOutsideViewport(line);
        }
        // Intentionally only depends on pausedStep — re-running on source
        // changes would interfere with editing while paused.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [pausedStep]);

    useEffect(() => {
        if (!monacoRef.current) return;
        const monaco = monacoRef.current;
        const diagnostics = validation?.diagnostics ?? [];
        const models = monaco.editor.getModels();
        for (const model of models) {
            monaco.editor.setModelMarkers(
                model,
                "forage",
                diagnostics.map((d) => ({
                    // Monaco's MarkerSeverity: 8 = Error, 4 = Warning.
                    severity: d.severity === "error" ? 8 : 4,
                    message: d.message,
                    code: d.code,
                    // Monaco is 1-based for lines AND columns; backend is
                    // 0-based for both. Bump by one. End-col is exclusive
                    // in our spans and exclusive in Monaco's marker shape,
                    // so no off-by-one adjustment beyond the bias.
                    startLineNumber: d.start_line + 1,
                    startColumn: d.start_col + 1,
                    endLineNumber: d.end_line + 1,
                    endColumn: d.end_col + 1,
                })),
            );
        }
    }, [validation]);

    return (
        <div className="flex-1 flex flex-col min-h-0">
            <div className="flex-1 min-h-0">
                <Editor
                    height="100%"
                    language={FORAGE_LANG_ID}
                    theme="vs-dark"
                    value={source}
                    onChange={(v) => setSource(v ?? "")}
                    beforeMount={(monaco) => {
                        monacoRef.current = monaco;
                        registerForageLanguage(monaco);
                    }}
                    onMount={(editor, monaco) => {
                        editorRef.current = editor;
                        monacoRef.current = monaco;
                        // Toggle breakpoint when the user clicks in the
                        // glyph margin on a `step` line. Clicks elsewhere
                        // (or on non-step lines) are ignored — there's no
                        // "breakpoint on emit" yet.
                        editor.onMouseDown((e) => {
                            const T = monaco.editor.MouseTargetType;
                            if (
                                e.target.type !==
                                T.GUTTER_GLYPH_MARGIN
                            ) {
                                return;
                            }
                            const line = e.target.position?.lineNumber;
                            if (!line) return;
                            const name = stepByLine.get(line);
                            if (name) toggleBreakpoint(name);
                        });
                    }}
                    options={{
                        fontSize: 13,
                        tabSize: 4,
                        minimap: { enabled: false },
                        wordWrap: "on",
                        scrollBeyondLastLine: false,
                        renderWhitespace: "selection",
                        glyphMargin: true,
                        lineNumbersMinChars: 3,
                    }}
                />
            </div>
            {paused && <DebuggerPanel />}
            {validation && !paused && (
                <ValidationStrip diagnostics={validation.diagnostics} />
            )}
        </div>
    );
}

function ValidationStrip({ diagnostics }: { diagnostics: import("../lib/api").Diagnostic[] }) {
    if (diagnostics.length === 0) {
        return (
            <div className="border-t border-zinc-800 px-4 py-2 text-xs">
                <span className="text-emerald-400">✓ validates</span>
            </div>
        );
    }
    return (
        <div className="border-t border-zinc-800 px-4 py-2 max-h-32 overflow-y-auto text-xs space-y-0.5">
            {diagnostics.map((d, i) => {
                const tone = d.severity === "error" ? "text-red-400" : "text-amber-400";
                const tag = d.severity === "error" ? "text-red-300" : "text-amber-300";
                return (
                    <div key={i} className={tone}>
                        <span className="text-zinc-500 font-mono">
                            {d.start_line + 1}:{d.start_col + 1}
                        </span>{" "}
                        <span className={`${tag} font-medium`}>{d.severity}:</span>{" "}
                        {d.message}
                    </div>
                );
            })}
        </div>
    );
}
