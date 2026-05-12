import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { Loader2, Pause, Play, RefreshCw, Save } from "lucide-react";

import { Sidebar } from "@/components/Sidebar";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Separator } from "@/components/ui/separator";
import {
    SidebarInset,
    SidebarProvider,
    SidebarTrigger,
} from "@/components/ui/sidebar";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

import { SourceTab } from "@/tabs/SourceTab";
import { FixturesTab } from "@/tabs/FixturesTab";
import { SnapshotTab } from "@/tabs/SnapshotTab";
import { DiagnosticTab } from "@/tabs/DiagnosticTab";
import { PublishTab } from "@/tabs/PublishTab";
import {
    api,
    DEBUG_PAUSED_EVENT,
    RUN_EVENT,
    type PausePayload,
    type RunEvent,
} from "@/lib/api";
import { useStudio, type Tab } from "@/lib/store";

const TABS: { id: Tab; label: string }[] = [
    { id: "source", label: "Source" },
    { id: "fixtures", label: "Fixtures" },
    { id: "snapshot", label: "Snapshot" },
    { id: "diagnostic", label: "Diagnostic" },
    { id: "publish", label: "Publish" },
];

export function App() {
    const qc = useQueryClient();
    const {
        activeSlug,
        source,
        dirty,
        tab,
        running,
        setSource,
        setTab,
        setValidation,
        setSnapshot,
        setRunError,
        markClean,
        runBegin,
        runAppend,
        runFinish,
        debugPause,
    } = useStudio();

    // Load source when active recipe changes.
    useEffect(() => {
        let cancelled = false;
        if (!activeSlug) return;
        api.loadRecipe(activeSlug)
            .then((s) => {
                if (!cancelled) {
                    setSource(s);
                    markClean();
                    setValidation(null);
                    setSnapshot(null);
                    setRunError(null);
                }
            })
            .catch((e) => !cancelled && setRunError(String(e)));
        return () => {
            cancelled = true;
        };
    }, [activeSlug]);

    const save = async () => {
        if (!activeSlug) return;
        const v = await api.saveRecipe(activeSlug, source);
        setValidation(v);
        markClean();
    };

    const run = async (replay: boolean) => {
        if (!activeSlug) return;
        if (useStudio.getState().running) return;
        await save();
        setTab("snapshot");
        runBegin();
        try {
            const r = await api.runRecipe(activeSlug, replay);
            if (r.ok && r.snapshot) {
                setSnapshot(r.snapshot);
            } else {
                setRunError(r.error || "unknown error");
            }
        } catch (e) {
            setRunError(String(e));
        } finally {
            runFinish();
        }
    };

    const cancel = () => {
        if (!useStudio.getState().running) return;
        // If we're paused in the debugger, the cancellation has to go
        // through the debugger too — the engine task is awaiting on the
        // resume oneshot and won't see the cancel notify until it wakes
        // up. Sending Stop drops it out of the pause; then cancel_run
        // cleans up any post-pause work.
        if (useStudio.getState().paused) {
            api.debugResume("stop").catch((e) =>
                console.warn("debug stop failed", e),
            );
        }
        api.cancelRun().catch((e) => console.warn("cancel failed", e));
    };

    // Subscribe once to the engine progress event stream. The `cancelled`
    // flag is load-bearing under React.StrictMode — see the original
    // version's comment for the gory details.
    useEffect(() => {
        let cancelled = false;
        let un: (() => void) | undefined;
        listen<RunEvent>(RUN_EVENT, (e) => runAppend(e.payload)).then((u) => {
            if (cancelled) {
                u();
            } else {
                un = u;
            }
        });
        return () => {
            cancelled = true;
            un?.();
        };
    }, []);

    // Same StrictMode-safe pattern for the debug pause stream. Fires when
    // the engine has parked at a breakpoint or after Step Over; switch
    // to the Source tab so the user sees the debugger pane + the line
    // highlight. The pane inside SourceTab handles resume.
    useEffect(() => {
        let cancelled = false;
        let un: (() => void) | undefined;
        listen<PausePayload>(DEBUG_PAUSED_EVENT, (e) => {
            debugPause(e.payload);
            useStudio.getState().setTab("source");
        }).then((u) => {
            if (cancelled) {
                u();
            } else {
                un = u;
            }
        });
        return () => {
            cancelled = true;
            un?.();
        };
    }, []);

    // ⌘S → save, ⌘R → run live, ⇧⌘R → replay, ⌘N → new recipe.
    useEffect(() => {
        const handler = (e: KeyboardEvent) => {
            if (!(e.metaKey || e.ctrlKey)) return;
            if (e.key === "s") {
                e.preventDefault();
                save();
            } else if (e.key === "r" && !e.shiftKey) {
                e.preventDefault();
                run(false);
            } else if (e.key === "r" && e.shiftKey) {
                e.preventDefault();
                run(true);
            } else if (e.key === "n") {
                e.preventDefault();
                api.createRecipe().then((slug) => {
                    qc.invalidateQueries({ queryKey: ["recipes"] });
                    useStudio.getState().setActive(slug);
                });
            }
        };
        window.addEventListener("keydown", handler);
        return () => window.removeEventListener("keydown", handler);
    }, [activeSlug, source]);

    // Native menu events.
    useEffect(() => {
        const offs: (() => void)[] = [];
        listen("menu:new_recipe", async () => {
            const slug = await api.createRecipe();
            qc.invalidateQueries({ queryKey: ["recipes"] });
            useStudio.getState().setActive(slug);
        }).then((un) => offs.push(un));
        listen("menu:save", () => save()).then((un) => offs.push(un));
        listen("menu:run_live", () => run(false)).then((un) => offs.push(un));
        listen("menu:run_replay", () => run(true)).then((un) => offs.push(un));
        listen("menu:validate", () => save()).then((un) => offs.push(un));
        listen("menu:publish", () => useStudio.getState().setTab("publish")).then((un) =>
            offs.push(un),
        );
        return () => {
            offs.forEach((u) => u());
        };
    }, [activeSlug, source]);

    return (
        <SidebarProvider defaultOpen>
            <Sidebar />
            <SidebarInset className="min-h-0">
                <Toolbar
                    activeSlug={activeSlug}
                    dirty={dirty}
                    running={running}
                    onSave={save}
                    onReplay={() => run(true)}
                    onRunLive={() => run(false)}
                    onCancel={cancel}
                />
                <Tabs
                    value={tab}
                    onValueChange={(v) => setTab(v as Tab)}
                    className="flex-1 min-h-0 gap-0"
                >
                    <div className="border-b px-3 shrink-0">
                        <TabsList variant="line" className="h-10">
                            {TABS.map((t) => (
                                <TabsTrigger key={t.id} value={t.id}>
                                    {t.label}
                                </TabsTrigger>
                            ))}
                        </TabsList>
                    </div>
                    {TABS.map((t) => (
                        <TabsContent
                            key={t.id}
                            value={t.id}
                            className="flex-1 min-h-0 m-0 flex flex-col data-[state=inactive]:hidden"
                        >
                            {t.id === "source" && <SourceTab />}
                            {t.id === "fixtures" && <FixturesTab />}
                            {t.id === "snapshot" && <SnapshotTab />}
                            {t.id === "diagnostic" && <DiagnosticTab />}
                            {t.id === "publish" && <PublishTab />}
                        </TabsContent>
                    ))}
                </Tabs>
            </SidebarInset>
        </SidebarProvider>
    );
}

