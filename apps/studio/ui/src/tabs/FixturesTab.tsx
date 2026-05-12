import { useQuery } from "@tanstack/react-query";

import { useStudio } from "../lib/store";

type FixtureInfo = {
    inputs: Record<string, unknown> | null;
    captureCount: number;
    expectedSnapshot: boolean;
};

export function FixturesTab() {
    const { activeSlug } = useStudio();
    const query = useQuery<FixtureInfo>({
        queryKey: ["fixtures", activeSlug],
        enabled: !!activeSlug,
        queryFn: async () => {
            // No dedicated Tauri command yet — peek via load + read.
            // We surface what we know from the sidebar `has_fixtures` flag.
            return { inputs: null, captureCount: 0, expectedSnapshot: false };
        },
    });

    return (
        <div className="p-6 overflow-y-auto">
            <h3 className="text-sm font-semibold text-zinc-200 mb-3">Fixtures</h3>
            <p className="text-xs text-zinc-400 mb-4 max-w-2xl">
                Fixtures live in <code className="text-zinc-200">~/Library/Forage/Recipes/{activeSlug ?? "&lt;slug&gt;"}/fixtures/</code>.
                Two files matter: <code className="text-zinc-200">inputs.json</code> for
                consumer-supplied inputs, and <code className="text-zinc-200">captures.jsonl</code>
                {" "}for recorded HTTP/browser captures used by replay mode.
            </p>
            <div className="space-y-3 text-xs">
                <div className="bg-zinc-900 rounded p-3">
                    <div className="text-zinc-400 mb-1">inputs.json</div>
                    <div className="text-zinc-200">
                        {query.data?.inputs
                            ? JSON.stringify(query.data.inputs, null, 2)
                            : "—"}
                    </div>
                </div>
                <div className="bg-zinc-900 rounded p-3">
                    <div className="text-zinc-400 mb-1">captures.jsonl</div>
                    <div className="text-zinc-200">
                        {query.data?.captureCount ?? 0} captures
                    </div>
                </div>
            </div>
            <p className="text-xs text-zinc-500 mt-6 max-w-2xl">
                To populate fixtures, run <code className="text-zinc-300">forage capture &lt;url&gt;</code>
                from the CLI (browser-engine recipes) or hand-edit{" "}
                <code className="text-zinc-300">captures.jsonl</code>. A dedicated
                Capture sheet inside Studio is on the roadmap.
            </p>
        </div>
    );
}
