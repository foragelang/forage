//! Response viewer column in the bottom debug panel.
//!
//! Three tabs (Tree / Raw / Headers) over the captured `StepResponse`
//! for whichever step the user picks in the step selector. The column
//! supports an in-panel Maximize toggle (Call Stack + Scope collapse)
//! and a pop-out to a separate Tauri window. Both controls are props
//! so the same component renders inside the bottom panel and inside
//! `ResponseWindow` (the OS pop-out), where the maximize / pop-out
//! controls don't apply.

import { useEffect, useMemo, useState } from "react";
import { ArrowUpRight, Maximize, Minimize } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

import type { StepResponse } from "@/bindings/StepResponse";
import { JsonTree } from "@/components/Debugger/JsonTree";
import { DomTree } from "@/components/Debugger/DomTree";
import { useStudioService } from "@/lib/services";
import { useStudio } from "@/lib/store";

const TAB_STORAGE_KEY = "forage:debugger-response-tab";

type Tab = "tree" | "raw" | "headers";

function isTab(s: string | null): s is Tab {
    return s === "tree" || s === "raw" || s === "headers";
}

export function ResponseColumn({
    responses,
    runId,
    onMaximize,
    isMaximized,
    onPopOut,
    /// Empty-state fallback copy. The bottom debug panel uses one
    /// message (waiting for the next pause); the Inspector + pop-out
    /// window use another (no responses captured yet).
    emptyStateLabel,
}: {
    /// Map shape matches the ts-rs binding for IndexMap — values are
    /// `StepResponse | undefined` because the TS narrow-from-key
    /// pattern can't prove the key is present. We never iterate
    /// over missing keys, but the type stays optional to satisfy
    /// the inferred binding.
    responses: { [key in string]?: StepResponse };
    runId: string | null;
    /// Optional — pop-out window doesn't show the maximize control.
    onMaximize?: () => void;
    isMaximized?: boolean;
    /// Optional — pop-out window doesn't show the pop-out control.
    onPopOut?: () => void;
    emptyStateLabel: string;
}) {
    const entries = Object.entries(responses).filter(
        (e): e is [string, StepResponse] => e[1] !== undefined,
    );
    const [selectedStep, setSelectedStep] = useState<string | null>(
        entries[0]?.[0] ?? null,
    );
    // Auto-pick the most recent step when new responses arrive. The
    // user can still steer by choosing a different one via the
    // dropdown.
    useEffect(() => {
        if (entries.length === 0) {
            setSelectedStep(null);
            return;
        }
        if (!selectedStep || !responses[selectedStep]) {
            setSelectedStep(entries[entries.length - 1]![0]);
        }
    }, [entries.length, responses, selectedStep]);

    const [tab, setTab] = useState<Tab>(() => {
        try {
            const v = localStorage.getItem(TAB_STORAGE_KEY);
            return isTab(v) ? v : "tree";
        } catch {
            return "tree";
        }
    });
    useEffect(() => {
        try {
            localStorage.setItem(TAB_STORAGE_KEY, tab);
        } catch {
            // Storage may be unavailable (sandboxed iframe, quota);
            // the in-memory tab still drives the UI.
        }
    }, [tab]);

    const response = selectedStep ? responses[selectedStep] : undefined;
    if (entries.length === 0 || !selectedStep || !response) {
        return (
            <Column>
                <Header
                    onMaximize={onMaximize}
                    isMaximized={isMaximized}
                    onPopOut={onPopOut}
                    selector={null}
                    tab={tab}
                    onTabChange={setTab}
                />
                <div className="p-6 text-xs text-muted-foreground italic">
                    {emptyStateLabel}
                </div>
            </Column>
        );
    }

    const selector
        = entries.length > 1
            ? (
                <Select
                    value={selectedStep}
                    onValueChange={(v) => setSelectedStep(v)}
                >
                    <SelectTrigger className="h-6 text-xs gap-1 w-40">
                        <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                        {entries.map(([step]) => (
                            <SelectItem key={step} value={step} className="font-mono text-xs">
                                {step}
                            </SelectItem>
                        ))}
                    </SelectContent>
                </Select>
            )
            : (
                <span className="font-mono text-xs text-foreground">
                    {selectedStep}
                </span>
            );

    return (
        <Column>
            <Header
                onMaximize={onMaximize}
                isMaximized={isMaximized}
                onPopOut={onPopOut}
                selector={selector}
                tab={tab}
                onTabChange={setTab}
            />
            <div className="flex-1 min-h-0 overflow-hidden">
                {tab === "tree" && <TreeTab response={response} onSwitchTab={setTab} />}
                {tab === "raw" && (
                    <RawTab
                        runId={runId}
                        stepName={selectedStep}
                        response={response}
                    />
                )}
                {tab === "headers" && <HeadersTab response={response} />}
            </div>
        </Column>
    );
}

