//! Stage picker — opens from the notebook's "Add stage" button.
//! This commit lands the dialog scaffold + workspace-recipe listing;
//! commit 2 extends it with type-shaped hub discovery.

import { useMemo, useState } from "react";

import {
    Dialog,
    DialogContent,
    DialogDescription,
    DialogHeader,
    DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useRecipes } from "@/hooks/useRecipes";
import { useStudio } from "@/lib/store";
import { cn } from "@/lib/utils";

export function StagePickerDialog() {
    const open = useStudio((s) => s.notebook.stagePickerOpen);
    const closePicker = useStudio((s) => s.closeStagePicker);
    return (
        <Dialog
            open={open}
            onOpenChange={(next) => {
                if (!next) closePicker();
            }}
        >
            <DialogContent className="max-w-2xl">
                <DialogHeader>
                    <DialogTitle>Add a stage</DialogTitle>
                    <DialogDescription>
                        Pick a recipe to append to the notebook chain. Its
                        output feeds the next stage's input.
                    </DialogDescription>
                </DialogHeader>
                <StagePickerBody />
            </DialogContent>
        </Dialog>
    );
}

function StagePickerBody() {
    const recipes = useRecipes();
    const [filter, setFilter] = useState("");
    const addStage = useStudio((s) => s.addNotebookStage);
    const closePicker = useStudio((s) => s.closeStagePicker);
    const workspaceRecipes = useMemo(() => {
        const all = recipes.data ?? [];
        const q = filter.trim().toLowerCase();
        return all.filter((r) => r.draft.kind === "valid").filter((r) =>
            q.length === 0 ? true : r.name.toLowerCase().includes(q),
        );
    }, [recipes.data, filter]);
    return (
        <div className="flex flex-col gap-3 min-h-0">
            <Input
                placeholder="Filter by name…"
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                autoFocus
            />
            <ScrollArea className="max-h-[360px]">
                <ul className="flex flex-col gap-1">
                    {workspaceRecipes.length === 0 && (
                        <li className="px-2 py-6 text-center text-sm text-muted-foreground">
                            No recipes match.
                        </li>
                    )}
                    {workspaceRecipes.map((r) => (
                        <li key={r.name}>
                            <button
                                type="button"
                                onClick={() => {
                                    addStage(r.name, null);
                                    closePicker();
                                }}
                                className={cn(
                                    "flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left",
                                    "hover:bg-accent hover:text-accent-foreground",
                                )}
                            >
                                <span className="font-mono text-sm flex-1 truncate">
                                    {r.name}
                                </span>
                                <span className="text-[10px] text-muted-foreground">
                                    workspace
                                </span>
                            </button>
                        </li>
                    ))}
                </ul>
            </ScrollArea>
        </div>
    );
}
