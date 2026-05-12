import { useQuery } from "@tanstack/react-query";
import { Database, FileJson, Info } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import {
    Card,
    CardContent,
    CardDescription,
    CardHeader,
    CardTitle,
} from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useStudio } from "@/lib/store";

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
            // No dedicated Tauri command yet — surface what we can.
            return { inputs: null, captureCount: 0, expectedSnapshot: false };
        },
    });

    return (
        <ScrollArea className="flex-1">
            <div className="p-6 max-w-3xl space-y-6">
                <div>
                    <h2 className="text-base font-heading font-medium mb-1">Fixtures</h2>
                    <p className="text-sm text-muted-foreground select-text">
                        Fixtures live in{" "}
                        <code className="text-foreground font-mono text-xs px-1 py-0.5 rounded bg-muted">
                            ~/Library/Forage/Recipes/{activeSlug ?? "<slug>"}/fixtures/
                        </code>
                        .
                    </p>
                </div>

                <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                    <Card size="sm">
                        <CardHeader>
                            <CardTitle className="flex items-center gap-2">
                                <FileJson className="size-4 text-muted-foreground" />
                                inputs.json
                            </CardTitle>
                            <CardDescription>
                                Consumer-supplied inputs for the recipe.
                            </CardDescription>
                        </CardHeader>
                        <CardContent>
                            <pre className="bg-muted/40 rounded p-3 text-xs font-mono whitespace-pre-wrap break-words select-text min-h-12">
                                {query.data?.inputs
                                    ? JSON.stringify(query.data.inputs, null, 2)
                                    : "—"}
                            </pre>
                        </CardContent>
                    </Card>

                    <Card size="sm">
                        <CardHeader>
                            <CardTitle className="flex items-center gap-2">
                                <Database className="size-4 text-muted-foreground" />
                                captures.jsonl
                            </CardTitle>
                            <CardDescription>
                                Recorded HTTP / browser captures used by replay mode.
                            </CardDescription>
                        </CardHeader>
                        <CardContent>
                            <div className="text-2xl font-mono tabular-nums">
                                {query.data?.captureCount ?? 0}
                                <span className="text-sm text-muted-foreground ml-1">
                                    captures
                                </span>
                            </div>
                        </CardContent>
                    </Card>
                </div>

                <Alert>
                    <Info />
                    <AlertTitle>Populating fixtures</AlertTitle>
                    <AlertDescription>
                        Run{" "}
                        <code className="font-mono text-xs px-1 py-0.5 rounded bg-muted text-foreground">
                            forage capture &lt;url&gt;
                        </code>{" "}
                        from the CLI (browser-engine recipes) or hand-edit{" "}
                        <code className="font-mono text-xs px-1 py-0.5 rounded bg-muted text-foreground">
                            captures.jsonl
                        </code>
                        . A dedicated Capture sheet inside Studio is on the roadmap.
                    </AlertDescription>
                </Alert>
            </div>
        </ScrollArea>
    );
}