function Column({ children }: { children: React.ReactNode }) {
    return <div className="flex flex-col min-h-0">{children}</div>;
}

function Header({
    onMaximize,
    isMaximized,
    onPopOut,
    selector,
    tab,
    onTabChange,
}: {
    onMaximize?: () => void;
    isMaximized?: boolean;
    onPopOut?: () => void;
    selector: React.ReactNode | null;
    tab: Tab;
    onTabChange: (t: Tab) => void;
}) {
    return (
        <div className="flex items-center gap-2 border-b px-2 py-1 shrink-0">
            <span className="text-[10px] uppercase tracking-wider font-semibold text-muted-foreground">
                Response
            </span>
            {selector}
            <div className="flex items-center gap-0.5 ml-2 border rounded">
                {(["tree", "raw", "headers"] as const).map((t) => (
                    <button
                        key={t}
                        type="button"
                        onClick={() => onTabChange(t)}
                        className={cn(
                            "px-2 py-0.5 text-xs",
                            tab === t
                                ? "bg-muted text-foreground"
                                : "text-muted-foreground hover:text-foreground",
                        )}
                    >
                        {t === "tree" ? "Tree" : t === "raw" ? "Raw" : "Headers"}
                    </button>
                ))}
            </div>
            <div className="ml-auto flex items-center gap-1">
                {onPopOut && (
                    <Tooltip>
                        <TooltipTrigger asChild>
                            <Button
                                size="icon-xs"
                                variant="ghost"
                                onClick={onPopOut}
                                aria-label="Pop out to window"
                            >
                                <ArrowUpRight />
                            </Button>
                        </TooltipTrigger>
                        <TooltipContent>Open in a separate window</TooltipContent>
                    </Tooltip>
                )}
                {onMaximize && (
                    <Tooltip>
                        <TooltipTrigger asChild>
                            <Button
                                size="icon-xs"
                                variant="ghost"
                                onClick={onMaximize}
                                aria-label={isMaximized ? "Restore layout" : "Maximize"}
                            >
                                {isMaximized ? <Minimize /> : <Maximize />}
                            </Button>
                        </TooltipTrigger>
                        <TooltipContent>
                            {isMaximized ? "Restore 3-column layout" : "Maximize response viewer"}
                        </TooltipContent>
                    </Tooltip>
                )}
            </div>
        </div>
    );
}

function TreeTab({
    response,
    onSwitchTab,
}: {
    response: StepResponse;
    onSwitchTab: (t: Tab) => void;
}) {
    if (response.format === "json") {
        const parsed = useMemo(() => {
            try {
                return JSON.parse(response.body_raw);
            } catch {
                return null;
            }
        }, [response.body_raw]);
        if (parsed === null) {
            return (
                <div className="p-3 text-xs text-warning">
                    Body did not parse as JSON. Switch to{" "}
                    <button
                        type="button"
                        className="underline"
                        onClick={() => onSwitchTab("raw")}
                    >
                        Raw
                    </button>{" "}
                    to inspect the bytes.
                </div>
            );
        }
        return <JsonTree value={parsed} />;
    }
    if (response.format === "html" || response.format === "xml") {
        return (
            <DomTree
                source={response.body_raw}
                mime={response.format === "html" ? "text/html" : "application/xml"}
            />
        );
    }
    return (
        <div className="p-3 text-xs text-muted-foreground italic">
            Plain text — see{" "}
            <button
                type="button"
                className="underline text-foreground"
                onClick={() => onSwitchTab("raw")}
            >
                Raw
            </button>
            .
        </div>
    );
}

function RawTab({
    runId,
    stepName,
    response,
}: {
    runId: string | null;
    stepName: string;
    response: StepResponse;
}) {
    const service = useStudioService();
    const [fullBody, setFullBody] = useState<string | null>(null);
    const [loadingFull, setLoadingFull] = useState(false);
    const [loadError, setLoadError] = useState<string | null>(null);
    const display = useMemo(() => {
        const body = fullBody ?? response.body_raw;
        if (response.format !== "json") return body;
        try {
            return JSON.stringify(JSON.parse(body), null, 2);
        } catch {
            // Parse failure on a "json" format: render verbatim
            // (auto-prettifying broken JSON loses the original
            // shape) and leave format detection to the override pill.
            return body;
        }
    }, [fullBody, response.body_raw, response.format]);
    const showLoadFull = response.body_truncated && fullBody === null && runId !== null;
    return (
        <div className="flex flex-col min-h-0">
            {response.body_truncated && (
                <div className="flex items-center gap-2 border-b px-2 py-1 text-xs text-warning">
                    <span>Body truncated to 1 MiB.</span>
                    {showLoadFull && (
                        <Button
                            size="xs"
                            variant="outline"
                            disabled={loadingFull}
                            onClick={async () => {
                                setLoadingFull(true);
                                setLoadError(null);
                                try {
                                    const full = await service.loadFullStepBody(
                                        runId!,
                                        stepName,
                                    );
                                    setFullBody(full);
                                } catch (e) {
                                    setLoadError(String(e));
                                } finally {
                                    setLoadingFull(false);
                                }
                            }}
                        >
                            {loadingFull ? "Loading…" : "Load full"}
                        </Button>
                    )}
                    {loadError && <span className="text-destructive">{loadError}</span>}
                </div>
            )}
            <pre className="flex-1 overflow-auto p-2 text-xs whitespace-pre-wrap break-all select-text">
                {display}
            </pre>
        </div>
    );
}

