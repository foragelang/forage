//! Inline schedule editor — drops below the deployment header when
//! toggled. Three cadence cards (Manual / Interval / Cron) plus an
//! output path picker. Save calls `service.configureRun` and
//! invalidates the runs query so the header + sidebar pick up the
//! change.

import { useEffect, useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import cronstrue from "cronstrue";
import { Folder } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import type { Cadence } from "@/bindings/Cadence";
import type { Run } from "@/bindings/Run";
import type { RunConfig } from "@/bindings/RunConfig";
import type { TimeUnit } from "@/bindings/TimeUnit";
import { useStudioService } from "@/lib/services";
import { cn } from "@/lib/utils";

type Mode = "manual" | "interval" | "cron";

type Draft = {
    mode: Mode;
    everyN: number;
    unit: TimeUnit;
    cron: string;
    output: string;
    enabled: boolean;
};

function draftOf(run: Run): Draft {
    const c = run.cadence;
    if (c.kind === "manual") {
        return {
            mode: "manual",
            everyN: 6,
            unit: "h",
            cron: "",
            output: run.output,
            enabled: run.enabled,
        };
    }
    if (c.kind === "interval") {
        return {
            mode: "interval",
            everyN: c.every_n,
            unit: c.unit,
            cron: "",
            output: run.output,
            enabled: run.enabled,
        };
    }
    return {
        mode: "cron",
        everyN: 6,
        unit: "h",
        cron: c.expr,
        output: run.output,
        enabled: run.enabled,
    };
}

function cadenceFromDraft(d: Draft): Cadence {
    if (d.mode === "manual") return { kind: "manual" };
    if (d.mode === "interval") {
        return { kind: "interval", every_n: d.everyN, unit: d.unit };
    }
    return { kind: "cron", expr: d.cron };
}

export function ScheduleEditor({
    run,
    onClose,
}: {
    run: Run;
    onClose: () => void;
}) {
    const qc = useQueryClient();
    const service = useStudioService();
    const [draft, setDraft] = useState<Draft>(() => draftOf(run));

    // Reset draft when the underlying run changes (e.g. another tab
    // saved). Otherwise the editor would persist stale values from a
    // previous selection.
    useEffect(() => {
        setDraft(draftOf(run));
    }, [run]);

    const save = useMutation({
        mutationFn: async (cfg: RunConfig) =>
            service.configureRun(run.recipe_name, cfg),
        onSuccess: () => {
            qc.invalidateQueries({ queryKey: ["runs"] });
            onClose();
        },
    });

    // The daemon's `validate_cron` is the parser of record. Debounce the
    // call by 200ms so a typing user doesn't fire a backend command on
    // every keystroke. `cronstrue` is kept around purely as a humanizer
    // for the preview line — if it disagrees with the daemon
    // (different grammar) we still gate on the daemon.
    const [cronError, setCronError] = useState<string | null>(null);
    useEffect(() => {
        if (draft.mode !== "cron") {
            setCronError(null);
            return undefined;
        }
        if (!draft.cron.trim()) {
            setCronError("cron expression is required");
            return undefined;
        }
        let cancelled = false;
        const id = window.setTimeout(() => {
            service.validateCron(draft.cron).then(
                () => {
                    if (!cancelled) setCronError(null);
                },
                (e: unknown) => {
                    if (!cancelled) {
                        setCronError(
                            String(e instanceof Error ? e.message : e),
                        );
                    }
                },
            );
        }, 200);
        return () => {
            cancelled = true;
            window.clearTimeout(id);
        };
    }, [draft.mode, draft.cron, service]);

    const cronHumanized = useMemo(() => {
        if (draft.mode !== "cron" || !draft.cron) return null;
        try {
            return cronstrue.toString(draft.cron);
        } catch {
            return "(unparseable)";
        }
    }, [draft.mode, draft.cron]);

    const canSave =
        (draft.mode !== "cron" || (draft.cron.length > 0 && !cronError)) &&
        draft.output.length > 0 &&
        !save.isPending;

    return (
        <Card size="sm" className="m-3 gap-3 p-4 ring-0">
            <div className="grid grid-cols-1 gap-4">
                <CadenceSection draft={draft} setDraft={setDraft} cronError={cronError} />
                <OutputSection
                    output={draft.output}
                    onChange={(p) => setDraft({ ...draft, output: p })}
                />
            </div>

            <div className="flex items-center gap-3 border-t pt-3">
                <div className="text-xs text-muted-foreground font-mono">
                    <span className="mr-1">next:</span>
                    <span>{nextRunPreview(draft, cronHumanized)}</span>
                </div>
                <div className="ml-auto flex items-center gap-1.5">
                    <Button variant="ghost" size="sm" onClick={onClose}>
                        Cancel
                    </Button>
                    <Button
                        size="sm"
                        disabled={!canSave}
                        onClick={() =>
                            save.mutate({
                                cadence: cadenceFromDraft(draft),
                                output: draft.output,
                                enabled: draft.enabled,
                                inputs: run.inputs,
                            })
                        }
                    >
                        Save schedule
                    </Button>
                </div>
            </div>
            {save.error && (
                <div className="text-xs text-destructive">
                    Save failed: {String(save.error)}
                </div>
            )}
        </Card>
    );
}

// ── cadence section ──────────────────────────────────────────────────

function CadenceSection({
    draft,
    setDraft,
    cronError,
}: {
    draft: Draft;
    setDraft: (d: Draft) => void;
    cronError: string | null;
}) {
    return (
        <div className="space-y-2">
            <Label className="text-xs uppercase tracking-wider text-muted-foreground">
                Cadence
            </Label>
            <RadioGroup
                value={draft.mode}
                onValueChange={(v) => setDraft({ ...draft, mode: v as Mode })}
                className="grid grid-cols-1 gap-2"
            >
                <RadioCard
                    value="manual"
                    title="Manual only"
                    description="Triggered with Run now."
                    selected={draft.mode === "manual"}
                />
                <RadioCard
                    value="interval"
                    title="Every N"
                    description="Fixed interval after each run completes."
                    selected={draft.mode === "interval"}
                >
                    <div
                        className="flex items-center gap-2"
                        onClick={(e) => e.stopPropagation()}
                    >
                        <Input
                            type="number"
                            min={1}
                            max={999}
                            value={draft.everyN}
                            onChange={(e) =>
                                setDraft({
                                    ...draft,
                                    mode: "interval",
                                    everyN: Math.max(
                                        1,
                                        parseInt(e.target.value || "1", 10),
                                    ),
                                })
                            }
                            className="w-20 h-7"
                        />
                        <Select
                            value={draft.unit}
                            onValueChange={(v) =>
                                setDraft({
                                    ...draft,
                                    mode: "interval",
                                    unit: v as TimeUnit,
                                })
                            }
                        >
                            <SelectTrigger size="sm" className="w-28">
                                <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                                <SelectItem value="m">minutes</SelectItem>
                                <SelectItem value="h">hours</SelectItem>
                                <SelectItem value="d">days</SelectItem>
                            </SelectContent>
                        </Select>
                    </div>
                </RadioCard>
                <RadioCard
                    value="cron"
                    title="Cron"
                    description="Standard cron expression — five fields, UTC."
                    selected={draft.mode === "cron"}
                >
                    <div
                        className="flex flex-col gap-1"
                        onClick={(e) => e.stopPropagation()}
                    >
                        <Input
                            value={draft.cron}
                            placeholder="0 */6 * * *"
                            onChange={(e) =>
                                setDraft({ ...draft, mode: "cron", cron: e.target.value })
                            }
                            className="font-mono h-7"
                        />
                        {cronError && (
                            <span className="text-[11px] text-destructive">
                                {cronError}
                            </span>
                        )}
                    </div>
                </RadioCard>
            </RadioGroup>
        </div>
    );
}

function RadioCard({
    value,
    title,
    description,
    selected,
    children,
}: {
    value: string;
    title: string;
    description: string;
    selected: boolean;
    children?: React.ReactNode;
}) {
    return (
        <label
            className={cn(
                "flex items-start gap-3 rounded-md border p-3 cursor-pointer",
                "transition-colors hover:bg-muted/40",
                selected && "border-foreground/40 bg-muted/40",
            )}
        >
            <RadioGroupItem value={value} className="mt-0.5" />
            <div className="flex-1 space-y-1">
                <div className="text-sm font-medium">{title}</div>
                <div className="text-xs text-muted-foreground">{description}</div>
                {children}
            </div>
        </label>
    );
}

// ── output section ───────────────────────────────────────────────────

function OutputSection({
    output,
    onChange,
}: {
    output: string;
    onChange: (p: string) => void;
}) {
    const service = useStudioService();
    return (
        <div className="space-y-2">
            <Label className="text-xs uppercase tracking-wider text-muted-foreground">
                Output store
            </Label>
            <div className="flex items-center gap-2">
                <Input
                    value={output}
                    onChange={(e) => onChange(e.target.value)}
                    className="font-mono h-7"
                />
                <Button
                    variant="ghost"
                    size="sm"
                    onClick={async () => {
                        const folder = await service.pickDirectory(
                            "Pick output folder",
                        );
                        if (folder) {
                            // Replace the parent folder while keeping the file name.
                            const filename = output.split("/").pop() ?? "out.sqlite";
                            onChange(`${folder}/${filename}`);
                        }
                    }}
                >
                    <Folder />
                    Browse
                </Button>
            </div>
            <div className="text-[11px] text-muted-foreground">
                SQLite file. Tables are created on first run from the recipe&apos;s
                output catalog.
            </div>
        </div>
    );
}

// ── helpers ──────────────────────────────────────────────────────────

function nextRunPreview(d: Draft, human: string | null): string {
    if (d.mode === "manual") return "never (manual only)";
    if (d.mode === "interval") {
        return `every ${d.everyN}${d.unit} after each run completes`;
    }
    if (d.mode === "cron") {
        return human ?? "enter a valid cron expression";
    }
    return "";
}