function Toolbar(props: {
    activeSlug: string | null;
    dirty: boolean;
    running: boolean;
    onSave: () => void;
    onReplay: () => void;
    onRunLive: () => void;
    onCancel: () => void;
}) {
    return (
        <header className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
            <SidebarTrigger />
            <Separator orientation="vertical" className="!h-4" />
            <span className="font-mono text-sm text-muted-foreground select-text">
                {props.activeSlug ?? "(no recipe)"}
            </span>
            {props.dirty && (
                <Badge variant="warning">
                    <span className="size-1.5 rounded-full bg-warning" />
                    unsaved
                </Badge>
            )}
            {props.running && <RunStatus />}
            <div className="ml-auto flex items-center gap-1">
                {props.running ? (
                    <Button variant="destructive" size="sm" onClick={props.onCancel}>
                        <Loader2 className="animate-spin" />
                        Cancel
                    </Button>
                ) : (
                    <>
                        <ToolbarButton
                            onClick={props.onSave}
                            disabled={!props.activeSlug}
                            label="Save"
                            shortcut={["⌘", "S"]}
                            icon={<Save />}
                            variant="ghost"
                        />
                        <ToolbarButton
                            onClick={props.onReplay}
                            disabled={!props.activeSlug}
                            label="Replay"
                            shortcut={["⇧", "⌘", "R"]}
                            icon={<RefreshCw />}
                            variant="ghost"
                        />
                        <ToolbarButton
                            onClick={props.onRunLive}
                            disabled={!props.activeSlug}
                            label="Run live"
                            shortcut={["⌘", "R"]}
                            icon={<Play />}
                            variant="default"
                        />
                    </>
                )}
            </div>
        </header>
    );
}

function ToolbarButton(props: {
    onClick: () => void;
    disabled?: boolean;
    label: string;
    shortcut: string[];
    icon: React.ReactNode;
    variant: "default" | "ghost";
}) {
    return (
        <Tooltip>
            <TooltipTrigger asChild>
                <Button
                    size="sm"
                    variant={props.variant}
                    onClick={props.onClick}
                    disabled={props.disabled}
                >
                    {props.icon}
                    {props.label}
                </Button>
            </TooltipTrigger>
            <TooltipContent>
                <div className="flex items-center gap-1">
                    {props.shortcut.map((k) => (
                        <Kbd key={k}>{k}</Kbd>
                    ))}
                </div>
            </TooltipContent>
        </Tooltip>
    );
}

function RunStatus() {
    const startedAt = useStudio((s) => s.runStartedAt);
    const paused = useStudio((s) => s.paused);
    const [now, setNow] = useState(Date.now());
    useEffect(() => {
        const id = window.setInterval(() => setNow(Date.now()), 250);
        return () => window.clearInterval(id);
    }, []);
    if (!startedAt) return null;
    const seconds = Math.max(0, Math.floor((now - startedAt) / 1000));
    if (paused) {
        const label =
            paused.kind === "step"
                ? `step ${paused.step}`
                : `iter ${paused.iteration + 1}/${paused.total} of $${paused.variable}`;
        return (
            <Badge variant="warning" className="font-mono tabular-nums">
                <Pause />
                paused at {label}
            </Badge>
        );
    }
    return (
        <Badge variant="success" className="font-mono tabular-nums">
            <span className="size-1.5 rounded-full bg-success" />
            running {seconds}s
        </Badge>
    );
}
