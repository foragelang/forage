//! Notebook view — third Studio mode alongside editor and deployment.
//! The user composes a linear pipeline by stacking recipe stages
//! top-to-bottom; the chain runs through `notebook_run` and publishes
//! as a regular recipe through `notebook_save` + the existing publish
//! flow (per sub-plan 5's design commitment: notebooks ARE recipes).
//!
//! The view is intentionally minimal in this initial cut: a header
//! with the notebook name + run/publish buttons, a stage column, and
//! an inspector pane that renders the most recent run's snapshot. The
//! recipe-picker dialog and the publish flow live in sibling files
//! to keep this top-level layout readable.

import { ChevronDown, GripVertical, Plus, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { SidebarTrigger } from "@/components/ui/sidebar";
import { Separator } from "@/components/ui/separator";
import { cn } from "@/lib/utils";
import { useStudio, type NotebookStage } from "@/lib/store";

import { notebookPublishAction, notebookRunAction } from "./notebookActions";
import { StagePickerDialog } from "./StagePickerDialog";

export function NotebookView() {
    return (
        <div className="flex flex-1 min-h-0 min-w-0 flex-col overflow-hidden">
            <NotebookHeader />
            <div className="flex flex-1 min-h-0 min-w-0">
                <ScrollArea className="flex-1 min-h-0 min-w-0">
                    <div className="flex flex-col gap-3 p-6 max-w-3xl">
                        <NotebookStageList />
                        <AddStageRow />
                    </div>
                </ScrollArea>
                <Separator orientation="vertical" />
                <NotebookInspector />
            </div>
            <StagePickerDialog />
        </div>
    );
}

function NotebookHeader() {
    const name = useStudio((s) => s.notebook.name);
    const setName = useStudio((s) => s.setNotebookName);
    const running = useStudio((s) => s.notebook.running);
    const stageCount = useStudio((s) => s.notebook.stages.length);
    const runDisabled = running || stageCount === 0;
    return (
        <header className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
            <SidebarTrigger />
            <Separator orientation="vertical" className="!h-4" />
            <span className="font-mono text-sm text-muted-foreground italic">
                notebook
            </span>
            <span className="text-muted-foreground/60">/</span>
            <Input
                value={name}
                onChange={(e) => setName(e.target.value)}
                className="h-7 max-w-[240px] font-mono text-sm"
                aria-label="Notebook name"
            />
            <div className="ml-auto flex items-center gap-1">
                <Button
                    size="sm"
                    variant="ghost"
                    disabled={runDisabled}
                    onClick={() => void notebookRunAction()}
                    aria-label="Run notebook"
                >
                    Run
                </Button>
                <Button
                    size="sm"
                    variant="default"
                    disabled={stageCount === 0 || running}
                    onClick={() => void notebookPublishAction()}
                    aria-label="Publish notebook"
                >
                    Publish
                </Button>
            </div>
        </header>
    );
}

function NotebookStageList() {
    const stages = useStudio((s) => s.notebook.stages);
    if (stages.length === 0) {
        return (
            <div className="rounded-md border border-dashed p-6 text-center text-sm text-muted-foreground">
                Add a recipe stage to start composing. Each stage's output
                flows into the next stage's input.
            </div>
        );
    }
    return (
        <ol className="flex flex-col gap-2">
            {stages.map((stage, index) => (
                <StageRow
                    key={stage.id}
                    stage={stage}
                    index={index}
                    isLast={index === stages.length - 1}
                />
            ))}
        </ol>
    );
}

function StageRow({
    stage,
    index,
    isLast,
}: {
    stage: NotebookStage;
    index: number;
    isLast: boolean;
}) {
    const remove = useStudio((s) => s.removeNotebookStage);
    const move = useStudio((s) => s.moveNotebookStage);
    return (
        <>
            <li
                className={cn(
                    "group/stage flex items-center gap-2 rounded-md border bg-card p-3",
                )}
            >
                <span className="font-mono text-xs text-muted-foreground tabular-nums w-6 text-right">
                    {index + 1}.
                </span>
                <button
                    type="button"
                    onClick={() => move(index, index - 1)}
                    disabled={index === 0}
                    aria-label="Move stage up"
                    className="text-muted-foreground hover:text-foreground disabled:opacity-30"
                >
                    <GripVertical className="size-4" />
                </button>
                <div className="flex flex-1 min-w-0 flex-col">
                    <span className="font-mono text-sm truncate">
                        {stage.author ? `@${stage.author}/` : ""}
                        {stage.name}
                    </span>
                    <span className="text-[10px] text-muted-foreground">
                        {stage.author
                            ? "hub-pulled recipe"
                            : "workspace recipe"}
                    </span>
                </div>
                <button
                    type="button"
                    onClick={() => remove(index)}
                    aria-label="Remove stage"
                    className="text-muted-foreground hover:text-destructive"
                >
                    <Trash2 className="size-4" />
                </button>
            </li>
            {!isLast && (
                <li
                    aria-hidden
                    className="flex items-center justify-center pl-10 text-muted-foreground"
                >
                    <ChevronDown className="size-4" />
                </li>
            )}
        </>
    );
}

function AddStageRow() {
    const openPicker = useStudio((s) => s.openStagePicker);
    return (
        <button
            type="button"
            onClick={openPicker}
            className={cn(
                "flex items-center justify-center gap-2 rounded-md border border-dashed",
                "bg-card/40 p-3 text-sm text-muted-foreground hover:bg-card hover:text-foreground",
            )}
        >
            <Plus className="size-4" />
            Add stage
        </button>
    );
}

function NotebookInspector() {
    const snapshot = useStudio((s) => s.notebook.snapshot);
    const runError = useStudio((s) => s.notebook.runError);
    const running = useStudio((s) => s.notebook.running);
    return (
        <aside className="w-[360px] shrink-0 flex flex-col min-h-0">
            <div className="border-b px-3 py-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                Preview
            </div>
            <ScrollArea className="flex-1 min-h-0">
                <div className="p-3 text-sm">
                    {running && (
                        <p className="text-muted-foreground">Running…</p>
                    )}
                    {!running && runError && (
                        <pre className="whitespace-pre-wrap text-destructive font-mono text-xs">
                            {runError}
                        </pre>
                    )}
                    {!running && !runError && snapshot && (
                        <SnapshotSummary
                            counts={snapshot.records.reduce<Record<string, number>>(
                                (acc, r) => {
                                    acc[r.typeName] = (acc[r.typeName] ?? 0) + 1;
                                    return acc;
                                },
                                {},
                            )}
                        />
                    )}
                    {!running && !runError && !snapshot && (
                        <p className="text-muted-foreground">
                            Run the notebook to see the chain's output.
                        </p>
                    )}
                </div>
            </ScrollArea>
        </aside>
    );
}

function SnapshotSummary({ counts }: { counts: Record<string, number> }) {
    const entries = Object.entries(counts).sort();
    if (entries.length === 0) {
        return <p className="text-muted-foreground">No records emitted.</p>;
    }
    return (
        <div className="flex flex-col gap-1">
            <Label className="text-xs">Records by type</Label>
            <ul className="flex flex-col gap-0.5">
                {entries.map(([type, count]) => (
                    <li
                        key={type}
                        className="flex items-baseline justify-between font-mono text-xs"
                    >
                        <span>{type}</span>
                        <span className="text-muted-foreground tabular-nums">
                            {count}
                        </span>
                    </li>
                ))}
            </ul>
        </div>
    );
}

