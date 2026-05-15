//! Stage picker — opens from the notebook's "Add stage" button.
//! Type-shaped search powers both tabs: workspace recipes carry their
//! `output` declaration directly from the parser; the hub tab queries
//! the typed-discover index for a fully-qualified type id
//! (`@author/Name`) and parses only the matched recipes to project
//! their signatures. The picker filters by output type so the user
//! finds "recipes that emit `@upstream/Product`" rather than scrolling
//! slug lists.

import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import {
    Dialog,
    DialogContent,
    DialogDescription,
    DialogHeader,
    DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
    useStudioService,
    type RecipeSignatureWire,
    type StudioService,
} from "@/lib/services";
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
                        Pick a recipe to append to the notebook chain. The
                        output-type filter narrows to recipes that emit a
                        given record type — pair with the prior stage's
                        output to find compatible downstreams.
                    </DialogDescription>
                </DialogHeader>
                <StagePickerBody />
            </DialogContent>
        </Dialog>
    );
}

function StagePickerBody() {
    const service = useStudioService();
    const stages = useStudio((s) => s.notebook.stages);
    const tailName = stages.length > 0 ? stages[stages.length - 1]?.name : null;
    // Seed the type filter from the tail stage's output type. The
    // picker opens pre-filtered to "recipes that emit T" where T is
    // whatever the chain's last stage produces — the common move when
    // extending an existing chain. Workspace-side cache lookup
    // covers the local case; tail stages from the hub fall back to
    // an empty filter (the user types the type they want).
    const workspaceSigs = useQuery({
        queryKey: ["notebook.workspaceSignatures"],
        queryFn: () => service.listWorkspaceRecipeSignatures(),
        staleTime: 4_000,
    });
    const seedFilter = useMemo(() => {
        if (!tailName) return "";
        const sig = workspaceSigs.data?.find((s) => s.name === tailName);
        return sig?.outputs[0] ?? "";
    }, [tailName, workspaceSigs.data]);
    const [tab, setTab] = useState<"workspace" | "hub">("workspace");
    const [typeFilter, setTypeFilter] = useState(seedFilter);
    const [nameFilter, setNameFilter] = useState("");
    return (
        <div className="flex flex-col gap-3 min-h-0">
            <div className="grid grid-cols-2 gap-2">
                <div className="flex flex-col gap-1">
                    <Label htmlFor="type-filter" className="text-xs">
                        Output type
                    </Label>
                    <Input
                        id="type-filter"
                        placeholder="Product (workspace) or @author/Product (hub)"
                        value={typeFilter}
                        onChange={(e) => setTypeFilter(e.target.value)}
                        autoFocus
                    />
                </div>
                <div className="flex flex-col gap-1">
                    <Label htmlFor="name-filter" className="text-xs">
                        Name
                    </Label>
                    <Input
                        id="name-filter"
                        placeholder="scrape-amazon"
                        value={nameFilter}
                        onChange={(e) => setNameFilter(e.target.value)}
                    />
                </div>
            </div>
            <Tabs
                value={tab}
                onValueChange={(v) => setTab(v as "workspace" | "hub")}
            >
                <TabsList>
                    <TabsTrigger value="workspace">Workspace</TabsTrigger>
                    <TabsTrigger value="hub">Hub</TabsTrigger>
                </TabsList>
                <TabsContent value="workspace" className="mt-2">
                    <WorkspaceResults
                        typeFilter={typeFilter}
                        nameFilter={nameFilter}
                    />
                </TabsContent>
                <TabsContent value="hub" className="mt-2">
                    <HubResults
                        typeFilter={typeFilter}
                        nameFilter={nameFilter}
                    />
                </TabsContent>
            </Tabs>
        </div>
    );
}

function WorkspaceResults({
    typeFilter,
    nameFilter,
}: {
    typeFilter: string;
    nameFilter: string;
}) {
    const service = useStudioService();
    const sigs = useQuery({
        queryKey: ["notebook.workspaceSignatures"],
        queryFn: () => service.listWorkspaceRecipeSignatures(),
        staleTime: 4_000,
    });
    const filtered = useMemo(() => {
        const all = sigs.data ?? [];
        return filterSignatures(all, typeFilter, nameFilter);
    }, [sigs.data, typeFilter, nameFilter]);
    return (
        <ResultList
            entries={filtered.map((s) => ({
                key: `ws:${s.name}`,
                signature: s,
                author: null,
                sourceLabel: "workspace",
            }))}
            empty={
                sigs.isLoading
                    ? "Loading workspace recipes…"
                    : "No matching workspace recipes."
            }
        />
    );
}

function HubResults({
    typeFilter,
    nameFilter,
}: {
    typeFilter: string;
    nameFilter: string;
}) {
    const service = useStudioService();
    // The hub-side index is keyed by fully-qualified type id; bare
    // names ("Product") can't disambiguate between @alice/Product and
    // @bob/Product, so the hub tab waits for the user to specify the
    // author. The workspace tab continues to take bare names because
    // a workspace resolves them through its own import graph.
    const parsedTypeId = useMemo(() => parseTypeId(typeFilter), [typeFilter]);
    const sigs = useQuery({
        queryKey: [
            "notebook.hubSignatures",
            parsedTypeId?.author ?? null,
            parsedTypeId?.name ?? null,
            nameFilter,
        ],
        queryFn: () =>
            fetchHubSignatures(service, parsedTypeId!, nameFilter),
        staleTime: 60_000,
        enabled: service.capabilities.hubPackages && parsedTypeId !== null,
    });
    if (parsedTypeId === null) {
        return (
            <ResultList
                entries={[]}
                empty={
                    typeFilter.trim().length === 0
                        ? "Enter a fully-qualified type id (@author/Name) to search the hub."
                        : `Hub search needs a fully-qualified type id; got "${typeFilter}". Use @author/Name.`
                }
            />
        );
    }
    return (
        <ResultList
            entries={(sigs.data ?? []).map((s) => ({
                key: `hub:${s.author}/${s.signature.name}`,
                signature: s.signature,
                author: s.author,
                sourceLabel: `@${s.author} v${s.version}`,
            }))}
            empty={
                sigs.isLoading
                    ? "Searching hub…"
                    : sigs.isError
                      ? `Hub search failed: ${String(sigs.error)}`
                      : "No matching hub recipes."
            }
        />
    );
}

