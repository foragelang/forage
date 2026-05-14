//! Phase 4 stub of the editor toolbar — lifted from the inline
//! `Toolbar` that lived in `App.tsx`. Reads from the store leaf-by-
//! leaf; buttons dispatch through `studioActions`.
//!
//! Phase 5 adds the Runs chip / Configure-run button per
//! DESIGN_HANDOFF.md.

import { useEffect, useState } from "react";
import { Loader2, Pause, Play, RefreshCw, Save } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Separator } from "@/components/ui/separator";
import { SidebarTrigger } from "@/components/ui/sidebar";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { slugOf } from "@/lib/path";
import { useStudio } from "@/lib/store";
import { cancelActive, runActive, saveActive } from "@/lib/studioActions";

export function EditorToolbar() {
    const activeFilePath = useStudio((s) => s.activeFilePath);
    const dirty = useStudio((s) => s.dirty);
    const running = useStudio((s) => s.running);
    const crumb = activeFilePath
        ? (slugOf(activeFilePath) ?? activeFilePath)
        : "(no file)";
    const disabled = !activeFilePath;
    return (
        <header className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
            <SidebarTrigger />
            <Separator orientation="vertical" className="!h-4" />
            <span className="font-mono text-sm text-muted-foreground select-text">
                {crumb}
            </span>
            {dirty && (
                <Badge variant="warning">
                    <span className="size-1.5 rounded-full bg-warning" />
                    unsaved
                </Badge>
            )}
            {running && <RunStatus />}
            {/* Phase 5: Runs chip / Configure-run button. */}
            <div className="ml-auto flex items-center gap-1">
                {running ? (
                    <Button
                        variant="destructive"
                        size="sm"
                        onClick={cancelActive}
                    >
                        <Loader2 className="animate-spin" />
                        Cancel
                    </Button>
                ) : (
                    <>
                        <ToolbarButton
                            onClick={() => void saveActive()}
                            disabled={disabled}
                            label="Save"
                            shortcut={["⌘", "S"]}
                            icon={<Save />}
                            variant="ghost"
                        />
                        <ToolbarButton
                            onClick={() => void runActive(true)}
                            disabled={disabled}
                            label="Replay"
                            shortcut={["⇧", "⌘", "R"]}
                            icon={<RefreshCw />}
                            variant="ghost"
                        />
                        <ToolbarButton
                            onClick={() => void runActive(false)}
                            disabled={disabled}
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
