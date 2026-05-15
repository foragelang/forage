//! Publish dialog — runs after the user clicks "Publish notebook"
//! in the header. Collects the description / category / tags the
//! hub publish endpoint requires, shows a synthesized-source
//! preview, and on submit calls the unified save-then-publish flow.
//!
//! The dialog requires a signed-in author. Surfaces the device-flow
//! prompt when the user isn't authed — clicking through bounces to
//! the hub login (same shape as the editor's hypothetical publish
//! dialog would use).

import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import {
    Dialog,
    DialogContent,
    DialogDescription,
    DialogFooter,
    DialogHeader,
    DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { useStudioService } from "@/lib/services";
import { useStudio } from "@/lib/store";

import { commitNotebookPublish } from "./notebookActions";

export function NotebookPublishDialog() {
    const open = useStudio((s) => s.notebook.publishDialogOpen);
    const close = useStudio((s) => s.closePublishDialog);
    return (
        <Dialog
            open={open}
            onOpenChange={(next) => {
                if (!next) close();
            }}
        >
            <DialogContent className="max-w-xl">
                <DialogHeader>
                    <DialogTitle>Publish notebook</DialogTitle>
                    <DialogDescription>
                        Save the notebook as a workspace recipe and publish
                        it to the hub. The recipe's body is a composition
                        chain — it shows up under @author/{useStudio.getState().notebook.name}.
                    </DialogDescription>
                </DialogHeader>
                <PublishDialogBody />
            </DialogContent>
        </Dialog>
    );
}

function PublishDialogBody() {
    const service = useStudioService();
    const name = useStudio((s) => s.notebook.name);
    const stages = useStudio((s) => s.notebook.stages);
    const close = useStudio((s) => s.closePublishDialog);

    const whoami = useQuery({
        queryKey: ["auth.whoami"],
        queryFn: () => service.authWhoami(),
    });

    const [description, setDescription] = useState("");
    const [category, setCategory] = useState("");
    const [tags, setTags] = useState("");
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [preview, setPreview] = useState<string>("");

    // Preview the synthesized recipe so the user sees what's about
    // to land on disk before committing. Re-renders whenever the
    // stage list or name change.
    useEffect(() => {
        let cancelled = false;
        service
            .composeNotebookSource(
                name,
                stages.map((s) => s.name),
            )
            .then((src) => {
                if (!cancelled) setPreview(src);
            })
            .catch(() => {
                /* preview is best-effort */
            });
        return () => {
            cancelled = true;
        };
    }, [service, name, stages]);

    const submit = async () => {
        if (!whoami.data) {
            setError("Sign in to the hub before publishing.");
            return;
        }
        setBusy(true);
        setError(null);
        try {
            const result = await commitNotebookPublish({
                author: whoami.data,
                description: description.trim(),
                category: category.trim(),
                tags: tags
                    .split(",")
                    .map((t) => t.trim())
                    .filter((t) => t.length > 0),
            });
            if (result.published) {
                close();
            } else {
                setError(result.error ?? "publish failed");
            }
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="flex flex-col gap-3">
            <div className="grid grid-cols-3 gap-2">
                <div className="col-span-2 flex flex-col gap-1">
                    <Label htmlFor="publish-description" className="text-xs">
                        Description
                    </Label>
                    <Input
                        id="publish-description"
                        value={description}
                        onChange={(e) => setDescription(e.target.value)}
                        placeholder="What this composition does"
                    />
                </div>
                <div className="flex flex-col gap-1">
                    <Label htmlFor="publish-category" className="text-xs">
                        Category
                    </Label>
                    <Input
                        id="publish-category"
                        value={category}
                        onChange={(e) => setCategory(e.target.value)}
                        placeholder="data-pipelines"
                    />
                </div>
            </div>
            <div className="flex flex-col gap-1">
                <Label htmlFor="publish-tags" className="text-xs">
                    Tags
                </Label>
                <Input
                    id="publish-tags"
                    value={tags}
                    onChange={(e) => setTags(e.target.value)}
                    placeholder="comma, separated, tags"
                />
            </div>
            <Separator />
            <div className="flex flex-col gap-1">
                <Label className="text-xs">Source preview</Label>
                <ScrollArea className="max-h-[200px] rounded-md border bg-muted/40 p-2">
                    <pre className="font-mono text-[11px] whitespace-pre">
                        {preview}
                    </pre>
                </ScrollArea>
            </div>
            {error && (
                <p className="text-xs text-destructive font-mono whitespace-pre-wrap">
                    {error}
                </p>
            )}
            <DialogFooter>
                <Button variant="ghost" onClick={close} disabled={busy}>
                    Cancel
                </Button>
                <Button
                    variant="default"
                    onClick={() => void submit()}
                    disabled={
                        busy ||
                        stages.length === 0 ||
                        !description.trim() ||
                        !category.trim() ||
                        whoami.isLoading
                    }
                >
                    {busy
                        ? "Publishing…"
                        : whoami.data
                          ? `Publish as @${whoami.data}`
                          : "Sign in to publish"}
                </Button>
            </DialogFooter>
        </div>
    );
}
