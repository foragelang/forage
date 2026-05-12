import Editor, { type Monaco } from "@monaco-editor/react";
import type * as MonacoNs from "monaco-editor";
import { useEffect, useMemo, useRef } from "react";

import { FORAGE_LANG_ID, registerForageLanguage } from "../lib/monaco-forage";
import { useStudio } from "../lib/store";
import { DebuggerPanel } from "../components/DebuggerPanel";

type Editor = MonacoNs.editor.IStandaloneCodeEditor;

/// Parse the recipe source for `step <name> { … }` declarations and return
/// `name → 1-based line number`. The full parser lives in Rust — for gutter
/// markers we don't need full AST fidelity; we only need to map step names
/// to lines, which the surface syntax pins down unambiguously: `step` is a
/// keyword and the immediately following identifier is the step name.
function stepLines(source: string): Map<string, number> {
    const out = new Map<string, number>();
    const lines = source.split("\n");
    const re = /^\s*step\s+([A-Za-z_][A-Za-z_0-9]*)/;
    for (let i = 0; i < lines.length; i++) {
        const m = lines[i].match(re);
        if (m) out.set(m[1], i + 1);
    }
    return out;
}

export function SourceTab() {
    const {
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

    const stepByLine = useMemo(() => {
        // Reverse map for the gutter click handler — given a clicked line,
        // tell me which step name it belongs to (if any).
        const lines = stepLines(source);
        const byLine = new Map<number, string>();
        for (const [name, line] of lines) byLine.set(line, name);
        return byLine;
    }, [source]);

    // Push gutter decorations + paused-line highlight whenever any input
    // changes. deltaDecorations replaces the previous set in one shot so
    // we don't have to track stale IDs across renders.
    useEffect(() => {
        const ed = editorRef.current;
        const monaco = monacoRef.current;
        if (!ed || !monaco) return;
        const lines = stepLines(source);
        const decos: MonacoNs.editor.IModelDeltaDecoration[] = [];
        for (const [name, line] of lines) {
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
        if (paused) {
            const line = lines.get(paused.step);
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
    }, [source, breakpoints, paused]);

    // Reveal the paused line so the user doesn't have to scroll to find
    // where the engine stopped. Only fire on the rising edge of `paused`
    // — otherwise we'd fight the user every time decorations re-render.
    const pausedStep = paused?.step ?? null;
    useEffect(() => {
        const ed = editorRef.current;
        if (!ed || !pausedStep) return;
        const lines = stepLines(source);
        const line = lines.get(pausedStep);
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
