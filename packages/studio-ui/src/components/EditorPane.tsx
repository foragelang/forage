//! Editor pane — Monaco wrapper, gutter decorations, inline step-stats
//! content widgets, validation bar, and a status strip below.
//!
//! Lives in the editor view's center column. Reactive-UI rules:
//! - leaf reads from the store, no destructuring;
//! - commands flow in via DOM events (`forage:reveal-line`), not via
//!   observable state fields that we'd have to reset.

import Editor, { type Monaco } from "@monaco-editor/react";
import type * as MonacoNs from "monaco-editor";
import { useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, CheckCircle2, CircleAlert, CircleX } from "lucide-react";

import { Alert, AlertDescription } from "@/components/ui/alert";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { Diagnostic } from "@/bindings/Diagnostic";
import type { PausePoint } from "@/bindings/PausePoint";
import { useStudioService } from "@/lib/services";
import { onRevealLine } from "@/lib/editorCommands";
import { FORAGE_LANG_ID, registerForageLanguage } from "@/lib/monaco-forage";
import { useRecipeNameOf } from "@/hooks/useRecipes";
import { useStudio, type StepStat } from "@/lib/store";

type IEditor = MonacoNs.editor.IStandaloneCodeEditor;

/// localStorage key for the editor-wide Vim mode toggle. The toggle
/// itself is global (not per-recipe) because the user's editing
/// preference doesn't change between recipes; sticking it to a recipe
/// would force them to re-toggle on every file switch.
const VIM_MODE_STORAGE_KEY = "forage:vim-mode";

/// Validate the source on every change, debounced. Result lands in the
/// Studio store via `setValidation` so the editor's marker effect picks
/// it up and re-paints squigglies without waiting for a save.
function useLiveValidation(source: string, recipeName: string | null, delayMs = 250) {
    const service = useStudioService();
    const setValidation = useStudio((s) => s.setValidation);
    useEffect(() => {
        if (!recipeName) return;
        let cancelled = false;
        const id = window.setTimeout(() => {
            service.validateRecipe(source)
                .then((v) => {
                    if (!cancelled) setValidation(v);
                })
                .catch((e) => console.warn("validate_recipe failed", e));
        }, delayMs);
        return () => {
            cancelled = true;
            window.clearTimeout(id);
        };
    }, [source, recipeName, delayMs, setValidation, service]);
}

/// Subscribe to the parser-driven outline of the current source.
/// Debounced so we don't fire a backend call on every keystroke. The
/// backend returns an empty outline when the source doesn't parse,
/// which is fine — the gutter just shows nothing until the syntax is
/// valid.
function useRecipeOutline(source: string, delayMs = 150): PausePoint[] {
    const service = useStudioService();
    const [points, setPoints] = useState<PausePoint[]>([]);
    useEffect(() => {
        let cancelled = false;
        const id = window.setTimeout(() => {
            service.recipeOutline(source)
                .then((o) => {
                    if (!cancelled) setPoints(o.pause_points);
                })
                .catch((e) => console.warn("recipe_outline failed", e));
        }, delayMs);
        return () => {
            cancelled = true;
            window.clearTimeout(id);
        };
    }, [source, delayMs, service]);
    return points;
}

/// Human-readable label for a pause point, used in gutter tooltips
/// and the inline step-stats pill.
function pauseLabel(p: PausePoint): string {
    if (p.kind === "step") return `step \`${p.name}\``;
    if (p.kind === "emit") return `emit \`${p.type_name}\``;
    return `for \`$${p.variable}\``;
}

