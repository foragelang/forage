import { useMutation, useQuery } from "@tanstack/react-query";
import { useState } from "react";

import { api } from "../lib/api";
import { useStudio } from "../lib/store";

export function PublishTab() {
    const { activeSlug } = useStudio();
    const [hubUrl, setHubUrl] = useState("https://api.foragelang.com");
    const [showSignIn, setShowSignIn] = useState(false);
    const whoami = useQuery({
        queryKey: ["whoami", hubUrl],
        queryFn: () => api.authWhoami(hubUrl),
        staleTime: 30_000,
    });

    const publish = useMutation({
        mutationFn: ({ dryRun }: { dryRun: boolean }) =>
            api.publishRecipe(activeSlug!, hubUrl, dryRun),
    });

    return (
        <div className="p-6 overflow-y-auto max-w-2xl">
            <h3 className="text-sm font-semibold text-zinc-200 mb-3">Publish</h3>

            <div className="space-y-4 text-sm">
                <Field label="Hub URL">
                    <input
                        type="text"
                        value={hubUrl}
                        onChange={(e) => setHubUrl(e.target.value)}
                        className="w-full bg-zinc-900 border border-zinc-700 rounded px-3 py-1.5 text-sm font-mono"
                    />
                </Field>
                <Field label="Signed in as">
                    {whoami.data ? (
                        <div className="flex items-center gap-2">
                            <span className="text-emerald-400 text-sm">{whoami.data}</span>
                            <button
                                onClick={async () => {
                                    await api.authLogout(hubUrl);
                                    whoami.refetch();
                                }}
                                className="text-xs text-zinc-500 hover:text-zinc-300"
                            >
                                Sign out
                            </button>
                        </div>
                    ) : (
                        <button
                            onClick={() => setShowSignIn(true)}
                            className="px-3 py-1.5 text-sm bg-zinc-800 hover:bg-zinc-700 rounded"
                        >
                            Sign in with GitHub
                        </button>
                    )}
                </Field>
                <Field label="Slug">
                    <span className="font-mono text-zinc-300">{activeSlug}</span>
                </Field>
            </div>

            <div className="mt-6 flex gap-2">
                <button
                    disabled={!activeSlug || publish.isPending}
                    onClick={() => publish.mutate({ dryRun: true })}
                    className="px-4 py-2 text-sm bg-zinc-800 hover:bg-zinc-700 rounded disabled:opacity-50"
                >
                    Preview (dry-run)
                </button>
                <button
                    disabled={!activeSlug || !whoami.data || publish.isPending}
                    onClick={() => publish.mutate({ dryRun: false })}
                    className="px-4 py-2 text-sm bg-emerald-700 hover:bg-emerald-600 rounded disabled:opacity-50"
                >
                    Publish
                </button>
            </div>

            {publish.data && (
                <pre className="mt-6 bg-zinc-900 rounded p-3 text-xs whitespace-pre-wrap text-zinc-300">
                    {publish.data.error || JSON.stringify(publish.data, null, 2)}
                </pre>
            )}
            {publish.error && (
                <div className="mt-6 text-red-400 text-xs">
                    {String(publish.error)}
                </div>
            )}

            {showSignIn && (
                <SignInSheet
                    hubUrl={hubUrl}
                    onDone={() => {
                        setShowSignIn(false);
                        whoami.refetch();
                    }}
                />
            )}
        </div>
    );
}

function Field(props: { label: string; children: React.ReactNode }) {
    return (
        <div className="grid grid-cols-[140px_1fr] items-center gap-3">
            <label className="text-xs text-zinc-500 text-right">{props.label}</label>
            <div>{props.children}</div>
        </div>
    );
}

function SignInSheet(props: { hubUrl: string; onDone: () => void }) {
    const [stage, setStage] = useState<"idle" | "started" | "polling" | "ok" | "error">("idle");
    const [code, setCode] = useState("");
    const [url, setUrl] = useState("");
    const [err, setErr] = useState("");

    const begin = async () => {
        try {
            setStage("started");
            const s = await api.authStartDeviceFlow(props.hubUrl);
            setCode(s.user_code);
            setUrl(s.verification_url);
            setStage("polling");
            const start = Date.now();
            const deadline = start + s.expires_in * 1000;
            while (Date.now() < deadline) {
                await new Promise((r) => setTimeout(r, s.interval * 1000));
                const p = await api.authPollDevice(props.hubUrl, s.device_code);
                if (p.status === "ok") {
                    setStage("ok");
                    setTimeout(props.onDone, 800);
                    return;
                }
                if (p.status === "expired") {
                    setErr("Device code expired. Try again.");
                    setStage("error");
                    return;
                }
            }
            setErr("Timed out waiting for the browser confirmation.");
            setStage("error");
        } catch (e) {
            setErr(String(e));
            setStage("error");
        }
    };

    return (
        <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
            <div className="bg-zinc-950 border border-zinc-800 rounded-lg shadow-2xl w-[480px] p-6">
                <h2 className="text-lg font-semibold mb-4">Sign in to {props.hubUrl}</h2>
                {stage === "idle" && (
                    <div>
                        <p className="text-sm text-zinc-400 mb-4">
                            Sign in to publish recipes under your GitHub account.
                        </p>
                        <div className="flex gap-2">
                            <button
                                onClick={begin}
                                className="px-4 py-2 text-sm bg-emerald-700 hover:bg-emerald-600 rounded"
                            >
                                Start device-code flow
                            </button>
                            <button
                                onClick={props.onDone}
                                className="px-4 py-2 text-sm bg-zinc-800 hover:bg-zinc-700 rounded"
                            >
                                Cancel
                            </button>
                        </div>
                    </div>
                )}
                {(stage === "started" || stage === "polling") && (
                    <div>
                        <p className="text-sm text-zinc-400 mb-3">
                            1. Open <a href={url} target="_blank" rel="noreferrer" className="text-emerald-400 underline">{url || "the verification URL"}</a>
                        </p>
                        <p className="text-sm text-zinc-400 mb-2">2. Enter this code:</p>
                        <div className="text-2xl font-mono tracking-wider bg-zinc-900 rounded p-3 text-center mb-4 select-all">
                            {code || "…"}
                        </div>
                        <p className="text-xs text-zinc-500">Polling…</p>
                    </div>
                )}
                {stage === "ok" && (
                    <div className="text-emerald-400 text-sm">Signed in ✓</div>
                )}
                {stage === "error" && (
                    <div>
                        <div className="text-red-400 text-sm mb-3">{err}</div>
                        <button
                            onClick={() => setStage("idle")}
                            className="px-4 py-2 text-sm bg-zinc-800 hover:bg-zinc-700 rounded"
                        >
                            Try again
                        </button>
                    </div>
                )}
            </div>
        </div>
    );
}
