//! Phase 4 stub of the right-inspector. Segmented Tabs at the top
//! (This run / History / Records). The "This run" panel reuses the
//! existing SnapshotTab so the regression-test flow (run a recipe,
//! see records) keeps working. History and Records render a
//! placeholder pending Phase 5.
//!
//! Reactive-UI rule: leaf-reads from the store; Tabs.value is bound
//! to `inspectorMode`, write-back through `setInspectorMode`.

import {
    Tabs,
    TabsContent,
    TabsList,
    TabsTrigger,
} from "@/components/ui/tabs";
import { SnapshotTab } from "@/tabs/SnapshotTab";
import { useStudio, type InspectorMode } from "@/lib/store";

export function Inspector() {
    const inspectorMode = useStudio((s) => s.inspectorMode);
    const setInspectorMode = useStudio((s) => s.setInspectorMode);
    return (
        <aside className="w-[420px] shrink-0 border-l min-h-0 flex flex-col">
            <Tabs
                value={inspectorMode}
                onValueChange={(v) => setInspectorMode(v as InspectorMode)}
                className="flex-1 min-h-0 gap-0"
            >
                <div className="border-b px-3 shrink-0">
                    <TabsList variant="line" className="h-10">
                        <TabsTrigger value="run">This run</TabsTrigger>
                        <TabsTrigger value="history">History</TabsTrigger>
                        <TabsTrigger value="records">Records</TabsTrigger>
                    </TabsList>
                </div>
                <TabsContent
                    value="run"
                    className="flex-1 min-h-0 m-0 flex flex-col data-[state=inactive]:hidden"
                >
                    <SnapshotTab />
                </TabsContent>
                <TabsContent
                    value="history"
                    className="flex-1 min-h-0 m-0 flex flex-col data-[state=inactive]:hidden"
                >
                    <Placeholder label="History (Phase 5)" />
                </TabsContent>
                <TabsContent
                    value="records"
                    className="flex-1 min-h-0 m-0 flex flex-col data-[state=inactive]:hidden"
                >
                    <Placeholder label="Records (Phase 5)" />
                </TabsContent>
            </Tabs>
        </aside>
    );
}

function Placeholder({ label }: { label: string }) {
    return (
        <div className="flex-1 flex items-center justify-center p-6 text-sm text-muted-foreground">
            {label}
        </div>
    );
}