export function EditorPane() {
    const activeFilePath = useStudio((s) => s.activeFilePath);
    const source = useStudio((s) => s.source);
    const setSource = useStudio((s) => s.setSource);
    const validation = useStudio((s) => s.validation);
    const breakpoints = useStudio((s) => s.breakpoints);
    const paused = useStudio((s) => s.paused);
    const toggleBreakpoint = useStudio((s) => s.toggleBreakpoint);
    const stepStats = useStudio((s) => s.stepStats);
    const recipeName = useRecipeNameOf(activeFilePath);
    const monacoRef = useRef<Monaco | null>(null);
    const editorRef = useRef<IEditor | null>(null);
    const decorationsRef = useRef<string[]>([]);
    // Bumped from onMount once the editor instance is in place. The
    // effects that key on the editor (gutter clicks, decorations,
    // step-stats widgets, Vim mode) include this in their dep array
    // so they re-fire when the ref lands — without it, the first
    // effect pass sees a null `editorRef.current` and bails out, then
    // never re-runs.
    const [editorReady, setEditorReady] = useState(0);
    const [cursor, setCursor] = useState<{ line: number; column: number } | null>(
        null,
    );
    const [hoverLine, setHoverLine] = useState<number | null>(null);

    useLiveValidation(source, recipeName);
    const pausePoints = useRecipeOutline(source);

    // Map for the gutter click handler: clicked 1-based Monaco line →
    // pause point on that line. Pause points carry 0-based start lines,
    // so we shift here once at memo time.
    const pointByLine = useMemo(() => {
        const m = new Map<number, PausePoint>();
        for (const p of pausePoints) m.set(p.start_line + 1, p);
        return m;
    }, [pausePoints]);

    // Push gutter decorations + paused-line highlight whenever any input
    // changes. deltaDecorations replaces the previous set in one shot so
    // we don't have to track stale IDs across renders.
    useEffect(() => {
        const ed = editorRef.current;
        const monaco = monacoRef.current;
        if (!ed || !monaco) return;
        const decos: MonacoNs.editor.IModelDeltaDecoration[] = [];
        for (const [line, point] of pointByLine) {
            if (breakpoints.has(line - 1)) {
                decos.push({
                    range: new monaco.Range(line, 1, line, 1),
                    options: {
                        isWholeLine: false,
                        glyphMarginClassName: "forage-bp-glyph",
                        glyphMarginHoverMessage: {
                            value: `Breakpoint on ${pauseLabel(point)}`,
                        },
                    },
                });
            } else if (hoverLine === line) {
                // Hover-preview affordance: when the mouse is over a
                // pause-able line with no BP set, render a faded
                // version of the BP glyph so the user knows clicking
                // here would set one. The `forage-bp-hover` class
                // pairs with the same glyph at reduced opacity.
                decos.push({
                    range: new monaco.Range(line, 1, line, 1),
                    options: {
                        isWholeLine: false,
                        glyphMarginClassName: "forage-bp-hover",
                        glyphMarginHoverMessage: {
                            value: `Click to set breakpoint on ${pauseLabel(point)}`,
                        },
                    },
                });
            }
        }
        // Paused-line decoration: every payload variant carries
        // `start_line`, so the highlight works for step / emit / for
        // pauses with one branch.
        if (paused) {
            const line = paused.start_line + 1;
            decos.push({
                range: new monaco.Range(line, 1, line, 1),
                options: {
                    isWholeLine: true,
                    className: "forage-paused-line",
                    glyphMarginClassName: "forage-paused-glyph",
                },
            });
        }
        decorationsRef.current = ed.deltaDecorations(
            decorationsRef.current,
            decos,
        );
    }, [pointByLine, breakpoints, paused, hoverLine, editorReady]);

    // Reveal the paused line so the user doesn't have to scroll to find
    // where the engine stopped. Only fire on the rising edge of the
    // pause's line number — otherwise we'd fight the user every time
    // decorations re-render.
    const pausedLine = paused ? paused.start_line + 1 : null;
    useEffect(() => {
        const ed = editorRef.current;
        if (!ed || pausedLine === null) return;
        ed.revealLineInCenterIfOutsideViewport(pausedLine);
        // Intentionally only depends on pausedLine — re-running on
        // source changes would interfere with editing while paused.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [pausedLine]);

    // External reveal-line commands (e.g. clicking a diagnostic's
    // `recipe:L` badge in the inspector). The handler is registered on
    // mount and torn down on unmount — the channel is pub/sub via a
    // CustomEvent on window, so nothing else has to know we're here.
    useEffect(() => {
        const off = onRevealLine((line) => {
            const ed = editorRef.current;
            if (!ed) return;
            ed.revealLineInCenter(line);
            ed.setPosition({ lineNumber: line, column: 1 });
            ed.focus();
        });
        return off;
    }, []);

    // Gutter click handler — registered in an effect keyed on the
    // point-by-line map so the captured map stays current with
    // outline updates. A handler installed in onMount would close
    // over the first render's map and stay stale across recipe
    // edits; rebinding on every map identity keeps the click site
    // pointed at the freshest pause points.
    useEffect(() => {
        const ed = editorRef.current;
        const monaco = monacoRef.current;
        if (!ed || !monaco) return;
        const T = monaco.editor.MouseTargetType;
        const downSub = ed.onMouseDown((e) => {
            if (e.target.type !== T.GUTTER_GLYPH_MARGIN) return;
            const line = e.target.position?.lineNumber;
            if (!line) return;
            if (pointByLine.has(line)) {
                // The breakpoint set keys on 0-based line; the
                // toggle action takes that shape directly.
                toggleBreakpoint(line - 1);
            }
        });
        const moveSub = ed.onMouseMove((e) => {
            if (e.target.type !== T.GUTTER_GLYPH_MARGIN) {
                setHoverLine(null);
                return;
            }
            const line = e.target.position?.lineNumber ?? null;
            setHoverLine(line && pointByLine.has(line) ? line : null);
        });
        const leaveSub = ed.onMouseLeave(() => setHoverLine(null));
        return () => {
            downSub.dispose();
            moveSub.dispose();
            leaveSub.dispose();
        };
    }, [pointByLine, toggleBreakpoint, editorReady]);

    // Vim mode (lazy-loaded monaco-vim). Toggle persists in
    // localStorage; the actual mode instance lives in a ref so the
    // effect can dispose it on toggle-off or unmount. Status node is
    // a DOM element the strip below renders; the lazy import wires
    // its mode label into that node.
    const [vimEnabled, setVimEnabled] = useState<boolean>(() => {
        try {
            return localStorage.getItem(VIM_MODE_STORAGE_KEY) === "1";
        } catch {
            return false;
        }
    });
    const vimStatusNodeRef = useRef<HTMLSpanElement | null>(null);
    const vimInstanceRef = useRef<{ dispose: () => void } | null>(null);
    useEffect(() => {
        const ed = editorRef.current;
        if (!ed) return;
        const statusNode = vimStatusNodeRef.current;
        if (!vimEnabled) {
            vimInstanceRef.current?.dispose();
            vimInstanceRef.current = null;
            return;
        }
        let disposed = false;
        // Lazy import keeps the monaco-vim chunk out of the initial
        // bundle for users who never toggle the mode on.
        import("monaco-vim")
            .then((mod) => {
                if (disposed) return;
                const init = (mod.initVimMode ?? (mod as unknown as { default: typeof mod.initVimMode }).default) as
                    typeof mod.initVimMode;
                vimInstanceRef.current = init(ed, statusNode ?? undefined);
            })
            .catch((e) => console.warn("monaco-vim import failed", e));
        return () => {
            disposed = true;
            vimInstanceRef.current?.dispose();
            vimInstanceRef.current = null;
        };
    }, [vimEnabled, editorReady]);
    const toggleVim = () => {
        setVimEnabled((prev) => {
            const next = !prev;
            try {
                localStorage.setItem(VIM_MODE_STORAGE_KEY, next ? "1" : "0");
            } catch {
                // Ignore storage failures — the in-memory flag drives
                // the editor regardless.
            }
            return next;
        });
    };

    // Mount inline step-stats content widgets. Each widget is one DOM
    // node anchored to the end of the step's first line. We rebuild the
    // full set whenever the pause points or the per-step stats change
    // — Monaco diffs internally via the widget id, so existing widgets
    // stay in place across re-renders.
    useEffect(() => {
        const ed = editorRef.current;
        if (!ed) return;
        if (Object.keys(stepStats).length === 0) {
            return undefined;
        }
        const widgets: MonacoNs.editor.IContentWidget[] = [];
        for (const p of pausePoints) {
            if (p.kind !== "step") continue;
            const labels = formatStepStat(stepStats[p.name]);
            if (!labels) continue;
            const dom = document.createElement("span");
            dom.className = `step-stat step-stat-${labels.tone}`;
            dom.textContent = labels.text;
            const widget: MonacoNs.editor.IContentWidget = {
                getId: () => `forage:step-stat:${p.name}`,
                getDomNode: () => dom,
                getPosition: () => ({
                    position: { lineNumber: p.start_line + 1, column: Number.MAX_SAFE_INTEGER },
                    preference: [1 /* EXACT */],
                }),
            };
            ed.addContentWidget(widget);
            widgets.push(widget);
        }
        return () => {
            for (const w of widgets) ed.removeContentWidget(w);
        };
    }, [pausePoints, stepStats, editorReady]);

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
                    severity: d.severity === "error" ? 8 : 4,
                    message: d.message,
                    code: d.code,
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
                        setEditorReady((n) => n + 1);
                        // Track caret for the status strip below.
                        const initial = editor.getPosition();
                        if (initial) {
                            setCursor({ line: initial.lineNumber, column: initial.column });
                        }
                        editor.onDidChangeCursorPosition((e) => {
                            setCursor({
                                line: e.position.lineNumber,
                                column: e.position.column,
                            });
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
            {validation && <ValidationBar diagnostics={validation.diagnostics} />}
            <StatusStrip
                cursor={cursor}
                vimEnabled={vimEnabled}
                onToggleVim={toggleVim}
                vimStatusNodeRef={vimStatusNodeRef}
            />
        </div>
    );
}

/// Compute the inline pill text + tone for a step. Step-stats are a
/// live-run feedback signal — pills appear while the engine is firing
/// requests against a step and stick around until the run finishes.
/// Authoring with no live stats shows no pill.
function formatStepStat(
    live: StepStat | undefined,
): { text: string; tone: "ok" | "warn" | "fail" } | null {
    if (!live) return null;
    const parts: string[] = [];
    if (live.requests > 0) parts.push(`${live.requests} req`);
    if (live.emits > 0) parts.push(`${live.emits.toLocaleString()} emit`);
    if (live.duration_ms != null) parts.push(formatDuration(live.duration_ms));
    if (parts.length === 0) return null;
    const tone: "ok" | "warn" | "fail" = live.failed
        ? "fail"
        : !live.done
          ? "ok"
          : live.requests > 0 && live.emits === 0
            ? "warn"
            : "ok";
    return { text: parts.join(" · "), tone };
}

function formatDuration(ms: number): string {
    if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`;
    return `${ms}ms`;
}

function ValidationBar({ diagnostics }: { diagnostics: Diagnostic[] }) {
    if (diagnostics.length === 0) {
        return (
            <Alert
                variant="success"
                className="rounded-none border-x-0 border-b-0 px-4 py-2"
            >
                <CheckCircle2 />
                <AlertDescription className="text-success">
                    Validates cleanly.
                </AlertDescription>
            </Alert>
        );
    }
    return (
        <ScrollArea className="max-h-32 border-t shrink-0">
            <div className="px-4 py-2 space-y-1 text-xs select-text">
                {diagnostics.map((d, i) => {
                    const isError = d.severity === "error";
                    const tone = isError ? "text-destructive" : "text-warning";
                    const Icon = isError ? CircleX : CircleAlert;
                    return (
                        <div key={i} className={`flex items-start gap-2 ${tone}`}>
                            <Icon className="size-3.5 mt-0.5 shrink-0" />
                            <span>
                                <span className="text-muted-foreground font-mono mr-2 tabular-nums">
                                    {d.start_line + 1}:{d.start_col + 1}
                                </span>
                                <span className="font-medium">{d.severity}:</span>{" "}
                                {d.message}
                            </span>
                        </div>
                    );
                })}
            </div>
        </ScrollArea>
    );
}

/// Below-editor strip: parses · N errors · M warnings · browser engine ·
/// cursor position · language. Cursor position threads in as a prop —
/// it's a Monaco-side observation, not store state, so leaf-reading
/// would force a parallel signal.
function StatusStrip({
    cursor,
    vimEnabled,
    onToggleVim,
    vimStatusNodeRef,
}: {
    cursor: { line: number; column: number } | null;
    vimEnabled: boolean;
    onToggleVim: () => void;
    vimStatusNodeRef: React.RefObject<HTMLSpanElement | null>;
}) {
    const validation = useStudio((s) => s.validation);
    const errCount =
        validation?.diagnostics.filter((d) => d.severity === "error").length ?? 0;
    const warnCount =
        validation?.diagnostics.filter((d) => d.severity === "warning").length ?? 0;
    const ok = errCount === 0;
    return (
        <div className="flex items-center gap-3 border-t px-3 py-1.5 text-[11px] text-muted-foreground select-none">
            <span
                className={`flex items-center gap-1 ${ok ? "text-success" : "text-destructive"}`}
            >
                {ok ? (
                    <CheckCircle2 className="size-3" />
                ) : (
                    <CircleX className="size-3" />
                )}
                <span>Parses</span>
            </span>
            <span className="flex items-center gap-1 text-destructive">
                <span>·</span>
                <span>
                    {errCount} error{errCount === 1 ? "" : "s"}
                </span>
            </span>
            <span className="flex items-center gap-1 text-warning">
                <span>·</span>
                {warnCount > 0 && <AlertTriangle className="size-3" />}
                <span>
                    {warnCount} warning{warnCount === 1 ? "" : "s"}
                </span>
            </span>
            <span className="ml-auto">browser engine · wry</span>
            <span className="opacity-50">·</span>
            <button
                type="button"
                onClick={onToggleVim}
                aria-pressed={vimEnabled}
                className={`px-1 font-mono select-none ${vimEnabled ? "text-amber-500" : "text-muted-foreground hover:text-foreground"}`}
                title={vimEnabled ? "Disable Vim mode" : "Enable Vim mode"}
            >
                vim {vimEnabled ? "on" : "off"}
            </button>
            <span
                ref={vimStatusNodeRef}
                className="font-mono text-[10px] text-muted-foreground"
            />
            <span className="opacity-50">·</span>
            <span className="font-mono tabular-nums">
                {cursor
                    ? `Ln ${cursor.line}, Col ${cursor.column}`
                    : "Ln —, Col —"}
            </span>
            <span className="opacity-50">·</span>
            <span className="font-mono">forage</span>
        </div>
    );
}
