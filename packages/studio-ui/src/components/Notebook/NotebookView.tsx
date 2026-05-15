//! Notebook view — third Studio mode alongside editor and deployment.
//! The user composes a linear pipeline by stacking recipe stages
//! top-to-bottom; the chain runs through `notebook_run` and publishes
//! as a regular recipe through `notebook_save` + the existing publish
//! flow. Notebooks ARE recipes; there is no separate notebook citizen.
//!
//! The view is intentionally minimal in this initial cut: a header
//! with the notebook name + run/publish buttons, a stage column, and
//! an inspector pane that renders the most recent run's snapshot. The
//! recipe-picker dialog and the publish flow live in sibling files
//! to keep this top-level layout readable.

import { ChevronDown, GripVertical, Plus, Settings, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
    Popover,
    PopoverContent,
    PopoverTrigger,
} from "@/components/ui/popover";
import { ScrollArea } from "@/components/ui/scroll-area";
import { SidebarTrigger } from "@/components/ui/sidebar";
import { Separator } from "@/components/ui/separator";
import { cn } from "@/lib/utils";
import { useStudio, type NotebookStage } from "@/lib/store";

import { notebookPublishAction, notebookRunAction } from "./notebookActions";
import { NotebookPublishDialog } from "./NotebookPublishDialog";
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
            <NotebookPublishDialog />
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
                <NotebookRunFlagsPopover disabled={runDisabled} />
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

/// Run-flags popover for the notebook header. The notebook shares
/// the editor's `runFlags` slot — there's a single "Run" preset
/// across views — but its backend always forces ephemeral on, so
/// the persisted-output toggle is a no-op here. Surfacing it anyway
/// keeps the chip's vocabulary consistent with the editor toolbar.
function NotebookRunFlagsPopover({ disabled }: { disabled: boolean }) {
    const flags = useStudio((s) => s.runFlags);
    const setRunFlags = useStudio((s) => s.setRunFlags);
    const preset = describePreset(flags);
    return (
        <Popover>
            <PopoverTrigger asChild>
                <Button
                    size="sm"
                    variant="ghost"
                    disabled={disabled}
                    aria-label="Run flags"
                    className="gap-1.5"
                >
                    <Settings className="size-3.5" />
                    <span className="text-xs font-mono">{preset}</span>
                    <ChevronDown className="size-3" />
                </Button>
            </PopoverTrigger>
            <PopoverContent align="end" className="w-72 space-y-3">
                <div>
                    <Label className="text-xs">Preset</Label>
                    <div className="mt-1 flex gap-1">
                        <Button
                            size="sm"
                            variant={preset === "dev" ? "default" : "outline"}
                            onClick={() =>
                                setRunFlags({
                                    sample_limit: 10,
                                    replay: true,
                                    ephemeral: true,
                                })
                            }
                            className="flex-1"
                        >
                            dev
                        </Button>
                        <Button
                            size="sm"
                            variant={preset === "prod" ? "default" : "outline"}
                            onClick={() =>
                                setRunFlags({
                                    sample_limit: null,
                                    replay: false,
                                    ephemeral: false,
                                })
                            }
                            className="flex-1"
                        >
                            prod
                        </Button>
                    </div>
                </div>
                <Separator />
                <div className="space-y-2">
                    <div className="flex items-center justify-between gap-2">
                        <div className="flex flex-col">
                            <Label htmlFor="notebook-sample" className="text-xs">
                                Sample limit
                            </Label>
                            <span className="text-[10px] text-muted-foreground">
                                Caps each stage's top-level for-loop.
                            </span>
                        </div>
                        <Input
                            id="notebook-sample"
                            type="number"
                            min={1}
                            value={flags.sample_limit ?? ""}
                            placeholder="off"
                            onChange={(e) => {
                                const raw = e.target.value;
                                if (raw === "") {
                                    setRunFlags({ sample_limit: null });
                                    return;
                                }
                                const n = Number.parseInt(raw, 10);
                                if (Number.isNaN(n) || n < 1) return;
                                setRunFlags({ sample_limit: n });
                            }}
                            className="w-20 h-7 text-xs"
                        />
                    </div>
                    <NotebookFlagToggle
                        id="notebook-replay"
                        label="Replay"
                        hint="Replay against stage 1's _fixtures/<recipe>.jsonl."
                        checked={flags.replay}
                        onChange={(v) => setRunFlags({ replay: v })}
                    />
                </div>
            </PopoverContent>
        </Popover>
    );
}

function NotebookFlagToggle(props: {
    id: string;
    label: string;
    hint: string;
    checked: boolean;
    onChange: (v: boolean) => void;
}) {
    return (
        <div className="flex items-center justify-between gap-2">
            <div className="flex flex-col">
                <Label htmlFor={props.id} className="text-xs">
                    {props.label}
                </Label>
                <span className="text-[10px] text-muted-foreground">
                    {props.hint}
                </span>
            </div>
            <button
                id={props.id}
                type="button"
                role="switch"
                aria-checked={props.checked}
                onClick={() => props.onChange(!props.checked)}
                className={cn(
                    "relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center",
                    "rounded-full transition-colors",
                    props.checked ? "bg-primary" : "bg-muted",
                )}
            >
                <span
                    className={cn(
                        "inline-block size-4 transform rounded-full bg-background",
                        "transition-transform",
                        props.checked ? "translate-x-4" : "translate-x-0.5",
                    )}
                />
            </button>
        </div>
    );
}

function describePreset(flags: {
    sample_limit: number | null;
    replay: boolean;
    ephemeral: boolean;
}): "dev" | "prod" | "custom" {
    if (flags.sample_limit === 10 && flags.replay && flags.ephemeral) {
        return "dev";
    }
    if (flags.sample_limit === null && !flags.replay && !flags.ephemeral) {
        return "prod";
    }
    return "custom";
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

