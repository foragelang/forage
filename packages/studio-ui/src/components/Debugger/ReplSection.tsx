//! Interactive REPL / console section. Sits below Watches in the
//! Scope column. The user types a Forage expression, hits Enter, the
//! evaluator runs against the paused scope, and the result lands in a
//! log-style transcript above the input.
//!
//! Persistence:
//! - Input history (last 200 entries) per recipe in localStorage.
//! - Transcript lives on the store because `DebuggerPanel` unmounts
//!   between pauses (paused → null → next pause); without
//!   store-hosting, the transcript would vanish at every Continue.

import { useEffect, useRef, useState } from "react";
import { Send, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useStudioService } from "@/lib/services";
import { useStudio } from "@/lib/store";

import { JsonNode } from "./JsonNode";

const HISTORY_CAP = 200;

function historyKey(recipeName: string | null): string | null {
    if (!recipeName) return null;
    return `forage:repl-history:${recipeName}`;
}

function loadHistory(recipeName: string | null): string[] {
    const key = historyKey(recipeName);
    if (!key) return [];
    try {
        const raw = localStorage.getItem(key);
        if (!raw) return [];
        const parsed = JSON.parse(raw);
        if (Array.isArray(parsed) && parsed.every((s) => typeof s === "string")) {
            return parsed.slice(-HISTORY_CAP);
        }
        return [];
    } catch {
        return [];
    }
}

function saveHistory(recipeName: string | null, history: string[]): void {
    const key = historyKey(recipeName);
    if (!key) return;
    try {
        localStorage.setItem(key, JSON.stringify(history.slice(-HISTORY_CAP)));
    } catch {
        // Quota / disabled storage — silent drop.
    }
}

export function ReplSection() {
    const service = useStudioService();
    const recipeName = useStudio((s) => s.activeRecipeName);
    const transcript = useStudio((s) => s.replTranscript);
    const appendReplEntry = useStudio((s) => s.appendReplEntry);
    const clearReplTranscript = useStudio((s) => s.clearReplTranscript);
    const pauseId = useStudio((s) => s.pauseId);

    const [input, setInput] = useState("");
    const [history, setHistory] = useState<string[]>(() => loadHistory(recipeName));
    const [historyIdx, setHistoryIdx] = useState<number | null>(null);
    const scrollRef = useRef<HTMLDivElement | null>(null);
    const wasStuckRef = useRef(true);

    // Re-load history when the recipe changes (workspace open /
    // file switch).
    useEffect(() => {
        setHistory(loadHistory(recipeName));
        setHistoryIdx(null);
    }, [recipeName]);

    // Auto-scroll only when the user is already at the bottom of the
    // transcript. Detached scrolls (the user reading an earlier
    // entry) shouldn't be yanked back to the latest line.
    useEffect(() => {
        const el = scrollRef.current;
        if (!el) return;
        if (wasStuckRef.current) {
            el.scrollTop = el.scrollHeight;
        }
    }, [transcript.length]);

    async function submit() {
        const expr = input.trim();
        if (!expr) return;
        setInput("");
        setHistoryIdx(null);
        const nextHistory = [...history.filter((h) => h !== expr), expr].slice(-HISTORY_CAP);
        setHistory(nextHistory);
        saveHistory(recipeName, nextHistory);
        try {
            const value = await service.evalWatchExpression(expr);
            appendReplEntry({ kind: "result", input: expr, pauseId, value });
        } catch (e) {
            appendReplEntry({ kind: "error", input: expr, pauseId, message: String(e) });
        }
    }

    function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
        if (e.key === "ArrowUp") {
            if (history.length === 0) return;
            e.preventDefault();
            const next = historyIdx === null
                ? history.length - 1
                : Math.max(0, historyIdx - 1);
            setHistoryIdx(next);
            setInput(history[next] ?? "");
        } else if (e.key === "ArrowDown") {
            if (historyIdx === null) return;
            e.preventDefault();
            const next = historyIdx + 1;
            if (next >= history.length) {
                setHistoryIdx(null);
                setInput("");
            } else {
                setHistoryIdx(next);
                setInput(history[next] ?? "");
            }
        }
    }

    return (
        <section className="flex flex-col min-h-0">
            <div className="flex items-center justify-between mb-2">
                <h3 className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold">
                    REPL
                </h3>
                {transcript.length > 0 && (
                    <button
                        type="button"
                        onClick={clearReplTranscript}
                        className="text-muted-foreground hover:text-foreground"
                        aria-label="Clear transcript"
                        title="Clear transcript"
                    >
                        <Trash2 className="size-3" />
                    </button>
                )}
            </div>
            <div
                ref={scrollRef}
                onScroll={(e) => {
                    const el = e.currentTarget;
                    wasStuckRef.current
                        = el.scrollHeight - el.scrollTop - el.clientHeight < 16;
                }}
                className="max-h-64 overflow-y-auto space-y-1 font-mono text-xs select-text mb-2"
            >
                {transcript.map((entry, i) => {
                    const prev = transcript[i - 1];
                    const newPause = prev && prev.pauseId !== entry.pauseId;
                    return (
                        <div key={i}>
                            {newPause && (
                                <div className="border-t border-dashed border-border my-1 opacity-60" />
                            )}
                            <ReplEntryRow entry={entry} />
                        </div>
                    );
                })}
                {transcript.length === 0 && (
                    <div className="text-muted-foreground italic">
                        Evaluate ad-hoc expressions against the paused scope.
                    </div>
                )}
            </div>
            <form
                onSubmit={(e) => {
                    e.preventDefault();
                    void submit();
                }}
                className="flex gap-1"
            >
                <span className="text-muted-foreground self-center">&gt;</span>
                <Input
                    value={input}
                    onChange={(e) => {
                        setInput(e.target.value);
                        setHistoryIdx(null);
                    }}
                    onKeyDown={onKeyDown}
                    placeholder="$list.items | length"
                    className="h-7 text-xs font-mono flex-1"
                    aria-label="REPL input"
                />
                <Button
                    type="submit"
                    size="icon-xs"
                    variant="ghost"
                    aria-label="Evaluate"
                    disabled={input.trim() === ""}
                >
                    <Send />
                </Button>
            </form>
        </section>
    );
}

function ReplEntryRow({
    entry,
}: {
    entry: { kind: "result"; input: string; value: unknown } | { kind: "error"; input: string; message: string };
}) {
    return (
        <>
            <div className="text-muted-foreground">
                <span className="opacity-60">&gt;</span> {entry.input}
            </div>
            <div className="pl-3">
                {entry.kind === "result"
                    ? <JsonNode value={entry.value} />
                    : <span className="text-destructive break-all">{entry.message}</span>}
            </div>
        </>
    );
}
