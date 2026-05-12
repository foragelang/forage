import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";

import { Sidebar } from "./components/Sidebar";
import { SourceTab } from "./tabs/SourceTab";
import { FixturesTab } from "./tabs/FixturesTab";
import { SnapshotTab } from "./tabs/SnapshotTab";
import { DiagnosticTab } from "./tabs/DiagnosticTab";
import { PublishTab } from "./tabs/PublishTab";
import { api, RUN_EVENT, type RunEvent } from "./lib/api";
import { useStudio, type Tab } from "./lib/store";

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
        api.cancelRun().catch((e) => console.warn("cancel failed", e));
    };

    // Subscribe once to the engine progress event stream. The listener stays
    // active for the life of the app; events outside a run are ignored by
    // the store (runBegin clears the log before each run).
    //
    // The `cancelled` flag is load-bearing: React.StrictMode mounts the
    // effect, immediately unmounts it, then remounts. With an async
    // `.then(u => un = u)`, both cleanups run before either listen()
    // promise resolves — `un` is undefined in cleanup #1, so the first
    // listener never gets disposed. Result: two live listeners and every
    // event fires twice. Tracking the cancellation lets us dispose the
    // first handle the moment its promise resolves.
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
        <div className="grid grid-cols-[280px_1fr] h-screen text-zinc-200">
            <Sidebar />
            <main className="flex flex-col min-h-0">
                <Toolbar
                    activeSlug={activeSlug}
                    dirty={dirty}
                    running={running}
                    onSave={save}
                    onReplay={() => run(true)}
                    onRunLive={() => run(false)}
                    onCancel={cancel}
                />
                <Tabs current={tab} onSelect={setTab} />
                <div className="flex-1 flex flex-col min-h-0 bg-zinc-950">
                    {tab === "source" && <SourceTab />}
                    {tab === "fixtures" && <FixturesTab />}
                    {tab === "snapshot" && <SnapshotTab />}
                    {tab === "diagnostic" && <DiagnosticTab />}
                    {tab === "publish" && <PublishTab />}
                </div>
            </main>
        </div>
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
        <div className="px-4 h-12 border-b border-zinc-800 flex items-center gap-3 bg-zinc-950 flex-shrink-0">
            <span className="text-sm font-mono text-zinc-400">
                {props.activeSlug ?? "(no recipe)"}
            </span>
            {props.dirty && (
                <span className="text-xs text-amber-500">● unsaved</span>
            )}
            {props.running && <RunningPill />}
            <div className="ml-auto flex gap-2">
                {props.running ? (
                    <button
                        onClick={props.onCancel}
                        className="px-3 py-1.5 text-sm bg-red-700 hover:bg-red-600 rounded font-medium flex items-center gap-2"
                        title="Stop run"
                    >
                        <span className="inline-block w-3 h-3 rounded-full border-2 border-zinc-200 border-t-transparent animate-spin" />
                        Cancel
                    </button>
                ) : (
                    <>
                        <button
                            onClick={props.onSave}
                            disabled={!props.activeSlug}
                            className="px-3 py-1.5 text-sm bg-zinc-800 hover:bg-zinc-700 rounded disabled:opacity-50"
                            title="⌘S"
                        >
                            Save
                        </button>
                        <button
                            onClick={props.onReplay}
                            disabled={!props.activeSlug}
                            className="px-3 py-1.5 text-sm bg-zinc-800 hover:bg-zinc-700 rounded disabled:opacity-50"
                            title="⇧⌘R"
                        >
                            Replay
                        </button>
                        <button
                            onClick={props.onRunLive}
                            disabled={!props.activeSlug}
                            className="px-3 py-1.5 text-sm bg-emerald-700 hover:bg-emerald-600 rounded disabled:opacity-50 font-medium"
                            title="⌘R"
                        >
                            Run live
                        </button>
                    </>
                )}
            </div>
        </div>
    );
}

function RunningPill() {
    // Tick the elapsed-time display every 250ms so the user can see we're
    // still alive even when the engine is throttling between requests.
    const startedAt = useStudio((s) => s.runStartedAt);
    const [now, setNow] = useState(Date.now());
    useEffect(() => {
        const id = window.setInterval(() => setNow(Date.now()), 250);
        return () => window.clearInterval(id);
    }, []);
    if (!startedAt) return null;
    const seconds = Math.max(0, Math.floor((now - startedAt) / 1000));
    return (
        <span className="text-xs text-emerald-400 font-mono tabular-nums">
            ● running {seconds}s
        </span>
    );
}

function Tabs(props: { current: Tab; onSelect: (t: Tab) => void }) {
    return (
        <div className="border-b border-zinc-800 flex bg-zinc-950 flex-shrink-0">
            {TABS.map((t) => (
                <button
                    key={t.id}
                    onClick={() => props.onSelect(t.id)}
                    className={`px-4 py-2 text-sm border-b-2 transition-colors ${
                        props.current === t.id
                            ? "border-emerald-500 text-zinc-100"
                            : "border-transparent text-zinc-500 hover:text-zinc-300"
                    }`}
                >
                    {t.label}
                </button>
            ))}
        </div>
    );
}