function HeadersTab({ response }: { response: StepResponse }) {
    const statusTone = statusToneClass(response.status);
    // Sort headers case-insensitively so the rendered list stays
    // stable across re-renders + matches what curl -i shows.
    const headers = useMemo(
        () =>
            Object.entries(response.headers).sort(([a], [b]) =>
                a.toLowerCase().localeCompare(b.toLowerCase()),
            ),
        [response.headers],
    );
    // The override pill compares the recipe-resolved format against
    // what the server's Content-Type header would have detected.
    // When the response carries no Content-Type, the engine falls
    // through its JSON-first heuristic to set `format`; with no
    // header in hand there's no "server said X" to overrule, so we
    // never light the override pill in that case.
    const overrideActive
        = response.content_type_header !== null
            && detectFormat(response.content_type_header) !== response.format;
    return (
        <div className="flex flex-col min-h-0">
            <div className="flex items-center gap-2 border-b px-2 py-1 shrink-0">
                <Badge className={cn("tabular-nums", statusTone)}>
                    {response.status}
                </Badge>
                <span className="text-xs text-muted-foreground">
                    {response.format.toUpperCase()}
                </span>
                {overrideActive && (
                    <Badge variant="outline" className="text-[10px]">
                        Override: parsed as {response.format}
                        {response.content_type_header
                            ? ` (server said: ${response.content_type_header})`
                            : ""}
                    </Badge>
                )}
            </div>
            <div className="flex-1 overflow-auto p-2 text-xs font-mono select-text">
                <table className="w-full">
                    <tbody>
                        {headers.map(([k, v]) => (
                            <tr
                                key={k}
                                className="hover:bg-muted/30 cursor-pointer"
                                onClick={() => {
                                    void navigator.clipboard?.writeText(v);
                                }}
                                title="Click to copy value"
                            >
                                <td className="text-muted-foreground pr-3 py-0.5 align-top whitespace-nowrap">
                                    {k.toLowerCase()}
                                </td>
                                <td className="py-0.5 break-all">{v}</td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            </div>
        </div>
    );
}

function statusToneClass(status: number): string {
    if (status >= 500) return "bg-destructive text-destructive-foreground";
    if (status >= 400) return "bg-warning text-warning-foreground";
    if (status >= 300) return "bg-muted text-foreground";
    if (status >= 200) return "bg-success text-success-foreground";
    return "bg-muted text-muted-foreground";
}

/// Mirror of the engine-side `ParseFormat::from_content_type` —
/// minimal version that's enough to drive the override pill. Anything
/// not recognized falls back to "text" so the pill says
/// `Override: parsed as <fmt> (server said: <ct>)`.
function detectFormat(contentType: string): "json" | "html" | "xml" | "text" {
    const t = contentType.toLowerCase();
    if (t === "application/json" || t.endsWith("+json") || t.startsWith("application/json")) {
        return "json";
    }
    if (t === "text/html") return "html";
    if (
        t === "application/xml"
        || t === "text/xml"
        || t.endsWith("+xml")
        || t.startsWith("application/xml")
    ) {
        return "xml";
    }
    return "text";
}

/// Shared maximize hook for the in-panel column: caller toggles the
/// flag; the panel that mounts this hides the other columns when
/// it's true.
export function useMaximizeResponse(): {
    maximized: boolean;
    toggle: () => void;
} {
    const [maximized, setMaximized] = useState(false);
    // Re-render when the store's paused changes — restore default
    // (non-maximized) at each new pause so the user doesn't carry a
    // wedged maximize from a prior session.
    const paused = useStudio((s) => s.paused);
    useEffect(() => {
        setMaximized(false);
    }, [paused]);
    return {
        maximized,
        toggle: () => setMaximized((v) => !v),
    };
}
