import {
    AlertOctagon,
    AlertTriangle,
    CheckCircle2,
    PauseCircle,
    XCircle,
} from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useStudio } from "@/lib/store";

export function DiagnosticTab() {
    const { snapshot, runError } = useStudio();
    const d = snapshot?.diagnostic;

    if (runError) {
        return (
            <ScrollArea className="flex-1">
                <div className="p-6 max-w-3xl">
                    <Alert variant="destructive">
                        <XCircle />
                        <AlertTitle>Run errored before producing a diagnostic</AlertTitle>
                        <AlertDescription>
                            <pre className="mt-2 whitespace-pre-wrap font-mono text-xs select-text">
                                {runError}
                            </pre>
                        </AlertDescription>
                    </Alert>
                </div>
            </ScrollArea>
        );
    }

    if (!snapshot || !d) {
        return (
            <div className="flex-1 flex items-center justify-center p-6">
                <div className="text-sm text-muted-foreground">
                    Diagnostic appears here after a run.
                </div>
            </div>
        );
    }

    const anything =
        d.stall_reason ||
        (d.unmet_expectations && d.unmet_expectations.length > 0) ||
        (d.unfired_capture_rules && d.unfired_capture_rules.length > 0) ||
        (d.unmatched_captures && d.unmatched_captures.length > 0) ||
        (d.unhandled_affordances && d.unhandled_affordances.length > 0);

    if (!anything) {
        return (
            <ScrollArea className="flex-1">
                <div className="p-6 max-w-3xl">
                    <Alert variant="success">
                        <CheckCircle2 />
                        <AlertTitle>Clean run</AlertTitle>
                        <AlertDescription>
                            No diagnostic items — every step ran as expected.
                        </AlertDescription>
                    </Alert>
                </div>
            </ScrollArea>
        );
    }

    return (
        <ScrollArea className="flex-1">
            <div className="p-6 max-w-3xl space-y-4">
                {d.stall_reason && (
                    <Alert variant="warning">
                        <PauseCircle />
                        <AlertTitle>Stall reason</AlertTitle>
                        <AlertDescription>
                            <pre className="mt-2 whitespace-pre-wrap font-mono text-xs select-text">
                                {d.stall_reason}
                            </pre>
                        </AlertDescription>
                    </Alert>
                )}
                <DiagnosticSection
                    title="Unmet expectations"
                    items={d.unmet_expectations}
                    variant="destructive"
                    icon={<AlertOctagon />}
                />
                <DiagnosticSection
                    title="Unfired capture rules"
                    items={d.unfired_capture_rules}
                    variant="warning"
                    icon={<AlertTriangle />}
                />
                <DiagnosticSection
                    title="Unmatched captures"
                    items={d.unmatched_captures}
                    variant="warning"
                    icon={<AlertTriangle />}
                />
                <DiagnosticSection
                    title="Unhandled affordances"
                    items={d.unhandled_affordances}
                    variant="warning"
                    icon={<AlertTriangle />}
                />
            </div>
        </ScrollArea>
    );
}

function DiagnosticSection(props: {
    title: string;
    items: string[] | undefined;
    variant: "destructive" | "warning";
    icon: React.ReactNode;
}) {
    if (!props.items || props.items.length === 0) return null;
    return (
        <Alert variant={props.variant}>
            {props.icon}
            <AlertTitle>{props.title}</AlertTitle>
            <AlertDescription>
                <ul className="mt-2 space-y-1 text-xs font-mono select-text">
                    {props.items.map((m, i) => (
                        <li key={i} className="pl-3 border-l-2 border-current/40">
                            {m}
                        </li>
                    ))}
                </ul>
            </AlertDescription>
        </Alert>
    );
}
