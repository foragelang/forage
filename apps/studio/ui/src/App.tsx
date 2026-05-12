import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import Editor from "@monaco-editor/react";

import { useQuery, useQueryClient } from "@tanstack/react-query";

type RecipeEntry = {
    slug: string;
    path: string;
    has_fixtures: boolean;
};

type ValidationOutcome = {
    ok: boolean;
    errors: string[];
    warnings: string[];
};

type RunOutcome = {
    ok: boolean;
    snapshot?: unknown;
    error?: string;
};

export function App() {
    const qc = useQueryClient();
    const [active, setActive] = useState<string | null>(null);
    const [source, setSource] = useState<string>("");
    const [validation, setValidation] = useState<ValidationOutcome | null>(null);
    const [runResult, setRunResult] = useState<RunOutcome | null>(null);

    const recipes = useQuery({
        queryKey: ["recipes"],
        queryFn: () => invoke<RecipeEntry[]>("list_recipes"),
        staleTime: 5_000,
    });

    useEffect(() => {
        if (!active && recipes.data && recipes.data.length > 0) {
            setActive(recipes.data[0].slug);
        }
    }, [active, recipes.data]);

    useEffect(() => {
        if (!active) return;
        invoke<string>("load_recipe", { slug: active })
            .then((s) => setSource(s))
            .catch((e) => setSource(`// load_recipe failed: ${e}\n`));
        setValidation(null);
        setRunResult(null);
    }, [active]);

    const saveAndValidate = async () => {
        if (!active) return;
        try {
            const v = await invoke<ValidationOutcome>("save_recipe", {
                slug: active,
                source,
            });
            setValidation(v);
        } catch (e) {
            setValidation({ ok: false, errors: [String(e)], warnings: [] });
        }
    };

    const run = async (replay: boolean) => {
        if (!active) return;
        await saveAndValidate();
        try {
            const r = await invoke<RunOutcome>("run_recipe", { slug: active, replay });
            setRunResult(r);
        } catch (e) {
            setRunResult({ ok: false, error: String(e) });
        }
    };

    const newRecipe = async () => {
        try {
            const slug = await invoke<string>("create_recipe");
            await qc.invalidateQueries({ queryKey: ["recipes"] });
            setActive(slug);
        } catch (e) {
            console.error(e);
        }
    };

    return (
        <div className="grid grid-cols-[280px_1fr] h-screen">
            <aside className="border-r border-zinc-800 flex flex-col">
                <header className="px-4 py-3 border-b border-zinc-800 flex items-center justify-between">
                    <span className="font-semibold">Forage Studio</span>
                    <button
                        onClick={newRecipe}
                        className="px-2 py-1 text-xs bg-emerald-700 hover:bg-emerald-600 rounded"
                    >
                        + New
                    </button>
                </header>
                <ul className="flex-1 overflow-y-auto">
                    {(recipes.data ?? []).map((r) => (
                        <li
                            key={r.slug}
                            onClick={() => setActive(r.slug)}
                            className={`px-4 py-2 cursor-pointer hover:bg-zinc-900 ${
                                active === r.slug ? "bg-zinc-800" : ""
                            }`}
                        >
                            <div className="text-sm">{r.slug}</div>
                            {r.has_fixtures && (
                                <div className="text-xs text-zinc-500">has fixtures</div>
                            )}
                        </li>
                    ))}
                </ul>
            </aside>
            <main className="flex flex-col">
                <div className="px-4 py-2 border-b border-zinc-800 flex items-center gap-2">
                    <span className="text-sm text-zinc-400">{active ?? "(no recipe)"}</span>
                    <div className="ml-auto flex gap-2">
                        <button
                            onClick={saveAndValidate}
                            className="px-3 py-1 text-sm bg-zinc-800 hover:bg-zinc-700 rounded"
                        >
                            Save (⌘S)
                        </button>
                        <button
                            onClick={() => run(true)}
                            className="px-3 py-1 text-sm bg-zinc-800 hover:bg-zinc-700 rounded"
                        >
                            Replay
                        </button>
                        <button
                            onClick={() => run(false)}
                            className="px-3 py-1 text-sm bg-emerald-700 hover:bg-emerald-600 rounded"
                        >
                            Run live
                        </button>
                    </div>
                </div>
                <div className="flex-1 min-h-0">
                    <Editor
                        height="100%"
                        defaultLanguage="plaintext"
                        theme="vs-dark"
                        value={source}
                        onChange={(v) => setSource(v ?? "")}
                        options={{
                            fontSize: 13,
                            tabSize: 4,
                            minimap: { enabled: false },
                            wordWrap: "on",
                        }}
                    />
                </div>
                <footer className="border-t border-zinc-800 px-4 py-2 text-xs text-zinc-400 max-h-48 overflow-y-auto">
                    {validation && (
                        <div className="mb-1">
                            {validation.ok ? (
                                <span className="text-emerald-400">✓ validates</span>
                            ) : (
                                <ul className="text-red-400">
                                    {validation.errors.map((e, i) => (
                                        <li key={i}>{e}</li>
                                    ))}
                                </ul>
                            )}
                            {validation.warnings.map((w, i) => (
                                <div key={`w${i}`} className="text-amber-400">
                                    {w}
                                </div>
                            ))}
                        </div>
                    )}
                    {runResult && (
                        <pre className="whitespace-pre-wrap">
                            {runResult.error ??
                                JSON.stringify(runResult.snapshot, null, 2).slice(0, 4000)}
                        </pre>
                    )}
                </footer>
            </main>
        </div>
    );
}