type ResultEntry = {
    key: string;
    signature: RecipeSignatureWire;
    author: string | null;
    sourceLabel: string;
};

function ResultList({
    entries,
    empty,
}: {
    entries: ResultEntry[];
    empty: string;
}) {
    const addStage = useStudio((s) => s.addNotebookStage);
    const closePicker = useStudio((s) => s.closeStagePicker);
    return (
        <ScrollArea className="max-h-[360px]">
            <ul className="flex flex-col gap-1">
                {entries.length === 0 && (
                    <li className="px-2 py-6 text-center text-sm text-muted-foreground">
                        {empty}
                    </li>
                )}
                {entries.map((entry) => (
                    <li key={entry.key}>
                        <button
                            type="button"
                            onClick={() => {
                                addStage(
                                    entry.signature.name,
                                    entry.author,
                                    entry.signature.outputs[0] ?? null,
                                );
                                closePicker();
                            }}
                            className={cn(
                                "flex w-full flex-col gap-0.5 rounded-sm px-2 py-1.5 text-left",
                                "hover:bg-accent hover:text-accent-foreground",
                            )}
                        >
                            <div className="flex items-baseline gap-2">
                                <span className="font-mono text-sm truncate">
                                    {entry.author
                                        ? `@${entry.author}/${entry.signature.name}`
                                        : entry.signature.name}
                                </span>
                                <span className="ml-auto text-[10px] text-muted-foreground">
                                    {entry.sourceLabel}
                                </span>
                            </div>
                            <div className="flex items-baseline gap-2 text-[11px] text-muted-foreground">
                                <span className="font-mono">
                                    in: {summariseInputs(entry.signature)}
                                </span>
                                <span className="font-mono">
                                    out: {summariseOutputs(entry.signature)}
                                </span>
                            </div>
                        </button>
                    </li>
                ))}
            </ul>
        </ScrollArea>
    );
}

function summariseInputs(sig: RecipeSignatureWire): string {
    if (sig.inputs.length === 0) return "()";
    return sig.inputs
        .map((i) => `${i.name}: ${i.ty}${i.optional ? "?" : ""}`)
        .join(", ");
}

function summariseOutputs(sig: RecipeSignatureWire): string {
    if (sig.outputs.length === 0) return "(none)";
    return sig.outputs.join(" | ");
}

function filterSignatures(
    all: RecipeSignatureWire[],
    typeFilter: string,
    nameFilter: string,
): RecipeSignatureWire[] {
    const t = typeFilter.trim();
    const n = nameFilter.trim().toLowerCase();
    return all.filter((s) => {
        if (t.length > 0 && !s.outputs.includes(t)) return false;
        if (n.length > 0 && !s.name.toLowerCase().includes(n)) return false;
        return true;
    });
}

/// Producers-of query: a single index call yields every recipe whose
/// latest version emits the given hub type. We then fetch each
/// matched package's latest version and parse its recipe source to
/// surface the same `RecipeSignatureWire` shape the workspace tab
/// uses — keeps the result rows visually identical between tabs.
/// `nameFilter` is applied client-side after parsing.
async function fetchHubSignatures(
    service: StudioService,
    typeId: { author: string; name: string },
    nameFilter: string,
): Promise<
    Array<{
        author: string;
        slug: string;
        version: number;
        signature: RecipeSignatureWire;
    }>
> {
    const listings = await service.discoverProducers(typeId.author, typeId.name);
    const enriched = await Promise.all(
        listings.map(async (listing) => {
            const version = await service.getPackageVersion(
                listing.author,
                listing.slug,
                "latest",
            );
            const sig = await service.parseRecipeSignature(version.recipe);
            if (sig === null) return null;
            return {
                author: listing.author,
                slug: listing.slug,
                version: version.version,
                signature: sig,
            };
        }),
    );
    const present = enriched.filter(
        (e): e is NonNullable<typeof e> => e !== null,
    );
    const n = nameFilter.trim().toLowerCase();
    if (n.length === 0) return present;
    return present.filter((e) => e.signature.name.toLowerCase().includes(n));
}

/// `@author/Name` (leading `@` optional). Returns null for shapes that
/// can't address the typed-discover index — bare names, missing
/// segments, malformed authors / type names. Validation is permissive
/// here; the hub-api rejects with a precise error if anything slips
/// through.
function parseTypeId(
    raw: string,
): { author: string; name: string } | null {
    const trimmed = raw.trim();
    if (trimmed.length === 0) return null;
    const stripped = trimmed.startsWith("@") ? trimmed.slice(1) : trimmed;
    const slash = stripped.indexOf("/");
    if (slash <= 0 || slash === stripped.length - 1) return null;
    const author = stripped.slice(0, slash);
    const name = stripped.slice(slash + 1);
    if (!/^[a-z0-9][a-z0-9-]*$/.test(author)) return null;
    if (!/^[A-Z][A-Za-z0-9]*$/.test(name)) return null;
    return { author, name };
}
